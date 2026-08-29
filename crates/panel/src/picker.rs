//! WalkTo picker: a collision-dot map window over the baked nav world.
//!
//! Draws the walkable tiles of the loaded [`NavWorld`] as amber dots inside
//! a child canvas. Click snaps to the nearest walkable tile (highlight
//! only); **Walk** arms `session.arm_walk_on` and closes. The world is
//! loaded once per process from `$NAV_PACK` or `~/.274bot/274bot.navpack`.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use api::snapshot::WorldTile;
use dear_imgui_rs::{Key, MouseButton, Ui, WindowFlags};
#[cfg(test)]
use dear_imgui_rs::Condition;
use nav::paint::{collision_at, flood_components, remaining_path_tiles};
use nav::router::Route;
use nav::tile::{chebyshev, Tile};
use nav::world::NavWorld;

use crate::nav_settings::{effective, parse_html_color, NavSettings};
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
/// Sub-tile pan remainder in millitiles, carried between drag/wheel frames
/// so a slow drag never loses its fractional tiles.
static PAN_REM_X: AtomicI32 = AtomicI32::new(0);
static PAN_REM_Z: AtomicI32 = AtomicI32::new(0);
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

/// The baked world, loaded once per process; `None` when the pack is
/// missing or corrupt. The session's scene-paint publish reads the same
/// world the picker maps.
pub(crate) fn pack() -> Option<&'static NavWorld> {
    PACK.get_or_init(|| NavWorld::load_pack(&default_pack_path()).ok())
        .as_ref()
}

/// The walkable tiles of the world on `level`, row-major (z then x): a
/// tile is a "dot" when the collision's blanket `walkable` check passes
/// (the standable test, not a directional mask).
fn world_dots(world: &NavWorld, level: i32) -> impl Iterator<Item = Tile> + '_ {
    let c = &world.collision;
    let o = c.origin;
    (0..c.height)
        .flat_map(move |z| {
            (0..c.width).map(move |x| Tile {
                x: o.x + x as i32,
                z: o.z + z as i32,
                level,
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

/// Levels with a baked plane: level 0 is the ground plane and always
/// exists; every level 1..=3 whose plane carries any collision flag (MAP
/// blocks, loc footprints, walls) is listed too. Empty planes are not —
/// this is the WalkTo level dropdown's option list.
pub fn available_levels(world: &NavWorld) -> Vec<i32> {
    let c = &world.collision;
    let plane = c.width * c.height;
    let mut levels = vec![0];
    for level in 1..4 {
        let base = level * plane;
        // The len guard keeps synthetic single-plane test worlds on [0].
        if c.flags.len() >= base + plane && c.flags[base..base + plane].iter().any(|&f| f != 0) {
            levels.push(level as i32);
        }
    }
    levels
}

/// The nearest walkable tile on the world's `level` plane to the float
/// point `(x, z)`, or the click's own tile when it is already walkable.
/// Distance is Chebyshev with Manhattan breaking ties. `None` when the
/// level is not one of the baked planes (see [`available_levels`]) or has
/// no walkable tile.
pub fn snap(world: &NavWorld, x: f32, z: f32, level: i32) -> Option<Tile> {
    if !available_levels(world).contains(&level) {
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
    world_dots(world, level).min_by_key(|t| {
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
    let (tx, tz) = world_from_canvas(centre, scale, size, click);
    snap(world, tx, tz, level)
}

/// Canvas-local px of world tile `(tx, tz)`: +x east is right, +z north is
/// up (imgui Y grows down, so z is negated). The old mapping put north at
/// the bottom (south-facing).
fn canvas_from_world(
    centre: (i32, i32),
    scale: f32,
    size: [f32; 2],
    tx: f32,
    tz: f32,
) -> [f32; 2] {
    [
        size[0] / 2.0 + (tx - centre.0 as f32) * scale,
        size[1] / 2.0 - (tz - centre.1 as f32) * scale,
    ]
}

/// Inverse of [`canvas_from_world`].
fn world_from_canvas(
    centre: (i32, i32),
    scale: f32,
    size: [f32; 2],
    click: [f32; 2],
) -> (f32, f32) {
    (
        centre.0 as f32 + (click[0] - size[0] / 2.0) / scale,
        centre.1 as f32 - (click[1] - size[1] / 2.0) / scale,
    )
}

/// Apply a pixel pan. `rem` is the leftover tile fraction in (-1, 1) from
/// previous pans, which keeps sub-tile movement instead of rounding it away.
/// Negative px moves the view the way the mouse dragged (content follows the
/// cursor): a positive `dx_px` decreases `centre.0`, matching the existing
/// `CENTRE_X -= delta/scale` sign.
pub(crate) fn pan_by(
    centre: (i32, i32),
    rem: (f32, f32),
    dx_px: f32,
    dz_px: f32,
    scale: f32,
) -> ((i32, i32), (f32, f32)) {
    let pan = |c: i32, r: f32, px: f32| {
        let move_tiles = -px / scale;
        // Split the move into whole tiles (truncated toward zero) and the
        // fractional leftover, added to the carried remainder.
        let whole = move_tiles.trunc() as i32;
        let mut c = c + whole;
        let mut r = r + move_tiles - whole as f32;
        // A remainder that reached a full tile carries into the centre so it
        // stays in (-1, 1).
        if r >= 1.0 {
            c += 1;
            r -= 1.0;
        } else if r <= -1.0 {
            c -= 1;
            r += 1.0;
        }
        (c, r)
    };
    let (cx, rx) = pan(centre.0, rem.0, dx_px);
    let (cz, rz) = pan(centre.1, rem.1, dz_px);
    ((cx, cz), (rx, rz))
}

/// WalkTo window flags: no docking, and the imgui window must not steal
/// wheel (that pans the map). `NO_SCROLLBAR` hides the bar; without
/// `NO_SCROLL_WITH_MOUSE` the window still scrolls once content overflows.
#[cfg(test)]
fn walkto_window_flags() -> WindowFlags {
    WindowFlags::NO_DOCKING | WindowFlags::NO_SCROLLBAR | WindowFlags::NO_SCROLL_WITH_MOUSE
}

/// Canvas child: same wheel capture. A default child grows a scrollbar
/// when its content rect exceeds the view (the pack-map draw list does
/// not, but an InvisibleButton fills the child so hover is the grid).
fn walkto_canvas_flags() -> WindowFlags {
    WindowFlags::NO_SCROLLBAR | WindowFlags::NO_SCROLL_WITH_MOUSE
}

/// Window-relative X so a `width`-wide cluster sits on the content-region
/// right edge. `cursor_x + avail_x` is that edge in imgui window coords.
fn right_align_x(cursor_x: f32, avail_x: f32, width: f32) -> f32 {
    cursor_x + avail_x - width
}

/// Footer labels: Teleport only on a local engine.
fn walkto_footer_labels(local: bool) -> &'static [&'static str] {
    if local {
        &["recentre", "Walk", "Teleport"]
    } else {
        &["recentre", "Walk"]
    }
}

/// Combo width on the Level/Zoom toolbar so they do not eat the row
/// (default item width is the remaining content region).
const TOOLBAR_COMBO_W: f32 = 140.0;

/// The width a text-only button of `label` occupies under the current style,
/// for right-aligning a button against the content region edge.
fn button_w(ui: &Ui, label: &str) -> f32 {
    let font = ui.current_font();
    let text = font.calc_text_size(ui.current_font_size(), f32::MAX, 0.0, label)[0];
    text + 2.0 * ui.clone_style().frame_padding()[0]
}

/// The pack-map paints of one visible tile. Only layers that are on mark
/// tiles: `blocked` fills under `collision_fill`, `path`/`transport` draw
/// the remaining route under `show_nav_path`, and `flood` (0 = player
/// seed, 1 = dest seed) colours the component under `component_flood`.
pub(crate) struct PackMapTile {
    pub tile: Tile,
    pub blocked: bool,
    pub path: bool,
    pub transport: bool,
    pub flood: Option<u32>,
}

/// The visible canvas as a tile rectangle on the selected plane:
/// `width`×`height` tiles starting at `(x0, z0)`. The plane itself is
/// passed with the paints (see [`pack_map_tiles`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PackView {
    pub x0: i32,
    pub z0: i32,
    pub width: i32,
    pub height: i32,
}

/// Flood component A (the player seed): `#0000FF`.
const FLOOD_A: [f32; 4] = [0.0, 0.0, 1.0, 1.0];
/// Flood component B (the dest seed): `#C828F0`.
const FLOOD_B: [f32; 4] = [200.0 / 255.0, 40.0 / 255.0, 240.0 / 255.0, 1.0];

/// The panel tile of a world tile (structurally identical fields).
fn tile_from(w: WorldTile) -> Tile {
    Tile {
        x: w.x,
        z: w.z,
        level: w.level,
    }
}

/// Cached flood components for a seed pair. The whole-world BFS spans
/// hundreds of thousands of tiles on the real pack (~20 ms per component),
/// so a picker frame must never re-flood; only a changed seed pair
/// recomputes.
struct FloodCache {
    /// The collision grid the sets were computed from (origin + dims).
    key: (i32, i32, usize, usize),
    seeds: Vec<WorldTile>,
    components: Vec<Arc<HashSet<WorldTile>>>,
}

static FLOOD_CACHE: Mutex<Option<FloodCache>> = Mutex::new(None);

/// The step-ok reachable sets for `seeds`, computed once per seed pair and
/// cached; a cache hit only bumps `Arc` refcounts.
fn flood_sets_for(world: &NavWorld, seeds: &[WorldTile]) -> Vec<Arc<HashSet<WorldTile>>> {
    let c = &world.collision;
    let key = (c.origin.x, c.origin.z, c.width, c.height);
    let mut cache = FLOOD_CACHE.lock().unwrap();
    let fresh = cache
        .as_ref()
        .is_some_and(|f| f.key == key && f.seeds.as_slice() == seeds);
    if !fresh {
        let components: Vec<Arc<HashSet<WorldTile>>> = flood_components(c, seeds)
            .into_iter()
            .map(Arc::new)
            .collect();
        *cache = Some(FloodCache {
            key,
            seeds: seeds.to_vec(),
            components: components.clone(),
        });
        components
    } else {
        cache.as_ref().unwrap().components.clone()
    }
}

/// Last `nav-flood` line reported on stderr, keyed by the arm generation
/// (each new arm re-reports even for the same tiles), the seed pair, and
/// the component sizes (which change when the player steps into the
/// dest's component).
static FLOOD_REPORT: Mutex<Option<(u64, WorldTile, WorldTile, usize, usize)>> = Mutex::new(None);

/// The `nav-flood` line to print this frame, `None` when nothing changed
/// since the last report. The arm generation is part of the key, so a
/// second arm with the same tiles still reports.
fn flood_report_line(
    last: Option<(u64, WorldTile, WorldTile, usize, usize)>,
    arm_gen: u64,
    player: WorldTile,
    dest: WorldTile,
    n: usize,
    m: usize,
) -> Option<String> {
    if last == Some((arm_gen, player, dest, n, m)) {
        return None;
    }
    Some(format!("nav-flood: player {n} dest {m}"))
}

/// The `(n, m)` report sizes from the cached component sets: the player
/// component size and the dest component size. Connected seeds (the dest
/// inside the player's flood) share one size.
fn flood_report_sizes(comps: &[Arc<HashSet<WorldTile>>], dest: WorldTile) -> (usize, usize) {
    let n = comps[0].len();
    let m = if comps.len() > 1 && !comps[0].contains(&dest) {
        comps[1].len()
    } else {
        n
    };
    (n, m)
}

/// `nav-flood: player {n} dest {m}` on stderr, once per arm or when the
/// component sizes change. The sizes come from the cached flood sets, so
/// an arm never runs a second world BFS.
fn report_flood_sizes(world: &NavWorld, player: WorldTile, dest: WorldTile, arm_gen: u64) {
    let comps = flood_sets_for(world, &[player, dest]);
    let (n, m) = flood_report_sizes(&comps, dest);
    let mut last = FLOOD_REPORT.lock().unwrap();
    if let Some(line) = flood_report_line(*last, arm_gen, player, dest, n, m) {
        eprintln!("{line}");
        *last = Some((arm_gen, player, dest, n, m));
    }
}

/// The pack-map paints for the visible canvas: every viewport tile a layer
/// draws (blocked collision fill, remaining path / transport hop, flood
/// component), and nothing outside the view. `route`/`here`/`dest` are the
/// focused walk arm's inputs; `layers` are the effective nav settings.
pub(crate) fn pack_map_tiles(
    world: &NavWorld,
    view: PackView,
    route: Option<&Route>,
    here: Option<WorldTile>,
    dest: Option<WorldTile>,
    layers: &NavSettings,
    level: i32,
) -> Vec<PackMapTile> {
    let path: HashMap<Tile, bool> = if layers.show_nav_path {
        route
            .map(|r| remaining_path_tiles(r, here))
            .unwrap_or_default()
            .into_iter()
            .map(|p| (tile_from(p.tile), p.transport))
            .collect()
    } else {
        HashMap::new()
    };
    let seeds: Vec<WorldTile> = if layers.component_flood {
        [here, dest].into_iter().flatten().collect()
    } else {
        Vec::new()
    };
    let floods = if seeds.is_empty() {
        Vec::new()
    } else {
        flood_sets_for(world, &seeds)
    };
    let mut out = Vec::new();
    for z in view.z0..view.z0 + view.height {
        for x in view.x0..view.x0 + view.width {
            let t = Tile { x, z, level };
            let wt = WorldTile { x, z, level };
            let blocked = layers.collision_fill && collision_at(&world.collision, wt).blocked;
            let (is_path, transport) = path
                .get(&t)
                .map(|&tr| (true, tr))
                .unwrap_or((false, false));
            let flood = if floods.is_empty() {
                None
            } else {
                floods
                    .iter()
                    .position(|f| f.contains(&wt))
                    .map(|i| i as u32)
            };
            if blocked || is_path || flood.is_some() {
                out.push(PackMapTile {
                    tile: t,
                    blocked,
                    path: is_path,
                    transport,
                    flood,
                });
            }
        }
    }
    out
}

/// The focused walk arm's route clone, `None` when nothing is armed.
fn focused_route(session: &Session) -> Option<Route> {
    let name = session.focused_name()?;
    session
        .travellers
        .lock()
        .unwrap()
        .get(&name)
        .cloned()?
        .lock()
        .unwrap()
        .route
        .clone()
}

/// `[u8; 3]` to an opaque RGBA float colour for the draw list.
fn color_rgb([r, g, b]: [u8; 3]) -> [f32; 4] {
    [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0]
}

/// Call when the Game pane is not showing WalkTo so the next open resets
/// the view (the WalkTo chrome button toggles without running the body).
pub fn note_closed() {
    PREV_OPEN.store(false, Ordering::Relaxed);
}

/// WalkTo map in the Game pane (same dock space as the applet).
pub fn draw_picker(ui: &Ui, session: &mut Session) {
    match pack() {
        Some(world) => picker_map_body(ui, session, world),
        None => {
            ui.text_wrapped("no nav pack — run nav-pack");
        }
    }
    if !session.walkto_open {
        PREV_OPEN.store(false, Ordering::Relaxed);
        session.picker_sel = None;
    }
}

/// The collision-dot map window. `open` is the window's live open flag;
/// confirm Walk closes it. Headless tests wrap the body in a window.
#[cfg(test)]
fn picker_map_window(ui: &Ui, session: &mut Session, world: &NavWorld, open: &mut bool) {
    let _ = ui
        .window("WalkTo")
        .opened(open)
        .flags(walkto_window_flags())
        .size([720.0, 560.0], Condition::FirstUseEver)
        .size_constraints([480.0, 360.0], [f32::MAX, f32::MAX])
        .build(|| {
            picker_map_body(ui, session, world);
        });
}

/// Toolbar, canvas, and footer. Used inside the Game pane and the test window.
fn picker_map_body(ui: &Ui, session: &mut Session, world: &NavWorld) {
    // Reset the view when the picker opens fresh.
    if !PREV_OPEN.swap(true, Ordering::Relaxed) {
        let (cx, cz) = session.focused_tile().unwrap_or(DEFAULT_CENTRE);
        CENTRE_X.store(cx, Ordering::Relaxed);
        CENTRE_Z.store(cz, Ordering::Relaxed);
        PAN_REM_X.store(0, Ordering::Relaxed);
        PAN_REM_Z.store(0, Ordering::Relaxed);
        LEVEL.store(available_levels(world)[0], Ordering::Relaxed);
        session.picker_sel = None;
    }
    let levels = available_levels(world);
    let mut lvl_idx = levels
        .iter()
        .position(|l| *l == LEVEL.load(Ordering::Relaxed))
        .unwrap_or(0);
    ui.set_next_item_width(TOOLBAR_COMBO_W);
    if ui.combo("##walkto-level", &mut lvl_idx, &levels, |l: &i32| {
        Cow::Owned(format!("level {l}"))
    }) {
        LEVEL.store(levels[lvl_idx], Ordering::Relaxed);
    }
    ui.same_line();
    let mut zoom = ZOOM.load(Ordering::Relaxed) as usize;
    ui.set_next_item_width(TOOLBAR_COMBO_W);
    if ui.combo("##walkto-zoom", &mut zoom, &ZOOMS, |z: &f32| {
        Cow::Owned(format!("{z:.0}px/tile"))
    }) {
        ZOOM.store(zoom as i32, Ordering::Relaxed);
    }
    let footer_h = ui.frame_height() + ui.clone_style().item_spacing()[1];
    let avail = ui.content_region_avail();
    let canvas_h = (avail[1] - footer_h).max(120.0);
    draw_canvas(ui, session, world, canvas_h);
    match session.picker_sel {
        Some(t) => ui.text_disabled(format!("selected {} {} {}", t.x, t.z, t.level)),
        None => ui.text_disabled("click a tile, then Walk"),
    }
    let spacing = ui.clone_style().item_spacing()[0];
    let local = session.debug_ui();
    let labels = walkto_footer_labels(local);
    let cluster = labels
        .iter()
        .map(|l| button_w(ui, l))
        .sum::<f32>()
        + spacing * (labels.len().saturating_sub(1) as f32);
    let x = right_align_x(
        ui.cursor_pos()[0],
        ui.content_region_avail()[0],
        cluster,
    );
    ui.same_line_with_pos(x);
    if ui.button("recentre") {
        let (cx, cz) = session.focused_tile().unwrap_or(DEFAULT_CENTRE);
        CENTRE_X.store(cx, Ordering::Relaxed);
        CENTRE_Z.store(cz, Ordering::Relaxed);
        PAN_REM_X.store(0, Ordering::Relaxed);
        PAN_REM_Z.store(0, Ordering::Relaxed);
    }
    ui.same_line();
    let can_walk = session.picker_sel.is_some();
    let _off = ui.begin_disabled_with_cond(!can_walk);
    if ui.button("Walk") && can_walk && session.confirm_picker_walk(world) {
        session.walkto_open = false;
        PREV_OPEN.store(false, Ordering::Relaxed);
    }
    if local {
        ui.same_line();
        if ui.button("Teleport") {
            if let Some(t) = session.picker_sel.take() {
                session.cheat_focused(&crate::session::walkto_tele_cmd(t));
                session.walkto_open = false;
                PREV_OPEN.store(false, Ordering::Relaxed);
            }
        }
    }
}

/// The child canvas: amber dots, drag-to-pan, wheel-to-pan, click-to-select
/// (does not arm).
fn draw_canvas(ui: &Ui, session: &mut Session, world: &NavWorld, height: f32) {
    let mut rect: Option<([f32; 2], [f32; 2])> = None;
    let mut pick: Option<[f32; 2]> = None;
    let mut hovered = false;
    ui.child_window("##walkto-canvas")
        .size([0.0, height])
        .flags(walkto_canvas_flags())
        .build(ui, || {
            let draw = ui.get_window_draw_list();
            let origin = ui.cursor_screen_pos();
            let size = ui.content_region_avail();
            // Fill the child so its content size matches the view. Hit
            // testing is the painted rect (`is_mouse_hovering_rect`): the
            // InvisibleButton's `is_item_hovered` is false when imgui's
            // hovered window is the parent Game/WalkTo pane, not this
            // child — that is why click-to-pick never fired.
            ui.invisible_button("##walkto-hit", size);
            let (min, max) = (origin, [origin[0] + size[0], origin[1] + size[1]]);
            hovered = ui.is_mouse_hovering_rect(min, max);
            if hovered
                && ui.is_mouse_clicked(MouseButton::Left)
                && !ui.is_mouse_dragging_with_threshold(MouseButton::Left, 5.0)
            {
                pick = Some(ui.io().mouse_pos());
            }
            rect = Some((min, max));
            let (cx, cz) = (
                CENTRE_X.load(Ordering::Relaxed) as f32,
                CENTRE_Z.load(Ordering::Relaxed) as f32,
            );
            let scale = ZOOMS[ZOOM.load(Ordering::Relaxed) as usize];
            let centre_i = (
                CENTRE_X.load(Ordering::Relaxed),
                CENTRE_Z.load(Ordering::Relaxed),
            );
            // Only draw tiles inside the visible window (north-up).
            let (wx0, wx1) = (cx - size[0] / 2.0 / scale, cx + size[0] / 2.0 / scale);
            let (wz0, wz1) = (cz - size[1] / 2.0 / scale, cz + size[1] / 2.0 / scale);
            let dot = (scale * 0.72).clamp(1.5, 5.0);
            let sel = session.picker_sel;
            // The visible tile rectangle and the pack-map paints inside it.
            // Layers paint only these tiles; the bake outside the view is
            // never iterated.
            let layers = effective(&session.ui.nav, session.nav_live_force_layers);
            let any_layer = layers.collision_fill || layers.show_nav_path || layers.component_flood;
            let level = LEVEL.load(Ordering::Relaxed);
            let here = session.focused_tile().map(|(x, z)| WorldTile { x, z, level });
            let dest = session.walk_dest.map(|t| WorldTile { x: t.x, z: t.z, level });
            let view = PackView {
                x0: wx0.ceil() as i32,
                z0: wz0.ceil() as i32,
                width: (wx1.floor() as i32 - wx0.ceil() as i32 + 1).max(0),
                height: (wz1.floor() as i32 - wz0.ceil() as i32 + 1).max(0),
            };
            let paints = if any_layer {
                pack_map_tiles(
                    world,
                    view,
                    focused_route(session).as_ref(),
                    here,
                    dest,
                    &layers,
                    level,
                )
            } else {
                Vec::new()
            };
            let painted: HashSet<Tile> = paints.iter().map(|p| p.tile).collect();
            if layers.component_flood {
                if let (Some(h), Some(d)) = (here, dest) {
                    report_flood_sizes(world, h, d, session.route_gen());
                }
            }
            let path_col = color_rgb(parse_html_color(&layers.color_path, [255, 0, 0]));
            let transport_col = color_rgb(parse_html_color(&layers.color_transport, [0, 255, 0]));
            let collision_col = color_rgb(parse_html_color(&layers.color_collision, [0, 128, 255]));
            // Layer fills: the route wins over the flood region, the flood
            // over the blocked ground (a blocked tile is never on a route
            // or in a flood).
            for pt in &paints {
                let t = pt.tile;
                let (tx, tz) = (t.x as f32, t.z as f32);
                let [sx, sy] = canvas_from_world(centre_i, scale, size, tx, tz);
                let [sx1, sy1] =
                    canvas_from_world(centre_i, scale, size, tx + 1.0, tz + 1.0);
                let (x0, y0) = (sx.min(sx1) + min[0], sy.min(sy1) + min[1]);
                let (x1, y1) = (sx.max(sx1) + min[0], sy.max(sy1) + min[1]);
                let color = if pt.path {
                    if pt.transport {
                        transport_col
                    } else {
                        path_col
                    }
                } else if let Some(id) = pt.flood {
                    if id == 0 {
                        FLOOD_A
                    } else {
                        FLOOD_B
                    }
                } else {
                    debug_assert!(pt.blocked, "pack_map_tiles returns only painted tiles");
                    collision_col
                };
                draw.add_rect([x0, y0], [x1, y1], color)
                    .filled(true)
                    .build();
                if sel.is_some_and(|s| s == t) {
                    let d = (dot + 2.0).min(scale.max(3.0));
                    let h = d / 2.0;
                    let [mx, my] =
                        canvas_from_world(centre_i, scale, size, tx + 0.5, tz + 0.5);
                    draw.add_rect(
                        [min[0] + mx - h, min[1] + my - h],
                        [min[0] + mx + h, min[1] + my + h],
                        TEXT,
                    )
                    .filled(true)
                    .build();
                }
            }
            // Amber dots: walkable view tiles no layer coloured.
            for z in view.z0..view.z0 + view.height {
                for x in view.x0..view.x0 + view.width {
                    let t = Tile { x, z, level };
                    if !world
                        .collision
                        .walkable(WorldTile { x, z, level })
                        || (any_layer && painted.contains(&t))
                    {
                        continue;
                    }
                    let (tx, tz) = (t.x as f32, t.z as f32);
                    let selected = sel.is_some_and(|s| s == t);
                    let (color, d) = if selected {
                        (TEXT, (dot + 2.0).min(scale.max(3.0)))
                    } else {
                        (ACCENT, dot)
                    };
                    let h = d / 2.0;
                    let [mx, my] =
                        canvas_from_world(centre_i, scale, size, tx + 0.5, tz + 0.5);
                    draw.add_rect(
                        [min[0] + mx - h, min[1] + my - h],
                        [min[0] + mx + h, min[1] + my + h],
                        color,
                    )
                    .filled(true)
                    .build();
                }
            }
        });
    let Some((min, max)) = rect else {
        return;
    };
    if !hovered {
        return;
    }
    let scale = ZOOMS[ZOOM.load(Ordering::Relaxed) as usize];
    if ui.is_mouse_dragging_with_threshold(MouseButton::Left, 5.0) {
        let delta = ui.io().mouse_delta();
        let (centre, rem) = pan_by(
            (
                CENTRE_X.load(Ordering::Relaxed),
                CENTRE_Z.load(Ordering::Relaxed),
            ),
            (
                PAN_REM_X.load(Ordering::Relaxed) as f32 / 1000.0,
                PAN_REM_Z.load(Ordering::Relaxed) as f32 / 1000.0,
            ),
            delta[0],
            -delta[1],
            scale,
        );
        CENTRE_X.store(centre.0, Ordering::Relaxed);
        CENTRE_Z.store(centre.1, Ordering::Relaxed);
        PAN_REM_X.store((rem.0 * 1000.0) as i32, Ordering::Relaxed);
        PAN_REM_Z.store((rem.1 * 1000.0) as i32, Ordering::Relaxed);
        return;
    }
    // Wheel pans over the canvas: the vertical wheel always moves z, the
    // horizontal wheel moves x. Shift+vertical wheel also adds to x (mice
    // with no horizontal wheel) without cancelling z; both axes may apply in
    // the same frame (trackpad diagonal). 16px per notch keeps the step in
    // tiles zoom-independent (`tiles_per_notch = 16 / scale`).
    let wheel = ui.io().mouse_wheel();
    let wheel_h = ui.io().mouse_wheel_h();
    if wheel != 0.0 || wheel_h != 0.0 {
        let shift = ui.is_key_down(Key::LeftShift) || ui.is_key_down(Key::RightShift);
        let tiles_per_notch = 16.0 / scale;
        let (centre, rem) = pan_by(
            (
                CENTRE_X.load(Ordering::Relaxed),
                CENTRE_Z.load(Ordering::Relaxed),
            ),
            (
                PAN_REM_X.load(Ordering::Relaxed) as f32 / 1000.0,
                PAN_REM_Z.load(Ordering::Relaxed) as f32 / 1000.0,
            ),
            (wheel_h + if shift { wheel } else { 0.0 }) * tiles_per_notch * scale,
            -wheel * tiles_per_notch * scale,
            scale,
        );
        CENTRE_X.store(centre.0, Ordering::Relaxed);
        CENTRE_Z.store(centre.1, Ordering::Relaxed);
        PAN_REM_X.store((rem.0 * 1000.0) as i32, Ordering::Relaxed);
        PAN_REM_Z.store((rem.1 * 1000.0) as i32, Ordering::Relaxed);
    }
    let Some(mouse) = pick else {
        return;
    };
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

    use super::{
        available_levels, click_to_tile, default_pack_path, pack_map_tiles, pan_by,
        picker_map_window, right_align_x, snap, walkto_canvas_flags, walkto_footer_labels,
        walkto_window_flags, PackView,
    };
    use dear_imgui_rs::WindowFlags;
    use crate::nav_settings::NavSettings;
    use crate::session::Session;
    use nav::router::{Leg, Route};

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
    fn available_levels_lists_planes_with_content() {
        let w = open_world(3, 3);
        // An all-walkable single-plane world lists the ground plane only.
        assert_eq!(available_levels(&w), vec![0]);
        // A 4-plane world with a stamped level-1 plane lists it too.
        let mut flags = vec![0u32; 4 * 9];
        flags[9 + 4] = CollisionFlag::WALK_SCENERY as u32;
        let w2 = NavWorld {
            collision: WorldCollision {
                origin: WorldTile {
                    x: 0,
                    z: 0,
                    level: 0,
                },
                width: 3,
                height: 3,
                walkable: nav::collision::derive_walkable(&flags),
                flags,
            },
            graph: TransportGraph::default(),
        };
        assert_eq!(available_levels(&w2), vec![0, 1]);
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
        // 14px right of centre at 10px/tile -> east (higher x).
        let t = click_to_tile(&w, (1, 1), 10.0, [64.0, 50.0], [100.0, 100.0], 0).unwrap();
        assert_eq!(
            t,
            Tile {
                x: 2,
                z: 1,
                level: 0
            }
        );
        // 20px below centre is south (lower z): north is up on the canvas.
        let t = click_to_tile(&w, (1, 1), 10.0, [50.0, 70.0], [100.0, 100.0], 0).unwrap();
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
    fn pan_by_accumulates_sub_tile_remainder_at_fine_and_coarse_zoom() {
        // 2px/tile: a 3px drag is 1.5 tiles; the whole tile lands now and the
        // half tile stays in the remainder. Sign pins "content follows the
        // cursor": positive mouse dx pans west (smaller centre.x), matching
        // the existing `CENTRE_X -= delta/scale`.
        let ((cx, _cz), rem) = pan_by((3200, 3200), (0.0, 0.0), -3.0, 0.0, 2.0);
        assert_eq!(cx, 3201);
        assert!((rem.0 - 0.5).abs() < 1e-5);
        let ((cx, _), rem) = pan_by((3200, 3200), (0.0, 0.0), 3.0, 0.0, 2.0);
        assert_eq!(cx, 3199);
        assert!((rem.0 + 0.5).abs() < 1e-5);
        // The leftover half tile plus another 1px (0.5 tile) completes a
        // second tile.
        let ((cx2, _), rem2) = pan_by((cx, 3200), rem, 1.0, 0.0, 2.0);
        assert_eq!(cx2, 3198);
        assert!(rem2.0.abs() < 1e-5);
        // 16px/tile: an 8px drag is 0.5 tile — the centre must not stay stuck
        // waiting for a full tile.
        let (c, rem) = pan_by((3200, 3200), (0.0, 0.0), 8.0, 0.0, 16.0);
        assert_eq!(c.0, 3200);
        assert!(rem.0.abs() > 0.4);
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
    fn walkto_window_flags_capture_wheel_on_the_canvas() {
        let w = walkto_window_flags();
        assert!(
            w.contains(WindowFlags::NO_SCROLLBAR),
            "hide the imgui window scrollbar"
        );
        assert!(
            w.contains(WindowFlags::NO_SCROLL_WITH_MOUSE),
            "wheel over WalkTo must pan the map, not scroll the window"
        );
        let c = walkto_canvas_flags();
        assert!(c.contains(WindowFlags::NO_SCROLLBAR));
        assert!(c.contains(WindowFlags::NO_SCROLL_WITH_MOUSE));
    }

    #[test]
    fn right_align_x_sits_the_cluster_on_the_content_edge() {
        // cursor 12, 400px remaining, 80px cluster → 332.
        assert_eq!(right_align_x(12.0, 400.0, 80.0), 332.0);
    }

    #[test]
    fn walkto_footer_adds_teleport_on_local_engine() {
        assert_eq!(walkto_footer_labels(false), &["recentre", "Walk"]);
        assert_eq!(
            walkto_footer_labels(true),
            &["recentre", "Walk", "Teleport"]
        );
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

    fn picker_click_frame(
        ctx: &mut dear_imgui_rs::Context,
        session: &mut Session,
        world: &NavWorld,
        mouse: [f32; 2],
        left_down: bool,
    ) {
        ctx.prepare_frame(
            dear_imgui_rs::FramePrepareOptions::new([900.0, 700.0], 1.0 / 60.0)
                .renderer_has_textures(),
        );
        ctx.io_mut().add_mouse_pos_event(mouse);
        ctx.io_mut()
            .add_mouse_button_event(dear_imgui_rs::MouseButton::Left, left_down);
        {
            let ui = ctx.frame();
            let mut open = true;
            picker_map_window(ui, session, world, &mut open);
        }
        ctx.render();
    }

    #[test]
    fn picker_click_selects_a_walkable_tile() {
        let _guard = crate::IMGUI_CTX_TEST_GUARD.lock().unwrap();
        super::note_closed();
        let mut ctx = dear_imgui_rs::Context::create();
        let world = open_world(3, 3);
        let mut s = Session::new();
        s.walkto_open = true;
        // FirstUseEver WalkTo is 720×560 at the default imgui origin; the
        // canvas sits under the toolbar. A click in the window interior
        // must set picker_sel — not pan, not miss the hit target.
        let mouse = [360.0, 320.0];
        picker_click_frame(&mut ctx, &mut s, &world, mouse, false);
        picker_click_frame(&mut ctx, &mut s, &world, mouse, true);
        picker_click_frame(&mut ctx, &mut s, &world, mouse, false);
        assert!(
            s.picker_sel.is_some(),
            "click on the WalkTo canvas must snap a tile, got {:?}",
            s.picker_sel
        );
    }

    /// A `w`×`h` level-0 bake at (0,0) with the given per-tile flags OR'd in.
    fn bake_world(w: usize, h: usize, extras: &[(i32, i32, u32)]) -> NavWorld {
        let mut flags = vec![0u32; w * h];
        for &(x, z, f) in extras {
            flags[z as usize * w + x as usize] |= f;
        }
        NavWorld {
            collision: WorldCollision {
                origin: WorldTile {
                    x: 0,
                    z: 0,
                    level: 0,
                },
                width: w,
                height: h,
                flags: flags.clone(),
                walkable: nav::collision::derive_walkable(&flags),
            },
            graph: TransportGraph::default(),
        }
    }

    /// A 7×7 bake: a 3×3 open corner plus an isolated open tile moated by
    /// WR_GRND; everything else blocked ground.
    fn disconnected_world() -> NavWorld {
        let mut extras = Vec::new();
        for z in 0..7 {
            for x in 0..7 {
                let open = (x < 3 && z < 3) || (x == 5 && z == 5);
                if !open {
                    extras.push((x, z, CollisionFlag::WR_GRND as u32));
                }
            }
        }
        bake_world(7, 7, &extras)
    }

    #[test]
    fn pack_map_collision_only_in_viewport() {
        // A 5-wide bake with a WR_GRND wall at x=2; the small view covers
        // the wall and its open neighbour.
        let world = bake_world(5, 1, &[(2, 0, CollisionFlag::WR_GRND as u32)]);
        let view = PackView {
            x0: 1,
            z0: 0,
            width: 2,
            height: 1,
        };
        let layers = NavSettings {
            collision_fill: true,
            ..Default::default()
        };
        let tiles = pack_map_tiles(&world, view, None, None, None, &layers, 0);
        let in_view = |t: super::Tile| {
            t.x >= view.x0
                && t.x < view.x0 + view.width
                && t.z >= view.z0
                && t.z < view.z0 + view.height
        };
        assert!(tiles.iter().all(|t| in_view(t.tile)));
        assert!(tiles.iter().any(|t| t.blocked));
    }

    #[test]
    fn pack_map_paints_the_selected_plane() {
        // A 3x3 two-plane world: level 0 is open, level 1 carries a WR_GRND
        // wall at (1,1). Painting must read the passed level, not the bake's
        // origin plane.
        let mut flags = vec![0u32; 2 * 9];
        flags[9 + 1 * 3 + 1] = CollisionFlag::WR_GRND as u32;
        let world = NavWorld {
            collision: WorldCollision {
                origin: WorldTile {
                    x: 0,
                    z: 0,
                    level: 0,
                },
                width: 3,
                height: 3,
                walkable: nav::collision::derive_walkable(&flags),
                flags,
            },
            graph: TransportGraph::default(),
        };
        let view = PackView {
            x0: 0,
            z0: 0,
            width: 3,
            height: 3,
        };
        let layers = NavSettings {
            collision_fill: true,
            ..Default::default()
        };
        let tiles = pack_map_tiles(&world, view, None, None, None, &layers, 1);
        assert!(
            tiles.iter().all(|p| p.tile.level == 1),
            "paints live on the selected plane, not the bake's origin plane"
        );
        assert!(
            tiles
                .iter()
                .any(|p| p.blocked && p.tile.x == 1 && p.tile.z == 1),
            "the level-1 wall blocks its own plane"
        );
    }

    #[test]
    fn pack_map_flood_marks_two_components() {
        let layers = NavSettings {
            component_flood: true,
            ..Default::default()
        };
        let tiles = pack_map_tiles(
            &disconnected_world(),
            PackView {
                x0: 0,
                z0: 0,
                width: 7,
                height: 7,
            },
            None,
            Some(WorldTile {
                x: 0,
                z: 0,
                level: 0,
            }),
            Some(WorldTile {
                x: 5,
                z: 5,
                level: 0,
            }),
            &layers,
            0,
        );
        let ids: std::collections::HashSet<_> = tiles.iter().filter_map(|t| t.flood).collect();
        assert_eq!(ids.len(), 2, "the player and dest components both flood");
    }

    #[test]
    fn pack_map_marks_remaining_path_in_view() {
        let world = bake_world(5, 1, &[]);
        let tiles: Vec<WorldTile> = (0..5)
            .map(|x| WorldTile { x, z: 0, level: 0 })
            .collect();
        let route = Route {
            dest: tiles[4],
            legs: vec![Leg::Walk { tiles: tiles.clone() }],
            ticks: 0.0,
        };
        let layers = NavSettings {
            show_nav_path: true,
            ..Default::default()
        };
        let view = PackView {
            x0: 0,
            z0: 0,
            width: 5,
            height: 1,
        };
        let marks = pack_map_tiles(&world, view, Some(&route), None, None, &layers, 0);
        let path: Vec<i32> = marks.iter().filter(|t| t.path).map(|t| t.tile.x).collect();
        assert_eq!(path, vec![0, 1, 2, 3, 4]);
        assert!(marks.iter().all(|t| !t.transport), "a walk-only route has no hops");
    }

    #[test]
    fn pack_map_flood_report_line_reports_each_arm() {
        let player = WorldTile { x: 0, z: 0, level: 0 };
        let dest = WorldTile { x: 5, z: 5, level: 0 };
        let first = super::flood_report_line(None, 1, player, dest, 9, 1).unwrap();
        assert_eq!(first, "nav-flood: player 9 dest 1");
        // A second arm with the same tiles still reports: the arm
        // generation is part of the report key.
        let second = super::flood_report_line(Some((1, player, dest, 9, 1)), 2, player, dest, 9, 1)
            .unwrap();
        assert_eq!(second, first);
        // The same arm and sizes do not re-report; a size change does.
        assert!(
            super::flood_report_line(Some((2, player, dest, 9, 1)), 2, player, dest, 9, 1).is_none()
        );
        assert!(
            super::flood_report_line(Some((2, player, dest, 9, 1)), 2, player, dest, 9, 9).is_some()
        );
    }

    #[test]
    fn pack_map_flood_report_sizes_from_cached_sets() {
        let world = disconnected_world();
        let player = WorldTile { x: 0, z: 0, level: 0 };
        let dest = WorldTile { x: 5, z: 5, level: 0 };
        let comps = super::flood_sets_for(&world, &[player, dest]);
        assert_eq!(
            super::flood_report_sizes(&comps, dest),
            (9, 1),
            "the 3x3 corner and the isolated dest tile"
        );
        // Connected seeds (the dest inside the player's flood) share one size.
        let inside = WorldTile { x: 1, z: 1, level: 0 };
        let comps = super::flood_sets_for(&world, &[player, inside]);
        assert_eq!(super::flood_report_sizes(&comps, inside), (9, 9));
    }
}
