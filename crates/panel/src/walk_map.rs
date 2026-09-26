//! Application-owned WalkTo map renderer. One instance, not per bot.
//!
//! Terrain uses A's `select_lod` (≤24 textures of 258×258). Until a ReadyImages
//! handle is bound, the production map shows a grid and "map imagery unavailable — cache
//! not bound". Catalogue POIs and observed services draw as soon as they exist. Optional map-owned grid/collision/NSEW/reach/flood
//! layers composite into one viewport overlay. Route and destination are vector
//! markers. Close unregisters GPU textures and drops CPU pixels; late decode
//! results cannot resurrect a closed generation.

mod fixtures;
mod overlay;

use std::collections::HashSet;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::thread::JoinHandle;

use api::snapshot::WorldTile;
use dear_imgui_rs::{TextureId, Ui};
use nav::map::identity::{Digest, ImageIdentity};
use nav::map::poi::PoiKind;
use nav::map::spatial::{
    select_lod, snap_walkable, GameTile, TileKey, View, VisibleTiles, INTERIOR_UV, TEXTURE_CAP,
    TILE_PIXELS, TILE_RGBA_BYTES,
};
use nav::map::MapError;
use nav::tile::Tile;
use nav::world::NavWorld;

use host_play::map_cache::ReadyImages;
use host_play::walk_map::{Catalogue, ObservedService, Search};

use crate::game_view::FrameGpu;
use crate::nav_settings::{parse_html_color, NavSettings};
use crate::theme::{ACCENT, TEXT, TEXT_DIM};

#[cfg(test)]
pub use fixtures::encode_tile;
pub use fixtures::{decode_tile, Store as FixtureStore};
pub use overlay::{
    overlay_fit, rasterize as rasterize_overlay, OverlayColors, OverlayFit, OverlayLayers,
    OverlayPaint, MIN_CELL_PPT, NSEW_PPT, OVERLAY_BYTE_CAP, OVERLAY_MAX_H, OVERLAY_MAX_W,
};

pub const MAX_VTX_DEFAULT: i32 = 8_000;
pub const MAX_IDX_DEFAULT: i32 = 12_000;
pub const MAX_VTX_LAYERS: i32 = 16_000;
pub const MAX_IDX_LAYERS: i32 = 24_000;
pub const MAX_UPLOADS_PER_FRAME: usize = 2;
pub const MAX_SYMBOLS: usize = 256;
pub const MAX_LABELS: usize = 32;
const DECODE_LIMIT_BYTES: usize = 512 * 1024;
const DECODE_STAGING_CAP: usize = 1024 * 1024;
const MAX_ROUTE_SEGMENTS: usize = 512;
pub const CACHE_UNBOUND: &str = "map imagery unavailable — cache not bound";
pub const REACH_UNAVAILABLE: &str = "reach unavailable";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapPhase {
    Pending,
    Ready,
    Error,
}

#[derive(Debug, Clone)]
pub struct LiveCounters {
    pub generation: u64,
    pub phase: MapPhase,
    pub terrain_slots: usize,
    pub terrain_gpu_bytes: usize,
    pub terrain_cpu_bytes: usize,
    pub overlay_cpu_bytes: usize,
    pub overlay_gpu_bytes: usize,
    pub decode_staging_bytes: usize,
    pub in_flight: usize,
    pub pending_upload: usize,
    pub vtx: i32,
    pub idx: i32,
}

struct GpuTex {
    #[allow(dead_code)]
    texture: wgpu::Texture,
    #[allow(dead_code)]
    view: wgpu::TextureView,
    tex_id: TextureId,
}

struct TerrainSlot {
    key: TileKey,
    gpu: Option<GpuTex>,
}

struct OverlayGpu {
    texture: wgpu::Texture,
    #[allow(dead_code)]
    view: wgpu::TextureView,
    tex_id: TextureId,
    w: u32,
    h: u32,
}

struct DecodeJob {
    generation: u64,
    identity: ImageIdentity,
    key: TileKey,
    png: Vec<u8>,
}

struct DecodeDone {
    generation: u64,
    identity: ImageIdentity,
    key: TileKey,
    result: Result<Vec<u8>, String>,
}

enum WorkerCmd {
    Job(DecodeJob),
    Stop,
}

/// One map renderer owned by the panel app, shared across every bot slot.
pub struct WalkMapRenderer {
    generation: u64,
    open: bool,
    fixtures: fixtures::Store,
    ready: Option<Arc<ReadyImages>>,
    catalogue: Option<Arc<Catalogue>>,
    observed: Vec<ObservedService>,
    slots: Vec<TerrainSlot>,
    overlay_gpu: Option<OverlayGpu>,
    overlay_cpu: Option<Vec<u8>>,
    overlay_size: (u32, u32),
    overlay_key: Option<OverlayKey>,
    last_vis: Option<VisibleTiles>,
    pending_upload: Option<(u64, ImageIdentity, TileKey, Vec<u8>)>,
    in_flight: Option<(u64, TileKey)>,
    in_flight_bytes: usize,
    tex_pool: Vec<GpuTex>,
    compressed: Vec<u8>,
    worker_tx: Option<Sender<WorkerCmd>>,
    worker_rx: Option<Receiver<DecodeDone>>,
    worker: Option<JoinHandle<()>>,
    sync_decode: bool,
    pub show_basemap: bool,
    pub show_grid: bool,
    pub show_reach: bool,
    pub show_collision: bool,
    pub show_nsew: bool,
    pub show_flood: bool,
    nav_identity: Digest,
    world_geom: Option<(i32, i32, u32, u32)>,
    pub search: String,
    pub(crate) catalogue_search: Search,
    status: String,
    phase: MapPhase,
    last_vtx: i32,
    last_idx: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OverlayKey {
    west: i32,
    south: i32,
    east: i32,
    north: i32,
    plane: u8,
    w: u32,
    h: u32,
    layers: OverlayLayers,
    nav: Digest,
    origin_x: i32,
    origin_z: i32,
    width: u32,
    height: u32,
    flood_n: usize,
    flood_a: usize,
    flood_b: usize,
}

impl Default for WalkMapRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl WalkMapRenderer {
    pub fn new() -> Self {
        Self::with_sync(cfg!(test))
    }

    pub fn with_sync(sync_decode: bool) -> Self {
        Self {
            generation: 1,
            open: false,
            fixtures: fixtures::Store::empty(),
            ready: None,
            catalogue: None,
            observed: Vec::new(),
            slots: Vec::new(),
            overlay_gpu: None,
            overlay_cpu: None,
            overlay_size: (0, 0),
            overlay_key: None,
            last_vis: None,
            pending_upload: None,
            in_flight: None,
            in_flight_bytes: 0,
            tex_pool: Vec::new(),
            compressed: Vec::new(),
            worker_tx: None,
            worker_rx: None,
            worker: None,
            sync_decode,
            show_basemap: true,
            show_grid: false,
            show_reach: false,
            show_collision: false,
            show_nsew: false,
            show_flood: false,
            nav_identity: Digest([0; 32]),
            world_geom: None,
            search: String::new(),
            catalogue_search: Search::default(),
            status: String::from(CACHE_UNBOUND),
            phase: MapPhase::Pending,
            last_vtx: 0,
            last_idx: 0,
        }
    }

    pub fn fixtures(&self) -> &fixtures::Store {
        &self.fixtures
    }

    pub fn max_lod(&self) -> u8 {
        self.ready
            .as_ref()
            .map(|ready| ready.manifest().max_lod)
            .unwrap_or(self.fixtures.max_lod)
    }

    pub fn identity(&self) -> ImageIdentity {
        self.ready
            .as_ref()
            .map(|ready| ready.manifest().identity)
            .unwrap_or(self.fixtures.identity)
    }

    pub fn has_terrain(&self) -> bool {
        self.ready.is_some() || !self.fixtures.is_empty()
    }

    pub fn bind_ready_images(&mut self, images: Arc<ReadyImages>) {
        let identity = images.manifest().identity;
        let changed = self
            .ready
            .as_ref()
            .is_none_or(|ready| ready.manifest().identity != identity);
        self.ready = Some(images);
        if changed {
            self.slots.clear();
            self.pending_upload = None;
            self.in_flight = None;
            self.in_flight_bytes = 0;
        }
    }

    pub fn bind_catalogue(&mut self, catalogue: Option<Arc<Catalogue>>) {
        self.catalogue = catalogue;
    }

    pub fn set_observed(&mut self, observed: Vec<ObservedService>) {
        self.observed = observed;
    }

    pub fn nav_identity(&self) -> Digest {
        self.nav_identity
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    #[cfg(test)]
    pub fn bind_fixtures(&mut self, store: fixtures::Store) {
        self.fixtures = store;
    }

    /// Profile rebind / nav-world replacement. Frame owner calls this outside
    /// the Game pane window closure.
    pub fn sync_identity(
        &mut self,
        nav: Digest,
        geom: Option<(i32, i32, u32, u32)>,
        gpu: Option<&mut dyn FrameGpu>,
    ) {
        if self.nav_identity == nav && self.world_geom == geom {
            return;
        }
        self.nav_identity = nav;
        self.world_geom = geom;
        if self.open {
            self.invalidate_buffers(gpu);
        }
    }

    pub fn note_open(&mut self) {
        if !self.open {
            self.open = true;
            self.generation = self.generation.wrapping_add(1);
            self.search.clear();
            self.phase = MapPhase::Pending;
            self.status = String::from(CACHE_UNBOUND);
        }
    }

    /// Unregister textures and drop CPU pixels. Safe to call every closed frame.
    pub fn release(&mut self, gpu: Option<&mut dyn FrameGpu>) {
        self.open = false;
        self.ready = None;
        self.catalogue = None;
        self.observed.clear();
        self.invalidate_buffers(gpu);
        self.phase = MapPhase::Pending;
        self.status.clear();
    }

    fn invalidate_buffers(&mut self, mut gpu: Option<&mut dyn FrameGpu>) {
        self.generation = self.generation.wrapping_add(1);
        self.pending_upload = None;
        self.in_flight = None;
        self.in_flight_bytes = 0;
        self.compressed.clear();
        self.compressed.shrink_to_fit();
        self.stop_worker();
        drop_pool(&mut self.tex_pool, &mut gpu);
        drop_slots(&mut self.slots, &mut gpu);
        drop_overlay(&mut self.overlay_gpu, &mut gpu);
        self.overlay_cpu = None;
        self.overlay_size = (0, 0);
        self.overlay_key = None;
        self.last_vis = None;
    }

    fn stop_worker(&mut self) {
        if let Some(tx) = self.worker_tx.take() {
            let _ = tx.send(WorkerCmd::Stop);
        }
        if let Some(thread) = self.worker.take() {
            let _ = thread.join();
        }
        self.worker_rx = None;
        self.in_flight = None;
        self.in_flight_bytes = 0;
    }

    pub fn counters(&self) -> LiveCounters {
        let overlay_gpu_bytes = self
            .overlay_gpu
            .as_ref()
            .map(|o| (o.w as usize) * (o.h as usize) * 4)
            .unwrap_or(0);
        LiveCounters {
            generation: self.generation,
            phase: self.phase,
            terrain_slots: self.slots.len(),
            terrain_gpu_bytes: self.slots.len() * TILE_RGBA_BYTES,
            terrain_cpu_bytes: self.pending_upload.as_ref().map(|p| p.3.len()).unwrap_or(0),
            overlay_cpu_bytes: self.overlay_cpu.as_ref().map(Vec::len).unwrap_or(0),
            overlay_gpu_bytes,
            decode_staging_bytes: self.compressed.len()
                + self.in_flight_bytes
                + self.pending_upload.as_ref().map(|p| p.3.len()).unwrap_or(0),
            in_flight: usize::from(self.in_flight.is_some()),
            pending_upload: usize::from(self.pending_upload.is_some()),
            vtx: self.last_vtx,
            idx: self.last_idx,
        }
    }

    #[cfg(test)]
    pub fn force_staging_bytes(&mut self, compressed: usize, in_flight: usize, pending: usize) {
        self.compressed.resize(compressed, 0);
        self.in_flight_bytes = in_flight;
        if pending == 0 {
            self.pending_upload = None;
        } else {
            self.pending_upload = Some((
                self.generation,
                self.identity(),
                TileKey {
                    plane: 0,
                    lod: 0,
                    x: 0,
                    z: 0,
                },
                vec![0; pending],
            ));
        }
    }

    pub fn record_geometry(&mut self, vtx: i32, idx: i32) {
        self.last_vtx = vtx;
        self.last_idx = idx;
    }

    pub fn status_line(&self) -> &str {
        &self.status
    }

    pub fn phase(&self) -> MapPhase {
        self.phase
    }

    pub fn search_hits(&self) -> Vec<&nav::map::poi::PoiRecord> {
        let q = self.search.trim().to_ascii_lowercase();
        if q.is_empty() {
            return Vec::new();
        }
        self.fixtures
            .pois
            .iter()
            .filter(|p| p.name.as_str().to_ascii_lowercase().contains(&q))
            .take(MAX_LABELS)
            .collect()
    }

    pub fn parse_search_coord(&self) -> Option<GameTile> {
        parse_coord(self.search.trim())
    }

    /// Bind visible terrain, overlay, and draw images.
    #[allow(clippy::too_many_arguments)] // gpu, view, layers, and markers share one present
    pub fn present(
        &mut self,
        ui: &Ui,
        gpu: Option<&mut dyn FrameGpu>,
        origin: [f32; 2],
        size: [f32; 2],
        view: View,
        world: &NavWorld,
        layers: OverlayLayers,
        colors: OverlayColors,
        path: &[(WorldTile, bool)],
        floods: &[Arc<HashSet<WorldTile>>],
        reach: Option<&[u64]>,
        sel: Option<Tile>,
        here: Option<Tile>,
        dest: Option<Tile>,
        overlay_zoom_in: &mut bool,
    ) {
        if !self.open {
            return;
        }
        let mut gpu = gpu;
        // Route is vector-drawn; overlay must not depend on bot movement.
        self.sync_open(
            &mut gpu,
            view,
            world,
            layers,
            colors,
            &[],
            floods,
            reach,
            overlay_zoom_in,
        );
        let draw = ui.get_window_draw_list();
        draw.add_rect(
            origin,
            [origin[0] + size[0], origin[1] + size[1]],
            [0.05, 0.05, 0.06, 1.0],
        )
        .filled(true)
        .build();
        if self.show_basemap && self.has_terrain() {
            for slot in &self.slots {
                let Some(gpu_tex) = slot.gpu.as_ref() else {
                    continue;
                };
                let Ok([west, south, east, north]) = slot.key.bounds() else {
                    continue;
                };
                let p0 = canvas_point(view, origin, size, west as f64, north as f64);
                let p1 = canvas_point(view, origin, size, east as f64, south as f64);
                draw.add_image(
                    gpu_tex.tex_id,
                    p0,
                    p1,
                    INTERIOR_UV[0],
                    INTERIOR_UV[1],
                    [1.0, 1.0, 1.0, 1.0],
                );
            }
        }
        if !self.has_terrain() || !self.show_basemap {
            draw_mapsquare_grid(&draw, origin, size, view);
        }
        if let Some(over) = &self.overlay_gpu {
            draw.add_image(
                over.tex_id,
                origin,
                [origin[0] + size[0], origin[1] + size[1]],
                [0.0, 0.0],
                [1.0, 1.0],
                [1.0, 1.0, 1.0, 1.0],
            );
        }
        self.draw_pois(&draw, origin, size, view, sel, here);
        draw_route(&draw, origin, size, view, path, colors);
        if let Some(dest) = dest {
            draw_dest_marker(&draw, origin, size, view, dest, colors);
        }
        self.update_status(view, layers, reach);
    }

    fn update_status(&mut self, view: View, layers: OverlayLayers, reach: Option<&[u64]>) {
        if self.phase == MapPhase::Error {
            let vis = select_visible(view);
            if vis != self.last_vis {
                self.phase = MapPhase::Pending;
            } else {
                return;
            }
        }
        let mut parts = Vec::new();
        if !self.has_terrain() {
            self.phase = MapPhase::Ready;
            parts.push(CACHE_UNBOUND.to_string());
        } else if !self.show_basemap {
            self.phase = MapPhase::Ready;
            parts.push(String::from("basemap off"));
        } else if !self.slots.is_empty()
            && self.in_flight.is_none()
            && self.pending_upload.is_none()
        {
            self.phase = MapPhase::Ready;
            parts.push(format!("ready · {} tiles", self.slots.len()));
        } else {
            let vis_n = select_visible(view).map(|v| v.len()).unwrap_or(0);
            self.phase = MapPhase::Pending;
            parts.push(format!(
                "grid first — loading tiles {}/{vis_n}",
                self.slots.len()
            ));
        }
        if layers.reach && reach.is_none() {
            parts.push(REACH_UNAVAILABLE.to_string());
        }
        self.status = parts.join(" · ");
    }

    #[allow(clippy::too_many_arguments)] // open sync shares view plus overlay inputs
    fn sync_open(
        &mut self,
        gpu: &mut Option<&mut dyn FrameGpu>,
        view: View,
        world: &NavWorld,
        layers: OverlayLayers,
        colors: OverlayColors,
        path: &[(WorldTile, bool)],
        floods: &[Arc<HashSet<WorldTile>>],
        reach: Option<&[u64]>,
        overlay_zoom_in: &mut bool,
    ) {
        self.pump_decode();
        self.sync_terrain(view, gpu);
        *overlay_zoom_in = false;
        if layers.any() {
            match overlay::overlay_fit(view) {
                OverlayFit::Ready { w, h } => {
                    self.sync_overlay(view, w, h, world, layers, colors, path, floods, reach, gpu);
                }
                OverlayFit::ZoomIn => {
                    *overlay_zoom_in = true;
                    drop_overlay(&mut self.overlay_gpu, gpu);
                    self.overlay_cpu = None;
                    self.overlay_size = (0, 0);
                    self.overlay_key = None;
                }
            }
        } else {
            drop_overlay(&mut self.overlay_gpu, gpu);
            self.overlay_cpu = None;
            self.overlay_size = (0, 0);
            self.overlay_key = None;
        }
    }

    fn draw_pois(
        &self,
        draw: &dear_imgui_rs::DrawListMut<'_>,
        origin: [f32; 2],
        size: [f32; 2],
        view: View,
        sel: Option<Tile>,
        here: Option<Tile>,
    ) {
        let logical_ppt = if view.east > view.west {
            f64::from(size[0]) / (view.east - view.west)
        } else {
            0.0
        };
        let mut labels = 0usize;
        let mut symbols = 0usize;
        let mut mark = |x: f64, z: f64, kind: PoiKind, name: &str| -> bool {
            if x < view.west || x > view.east || z < view.south || z > view.north {
                return true;
            }
            if symbols >= MAX_SYMBOLS {
                return false;
            }
            symbols += 1;
            let p = canvas_point(view, origin, size, x, z);
            let color = match kind {
                PoiKind::Bank => [0.2, 0.7, 1.0, 1.0],
                _ => ACCENT,
            };
            draw.add_rect([p[0] - 3.0, p[1] - 3.0], [p[0] + 3.0, p[1] + 3.0], color)
                .filled(true)
                .build();
            if labels < MAX_LABELS && logical_ppt >= 4.0 && !name.is_empty() {
                draw.add_text([p[0] + 5.0, p[1] - 6.0], TEXT, name);
                labels += 1;
            }
            true
        };
        if let Some(catalogue) = &self.catalogue {
            for entry in catalogue.entries() {
                let tile = entry.anchor();
                if tile.level != i32::from(view.plane) {
                    continue;
                }
                if !mark(
                    f64::from(tile.x) + 0.5,
                    f64::from(tile.z) + 0.5,
                    entry.kind(),
                    entry.name(),
                ) {
                    break;
                }
            }
        }
        for service in &self.observed {
            if service.tile.level != i32::from(view.plane) {
                continue;
            }
            if !mark(
                f64::from(service.tile.x) + 0.5,
                f64::from(service.tile.z) + 0.5,
                service.kind,
                "",
            ) {
                break;
            }
        }
        for poi in &self.fixtures.pois {
            if poi.effective_plane != view.plane {
                continue;
            }
            if !mark(poi.display.x, poi.display.z, poi.kind, poi.name.as_str()) {
                break;
            }
        }
        if let Some(here) = here {
            if here.level == i32::from(view.plane) {
                let p = canvas_point(
                    view,
                    origin,
                    size,
                    f64::from(here.x) + 0.5,
                    f64::from(here.z) + 0.5,
                );
                draw.add_rect([p[0] - 4.0, p[1] - 4.0], [p[0] + 4.0, p[1] + 4.0], TEXT)
                    .filled(true)
                    .build();
            }
        }
        if let Some(sel) = sel {
            if sel.level == i32::from(view.plane) {
                let p = canvas_point(
                    view,
                    origin,
                    size,
                    f64::from(sel.x) + 0.5,
                    f64::from(sel.z) + 0.5,
                );
                draw.add_rect([p[0] - 5.0, p[1] - 5.0], [p[0] + 5.0, p[1] + 5.0], TEXT_DIM)
                    .filled(false)
                    .build();
            }
        }
    }

    fn sync_terrain(&mut self, view: View, gpu: &mut Option<&mut dyn FrameGpu>) {
        let vis = select_visible(view);
        if vis != self.last_vis && self.phase == MapPhase::Error {
            self.phase = MapPhase::Pending;
        }
        let Some(vis) = vis else {
            self.last_vis = None;
            return;
        };
        self.last_vis = Some(vis);
        if !self.show_basemap || !self.has_terrain() {
            return;
        }
        retain_visible(&mut self.slots, vis, &mut self.tex_pool, gpu);
        if let Some((_, _, key, _)) = &self.pending_upload {
            if !vis_contains(vis, *key) {
                self.pending_upload = None;
            }
        }
        self.request_missing(vis);
        self.upload_pending(gpu);
    }

    fn request_missing(&mut self, vis: VisibleTiles) {
        if self.pending_upload.is_some() || self.in_flight.is_some() {
            return;
        }
        if self.slots.len() >= TEXTURE_CAP {
            return;
        }
        for key in vis.keys() {
            if self.slots.iter().any(|s| s.key == key) {
                continue;
            }
            self.compressed.clear();
            if take_png(
                self.ready.as_deref(),
                &self.fixtures,
                key,
                &mut self.compressed,
            )
            .is_none()
            {
                continue;
            }
            if self.compressed.len() + TILE_RGBA_BYTES > DECODE_STAGING_CAP {
                self.compressed.clear();
                continue;
            }
            let gen = self.generation;
            let identity = self.identity();
            if self.sync_decode {
                match decode_checked(&self.compressed) {
                    Ok(rgba) => {
                        self.compressed.clear();
                        self.pending_upload = Some((gen, identity, key, rgba));
                    }
                    Err(e) => {
                        self.compressed.clear();
                        self.phase = MapPhase::Error;
                        self.status = format!("tile decode failed: {e}");
                    }
                }
                return;
            }
            self.ensure_worker();
            let png = std::mem::take(&mut self.compressed);
            let staging = png.len() + TILE_RGBA_BYTES;
            if let Some(tx) = &self.worker_tx {
                if tx
                    .send(WorkerCmd::Job(DecodeJob {
                        generation: gen,
                        identity,
                        key,
                        png,
                    }))
                    .is_ok()
                {
                    self.in_flight = Some((gen, key));
                    self.in_flight_bytes = staging;
                }
            }
            return;
        }
    }

    fn pump_decode(&mut self) {
        let Some(rx) = &self.worker_rx else {
            return;
        };
        match rx.try_recv() {
            Ok(done) => {
                self.in_flight = None;
                self.in_flight_bytes = 0;
                if done.generation != self.generation || !self.open {
                    return;
                }
                if done.identity != self.identity() {
                    return;
                }
                match done.result {
                    Ok(rgba) => {
                        self.pending_upload =
                            Some((done.generation, done.identity, done.key, rgba));
                    }
                    Err(e) => {
                        self.phase = MapPhase::Error;
                        self.status = format!("tile decode failed: {e}");
                    }
                }
            }
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => {}
        }
    }

    fn upload_pending(&mut self, gpu: &mut Option<&mut dyn FrameGpu>) {
        let mut uploaded = 0;
        while uploaded < MAX_UPLOADS_PER_FRAME {
            let Some((gen, identity, key, rgba)) = self.pending_upload.take() else {
                break;
            };
            if gen != self.generation || !self.open || identity != self.identity() {
                continue;
            }
            if self.slots.iter().any(|s| s.key == key) {
                continue;
            }
            if self.last_vis.is_some_and(|vis| !vis_contains(vis, key)) {
                continue;
            }
            if self.slots.len() >= TEXTURE_CAP {
                break;
            }
            match gpu.as_mut() {
                Some(g) => {
                    if let Some(gpu_tex) = upload_tile(*g, &mut self.tex_pool, &rgba) {
                        self.slots.push(TerrainSlot {
                            key,
                            gpu: Some(gpu_tex),
                        });
                        uploaded += 1;
                    }
                }
                None => {
                    self.slots.push(TerrainSlot { key, gpu: None });
                    uploaded += 1;
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)] // overlay rebuild packs world, layers, path, flood
    fn sync_overlay(
        &mut self,
        view: View,
        w: u32,
        h: u32,
        world: &NavWorld,
        layers: OverlayLayers,
        colors: OverlayColors,
        path: &[(WorldTile, bool)],
        floods: &[Arc<HashSet<WorldTile>>],
        reach: Option<&[u64]>,
        gpu: &mut Option<&mut dyn FrameGpu>,
    ) {
        let key = OverlayKey {
            west: view.west.floor() as i32,
            south: view.south.floor() as i32,
            east: view.east.ceil() as i32,
            north: view.north.ceil() as i32,
            plane: view.plane,
            w,
            h,
            layers,
            nav: self.nav_identity,
            origin_x: world.collision.origin.x,
            origin_z: world.collision.origin.z,
            width: world.collision.width as u32,
            height: world.collision.height as u32,
            flood_n: floods.len(),
            flood_a: flood_ident(floods.first().map(|s| s.as_ref())),
            flood_b: flood_ident(floods.get(1).map(|s| s.as_ref())),
        };
        if self.overlay_key == Some(key)
            && (self.overlay_gpu.is_some() || self.overlay_cpu.is_some())
        {
            return;
        }
        if w == 0 || h == 0 || w > overlay::OVERLAY_MAX_W || h > overlay::OVERLAY_MAX_H {
            drop_overlay(&mut self.overlay_gpu, gpu);
            self.overlay_cpu = None;
            self.overlay_size = (0, 0);
            self.overlay_key = None;
            return;
        }
        let bytes = (w as usize)
            .saturating_mul(h as usize)
            .saturating_mul(4)
            .min(overlay::OVERLAY_BYTE_CAP);
        let mut cpu = self.overlay_cpu.take().unwrap_or_default();
        cpu.resize(bytes, 0);
        overlay::rasterize(
            &mut cpu,
            w,
            h,
            view,
            OverlayPaint {
                world,
                layers,
                colors,
                path,
                floods,
                reach,
            },
        );
        if let Some(g) = gpu.as_mut() {
            upload_overlay(*g, &mut self.overlay_gpu, &cpu, w, h);
            cpu.clear();
            cpu.shrink_to_fit();
            self.overlay_cpu = None;
        } else {
            self.overlay_cpu = Some(cpu);
        }
        self.overlay_size = (w, h);
        self.overlay_key = Some(key);
    }

    fn ensure_worker(&mut self) {
        if self.worker_tx.is_some() {
            return;
        }
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("274-walk-map-decode".into())
            .spawn(move || decode_loop(cmd_rx, done_tx))
            .expect("walk-map decode worker");
        self.worker_tx = Some(cmd_tx);
        self.worker_rx = Some(done_rx);
        self.worker = Some(thread);
    }

    #[cfg(test)]
    pub fn overlay_cpu(&self) -> Option<&[u8]> {
        self.overlay_cpu.as_deref()
    }

    #[cfg(test)]
    pub fn overlay_size(&self) -> (u32, u32) {
        self.overlay_size
    }

    #[cfg(test)]
    pub fn slot_keys(&self) -> Vec<TileKey> {
        self.slots.iter().map(|s| s.key).collect()
    }

    #[cfg(test)]
    pub fn generation(&self) -> u64 {
        self.generation
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)] // test helper mirrors sync_open inputs
    pub fn test_sync(
        &mut self,
        gpu: Option<&mut dyn FrameGpu>,
        view: View,
        world: &NavWorld,
        layers: OverlayLayers,
        colors: OverlayColors,
        path: &[(WorldTile, bool)],
        floods: &[Arc<HashSet<WorldTile>>],
        reach: Option<&[u64]>,
    ) -> bool {
        if !self.open {
            return false;
        }
        let mut zoom_in = false;
        let mut gpu = gpu;
        self.sync_open(
            &mut gpu,
            view,
            world,
            layers,
            colors,
            path,
            floods,
            reach,
            &mut zoom_in,
        );
        zoom_in
    }
}

impl Drop for WalkMapRenderer {
    fn drop(&mut self) {
        self.stop_worker();
    }
}

fn take_png(
    ready: Option<&ReadyImages>,
    fixtures: &fixtures::Store,
    key: TileKey,
    buffer: &mut Vec<u8>,
) -> Option<()> {
    if let Some(ready) = ready {
        return match ready.read_tile_into(key, buffer) {
            Ok(Some(_)) => Some(()),
            _ => None,
        };
    }
    let png = fixtures.get(key)?;
    buffer.clear();
    buffer.extend_from_slice(png);
    Some(())
}

fn vis_contains(vis: VisibleTiles, key: TileKey) -> bool {
    key.plane == vis.plane
        && key.lod == vis.lod
        && key.x >= vis.min_x
        && key.x <= vis.max_x
        && key.z >= vis.min_z
        && key.z <= vis.max_z
}

fn retain_visible(
    slots: &mut Vec<TerrainSlot>,
    vis: VisibleTiles,
    pool: &mut Vec<GpuTex>,
    gpu: &mut Option<&mut dyn FrameGpu>,
) {
    let mut kept = Vec::with_capacity(slots.len());
    for slot in slots.drain(..) {
        if vis_contains(vis, slot.key) {
            kept.push(slot);
        } else if let Some(tex) = slot.gpu {
            if pool.len() < TEXTURE_CAP {
                pool.push(tex);
            } else if let Some(g) = gpu.as_mut() {
                g.unregister_texture(tex.tex_id);
            }
        }
    }
    *slots = kept;
}

fn select_visible(view: View) -> Option<VisibleTiles> {
    select_lod(view, TEXTURE_CAP).ok()
}

/// Cheap flood identity: size plus one sample tile. Not a whole-set hash.
fn flood_ident(set: Option<&HashSet<WorldTile>>) -> usize {
    let Some(set) = set else {
        return 0;
    };
    let sample = set
        .iter()
        .next()
        .map(|t| ((t.x as usize) << 16) ^ ((t.z as usize) << 4) ^ (t.level as usize))
        .unwrap_or(0);
    set.len().wrapping_shl(24) ^ sample
}

fn decode_checked(png: &[u8]) -> Result<Vec<u8>, MapError> {
    if png.len() > DECODE_LIMIT_BYTES {
        return Err(MapError::Limit("png bytes"));
    }
    fixtures::decode_tile(png)
}

fn decode_loop(cmd: Receiver<WorkerCmd>, done: Sender<DecodeDone>) {
    while let Ok(cmd) = cmd.recv() {
        match cmd {
            WorkerCmd::Stop => break,
            WorkerCmd::Job(job) => {
                let result = decode_checked(&job.png).map_err(|e| e.to_string());
                let _ = done.send(DecodeDone {
                    generation: job.generation,
                    identity: job.identity,
                    key: job.key,
                    result,
                });
            }
        }
    }
}

fn upload_tile(gpu: &mut dyn FrameGpu, pool: &mut Vec<GpuTex>, rgba: &[u8]) -> Option<GpuTex> {
    if rgba.len() != TILE_RGBA_BYTES {
        return None;
    }
    let cached = if let Some(existing) = pool.pop() {
        existing
    } else {
        let texture = gpu.device().create_texture(&wgpu::TextureDescriptor {
            label: Some("274 walk map tile"),
            size: wgpu::Extent3d {
                width: TILE_PIXELS,
                height: TILE_PIXELS,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let tex_id = gpu.register_texture(&texture, &view);
        GpuTex {
            texture,
            view,
            tex_id,
        }
    };
    gpu.queue().write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &cached.texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4 * TILE_PIXELS),
            rows_per_image: Some(TILE_PIXELS),
        },
        wgpu::Extent3d {
            width: TILE_PIXELS,
            height: TILE_PIXELS,
            depth_or_array_layers: 1,
        },
    );
    Some(cached)
}

fn upload_overlay(
    gpu: &mut dyn FrameGpu,
    slot: &mut Option<OverlayGpu>,
    rgba: &[u8],
    w: u32,
    h: u32,
) {
    let reuse = slot.as_ref().is_some_and(|s| s.w == w && s.h == h);
    if !reuse {
        if let Some(old) = slot.take() {
            gpu.unregister_texture(old.tex_id);
        }
        let texture = gpu.device().create_texture(&wgpu::TextureDescriptor {
            label: Some("274 walk map overlay"),
            size: wgpu::Extent3d {
                width: w.max(1),
                height: h.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let tex_id = gpu.register_texture(&texture, &view);
        *slot = Some(OverlayGpu {
            texture,
            view,
            tex_id,
            w: w.max(1),
            h: h.max(1),
        });
    }
    let cached = slot.as_ref().expect("overlay texture");
    gpu.queue().write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &cached.texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4 * cached.w),
            rows_per_image: Some(cached.h),
        },
        wgpu::Extent3d {
            width: cached.w,
            height: cached.h,
            depth_or_array_layers: 1,
        },
    );
}

fn drop_slots(slots: &mut Vec<TerrainSlot>, gpu: &mut Option<&mut dyn FrameGpu>) {
    if let Some(gpu) = gpu {
        for slot in slots.drain(..) {
            if let Some(tex) = slot.gpu {
                gpu.unregister_texture(tex.tex_id);
            }
        }
    } else {
        slots.clear();
    }
}

fn drop_overlay(overlay: &mut Option<OverlayGpu>, gpu: &mut Option<&mut dyn FrameGpu>) {
    if let Some(old) = overlay.take() {
        if let Some(gpu) = gpu {
            gpu.unregister_texture(old.tex_id);
        }
    }
}

fn drop_pool(pool: &mut Vec<GpuTex>, gpu: &mut Option<&mut dyn FrameGpu>) {
    if let Some(gpu) = gpu {
        for tex in pool.drain(..) {
            gpu.unregister_texture(tex.tex_id);
        }
    } else {
        pool.clear();
    }
}

fn rgba8(c: [u8; 4]) -> [f32; 4] {
    [
        f32::from(c[0]) / 255.0,
        f32::from(c[1]) / 255.0,
        f32::from(c[2]) / 255.0,
        f32::from(c[3]) / 255.0,
    ]
}

fn draw_mapsquare_grid(
    draw: &dear_imgui_rs::DrawListMut<'_>,
    origin: [f32; 2],
    size: [f32; 2],
    view: View,
) {
    let span = 64i32;
    let x0 = (view.west.floor() as i32 / span) * span;
    let z0 = (view.south.floor() as i32 / span) * span;
    let x1 = view.east.ceil() as i32;
    let z1 = view.north.ceil() as i32;
    let color = TEXT_DIM;
    let mut n = 0;
    let mut x = x0;
    while x <= x1 && n < 48 {
        let p0 = canvas_point(view, origin, size, f64::from(x), view.south);
        let p1 = canvas_point(view, origin, size, f64::from(x), view.north);
        draw.add_line(p0, p1, color).build();
        x += span;
        n += 1;
    }
    let mut z = z0;
    while z <= z1 && n < 96 {
        let p0 = canvas_point(view, origin, size, view.west, f64::from(z));
        let p1 = canvas_point(view, origin, size, view.east, f64::from(z));
        draw.add_line(p0, p1, color).build();
        z += span;
        n += 1;
    }
}

fn draw_route(
    draw: &dear_imgui_rs::DrawListMut<'_>,
    origin: [f32; 2],
    size: [f32; 2],
    view: View,
    path: &[(WorldTile, bool)],
    colors: OverlayColors,
) {
    if path.len() < 2 {
        if let Some((tile, _)) = path.first() {
            if tile.level == i32::from(view.plane) {
                let p = canvas_point(
                    view,
                    origin,
                    size,
                    f64::from(tile.x) + 0.5,
                    f64::from(tile.z) + 0.5,
                );
                draw.add_rect(
                    [p[0] - 2.0, p[1] - 2.0],
                    [p[0] + 2.0, p[1] + 2.0],
                    rgba8(colors.path),
                )
                .filled(true)
                .build();
            }
        }
        return;
    }
    let stride = path.len().div_ceil(MAX_ROUTE_SEGMENTS).max(1);
    let walk = rgba8(colors.path);
    let hop = rgba8(colors.transport);
    let mut prev: Option<(WorldTile, bool)> = None;
    for (i, &(tile, transport)) in path.iter().enumerate() {
        if i % stride != 0 && i + 1 != path.len() {
            continue;
        }
        if tile.level != i32::from(view.plane) {
            prev = Some((tile, transport));
            continue;
        }
        if let Some((last, last_tr)) = prev {
            if last.level == tile.level {
                let p0 = canvas_point(
                    view,
                    origin,
                    size,
                    f64::from(last.x) + 0.5,
                    f64::from(last.z) + 0.5,
                );
                let p1 = canvas_point(
                    view,
                    origin,
                    size,
                    f64::from(tile.x) + 0.5,
                    f64::from(tile.z) + 0.5,
                );
                let color = if last_tr || transport { hop } else { walk };
                draw.add_line(p0, p1, color).thickness(2.0).build();
            }
        }
        prev = Some((tile, transport));
    }
}

fn draw_dest_marker(
    draw: &dear_imgui_rs::DrawListMut<'_>,
    origin: [f32; 2],
    size: [f32; 2],
    view: View,
    dest: Tile,
    colors: OverlayColors,
) {
    if dest.level != i32::from(view.plane) {
        return;
    }
    let p = canvas_point(
        view,
        origin,
        size,
        f64::from(dest.x) + 0.5,
        f64::from(dest.z) + 0.5,
    );
    let c = rgba8(colors.path);
    draw.add_line([p[0] - 6.0, p[1]], [p[0] + 6.0, p[1]], c)
        .thickness(2.0)
        .build();
    draw.add_line([p[0], p[1] - 6.0], [p[0], p[1] + 6.0], c)
        .thickness(2.0)
        .build();
}

/// Map a world point onto the canvas using the view's world span, not physical
/// `pixels_per_tile` (ImGui draw lists are in logical points).
pub fn canvas_point(view: View, origin: [f32; 2], size: [f32; 2], x: f64, z: f64) -> [f32; 2] {
    let world_w = view.east - view.west;
    let world_h = view.north - view.south;
    if world_w <= 0.0 || world_h <= 0.0 {
        return origin;
    }
    let sx = (x - view.west) / world_w * f64::from(size[0]);
    let sy = (view.north - z) / world_h * f64::from(size[1]);
    [origin[0] + sx as f32, origin[1] + sy as f32]
}

pub fn view_from_canvas(
    centre: (i32, i32),
    rem: (f32, f32),
    scale: f32,
    size: [f32; 2],
    plane: u8,
    max_lod: u8,
    fb_scale: f32,
) -> View {
    let cx = f64::from(centre.0) + f64::from(rem.0);
    let cz = f64::from(centre.1) + f64::from(rem.1);
    let half_w = f64::from(size[0]) / (2.0 * f64::from(scale));
    let half_h = f64::from(size[1]) / (2.0 * f64::from(scale));
    View {
        west: cx - half_w,
        east: cx + half_w,
        south: cz - half_h,
        north: cz + half_h,
        pixels_per_tile: f64::from(scale) * f64::from(fb_scale.max(0.01)),
        plane,
        max_lod,
    }
}

pub fn snap_tile(world: &NavWorld, request: GameTile) -> Option<Tile> {
    let snapped = snap_walkable(request, |t| {
        world.collision.walkable(WorldTile {
            x: t.x,
            z: t.z,
            level: i32::from(t.plane),
        })
    })?;
    Some(Tile {
        x: snapped.x,
        z: snapped.z,
        level: i32::from(snapped.plane),
    })
}

pub fn parse_coord(raw: &str) -> Option<GameTile> {
    let mut parts = raw.split(',');
    let x: i32 = parts.next()?.trim().parse().ok()?;
    let z: i32 = parts.next()?.trim().parse().ok()?;
    let plane = match parts.next() {
        Some(p) => p.trim().parse().ok()?,
        None => 0u8,
    };
    if parts.next().is_some() || plane >= 4 {
        return None;
    }
    Some(GameTile { x, z, plane })
}

pub fn overlay_colors(nav: &NavSettings) -> OverlayColors {
    let rgba = |raw: &str, fallback: [u8; 3], a: u8| {
        let [r, g, b] = parse_html_color(raw, fallback);
        [r, g, b, a]
    };
    OverlayColors {
        grid: [255, 176, 0, 180],
        collision: rgba(&nav.color_collision, [0, 128, 255], 200),
        path: rgba(&nav.color_path, [255, 0, 0], 220),
        transport: rgba(&nav.color_transport, [0, 255, 0], 220),
        flood_a: [0, 0, 255, 160],
        flood_b: [200, 40, 240, 160],
        unreached: [200, 40, 240, 160],
        nsew: [220, 220, 220, 220],
    }
}

/// macOS `proc_pid_rusage` v4 physical footprint. Other platforms use RSS.
pub fn phys_footprint_bytes() -> Option<u64> {
    #[cfg(target_os = "macos")]
    {
        unsafe {
            let mut info: libc::rusage_info_v4 = std::mem::zeroed();
            let rc = libc::proc_pid_rusage(
                libc::getpid(),
                libc::RUSAGE_INFO_V4,
                &mut info as *mut libc::rusage_info_v4 as *mut libc::rusage_info_t,
            );
            if rc != 0 {
                return None;
            }
            let n = info.ri_phys_footprint;
            if n == 0 {
                None
            } else {
                Some(n)
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        host_play::current_resident_bytes()
    }
}

#[cfg(test)]
#[path = "walk_map_tests.rs"]
mod tests;
