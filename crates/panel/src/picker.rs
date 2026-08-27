//! WalkTo picker: a collision-dot map window over the baked nav world.
//!
//! Draws the walkable tiles of the loaded [`NavWorld`] as amber dots inside
//! a child canvas. Click snaps to the nearest walkable tile (highlight
//! only); **Walk** arms `session.arm_walk_on` and closes. The world is
//! loaded once per process from `$NAV_PACK` or `~/.274bot/274bot.navpack`.

use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::OnceLock;

use api::snapshot::WorldTile;
use dear_imgui_rs::{Condition, MouseButton, Ui, WindowFlags};
use nav::tile::{chebyshev, Tile};
use nav::world::NavWorld;

use crate::session::Session;
use crate::theme::{ACCENT, TEXT};

/// Default picker centre: the Lumbridge courtyard when the player's tile is
/// unknown.
const DEFAULT_CENTRE: (i32, i32) = (3220, 3220);
/// Zoom steps in pixels per tile. The coarsest step spans at most ~320 tiles
/// across the canvas (rs2b0t `TILES_AT_ZOOM1` cap); fine steps keep clicks
/// precise.
const ZOOMS: [f32; 4] = [2.0, 4.0, 8.0, 16.0];

/// The baked world, loaded once per process. `None` when the file is
/// missing or corrupt (the picker then shows the run-nav-pack hint).
static PACK: OnceLock<Option<NavWorld>> = OnceLock::new();
/// Persistent picker view state (survives frames, not the process).
static CENTRE_X: AtomicI32 = AtomicI32::new(DEFAULT_CENTRE.0);
static CENTRE_Z: AtomicI32 = AtomicI32::new(DEFAULT_CENTRE.1);
static LEVEL: AtomicI32 = AtomicI32::new(0);
static ZOOM: AtomicI32 = AtomicI32::new(0);
/// True while the picker window was drawn last frame; drives the view reset
/// when it opens fresh.
static PREV_OPEN: AtomicBool = AtomicBool::new(false);

/// Pack path: `$NAV_PACK`, else `~/.274bot/274bot.navpack`.
pub fn default_pack_path() -> PathBuf {
    match std::env::var("NAV_PACK") {
        Ok(p) => PathBuf::from(p),
        Err(_) => match std::env::var("HOME") {
            Ok(home) => PathBuf::from(format!("{home}/.274bot/274bot.navpack")),
            Err(_) => PathBuf::from(".274bot/274bot.navpack"),
        },
    }
}

fn pack() -> Option<&'static NavWorld> {
    PACK.get_or_init(|| NavWorld::load_pack(&default_pack_path()).ok())
        .as_ref()
}

/// The walkable tiles of the world's level-0 bake, row-major (z then x): a
/// tile is a "dot" when the collision's blanket `walkable` check passes
/// (the standable test, not a directional mask).
fn world_dots(world: &NavWorld) -> impl Iterator<Item = Tile> + '_ {
    let c = &world.collision;
    let o = c.origin;
    (0..c.height)
        .flat_map(move |z| {
            (0..c.width).map(move |x| Tile {
                x: o.x + x as i32,
                z: o.z + z as i32,
                level: o.level,
            })
        })
        .filter(move |t| {
            c.walkable(WorldTile {
                x: t.x,
                z: t.z,
                level: t.level,
            })
        })
}

/// Levels that contain at least one walkable tile. The v2 world is a
/// level-0 bake, so this is always `[0]`; the combo stays for shape.
pub fn available_levels(_world: &NavWorld) -> Vec<i32> {
    vec![0]
}

/// The nearest walkable tile on the world's level-0 bake to the float point
/// `(x, z)`, or the click's own tile when it is already walkable. Distance
/// is Chebyshev with Manhattan breaking ties. `None` when the level has no
/// walkable tile (only level 0 is baked).
pub fn snap(world: &NavWorld, x: f32, z: f32, level: i32) -> Option<Tile> {
    if level != world.collision.origin.level {
        return None;
    }
    let target = Tile {
        x: x.round() as i32,
        z: z.round() as i32,
        level,
    };
    if world
        .collision
        .walkable(WorldTile {
            x: target.x,
            z: target.z,
            level: target.level,
        })
    {
        return Some(target);
    }
    world_dots(world).min_by_key(|t| {
        let manhattan = (t.x - target.x).abs() + (t.z - target.z).abs();
        (chebyshev(*t, target), manhattan)
    })
}

/// Map a click in the canvas at `click` (canvas-local px) to the nearest
/// walkable tile. `centre` is the tile at the canvas centre, `scale` the
/// pixels per tile, `size` the canvas size.
pub fn click_to_tile(
    world: &NavWorld,
    centre: (i32, i32),
    scale: f32,
    click: [f32; 2],
    size: [f32; 2],
    level: i32,
) -> Option<Tile> {
    let tx = centre.0 as f32 + (click[0] - size[0] / 2.0) / scale;
    let tz = centre.1 as f32 + (click[1] - size[1] / 2.0) / scale;
    snap(world, tx, tz, level)
}

/// The "WalkTo" picker window. Draws the collision-dot map when the world
/// loads; shows a run-nav-pack hint otherwise.
pub fn picker_window(ui: &Ui, session: &mut Session) {
    let mut open = session.walkto_open;
    match pack() {
        Some(world) => picker_map_window(ui, session, world, &mut open),
        None => picker_no_pack_window(ui, &mut open),
    }
    session.walkto_open = open;
    if !open {
        PREV_OPEN.store(false, Ordering::Relaxed);
    }
}

/// Missing/corrupt pack: a hint instead of the map.
fn picker_no_pack_window(ui: &Ui, open: &mut bool) {
    let _ = ui
        .window("WalkTo")
        .opened(open)
        .flags(WindowFlags::NO_DOCKING)
        .size([360.0, 120.0], Condition::FirstUseEver)
        .build(|| {
            ui.text_wrapped("no nav pack — run nav-pack");
        });
}

/// The collision-dot map window. `open` is the window's live open flag;
/// confirm Walk closes it.
fn picker_map_window(ui: &Ui, session: &mut Session, world: &NavWorld, open: &mut bool) {
    // Reset the view when the picker opens fresh.
    if !PREV_OPEN.swap(true, Ordering::Relaxed) {
        let (cx, cz) = session.focused_tile().unwrap_or(DEFAULT_CENTRE);
        CENTRE_X.store(cx, Ordering::Relaxed);
        CENTRE_Z.store(cz, Ordering::Relaxed);
        LEVEL.store(available_levels(world)[0], Ordering::Relaxed);
        session.picker_sel = None;
    }
    let confirmed = ui
        .window("WalkTo")
        .opened(open)
        .flags(WindowFlags::NO_DOCKING)
        .size([440.0, 430.0], Condition::FirstUseEver)
        .build(|| {
            let levels = available_levels(world);
            let mut lvl_idx = levels
                .iter()
                .position(|l| *l == LEVEL.load(Ordering::Relaxed))
                .unwrap_or(0);
            if ui.combo("##walkto-level", &mut lvl_idx, &levels, |l: &i32| {
                Cow::Owned(format!("level {l}"))
            }) {
                LEVEL.store(levels[lvl_idx], Ordering::Relaxed);
            }
            ui.same_line();
            let mut zoom = ZOOM.load(Ordering::Relaxed) as usize;
            if ui.combo("##walkto-zoom", &mut zoom, &ZOOMS, |z: &f32| {
                Cow::Owned(format!("{z:.0}px/tile"))
            }) {
                ZOOM.store(zoom as i32, Ordering::Relaxed);
            }
            ui.same_line();
            if ui.button("recentre") {
                let (cx, cz) = session.focused_tile().unwrap_or(DEFAULT_CENTRE);
                CENTRE_X.store(cx, Ordering::Relaxed);
                CENTRE_Z.store(cz, Ordering::Relaxed);
            }
            draw_canvas(ui, session, world);
            match session.picker_sel {
                Some(t) => ui.text_disabled(format!("selected {} {} {}", t.x, t.z, t.level)),
                None => ui.text_disabled("click a tile, then Walk"),
            }
            let can_walk = session.picker_sel.is_some();
            let _off = ui.begin_disabled_with_cond(!can_walk);
            ui.button("Walk") && can_walk && session.confirm_picker_walk(world)
        })
        .unwrap_or(false);
    if !*open {
        PREV_OPEN.store(false, Ordering::Relaxed);
        session.picker_sel = None;
    }
    if confirmed {
        *open = false;
        PREV_OPEN.store(false, Ordering::Relaxed);
    }
}

/// The child canvas: amber dots, drag-to-pan, click-to-select (does not arm).
fn draw_canvas(ui: &Ui, session: &mut Session, world: &NavWorld) {
    let avail = ui.content_region_avail();
    let canvas_h = (avail[1] - 24.0).max(120.0);
    let mut rect: Option<([f32; 2], [f32; 2])> = None;
    ui.child_window("##walkto-canvas")
        .size([0.0, canvas_h])
        .build(ui, || {
            let draw = ui.get_window_draw_list();
            let pos = ui.window_pos();
            let size = ui.content_region_avail();
            let (min, max) = (pos, [pos[0] + size[0], pos[1] + size[1]]);
            rect = Some((min, max));
            let (cx, cz) = (
                CENTRE_X.load(Ordering::Relaxed) as f32,
                CENTRE_Z.load(Ordering::Relaxed) as f32,
            );
            let scale = ZOOMS[ZOOM.load(Ordering::Relaxed) as usize];
            // Only draw tiles inside the visible window.
            let (wx0, wx1) = (cx - size[0] / 2.0 / scale, cx + size[0] / 2.0 / scale);
            let (wz0, wz1) = (cz - size[1] / 2.0 / scale, cz + size[1] / 2.0 / scale);
            // Screen origin of tile (0,0) for this view.
            let ox = min[0] + size[0] / 2.0 - cx * scale;
            let oz = min[1] + size[1] / 2.0 - cz * scale;
            let dot = (scale * 0.72).clamp(1.5, 5.0);
            let sel = session.picker_sel;
            for t in world_dots(world) {
                let (tx, tz) = (t.x as f32, t.z as f32);
                if tx < wx0 || tx > wx1 || tz < wz0 || tz > wz1 {
                    continue;
                }
                let selected = sel.is_some_and(|s| s == t);
                let (color, d) = if selected {
                    (TEXT, (dot + 2.0).min(scale.max(3.0)))
                } else {
                    (ACCENT, dot)
                };
                let h = d / 2.0;
                let x0 = ox + tx * scale - h;
                let y0 = oz + tz * scale - h;
                draw.add_rect([x0, y0], [x0 + d, y0 + d], color)
                    .filled(true)
                    .build();
            }
        });
    let Some((min, max)) = rect else {
        return;
    };
    if !ui.is_item_hovered() {
        return;
    }
    let scale = ZOOMS[ZOOM.load(Ordering::Relaxed) as usize];
    if ui.is_mouse_dragging_with_threshold(MouseButton::Left, 5.0) {
        let delta = ui.io().mouse_delta();
        CENTRE_X.store(
            CENTRE_X.load(Ordering::Relaxed) - (delta[0] / scale).round() as i32,
            Ordering::Relaxed,
        );
        CENTRE_Z.store(
            CENTRE_Z.load(Ordering::Relaxed) - (delta[1] / scale).round() as i32,
            Ordering::Relaxed,
        );
        return;
    }
    if !ui.is_mouse_clicked(MouseButton::Left) {
        return;
    }
    let mouse = ui.io().mouse_pos();
    let size = [max[0] - min[0], max[1] - min[1]];
    let centre = (
        CENTRE_X.load(Ordering::Relaxed),
        CENTRE_Z.load(Ordering::Relaxed),
    );
    if let Some(tile) = click_to_tile(
        world,
        centre,
        scale,
        [mouse[0] - min[0], mouse[1] - min[1]],
        size,
        LEVEL.load(Ordering::Relaxed),
    ) {
        session.picker_sel = Some(tile);
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use api::snapshot::WorldTile;
    use client::dash3d::CollisionFlag;
    use nav::collision::WorldCollision;
    use nav::tile::Tile;
    use nav::transport::TransportGraph;
    use nav::world::NavWorld;

    use super::{available_levels, click_to_tile, default_pack_path, picker_map_window, snap};
    use crate::session::Session;

    /// A `w`×`h` all-walkable level-0 world at (0,0).
    fn open_world(w: usize, h: usize) -> NavWorld {
        NavWorld {
            collision: WorldCollision {
                origin: WorldTile {
                    x: 0,
                    z: 0,
                    level: 0,
                },
                width: w,
                height: h,
                flags: vec![0u32; w * h],
                walkable: vec![0u32; w * h],
            },
            graph: TransportGraph::default(),
        }
    }

    #[test]
    fn snap_click_to_nearest_walkable() {
        let w = open_world(3, 3);
        let t = snap(&w, 1.4, 1.4, 0).unwrap();
        assert_eq!(
            t,
            Tile {
                x: 1,
                z: 1,
                level: 0
            }
        );
    }

    #[test]
    fn snap_wall_click_lands_on_nearest_walkable() {
        // The world's x=2 tile is a wall; (1,0) wins the Chebyshev/Manhattan
        // tie over (3,0) by iteration order.
        let mut flags = vec![0u32; 5];
        flags[2] = CollisionFlag::WALK_BLOCK_FLAGS as u32;
        let w = NavWorld {
            collision: WorldCollision {
                origin: WorldTile {
                    x: 0,
                    z: 0,
                    level: 0,
                },
                width: 5,
                height: 1,
                walkable: nav::collision::derive_walkable(&flags),
                flags,
            },
            graph: TransportGraph::default(),
        };
        let t = snap(&w, 2.2, 0.1, 0).unwrap();
        assert_eq!(
            t,
            Tile {
                x: 1,
                z: 0,
                level: 0
            }
        );
    }

    #[test]
    fn snap_returns_none_on_level_without_walkables() {
        let w = open_world(3, 3);
        assert_eq!(snap(&w, 1.4, 1.4, 1), None);
    }

    #[test]
    fn available_levels_lists_only_the_level0_bake() {
        let w = open_world(3, 3);
        assert_eq!(available_levels(&w), vec![0]);
    }

    #[test]
    fn click_to_tile_maps_canvas_click_through_centre_and_scale() {
        let w = open_world(3, 3);
        // Canvas centre is the centre tile.
        let t = click_to_tile(&w, (1, 1), 10.0, [50.0, 50.0], [100.0, 100.0], 0).unwrap();
        assert_eq!(
            t,
            Tile {
                x: 1,
                z: 1,
                level: 0
            }
        );
        // 14px right of centre at 10px/tile -> (2.4, 1.4) -> snaps to (2,1).
        let t = click_to_tile(&w, (1, 1), 10.0, [64.0, 50.0], [100.0, 100.0], 0).unwrap();
        assert_eq!(
            t,
            Tile {
                x: 2,
                z: 1,
                level: 0
            }
        );
    }

    #[test]
    fn default_pack_path_follows_nav_pack_then_home_default() {
        let path = default_pack_path();
        match std::env::var("NAV_PACK") {
            Ok(p) => assert_eq!(path, PathBuf::from(p)),
            Err(_) => assert!(path.to_string_lossy().ends_with("274bot.navpack")),
        }
    }

    #[test]
    fn picker_map_window_builds_headless() {
        let _guard = crate::IMGUI_CTX_TEST_GUARD.lock().unwrap();
        let mut ctx = dear_imgui_rs::Context::create();
        ctx.prepare_frame(
            dear_imgui_rs::FramePrepareOptions::new([900.0, 700.0], 1.0 / 60.0)
                .renderer_has_textures(),
        );
        let ui = ctx.frame();
        let mut s = Session::new();
        s.walkto_open = true;
        let mut open = true;
        picker_map_window(ui, &mut s, &open_world(3, 3), &mut open);
        ctx.render();
        assert!(open, "the window must stay open until Walk is confirmed");
    }
}
