//! WalkTo picker: native map window over the baked nav world.
//!
//! Terrain and optional grid/collision/reach layers are drawn by the app-owned
//! [`crate::walk_map::WalkMapRenderer`]. Click, search, **Walk**, and **Teleport**
//! go through [`host_play::walk_map::MapModel`] on `Session`; confirmation
//! consumes the pending selection once. Walk needs a snapped target; debug
//! Teleport uses the requested tile when Local+loopback is authorized.
//! Missing origin or focus still refuse.
//! The world is the session's [`Play`] world, injected once via [`set_pack`] — the
//! picker never decodes the pack itself.

use std::borrow::Cow;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use api::snapshot::WorldTile;
use dear_imgui_rs::{Condition, Key, MouseButton, Ui, WindowFlags};
use host_play::walk_map::{
    select_route_source, ActionError, MapModel, RouteProjection, RouteSource, Selection,
};
use nav::map::spatial::GameTile;
use nav::paint::{bake_reach, flood_components, remaining_path_tiles};
use nav::router::Route;
use nav::tile::Tile;
use nav::world::NavWorld;

use crate::game_view::FrameGpu;
use crate::session::Session;
use crate::walk_map::{
    overlay_colors, snap_tile, view_from_canvas, OverlayLayers, WalkMapRenderer, MAX_LABELS,
    NSEW_PPT,
};

/// Default picker centre: the Lumbridge courtyard when the player's tile is
/// unknown.
const DEFAULT_CENTRE: (i32, i32) = (3220, 3220);
/// Zoom steps in pixels per tile. Coarse pyramid steps (0.25/0.5/1) sit in
/// front of the existing fine 2/4/8/16 choices. Default remains 2 px/tile.
const ZOOMS: [f32; 7] = [0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0];
const DEFAULT_ZOOM: i32 = 3;

/// The baked world shared with the session's [`Play`], injected by
/// [`set_pack`]; `None` when no play world is attached (the picker then
/// shows the run-nav-pack hint).
static PACK: Mutex<Option<Arc<NavWorld>>> = Mutex::new(None);
/// Persistent picker view state (survives frames, not the process).
static CENTRE_X: AtomicI32 = AtomicI32::new(DEFAULT_CENTRE.0);
static CENTRE_Z: AtomicI32 = AtomicI32::new(DEFAULT_CENTRE.1);
/// Sub-tile pan remainder in millitiles, carried between drag/wheel frames
/// so a slow drag never loses its fractional tiles.
static PAN_REM_X: AtomicI32 = AtomicI32::new(0);
static PAN_REM_Z: AtomicI32 = AtomicI32::new(0);
static LEVEL: AtomicI32 = AtomicI32::new(0);
static ZOOM: AtomicI32 = AtomicI32::new(DEFAULT_ZOOM);
/// True while the picker window was drawn last frame; drives the view reset
/// when it opens fresh.
static PREV_OPEN: AtomicBool = AtomicBool::new(false);

/// Attach the session's nav world (one `Arc` shared with [`Play`]'s slots);
/// `None` detaches when the play is dropped. The picker never decodes the
/// pack itself — this is the only source of the world it maps. Any mapped
/// flags sidecar is dropped: the next collision paint re-decodes for the
/// new world's grid instead of indexing stale geometry. The reach bake is
/// dropped too — it answers the new world's transport network.
pub fn set_pack(world: Option<Arc<NavWorld>>) {
    *PACK.lock().unwrap() = world;
    drop_flags_sidecar();
    *REACH.lock().unwrap() = None;
    release_map_leases();
}

/// The attached nav world; `None` when no play world is set. The returned
/// `Arc` keeps the world alive for the caller, so the picker and the
/// session paint the same bake without a second decode.
pub(crate) fn pack() -> Option<Arc<NavWorld>> {
    PACK.lock().unwrap().clone()
}

/// The raw baked collision flags decoded from the `.navflags` sidecar,
/// plus the grid header they were decoded for. Loaded once while a
/// collision paint toggle (`collision_fill`/`nsew_labels`) is on and
/// dropped when both go off; the paint only applies them to a
/// `WorldCollision` with the same geometry. A side table so the shared
/// [`NavWorld`] `Arc` stays immutable for the router and the walk grid
/// is never cloned.
struct FlagSidecar {
    origin: WorldTile,
    width: usize,
    height: usize,
    flags: Arc<[u32]>,
}

/// The session's decoded flags sidecar; `Unloaded` until paint-on.
enum FlagsSlot {
    Unloaded,
    Missing,
    Refused(#[allow(dead_code)] &'static str),
    Loaded(FlagSidecar),
}

/// The session's decoded flags sidecar; `Unloaded` while no collision paint
/// is on (see [`FlagSidecar`]).
static FLAGS: Mutex<FlagsSlot> = Mutex::new(FlagsSlot::Unloaded);
/// Immutable process-profile path installed before any panel session starts.
static BOUND_NAV_FLAGS: Mutex<Option<PathBuf>> = Mutex::new(None);
/// Expected flags digest from the selected nav identity. `None` means the
/// identity does not name flags, so a sidecar cannot be applied.
static EXPECTED_FLAGS_SHA256: Mutex<Option<String>> = Mutex::new(None);
/// True when the bound flags path is the build-stamped sibling of a bundled
/// pack (not an explicit `--nav-flags` / `NAV_FLAGS` override). Trusted
/// provenance skips runtime content hashing on first paint.
static FLAGS_TRUSTED_BUNDLED: AtomicBool = AtomicBool::new(false);
/// Content-hash attempts while loading the flags sidecar. Tests assert the
/// bundled fast path never increments this; external overrides always do.
static FLAGS_CONTENT_HASHES: AtomicU32 = AtomicU32::new(0);

/// Bind the process-profile flags path, expected digest, and provenance.
/// `trusted_bundled` is true only for the build-stamped sibling of a bundled
/// pack; an explicit flags override must pass `false` even when its path
/// equals that sibling.
pub(crate) fn set_navflags_binding(
    path: PathBuf,
    flags_sha256: Option<String>,
    trusted_bundled: bool,
) {
    *BOUND_NAV_FLAGS.lock().unwrap() = Some(path);
    *EXPECTED_FLAGS_SHA256.lock().unwrap() = flags_sha256;
    FLAGS_TRUSTED_BUNDLED.store(trusted_bundled, Ordering::Relaxed);
    drop_flags_sidecar();
}

/// The flags sidecar path: `$NAV_FLAGS`, else the pack path with its
/// extension swapped to `.navflags` (the `nav-pack` write target).
pub(crate) fn navflags_path() -> PathBuf {
    if let Some(path) = BOUND_NAV_FLAGS.lock().unwrap().clone() {
        return path;
    }
    match std::env::var("NAV_FLAGS") {
        Ok(p) => PathBuf::from(p),
        Err(_) => host_play::default_pack_path().with_extension("navflags"),
    }
}

/// Decode sidecar bytes into a [`FlagSidecar`]; `None` when decode fails.
fn decode_sidecar_bytes(bytes: &[u8]) -> Option<FlagSidecar> {
    let (origin, width, height, flags) = nav::pack::decode_flags_sidecar(bytes).ok()?;
    Some(FlagSidecar {
        origin,
        width,
        height,
        flags: flags.into(),
    })
}

/// Decode the sidecar file at `path` into a [`FlagSidecar`]; `None` when
/// the file is missing or fails the sidecar decode (the paint then falls
/// back to the walk word).
#[cfg(test)]
fn decode_sidecar_file(path: &PathBuf) -> Option<FlagSidecar> {
    let bytes = std::fs::read(path).ok()?;
    decode_sidecar_bytes(&bytes)
}

/// Decode the flags sidecar once while a collision paint is on; no-op
/// when already attempted for this paint-on. Missing, unknown, or
/// mismatched identity falls back to the walk word and is not retried
/// until the sidecar is dropped. Build-stamped bundled flags skip the
/// content hash; external overrides always validate against the digest.
pub(crate) fn ensure_flags_sidecar() {
    if !matches!(*FLAGS.lock().unwrap(), FlagsSlot::Unloaded) {
        return;
    }
    let expected = EXPECTED_FLAGS_SHA256.lock().unwrap().clone();
    let path = navflags_path();
    let trusted_bundled = FLAGS_TRUSTED_BUNDLED.load(Ordering::Relaxed);
    let slot = match expected {
        None => FlagsSlot::Refused("flags identity is unknown"),
        Some(expected) => match std::fs::read(&path) {
            Err(_) => FlagsSlot::Missing,
            Ok(bytes) => {
                let identity_ok = if trusted_bundled {
                    true
                } else {
                    FLAGS_CONTENT_HASHES.fetch_add(1, Ordering::Relaxed);
                    nav::manifest::hash_bytes(&bytes) == expected
                };
                if !identity_ok {
                    FlagsSlot::Refused("flags sidecar hash mismatch")
                } else {
                    match decode_sidecar_bytes(&bytes) {
                        Some(sidecar) => FlagsSlot::Loaded(sidecar),
                        None => FlagsSlot::Refused("flags sidecar is unreadable"),
                    }
                }
            }
        },
    };
    let mut guard = FLAGS.lock().unwrap();
    if matches!(*guard, FlagsSlot::Unloaded) {
        *guard = slot;
    }
}

/// Drop the decoded sidecar (both collision toggles off); the next
/// paint-on re-decodes.
pub(crate) fn drop_flags_sidecar() {
    *FLAGS.lock().unwrap() = FlagsSlot::Unloaded;
}

#[cfg(test)]
pub(crate) fn flags_content_hash_count() -> u32 {
    FLAGS_CONTENT_HASHES.load(Ordering::Relaxed)
}

#[cfg(test)]
pub(crate) fn reset_flags_content_hash_count() {
    FLAGS_CONTENT_HASHES.store(0, Ordering::Relaxed);
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FlagsSidecarState {
    Unloaded,
    Missing,
    Refused(&'static str),
    Applied,
}

#[cfg(test)]
pub(crate) fn flags_sidecar_state() -> FlagsSidecarState {
    match &*FLAGS.lock().unwrap() {
        FlagsSlot::Unloaded => FlagsSidecarState::Unloaded,
        FlagsSlot::Missing => FlagsSidecarState::Missing,
        FlagsSlot::Refused(reason) => FlagsSidecarState::Refused(reason),
        FlagsSlot::Loaded(_) => FlagsSidecarState::Applied,
    }
}

/// The decoded sidecar flags when they match the world's grid header (a
/// stale or foreign sidecar is never applied), `None` otherwise.
pub(crate) fn flags_sidecar_for(
    origin: WorldTile,
    width: usize,
    height: usize,
) -> Option<Arc<[u32]>> {
    let guard = FLAGS.lock().unwrap();
    match &*guard {
        FlagsSlot::Loaded(s) => sidecar_for_grid(s, origin, width, height),
        _ => None,
    }
}

/// The sidecar's flags only when its decoded grid header matches the
/// world it would be painted onto.
fn sidecar_for_grid(
    s: &FlagSidecar,
    origin: WorldTile,
    width: usize,
    height: usize,
) -> Option<Arc<[u32]>> {
    (s.origin == origin && s.width == width && s.height == height).then(|| Arc::clone(&s.flags))
}

struct ReachCache {
    key: (i32, i32, usize, usize),
    bits: Arc<[u64]>,
}

enum ReachBinding {
    Unbound,
    Bundled {
        bits: Arc<[u64]>,
        origin: WorldTile,
        width: usize,
        height: usize,
    },
}

static REACH: Mutex<Option<ReachCache>> = Mutex::new(None);
static REACH_BINDING: Mutex<ReachBinding> = Mutex::new(ReachBinding::Unbound);

/// Bind the process paint-reach bitset. Bundled provenance supplies the
/// decoded sidecar. The map never bakes; 3D paint may still one-time bake
/// when unbound.
pub(crate) fn set_reach_binding(
    bits: Option<Arc<[u64]>>,
    origin: WorldTile,
    width: usize,
    height: usize,
    trusted_bundled: bool,
) {
    *REACH_BINDING.lock().unwrap() = match (trusted_bundled, bits) {
        (true, Some(bits)) => ReachBinding::Bundled {
            bits,
            origin,
            width,
            height,
        },
        _ => ReachBinding::Unbound,
    };
    *REACH.lock().unwrap() = None;
}

fn bound_reach(world: &NavWorld) -> Option<Arc<[u64]>> {
    let c = &world.collision;
    let binding = REACH_BINDING.lock().unwrap();
    match &*binding {
        ReachBinding::Bundled {
            bits,
            origin,
            width,
            height,
        } if *origin == c.origin && *width == c.width && *height == c.height => {
            Some(Arc::clone(bits))
        }
        _ => None,
    }
}

fn reach_binding_is_bundled() -> bool {
    matches!(*REACH_BINDING.lock().unwrap(), ReachBinding::Bundled { .. })
}

/// Bound `.navreach` bits matching `world`, or `None` (the map then shows
/// "reach unavailable"). Never floods the world.
pub(crate) fn map_reach_bitset(world: &NavWorld) -> Option<Arc<[u64]>> {
    bound_reach(world)
}

/// 3D paint-reach: bound sidecar, else one cached `bake_reach` on the
/// external path. A bundled sidecar for a different world is not replaced
/// by a runtime flood. The map must not call this.
pub(crate) fn reach_bitset(world: &NavWorld) -> Option<Arc<[u64]>> {
    if let Some(bits) = bound_reach(world) {
        return Some(bits);
    }
    if reach_binding_is_bundled() {
        return None;
    }
    let c = &world.collision;
    let key = (c.origin.x, c.origin.z, c.width, c.height);
    let mut guard = REACH.lock().unwrap();
    let bits = match guard.as_ref() {
        Some(cache) if cache.key == key => cache.bits.clone(),
        _ => {
            let bits: Arc<[u64]> = bake_reach(c, &world.graph).into();
            *guard = Some(ReachCache {
                key,
                bits: bits.clone(),
            });
            bits
        }
    };
    Some(bits)
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
        if c.walk.len() < base + plane {
            continue;
        }
        // A plane has content when any face byte or any packed blocked
        // bit is set (planes can share a bit-plane word at small sizes,
        // so the word range alone cannot answer this).
        let content = (base..base + plane)
            .any(|i| c.walk[i] != 0 || (c.blocked[i >> 6] >> (i & 63)) & 1 != 0);
        if content {
            levels.push(level as i32);
        }
    }
    levels
}

/// The nearest walkable tile on the world's `level` plane to the float
/// point `(x, z)`, or the click's own tile when it is already walkable.
/// Uses A's radius-16 Chebyshev/Manhattan snap. `None` when the level is
/// not one of the baked planes (see [`available_levels`]) or has no
/// walkable tile inside the snap radius.
pub fn snap(world: &NavWorld, x: f32, z: f32, level: i32) -> Option<Tile> {
    if !(0..4).contains(&level) || !available_levels(world).contains(&level) {
        return None;
    }
    snap_tile(
        world,
        GameTile {
            x: x.floor() as i32,
            z: z.floor() as i32,
            plane: level as u8,
        },
    )
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
    click_to_tile_rem(world, centre, (0.0, 0.0), scale, click, size, level)
}

fn click_to_tile_rem(
    world: &NavWorld,
    centre: (i32, i32),
    rem: (f32, f32),
    scale: f32,
    click: [f32; 2],
    size: [f32; 2],
    level: i32,
) -> Option<Tile> {
    let (tx, tz) = world_from_canvas_rem(centre, rem, scale, size, click);
    snap(world, tx, tz, level)
}

fn click_requested_tile(
    centre: (i32, i32),
    rem: (f32, f32),
    scale: f32,
    click: [f32; 2],
    size: [f32; 2],
    level: i32,
) -> Option<Tile> {
    if !(0..4).contains(&level) {
        return None;
    }
    let (tx, tz) = world_from_canvas_rem(centre, rem, scale, size, click);
    Some(Tile {
        x: tx.floor() as i32,
        z: tz.floor() as i32,
        level,
    })
}

fn world_from_canvas_rem(
    centre: (i32, i32),
    rem: (f32, f32),
    scale: f32,
    size: [f32; 2],
    click: [f32; 2],
) -> (f32, f32) {
    (
        centre.0 as f32 + rem.0 + (click[0] - size[0] / 2.0) / scale,
        centre.1 as f32 + rem.1 - (click[1] - size[1] / 2.0) / scale,
    )
}

fn split_centre(x: f32, z: f32) -> ((i32, i32), (f32, f32)) {
    let ix = x.floor() as i32;
    let iz = z.floor() as i32;
    ((ix, iz), (x - ix as f32, z - iz as f32))
}

/// Keep the world point under `click` fixed while changing pixels/tile.
pub(crate) fn zoom_toward(
    centre: (i32, i32),
    rem: (f32, f32),
    old_scale: f32,
    new_scale: f32,
    size: [f32; 2],
    click: [f32; 2],
) -> ((i32, i32), (f32, f32)) {
    if old_scale <= 0.0 || new_scale <= 0.0 {
        return (centre, rem);
    }
    let (wx, wz) = world_from_canvas_rem(centre, rem, old_scale, size, click);
    let ncx = wx - (click[0] - size[0] / 2.0) / new_scale;
    let ncz = wz + (click[1] - size[1] / 2.0) / new_scale;
    split_centre(ncx, ncz)
}

fn map_layers(map: &WalkMapRenderer, view: nav::map::spatial::View) -> OverlayLayers {
    OverlayLayers {
        grid: map.show_grid,
        collision_fill: map.show_collision,
        reach: map.show_reach,
        nsew: map.show_nsew && view.pixels_per_tile >= NSEW_PPT,
        path: false,
        flood: map.show_flood,
    }
}

fn recenter_on(x: i32, z: i32) {
    CENTRE_X.store(x, Ordering::Relaxed);
    CENTRE_Z.store(z, Ordering::Relaxed);
    PAN_REM_X.store(0, Ordering::Relaxed);
    PAN_REM_Z.store(0, Ordering::Relaxed);
}

fn sync_view_from_model(model: &MapModel) {
    recenter_on(
        model.center[0].floor() as i32,
        model.center[1].floor() as i32,
    );
    LEVEL.store(i32::from(model.plane), Ordering::Relaxed);
}

fn bind_map_model(session: &mut Session, world: &NavWorld) {
    let context = session.picker_context(world);
    session.map_model.bind(context);
}

fn pending_highlight(session: &Session) -> Option<Tile> {
    session
        .map_model
        .pending()
        .map(|sel| sel.target.unwrap_or(sel.requested))
}

fn pending_walk_target(session: &Session) -> Option<Tile> {
    session.map_model.pending().and_then(|sel| sel.target)
}

fn poi_anchor(poi: &nav::map::poi::PoiRecord) -> Tile {
    Tile {
        x: poi.display.x.floor() as i32,
        z: poi.display.z.floor() as i32,
        level: i32::from(poi.effective_plane),
    }
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
/// wheel (that zooms the map). `NO_SCROLLBAR` hides the bar; without
/// `NO_SCROLL_WITH_MOUSE` the window still scrolls once content overflows.
/// `.opened` supplies the title-bar ✕.
fn walkto_window_flags() -> WindowFlags {
    WindowFlags::NO_DOCKING
        | WindowFlags::NO_SCROLLBAR
        | WindowFlags::NO_SCROLL_WITH_MOUSE
        | WindowFlags::NO_COLLAPSE
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

/// Footer labels: Teleport only when debug teleport is authorized.
pub(crate) fn walkto_footer_labels(teleport: bool) -> &'static [&'static str] {
    if teleport {
        &["recentre", "Walk", "Teleport"]
    } else {
        &["recentre", "Walk"]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WalktoCaption {
    None,
    Walk { requested: Tile, target: Tile },
    TeleportOnly { requested: Tile },
    Blocked { requested: Tile },
}

pub(crate) fn walkto_selection_caption(
    pending: Option<&Selection>,
    teleport: bool,
) -> WalktoCaption {
    match pending {
        None => WalktoCaption::None,
        Some(sel) => match sel.target {
            Some(target) => WalktoCaption::Walk {
                requested: sel.requested,
                target,
            },
            None if teleport => WalktoCaption::TeleportOnly {
                requested: sel.requested,
            },
            None => WalktoCaption::Blocked {
                requested: sel.requested,
            },
        },
    }
}

pub(crate) fn format_walkto_status(caption: WalktoCaption, status: &str) -> String {
    match caption {
        WalktoCaption::None => format!("click a tile, then Walk · {status}"),
        WalktoCaption::Walk { requested, target } => format!(
            "selected {} {} {} (walk target {} {}) · {status}",
            requested.x, requested.z, requested.level, target.x, target.z
        ),
        WalktoCaption::TeleportOnly { requested } => format!(
            "blocked {} {} {} (teleport only) · {status}",
            requested.x, requested.z, requested.level
        ),
        WalktoCaption::Blocked { requested } => format!(
            "blocked {} {} {} · {status}",
            requested.x, requested.z, requested.level
        ),
    }
}

/// Walk needs a snapped target. Teleport needs any selection when authorized.
pub(crate) fn walkto_actions_enabled(pending: Option<&Selection>, teleport: bool) -> (bool, bool) {
    (
        pending.and_then(|sel| sel.target).is_some(),
        teleport && pending.is_some(),
    )
}

/// Combo width on the Level/Zoom toolbar so they do not eat the row
/// (default item width is the remaining content region).
const TOOLBAR_COMBO_W: f32 = 140.0;
/// Search field shrinks to this before wrapping onto the next header row.
const TOOLBAR_SEARCH_MIN_W: f32 = 120.0;

/// Space left on the current widget row after the last item (screen space).
fn remaining_on_row(ui: &Ui) -> f32 {
    let spacing = ui.clone_style().item_spacing()[0];
    let right = ui.cursor_screen_pos()[0] + ui.content_region_avail()[0];
    (right - ui.item_rect_max()[0] - spacing).max(0.0)
}

/// Keep the next toolbar widget on this row when `next_w` still fits.
fn toolbar_continue(ui: &Ui, next_w: f32) {
    if remaining_on_row(ui) >= next_w {
        ui.same_line();
    }
}

/// ImGui checkbox width: square + inner gap + label.
fn checkbox_w(ui: &Ui, label: &str) -> f32 {
    let inner = ui.clone_style().item_inner_spacing()[0];
    let text = ui
        .current_font()
        .calc_text_size(ui.current_font_size(), f32::MAX, 0.0, label)[0];
    ui.frame_height() + inner + text
}

/// Footer is one status/action row plus the gap under the canvas child.
fn map_footer_reserve(ui: &Ui) -> f32 {
    ui.frame_height() + ui.clone_style().item_spacing()[1]
}

struct ToolbarGeom {
    header_max_x: f32,
    search_max_x: f32,
}

fn note_toolbar_item(geom: &mut ToolbarGeom, ui: &Ui) {
    geom.header_max_x = geom.header_max_x.max(ui.item_rect_max()[0]);
}

/// Plane, zoom, search, then wrapping layer checkboxes.
fn draw_walkto_toolbar(
    ui: &Ui,
    session: &mut Session,
    map: &mut WalkMapRenderer,
    world: &NavWorld,
    levels: &[i32],
    lvl_idx: &mut usize,
) -> ToolbarGeom {
    let mut geom = ToolbarGeom {
        header_max_x: 0.0,
        search_max_x: 0.0,
    };
    ui.set_next_item_width(TOOLBAR_COMBO_W);
    if ui.combo("##walkto-level", lvl_idx, levels, |l: &i32| {
        Cow::Owned(format!("level {l}"))
    }) {
        LEVEL.store(levels[*lvl_idx], Ordering::Relaxed);
        if let Ok(plane) = u8::try_from(levels[*lvl_idx]) {
            session.map_model.set_plane(plane);
        }
    }
    note_toolbar_item(&mut geom, ui);
    toolbar_continue(ui, TOOLBAR_COMBO_W);
    let mut zoom = ZOOM
        .load(Ordering::Relaxed)
        .clamp(0, ZOOMS.len() as i32 - 1) as usize;
    ui.set_next_item_width(TOOLBAR_COMBO_W);
    if ui.combo("##walkto-zoom", &mut zoom, &ZOOMS, |z: &f32| {
        Cow::Owned(if *z < 1.0 {
            format!("{z}px/tile")
        } else {
            format!("{z:.0}px/tile")
        })
    }) {
        ZOOM.store(zoom as i32, Ordering::Relaxed);
    }
    note_toolbar_item(&mut geom, ui);
    let search_remain = remaining_on_row(ui);
    if search_remain >= TOOLBAR_SEARCH_MIN_W {
        ui.same_line();
        ui.set_next_item_width(search_remain);
    } else {
        ui.set_next_item_width(ui.content_region_avail()[0].max(TOOLBAR_SEARCH_MIN_W));
    }
    ui.input_text("##walkto-search", &mut map.search)
        .hint("search / x,z,plane")
        .build();
    geom.search_max_x = ui.item_rect_max()[0];
    note_toolbar_item(&mut geom, ui);
    if ui.is_item_focused() && ui.is_key_pressed(Key::Enter) {
        apply_search_jump(session, map, world);
    }
    let toggles: [(&str, &mut bool); 6] = [
        ("basemap", &mut map.show_basemap),
        ("grid", &mut map.show_grid),
        ("reach", &mut map.show_reach),
        ("collision", &mut map.show_collision),
        ("nsew", &mut map.show_nsew),
        ("flood", &mut map.show_flood),
    ];
    for (i, (label, flag)) in toggles.into_iter().enumerate() {
        if i > 0 {
            toolbar_continue(ui, checkbox_w(ui, label));
        }
        ui.checkbox(label, flag);
        note_toolbar_item(&mut geom, ui);
    }
    geom
}

#[cfg(test)]
#[derive(Clone, Copy, Debug)]
struct PickerLayout {
    content_max: [f32; 2],
    header_max_x: f32,
    search_max_x: f32,
    canvas_inner_min: [f32; 2],
    canvas_inner_max: [f32; 2],
    canvas_item_max: [f32; 2],
    footer_max: [f32; 2],
    footer_reserve: f32,
}

#[cfg(test)]
static LAST_PICKER_LAYOUT: Mutex<Option<PickerLayout>> = Mutex::new(None);

fn record_picker_layout(
    ui: &Ui,
    toolbar: &ToolbarGeom,
    canvas_inner: Option<([f32; 2], [f32; 2])>,
    canvas_item_max: [f32; 2],
    footer_reserve: f32,
) {
    #[cfg(test)]
    {
        let pad = ui.clone_style().window_padding();
        let pos = ui.window_pos();
        let size = ui.window_size();
        let (inner_min, inner_max) = canvas_inner.unwrap_or(([0.0, 0.0], [0.0, 0.0]));
        *LAST_PICKER_LAYOUT.lock().unwrap() = Some(PickerLayout {
            content_max: [pos[0] + size[0] - pad[0], pos[1] + size[1] - pad[1]],
            header_max_x: toolbar.header_max_x,
            search_max_x: toolbar.search_max_x,
            canvas_inner_min: inner_min,
            canvas_inner_max: inner_max,
            canvas_item_max,
            footer_max: ui.item_rect_max(),
            footer_reserve,
        });
    }
    #[cfg(not(test))]
    {
        let _ = (ui, toolbar, canvas_inner, canvas_item_max, footer_reserve);
    }
}

#[cfg(test)]
fn last_picker_layout() -> PickerLayout {
    LAST_PICKER_LAYOUT
        .lock()
        .unwrap()
        .expect("picker_map_body records layout")
}

/// The width a text-only button of `label` occupies under the current style,
/// for right-aligning a button against the content region edge.
fn button_w(ui: &Ui, label: &str) -> f32 {
    let font = ui.current_font();
    let text = font.calc_text_size(ui.current_font_size(), f32::MAX, 0.0, label)[0];
    text + 2.0 * ui.clone_style().frame_padding()[0]
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

/// Drop this consumer's flood cache. Reach stays the bound `.navreach` sidecar.
pub fn release_map_leases() {
    *FLOOD_CACHE.lock().unwrap() = None;
    *FLOOD_REPORT.lock().unwrap() = None;
}

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
type FloodKey = (u64, WorldTile, WorldTile, usize, usize);
static FLOOD_REPORT: Mutex<Option<FloodKey>> = Mutex::new(None);

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

/// Remaining path tiles for the focused slot, borrowed through
/// [`host_play::Play::with_map_route`]: driven live, then script, then manual WalkTo.
fn focused_remaining_path(session: &Session, here: Option<WorldTile>) -> Vec<(WorldTile, bool)> {
    let tiles = |route: &Route| {
        remaining_path_tiles(route, here)
            .into_iter()
            .map(|p| (p.tile, p.transport))
            .collect()
    };
    let Some(name) = session.focused_name() else {
        return Vec::new();
    };
    let travellers = session.travellers.lock().unwrap();
    let manual_arc = travellers.get(&name).cloned();
    drop(travellers);
    let manual = manual_arc.as_ref().map(|arm| arm.lock().unwrap());
    let scenario = session.scenario.lock().unwrap();
    let live_route = scenario
        .as_ref()
        .and_then(|runner| runner.drives(&name).then(|| runner.armed_route()).flatten());
    let live = live_route.map(|route| RouteProjection::live(route, session.route_gen(), None));
    if let Some(play) = session.play.as_ref() {
        play.with_map_route(&name, manual.as_deref(), live, |proj| {
            proj.map(|p| tiles(p.route)).unwrap_or_default()
        })
    } else {
        match select_route_source(
            live.is_some(),
            false,
            manual.as_ref().is_some_and(|arm| arm.route.is_some()),
        ) {
            Some(RouteSource::Live) => live.map(|p| tiles(p.route)).unwrap_or_default(),
            Some(RouteSource::Manual) => manual
                .as_ref()
                .and_then(|arm| arm.route.as_ref())
                .map(tiles)
                .unwrap_or_default(),
            Some(RouteSource::Script) | None => Vec::new(),
        }
    }
}

/// Call when the Game pane is not showing WalkTo so the next open resets
/// the view (the WalkTo chrome button toggles without running the body).
pub fn note_closed() {
    PREV_OPEN.store(false, Ordering::Relaxed);
    release_map_leases();
}

/// WalkTo map in the Game pane as its own window so the title-bar ✕ closes it.
pub fn draw_picker(
    ui: &Ui,
    gpu: Option<&mut dyn FrameGpu>,
    session: &mut Session,
    map: &mut WalkMapRenderer,
) {
    let pos = ui.cursor_screen_pos();
    let avail = ui.content_region_avail();
    let mut open = true;
    ui.window("WalkTo")
        .opened(&mut open)
        .flags(walkto_window_flags() | WindowFlags::NO_MOVE | WindowFlags::NO_RESIZE)
        .position(pos, Condition::Always)
        .size(avail, Condition::Always)
        .build(|| match pack() {
            Some(world) => picker_map_body(ui, gpu, session, map, &world),
            None => {
                map.note_open();
                ui.text_wrapped("no nav pack — run nav-pack");
            }
        });
    if !open {
        session.walkto_open = false;
        PREV_OPEN.store(false, Ordering::Relaxed);
        session.map_model.close();
    }
}

/// The map window. `open` is the window's live open flag; confirm Walk closes
/// it. Headless tests wrap the body in a window.
#[cfg(test)]
fn picker_map_window(
    ui: &Ui,
    session: &mut Session,
    world: &NavWorld,
    open: &mut bool,
    gpu: Option<&mut dyn FrameGpu>,
    map: &mut WalkMapRenderer,
) {
    let _ = ui
        .window("WalkTo")
        .opened(open)
        .flags(walkto_window_flags())
        .position([0.0, 0.0], Condition::Always)
        .size([720.0, 560.0], Condition::Always)
        .size_constraints([480.0, 360.0], [f32::MAX, f32::MAX])
        .build(|| {
            picker_map_body(ui, gpu, session, map, world);
        });
}

/// WalkTo nested in a Game pane, matching production [`draw_picker`].
#[cfg(test)]
fn picker_nested_in_game(
    ui: &Ui,
    session: &mut Session,
    world: &NavWorld,
    map: &mut WalkMapRenderer,
    game_size: [f32; 2],
) {
    let _ = ui
        .window("Game")
        .flags(
            WindowFlags::NO_COLLAPSE
                | WindowFlags::NO_SCROLLBAR
                | WindowFlags::NO_SCROLL_WITH_MOUSE
                | WindowFlags::NO_RESIZE,
        )
        .position([0.0, 0.0], Condition::Always)
        .size(game_size, Condition::Always)
        .build(|| {
            let pos = ui.cursor_screen_pos();
            let avail = ui.content_region_avail();
            let mut open = true;
            ui.window("WalkTo")
                .opened(&mut open)
                .flags(walkto_window_flags() | WindowFlags::NO_MOVE | WindowFlags::NO_RESIZE)
                .position(pos, Condition::Always)
                .size(avail, Condition::Always)
                .build(|| {
                    picker_map_body(ui, None, session, map, world);
                });
        });
}

/// Toolbar, canvas, and footer. Used inside the Game pane and the test window.
fn picker_map_body(
    ui: &Ui,
    gpu: Option<&mut dyn FrameGpu>,
    session: &mut Session,
    map: &mut WalkMapRenderer,
    world: &NavWorld,
) {
    map.note_open();
    bind_map_model(session, world);
    // Reset the view when the picker opens fresh.
    if !PREV_OPEN.swap(true, Ordering::Relaxed) {
        let observed = session
            .focused_tile()
            .map(|(x, z, level)| Tile { x, z, level });
        session.map_model.recenter(observed);
        sync_view_from_model(&session.map_model);
    }
    let levels = available_levels(world);
    let mut lvl_idx = levels
        .iter()
        .position(|l| *l == LEVEL.load(Ordering::Relaxed))
        .unwrap_or(0);
    let toolbar = draw_walkto_toolbar(ui, session, map, world, &levels, &mut lvl_idx);
    if !map.search.trim().is_empty() {
        if let Ok(coord) = MapModel::parse_coordinates(map.search.trim()) {
            ui.text_disabled(format!("coord {} {} {}", coord.x, coord.z, coord.level));
        } else if let Some(coord) = map.parse_search_coord() {
            ui.text_disabled(format!("coord {} {} {}", coord.x, coord.z, coord.plane));
        } else {
            draw_search_hits(ui, session, map, world);
        }
    }
    let footer_h = map_footer_reserve(ui);
    let avail = ui.content_region_avail();
    let canvas_h = (avail[1] - footer_h).max(120.0);
    let mut overlay_zoom_in = false;
    let canvas_inner = draw_canvas(ui, gpu, session, map, world, canvas_h, &mut overlay_zoom_in);
    let canvas_item_max = ui.item_rect_max();
    let status = if overlay_zoom_in {
        "zoom in for tile layers"
    } else {
        map.status_line()
    };
    let teleport = session.map_teleport_authorized();
    ui.text_disabled(format_walkto_status(
        walkto_selection_caption(session.map_model.pending(), teleport),
        status,
    ));
    let spacing = ui.clone_style().item_spacing()[0];
    let labels = walkto_footer_labels(teleport);
    let cluster = labels.iter().map(|l| button_w(ui, l)).sum::<f32>()
        + spacing * (labels.len().saturating_sub(1) as f32);
    let x = right_align_x(ui.cursor_pos()[0], ui.content_region_avail()[0], cluster);
    ui.same_line_with_pos(x);
    if ui.button("recentre") {
        let observed = session
            .focused_tile()
            .map(|(x, z, level)| Tile { x, z, level });
        session.map_model.recenter(observed);
        sync_view_from_model(&session.map_model);
    }
    ui.same_line();
    let (can_walk, can_teleport) = walkto_actions_enabled(session.map_model.pending(), teleport);
    {
        let _off = ui.begin_disabled_with_cond(!can_walk);
        if ui.button("Walk") && can_walk && session.confirm_picker_walk(world) {
            session.walkto_open = false;
            PREV_OPEN.store(false, Ordering::Relaxed);
            session.map_model.close();
        }
    }
    if teleport {
        ui.same_line();
        let _off = ui.begin_disabled_with_cond(!can_teleport);
        if ui.button("Teleport") && can_teleport && session.confirm_picker_teleport(world) {
            session.walkto_open = false;
            PREV_OPEN.store(false, Ordering::Relaxed);
            session.map_model.close();
        }
    }
    record_picker_layout(ui, &toolbar, canvas_inner, canvas_item_max, footer_h);
}

fn draw_search_hits(ui: &Ui, session: &mut Session, map: &mut WalkMapRenderer, world: &NavWorld) {
    if let Some(catalogue) = session.map_catalogue.clone() {
        let query = map.search.clone();
        if let Err(error) = map.catalogue_search.update(&catalogue, query.trim()) {
            session.error = Some(error.to_string());
            return;
        }
        for &index in map.catalogue_search.results().iter().take(MAX_LABELS) {
            let Some(entry) = catalogue.entry(index) else {
                continue;
            };
            let anchor = entry.anchor();
            let label = format!(
                "{}  {} {} {}",
                entry.name(),
                anchor.x,
                anchor.z,
                anchor.level
            );
            if ui.selectable(&label) && session.select_picker_poi(index) {
                sync_view_from_model(&session.map_model);
            }
        }
        return;
    }
    for poi in map.search_hits() {
        let requested = poi_anchor(poi);
        let label = format!(
            "{}  {} {} {}",
            poi.name.as_str(),
            requested.x,
            requested.z,
            requested.level
        );
        if ui.selectable(&label) {
            recenter_on(requested.x, requested.z);
            LEVEL.store(requested.level, Ordering::Relaxed);
            session.select_picker_tile(world, requested);
        }
    }
}

fn apply_search_jump(session: &mut Session, map: &mut WalkMapRenderer, world: &NavWorld) {
    bind_map_model(session, world);
    match session
        .map_model
        .select_coordinates(world, map.search.trim())
    {
        Ok(_) => {
            sync_view_from_model(&session.map_model);
            return;
        }
        Err(ActionError::InvalidCoordinates) => {}
        Err(error) => {
            session.error = Some(error.to_string());
            return;
        }
    }
    if let Some(coord) = map.parse_search_coord() {
        let requested = Tile {
            x: coord.x,
            z: coord.z,
            level: i32::from(coord.plane),
        };
        session.select_picker_tile(world, requested);
        recenter_on(coord.x, coord.z);
        LEVEL.store(i32::from(coord.plane), Ordering::Relaxed);
        return;
    }
    if let Some(catalogue) = session.map_catalogue.clone() {
        let query = map.search.clone();
        match map.catalogue_search.update(&catalogue, query.trim()) {
            Ok(_) => {
                if let Some(&index) = map.catalogue_search.results().first() {
                    if session.select_picker_poi(index) {
                        sync_view_from_model(&session.map_model);
                    }
                    return;
                }
            }
            Err(error) => {
                session.error = Some(error.to_string());
                return;
            }
        }
    }
    if let Some(poi) = map.search_hits().into_iter().next() {
        let requested = poi_anchor(poi);
        recenter_on(requested.x, requested.z);
        LEVEL.store(requested.level, Ordering::Relaxed);
        session.select_picker_tile(world, requested);
    }
}

/// The child canvas: terrain images, one overlay, drag-to-pan, wheel-to-zoom,
/// click-to-select (does not arm).
fn draw_canvas(
    ui: &Ui,
    gpu: Option<&mut dyn FrameGpu>,
    session: &mut Session,
    map: &mut WalkMapRenderer,
    world: &NavWorld,
    height: f32,
    overlay_zoom_in: &mut bool,
) -> Option<([f32; 2], [f32; 2])> {
    let mut rect: Option<([f32; 2], [f32; 2])> = None;
    let mut pick: Option<[f32; 2]> = None;
    let mut hovered = false;
    ui.child_window("##walkto-canvas")
        .size([0.0, height])
        .flags(walkto_canvas_flags())
        .build(ui, || {
            let origin = ui.cursor_screen_pos();
            let size = ui.content_region_avail();
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
            let scale = ZOOMS[ZOOM
                .load(Ordering::Relaxed)
                .clamp(0, ZOOMS.len() as i32 - 1) as usize];
            let centre_i = (
                CENTRE_X.load(Ordering::Relaxed),
                CENTRE_Z.load(Ordering::Relaxed),
            );
            let rem = (
                PAN_REM_X.load(Ordering::Relaxed) as f32 / 1000.0,
                PAN_REM_Z.load(Ordering::Relaxed) as f32 / 1000.0,
            );
            let level = LEVEL.load(Ordering::Relaxed);
            let fb_scale = ui.io().display_framebuffer_scale()[0].max(0.01);
            let view = view_from_canvas(
                centre_i,
                rem,
                scale,
                size,
                level.clamp(0, 3) as u8,
                map.max_lod(),
                fb_scale,
            );
            let layers_nav = session.effective_nav();
            let layers = map_layers(map, view);
            let colors = overlay_colors(&layers_nav);
            let here_tile = session
                .focused_tile()
                .map(|(x, z, level)| Tile { x, z, level });
            let here = here_tile.map(|t| WorldTile {
                x: t.x,
                z: t.z,
                level: t.level,
            });
            let dest_tile = pending_walk_target(session).or(session.walk_dest);
            let dest = dest_tile.map(|t| WorldTile {
                x: t.x,
                z: t.z,
                level: t.level,
            });
            let path = focused_remaining_path(session, here);
            let seeds: Vec<WorldTile> = if layers.flood {
                [here, dest].into_iter().flatten().collect()
            } else {
                Vec::new()
            };
            let floods = if seeds.is_empty() {
                Vec::new()
            } else {
                flood_sets_for(world, &seeds)
            };
            if layers.flood {
                if let (Some(h), Some(d)) = (here, dest) {
                    report_flood_sizes(world, h, d, session.route_gen());
                }
            }
            let reach_bits = if layers.reach {
                map_reach_bitset(world)
            } else {
                None
            };
            map.present(
                ui,
                gpu,
                origin,
                size,
                view,
                world,
                layers,
                colors,
                &path,
                &floods,
                reach_bits.as_deref(),
                pending_highlight(session),
                here_tile,
                dest_tile,
                overlay_zoom_in,
            );
        });
    let (min, max) = rect?;
    let scale = ZOOMS[ZOOM
        .load(Ordering::Relaxed)
        .clamp(0, ZOOMS.len() as i32 - 1) as usize];
    let size = [max[0] - min[0], max[1] - min[1]];
    if let Some(mouse) = pick {
        let centre = (
            CENTRE_X.load(Ordering::Relaxed),
            CENTRE_Z.load(Ordering::Relaxed),
        );
        let rem = (
            PAN_REM_X.load(Ordering::Relaxed) as f32 / 1000.0,
            PAN_REM_Z.load(Ordering::Relaxed) as f32 / 1000.0,
        );
        if let Some(requested) = click_requested_tile(
            centre,
            rem,
            scale,
            [mouse[0] - min[0], mouse[1] - min[1]],
            size,
            LEVEL.load(Ordering::Relaxed),
        ) {
            session.select_picker_tile(world, requested);
        }
        return Some((min, max));
    }
    if !hovered {
        return Some((min, max));
    }
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
        return Some((min, max));
    }
    // Vertical wheel zooms toward the cursor. Horizontal wheel still pans.
    let wheel = ui.io().mouse_wheel();
    let wheel_h = ui.io().mouse_wheel_h();
    let mouse = ui.io().mouse_pos();
    let click = [mouse[0] - min[0], mouse[1] - min[1]];
    if wheel != 0.0 {
        let mut idx = ZOOM
            .load(Ordering::Relaxed)
            .clamp(0, ZOOMS.len() as i32 - 1);
        if wheel > 0.0 {
            idx = (idx + 1).min(ZOOMS.len() as i32 - 1);
        } else {
            idx = (idx - 1).max(0);
        }
        let new_scale = ZOOMS[idx as usize];
        if (new_scale - scale).abs() > f32::EPSILON {
            let (centre, rem) = zoom_toward(
                (
                    CENTRE_X.load(Ordering::Relaxed),
                    CENTRE_Z.load(Ordering::Relaxed),
                ),
                (
                    PAN_REM_X.load(Ordering::Relaxed) as f32 / 1000.0,
                    PAN_REM_Z.load(Ordering::Relaxed) as f32 / 1000.0,
                ),
                scale,
                new_scale,
                size,
                click,
            );
            ZOOM.store(idx, Ordering::Relaxed);
            CENTRE_X.store(centre.0, Ordering::Relaxed);
            CENTRE_Z.store(centre.1, Ordering::Relaxed);
            PAN_REM_X.store((rem.0 * 1000.0) as i32, Ordering::Relaxed);
            PAN_REM_Z.store((rem.1 * 1000.0) as i32, Ordering::Relaxed);
        }
    } else if wheel_h != 0.0 {
        let (centre, rem) = pan_by(
            (
                CENTRE_X.load(Ordering::Relaxed),
                CENTRE_Z.load(Ordering::Relaxed),
            ),
            (
                PAN_REM_X.load(Ordering::Relaxed) as f32 / 1000.0,
                PAN_REM_Z.load(Ordering::Relaxed) as f32 / 1000.0,
            ),
            wheel_h * 16.0,
            0.0,
            scale,
        );
        CENTRE_X.store(centre.0, Ordering::Relaxed);
        CENTRE_Z.store(centre.1, Ordering::Relaxed);
        PAN_REM_X.store((rem.0 * 1000.0) as i32, Ordering::Relaxed);
        PAN_REM_Z.store((rem.1 * 1000.0) as i32, Ordering::Relaxed);
    }
    Some((min, max))
}

#[cfg(test)]
#[path = "picker_tests.rs"]
mod tests;
