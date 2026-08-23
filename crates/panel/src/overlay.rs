//! Schematic nav-path overlay for the Game Image.
//!
//! The 274 client renders a perspective 3D view, so there is no exact
//! tile→pixel transform. The overlay treats the Image as a flat tile grid
//! centred on the focused slot's observed tile and rebuilds its polyline
//! vertices at the 1 s raster cadence (or immediately when a new route
//! arms) instead of every 50 fps UI frame.

use std::time::{Duration, Instant};

use dear_imgui_rs::Ui;
use nav::tile::Tile;

use crate::session::Session;
use crate::theme::ACCENT;

/// How often the overlay recomputes its vertices: the watch-only raster
/// cadence (50 ticks × 20 ms).
const REFRESH: Duration = Duration::from_millis(1000);

/// The Image is treated as a flat tile grid centred on the player. The
/// client culls world rendering past ±26 tiles from the camera, so the
/// grid is 52 tiles wide, aspect-corrected to the 765×503 applet.
const VIEW_TILES_W: i32 = 52;
const VIEW_TILES_H: i32 = 34;

/// Map a world tile to image-local pixels: the inverse of
/// [`host::map_image_to_applet`] conceptually (applet pixel → world tile is
/// the forward direction; here a tile lands on the centre of its cell in a
/// `tiles_w`×`tiles_h` grid whose top-left tile is `origin`).
pub fn tile_to_local(
    t: Tile,
    origin: Tile,
    tiles_w: i32,
    tiles_h: i32,
    img_w: f32,
    img_h: f32,
) -> [f32; 2] {
    let cell_w = img_w / tiles_w.max(1) as f32;
    let cell_h = img_h / tiles_h.max(1) as f32;
    [
        (t.x - origin.x) as f32 * cell_w + cell_w / 2.0,
        (t.z - origin.z) as f32 * cell_h + cell_h / 2.0,
    ]
}

/// Whether the cached polyline is stale: the route generation moved (a new
/// arm / focus switch) or `REFRESH` has elapsed since the last rebuild.
fn needs_refresh(cached_gen: u64, gen: u64, since: Duration) -> bool {
    gen != cached_gen || since >= REFRESH
}

/// Window-space vertices for the focused traveller's remaining route, or
/// empty when nothing is armed. The schematic grid is anchored on the
/// focused slot's observed tile; before the first position report the
/// route's own first tile (the tile it was routed from) anchors instead.
fn points_for(session: &Session, min: [f32; 2], size: [f32; 2]) -> Vec<[f32; 2]> {
    let Some(name) = session.focused_name() else {
        return Vec::new();
    };
    let Some(traveller) = session.travellers.lock().unwrap().get(&name).cloned() else {
        return Vec::new();
    };
    let here = session
        .focused_tile()
        .map(|(x, z)| Tile { x, z, level: 0 });
    let tiles = traveller.lock().unwrap().remaining_walk_tiles(here);
    if tiles.is_empty() {
        return Vec::new();
    }
    let anchor = here.unwrap_or(tiles[0]);
    let origin = Tile {
        x: anchor.x - VIEW_TILES_W / 2,
        z: anchor.z - VIEW_TILES_H / 2,
        level: anchor.level,
    };
    tiles
        .iter()
        .map(|t| {
            let local = tile_to_local(*t, origin, VIEW_TILES_W, VIEW_TILES_H, size[0], size[1]);
            [min[0] + local[0], min[1] + local[1]]
        })
        .collect()
}

/// Cached overlay polyline: window-space vertices rebuilt only when the
/// route generation changes (rising edge on a new arm / focus switch) or
/// [`REFRESH`] elapsed, so an unchanged route does not recompute or
/// re-allocate vertices every 50 fps UI frame.
pub struct PathOverlay {
    points: Vec<[f32; 2]>,
    gen: u64,
    last: Instant,
}

impl PathOverlay {
    pub fn new() -> Self {
        Self {
            points: Vec::new(),
            gen: 0,
            last: Instant::now(),
        }
    }

    /// The cached window-space vertices (empty while nothing is armed).
    pub fn points(&self) -> &[[f32; 2]] {
        &self.points
    }

    /// Rebuild-if-stale, then stroke the amber polyline over the Image.
    /// `min`/`size` are the Image widget's top-left corner and size.
    pub fn frame(&mut self, ui: &Ui, session: &Session, min: [f32; 2], size: [f32; 2]) {
        let gen = session.route_gen();
        let now = Instant::now();
        if needs_refresh(self.gen, gen, now.duration_since(self.last)) {
            self.gen = gen;
            self.last = now;
            self.points = points_for(session, min, size);
        }
        if !self.points.is_empty() {
            ui.get_window_draw_list()
                .add_polyline(self.points.clone(), ACCENT)
                .closed(false)
                .thickness(2.0)
                .build();
        }
    }
}

impl Default for PathOverlay {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use nav::grid::StepGrid;
    use nav::tile::Tile;

    use super::{needs_refresh, tile_to_local, PathOverlay};
    use crate::session::Session;

    #[test]
    fn overlay_tile_to_image_roundtrip() {
        // tile 0,0 in a 3x3 at image [0,0]-[90,90] → center of cell
        let p = tile_to_local(Tile { x: 0, z: 0, level: 0 }, Tile { x: 0, z: 0, level: 0 }, 3, 3, 90.0, 90.0);
        assert!(p[0] >= 0.0 && p[0] <= 30.0);
    }

    #[test]
    fn tile_to_local_centers_cells_from_origin() {
        // origin (1,1): tile (2,2) is the image centre of a 3x3 grid.
        let p = tile_to_local(
            Tile { x: 2, z: 2, level: 0 },
            Tile { x: 1, z: 1, level: 0 },
            3,
            3,
            90.0,
            90.0,
        );
        assert!((p[0] - 45.0).abs() < 0.001);
        assert!((p[1] - 45.0).abs() < 0.001);
    }

    #[test]
    fn tile_to_local_scales_to_image_size() {
        // Cells follow the image aspect: a 2x2 grid in a 100x60 image has
        // 50x30 cells, so tile (1,1) centres at (75,45).
        let p = tile_to_local(
            Tile { x: 1, z: 1, level: 0 },
            Tile { x: 0, z: 0, level: 0 },
            2,
            2,
            100.0,
            60.0,
        );
        assert!((p[0] - 75.0).abs() < 0.001);
        assert!((p[1] - 45.0).abs() < 0.001);
    }

    #[test]
    fn refresh_is_due_on_new_generation_or_one_second() {
        assert!(needs_refresh(0, 1, Duration::ZERO));
        assert!(needs_refresh(1, 1, Duration::from_secs(1)));
        assert!(!needs_refresh(1, 1, Duration::from_millis(999)));
    }

    #[test]
    fn overlay_builds_points_when_route_armed() {
        let _guard = crate::IMGUI_CTX_TEST_GUARD.lock().unwrap();
        let mut ctx = dear_imgui_rs::Context::create();
        ctx.prepare_frame(
            dear_imgui_rs::FramePrepareOptions::new([900.0, 700.0], 1.0 / 60.0)
                .renderer_has_textures(),
        );
        let ui = ctx.frame();
        let mut s = Session::new();
        s.focus.lock().unwrap().focused = Some("alice".into());
        let g = StepGrid::fixture_open_3x3();
        s.arm_walk_on(&g, Tile { x: 0, z: 0, level: 0 }, Tile { x: 2, z: 2, level: 0 });
        let mut overlay = PathOverlay::new();
        ui.window("##overlay-test").build(|| {
            overlay.frame(ui, &s, [10.0, 10.0], [90.0, 90.0]);
        });
        ctx.render();
        assert!(!overlay.points().is_empty(), "an armed route must paint");
        assert!(
            s.route_gen() != 0,
            "arming must bump the overlay generation"
        );
    }

    #[test]
    fn overlay_skips_draw_without_route() {
        let _guard = crate::IMGUI_CTX_TEST_GUARD.lock().unwrap();
        let mut ctx = dear_imgui_rs::Context::create();
        ctx.prepare_frame(
            dear_imgui_rs::FramePrepareOptions::new([900.0, 700.0], 1.0 / 60.0)
                .renderer_has_textures(),
        );
        let ui = ctx.frame();
        let s = Session::new();
        s.focus.lock().unwrap().focused = Some("alice".into());
        let mut overlay = PathOverlay::new();
        ui.window("##overlay-test").build(|| {
            overlay.frame(ui, &s, [10.0, 10.0], [90.0, 90.0]);
        });
        ctx.render();
        assert!(overlay.points().is_empty(), "no route -> no polyline");
    }

    #[test]
    fn overlay_refreshes_when_route_changes() {
        let _guard = crate::IMGUI_CTX_TEST_GUARD.lock().unwrap();
        let mut ctx = dear_imgui_rs::Context::create();
        ctx.prepare_frame(
            dear_imgui_rs::FramePrepareOptions::new([900.0, 700.0], 1.0 / 60.0)
                .renderer_has_textures(),
        );
        let mut s = Session::new();
        s.focus.lock().unwrap().focused = Some("alice".into());
        let g = StepGrid::fixture_open_3x3();
        s.arm_walk_on(&g, Tile { x: 0, z: 0, level: 0 }, Tile { x: 1, z: 1, level: 0 });
        let mut overlay = PathOverlay::new();
        {
            let ui = ctx.frame();
            ui.window("##overlay-test").build(|| {
                overlay.frame(ui, &s, [0.0, 0.0], [90.0, 90.0]);
            });
        }
        ctx.render();
        let first = overlay.points()[0];
        let last = *overlay.points().last().unwrap();
        // Re-arm a longer route: the new generation must rebuild now. Both
        // routes start on the same tile so the first point is stable, but
        // the line must reach the new dest.
        s.arm_walk_on(&g, Tile { x: 0, z: 0, level: 0 }, Tile { x: 2, z: 2, level: 0 });
        ctx.prepare_frame(
            dear_imgui_rs::FramePrepareOptions::new([900.0, 700.0], 1.0 / 60.0)
                .renderer_has_textures(),
        );
        {
            let ui = ctx.frame();
            ui.window("##overlay-test").build(|| {
                overlay.frame(ui, &s, [0.0, 0.0], [90.0, 90.0]);
            });
        }
        ctx.render();
        assert_eq!(overlay.points()[0], first);
        assert_ne!(overlay.points().last().copied(), Some(last));
    }
}
