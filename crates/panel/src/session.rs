//! Panel session: the native adapter over the shared operator core
//! ([`frontend_core::OperatorSession`], which owns the vault, the play,
//! fleet membership, the selected bot, removals and operation results).
//! The panel keeps the render `Focus` mirror the slot threads read, the
//! per-slot frame/input channels ([`SlotIo`] via [`PanelSurface`]), audio,
//! nav paint, WalkTo and the proof harness state.
//!
//! Flat slot model (M2 Task 2b): every wall member is its own full `Client`
//! on its own slot thread — there is no channel head and no lean baton.
//! Clicking a member is [`Session::select`]: the core selects it and the
//! Game pane samples that slot's `FrameBuf`. The single-client boot still
//! holds: unlock spawns **one** Client (the focused profile); MultiBox spawns
//! the rest.

use std::collections::{HashMap, HashSet};
use std::env;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use api::hostlog::{Category, Level, Source};
use api::snapshot::WorldTile;
use client::client::Client;
use client::render::nav_debug::{
    NavDebugCell, NavDebugColors, NavDebugHull, NavDebugPaint, CORNER_NE, CORNER_NW, CORNER_SE,
    CORNER_SW, FACE_E, FACE_N, FACE_S, FACE_W,
};
use client::sound::output::AudioOut;
use frontend_core::command::{DrawSwitch, RasterMode};
use frontend_core::frames::{AudioChange, AudioGate};
use frontend_core::launch::{
    BoundProfile, Passphrase, PlayConfig, PreparationCompletion, ProfileArgs, ProfileClass,
    ProfileEnv, ProfileError, ProfileLauncher, ProfilePreparation, ProfileTemplate, UnlockStage,
    ValidatedProfile,
};
use frontend_core::prefs::{PreferencePatch, UiPreferences, Update};
#[cfg(feature = "js")]
use frontend_core::scripts::{CardSource, SchemaRef};
use frontend_core::scripts::{LoadoutForm, PaintFrame, ScriptChoice, ScriptRunState};
use frontend_core::surface::{FrameBuf, InputEv, InputSender, SlotInput};
use frontend_core::walk_map::{
    select_walk_route_source, RouteProjectionView, WalkMapDemand, WalkMapJobState, WalkMapStage,
    WalkRoutePath, WalkRouteSource, WalkSlotStatus, WalkSourceStatus,
};
use frontend_core::MapBakeChoice;
use frontend_core::{CurePoison, SurvivalEdit};
#[cfg(test)]
use host_play::SlotStatus;
use nav_planner::paint::{
    collision_at_with, hop_captions, hull_targets, reached, remaining_path_tiles, remaining_trail,
    select_draw_indices, trail_tones, HopCaption, HullTarget, PathTile, TrailTone,
};
use nav_planner::router::{FindOptions, Route};
use nav_planner::tile::Tile;
use nav_planner::world::NavWorld;

use crate::focus::{draw_for_slot, full_rate_for};
use crate::nav_settings::{from_scenario, parse_html_color, NavSettings};
use crate::wall::Wall;
use frontend_core::{
    GuardianEdit, GuardianPolicy, OperatorCommand, OperatorSession, SlotAttach, SlotSurface,
    Targets,
};

/// Maximum panel-owned setting events retained between drains.
const PANEL_SETTING_EVENTS_CAP: usize = 256;

/// The refusal a native panel gives for a card that only the JavaScript
/// runtime can run: the core's own text, so the panel and the core refuse in
/// the same words. It is exactly what
/// `script::surface::SurfaceError::NoFactory(SurfaceKind::Js)` displays; the
/// command census refuses a new public frontend-core helper for it.
#[cfg(not(feature = "js"))]
const JS_SURFACE_REFUSAL: &str = "no surface factory for Js";

/// Scatter / mainland hop only on a cold world, not after a `lostCon`
/// reconnect (that would tele the re-handshaked slot on every DC).
fn seed_on_first_world(last_login_reconnect: Option<bool>) -> bool {
    last_login_reconnect != Some(true)
}

/// One Teles-popup dest: button label, `CLIENT_CHEAT` body, hover text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DebugDest {
    pub label: &'static str,
    pub cheat: &'static str,
    pub tooltip: &'static str,
}

/// Engine `::getvar tutorial` replies `get tutorial: 1000`.
pub fn parse_getvar_line(text: &str) -> Option<(&str, i32)> {
    let rest = text.strip_prefix("get ")?;
    let (name, value) = rest.rsplit_once(':')?;
    Some((name.trim(), value.trim().parse().ok()?))
}

/// Debug heading buttons. TutSkip is omitted once the profile is known
/// skipped or still unknown (a `getvar` is in flight).
/// Debug controls use the same resolved class displayed in the panel heading.
pub fn debug_main_buttons_for(class: ProfileClass, show_tutskip: bool) -> Vec<&'static str> {
    if class != ProfileClass::Local {
        return vec!["DebugPanel"];
    }
    let mut labels = vec!["DebugPanel"];
    if show_tutskip {
        labels.push("TutSkip");
    }
    labels.extend(["Lumbridge", "maxme", "Teles"]);
    labels
}

/// Named dest cheats for the Teles popup (`[debugproc]` names).
pub fn debug_dest_cheats() -> &'static [DebugDest] {
    // Engine debugprocs are `::<debugProcChar><name>` with debugProcChar
    // default `~` (`::~home`). Engine commands (`tele`, `setvar`,
    // `setstat`) have no tilde.
    &[
        DebugDest {
            label: "Lumbridge",
            cheat: "~home",
            tooltip: "Lumbridge courtyard",
        },
        DebugDest {
            label: "Varrock",
            cheat: "~varrock",
            tooltip: "Varrock square",
        },
        DebugDest {
            label: "Falador",
            cheat: "~falador",
            tooltip: "Falador square",
        },
        DebugDest {
            label: "Draynor",
            cheat: "~draynor",
            tooltip: "Draynor Village",
        },
        DebugDest {
            label: "PortSarim",
            cheat: "~portsarim",
            tooltip: "Port Sarim docks",
        },
        DebugDest {
            label: "Rimmington",
            cheat: "~rimmington",
            tooltip: "Rimmington",
        },
        DebugDest {
            label: "AlKharid",
            cheat: "~alkharid",
            tooltip: "Al Kharid",
        },
        DebugDest {
            label: "Seers",
            cheat: "~seers",
            tooltip: "Seers' Village",
        },
        DebugDest {
            label: "Giants",
            cheat: "~giants",
            tooltip: "Hill Giants (Edgeville dungeon)",
        },
        DebugDest {
            label: "Entrana",
            cheat: "~entrana",
            tooltip: "Entrana (no weapons/armour)",
        },
        DebugDest {
            label: "Brimhaven",
            cheat: "~brimhaven",
            tooltip: "Brimhaven",
        },
        DebugDest {
            label: "Ardy",
            cheat: "~ardy",
            tooltip: "East Ardougne",
        },
        DebugDest {
            label: "Kbd",
            cheat: "~kbd",
            tooltip: "King Black Dragon lair",
        },
        DebugDest {
            label: "Elvarg",
            cheat: "~elvarg",
            tooltip: "Elvarg on Crandor",
        },
        DebugDest {
            label: "Greenland",
            cheat: "~greenland",
            tooltip: "Fields north of Tree Gnome Stronghold (not green dragons)",
        },
        DebugDest {
            label: "Gb",
            cheat: "~gb",
            tooltip: "Gnome Ball pitch",
        },
        DebugDest {
            label: "Ma",
            cheat: "~ma",
            tooltip: "Mage Arena (Wilderness)",
        },
        DebugDest {
            label: "Pvp",
            cheat: "~pvp",
            tooltip: "PvP / bounty area",
        },
        DebugDest {
            label: "Duel",
            cheat: "~duel",
            tooltip: "Duel Arena",
        },
        DebugDest {
            label: "Trawler",
            cheat: "~trawler",
            tooltip: "Fishing Trawler",
        },
        DebugDest {
            label: "Gamesroom",
            cheat: "~gamesroom",
            tooltip: "Burthorpe Games Room",
        },
        DebugDest {
            label: "Mortton",
            cheat: "~mortton",
            tooltip: "Mort'ton",
        },
    ]
}

/// `setstat <skill> 99` for the skills `[debugproc,maxme]` advances.
pub fn debug_maxme_cheats() -> &'static [&'static str] {
    api::interact::MAXME_SETSTATS
}

/// Cooldown between cpal open retries after a device failure: a machine
/// without an audio device must not re-open (and re-log) every 20 ms frame.
const AUDIO_OPEN_RETRY: Duration = Duration::from_secs(5);
/// Cadence of the published-catalogue probe while a WalkTo map demand bakes.
const MAP_CATALOGUE_PROBE: Duration = Duration::from_secs(1);

/// Legacy local vault path used before a profile has been resolved.
pub fn default_vault_path() -> PathBuf {
    frontend_core::scripts::paths::bot_file("vault")
}

/// Panel-side per-slot IO: the frame mailbox the slot stores each rendered
/// `FrameOutput` into while its renderer is on (the panel `take`s it and
/// packs the `PixMap` or reads the `Texture` back at the consume site),
/// and the input channel it drains only while capture is on.
pub struct SlotIo {
    pub input: Arc<SlotInput>,
    pub pixels: Arc<FrameBuf>,
}

/// Panel adapter for the core's slot lifetimes: the per-slot input channel
/// and frame mailbox, the per-slot draw and audio policy fixed at spawn, and
/// the frontend facts dropped at a username-reuse boundary.
struct PanelSurface<'a> {
    focus: &'a Mutex<crate::focus::Focus>,
    audio: &'a AudioGate<AudioOut>,
    memory_override: Option<bool>,
    map_desk: &'a mut frontend_core::MapDesk,
    models: frontend_core::Models<'a>,
    /// The panel's per-frame client work, lent only to the vault rows
    /// that start the play.
    frame_hook: Option<Arc<dyn frontend_core::frames::FrameHook>>,
}

impl SlotSurface for PanelSurface<'_> {
    type Io = SlotIo;

    fn attach(
        &mut self,
        name: &str,
        profile: &mut frontend_core::SpawnProfile<'_>,
        retained: Option<SlotIo>,
    ) -> SlotAttach<SlotIo> {
        let SlotIo { input, pixels } = retained.unwrap_or_else(|| SlotIo {
            input: SlotInput::new(),
            pixels: FrameBuf::new(),
        });
        // Raster comes from the vault profile (the same source as
        // `bot_client_config`); a focus change never re-roles a live slot.
        let raster = profile.raster();
        // The session-only memory choice applies to this disposable spawn
        // profile only; it is never written to the vault.
        if let Some(lowmem) = self.memory_override {
            profile.set_lowmem(lowmem);
        }
        input.set_prefer_cpu(raster == RasterMode::Cpu);
        self.focus
            .lock()
            .unwrap()
            .renderer_by
            .insert(name.to_string(), raster != RasterMode::Off);
        SlotAttach::new(
            SlotIo {
                input: Arc::clone(&input),
                pixels: Arc::clone(&pixels),
            },
            Some(input),
            Some(pixels),
        )
    }

    fn lifetime_reset(&mut self, _name: &str, walk_was_armed: bool) {
        if walk_was_armed {
            self.map_desk.clear_walk_dest();
        }
    }

    fn released(&mut self, name: &str) {
        self.audio.release(name);
    }

    fn models(&mut self) -> frontend_core::Models<'_> {
        self.models.reborrow().with_map(&mut *self.map_desk)
    }

    /// The render mirror the slot threads read: the focused member's
    /// renderer, the capture pref and the frame cadences.
    fn draw_changed(&mut self, switch: frontend_core::command::DrawSwitch, on: bool) {
        use frontend_core::command::DrawSwitch;
        let mut focus = self.focus.lock().unwrap();
        match switch {
            DrawSwitch::GameRenderer => {
                focus.renderer = on;
                if let Some(name) = focus.focused.clone() {
                    focus.renderer_by.insert(name, on);
                }
            }
            DrawSwitch::CaptureInput => focus.capture = on,
            DrawSwitch::SidecarFullRate => focus.sidecar_50 = on,
            DrawSwitch::FocusedFullRate => focus.focused_50 = on,
            DrawSwitch::LiveFullRate => focus.live_full_rate = on,
        }
    }

    fn frame_hook(&mut self) -> Option<Arc<dyn frontend_core::frames::FrameHook>> {
        self.frame_hook.take()
    }
}

/// Combo highlight: `None` when nothing is focused so the widget cannot
/// display index 0 as selected.
pub fn combo_index(focused: Option<&str>, names: &[String]) -> Option<usize> {
    focused.and_then(|n| names.iter().position(|x| x == n))
}

pub use crate::input_capture::{maybe_send_click, stream_capture};

/// rs2b0t disable rule: a script is active while it holds the slot, so
/// Start and Browse (and Load) are disabled for those states.
pub fn script_active(state: ScriptRunState) -> bool {
    matches!(
        state,
        ScriptRunState::Starting
            | ScriptRunState::Running
            | ScriptRunState::Paused
            | ScriptRunState::Stopping
    )
}

/// Pause/Resume enable rule: enabled only while Running (Pause) or Paused
/// (Resume); the button label switches to "Resume" when paused.
pub fn script_pause_enabled(state: ScriptRunState) -> bool {
    matches!(
        state,
        ScriptRunState::Starting | ScriptRunState::Running | ScriptRunState::Paused
    )
}

/// Stop enable rule: enabled while active, but not while already Stopping.
pub fn script_stop_enabled(state: ScriptRunState) -> bool {
    script_active(state) && state != ScriptRunState::Stopping
}

/// The loaded scene's tile count per side (the client's `BUILD_AREA_SIZE`):
/// the collision paint only covers the region the client has built,
/// `[base_x, base_x+104) × [base_z, base_z+104)` in world tiles.
const SCENE_TILES: i32 = 104;

/// Transport hops up to this many remaining tiles ahead get hull strokes;
/// the client only projects the locs inside its loaded scene anyway.
const HULL_WINDOW: usize = 48;

/// One selected route for the scene painter. All route storage stays borrowed
/// while the painter builds its small output buffers.
enum ScenePaintRoute<'a> {
    Legacy(&'a Route),
    Script(RouteProjectionView<'a>),
}

impl ScenePaintRoute<'_> {
    fn projection(&self) -> WalkRoutePath<'_> {
        match self {
            Self::Legacy(route) => WalkRoutePath::Legacy(route),
            Self::Script(route) => route.route(),
        }
    }
}

/// Shared route-owner precedence with the map overlay: driven live, then
/// script, then manual WalkTo. Script routes stay borrowed through projection.
fn live_or_walk_paint<'a>(
    driven: bool,
    live: (Option<&'a Route>, Option<WorldTile>),
    walk: (Option<&'a Route>, Option<WorldTile>),
    script: Option<RouteProjectionView<'a>>,
) -> (Option<ScenePaintRoute<'a>>, Option<WorldTile>) {
    match select_walk_route_source(
        driven && live.0.is_some(),
        script.is_some(),
        walk.0.is_some(),
    ) {
        Some(WalkRouteSource::Live) => (live.0.map(ScenePaintRoute::Legacy), live.1),
        Some(WalkRouteSource::Script) => (
            script.map(ScenePaintRoute::Script),
            script.and_then(RouteProjectionView::aim),
        ),
        _ => (walk.0.map(ScenePaintRoute::Legacy), walk.1),
    }
}

fn scene_route_fingerprint(route: Option<&ScenePaintRoute<'_>>) -> u64 {
    match route {
        None => crate::nav_paint_cache::route_fingerprint(None),
        Some(ScenePaintRoute::Legacy(route)) => {
            crate::nav_paint_cache::route_fingerprint(Some(route))
        }
        Some(ScenePaintRoute::Script(route)) => {
            use std::hash::{Hash, Hasher};
            let mut state = std::collections::hash_map::DefaultHasher::new();
            let stamp = route.stamp();
            let source = match stamp.source() {
                WalkRouteSource::Manual => 0u8,
                WalkRouteSource::Script => 1,
                WalkRouteSource::Live => 2,
            };
            source.hash(&mut state);
            stamp.generation().hash(&mut state);
            let destination = stamp.destination();
            destination.x.hash(&mut state);
            destination.z.hash(&mut state);
            destination.level.hash(&mut state);
            state.finish()
        }
    }
}

/// Route-only outputs. No route borrow or owner lock reaches the scene painter
/// or the camera update.
#[derive(Default)]
struct SceneRoutePaint {
    path: Vec<PathTile>,
    captions: Vec<HopCaption>,
    hulls: Vec<HullTarget>,
}

impl SceneRoutePaint {
    fn build(
        world: &NavWorld,
        route: Option<WalkRoutePath<'_>>,
        here: Option<WorldTile>,
        path: bool,
        hops: bool,
    ) -> Self {
        let route = match route {
            Some(WalkRoutePath::Legacy(route)) => {
                nav_planner::producer::route_view(world, route).ok()
            }
            Some(WalkRoutePath::Compact(route)) => Some(route),
            None => None,
        };
        let Some(route) = route else {
            return Self::default();
        };
        Self {
            path: if path {
                remaining_path_tiles(route, here)
            } else {
                Vec::new()
            },
            captions: if hops {
                hop_captions(route, here)
            } else {
                Vec::new()
            },
            hulls: if hops {
                hull_targets(route, here, HULL_WINDOW)
            } else {
                Vec::new()
            },
        }
    }
}

/// Shared nav-paint settings and their content generation. Slot threads
/// clone the `Arc` cheaply; UI pumping advances the generation only when a
/// setting actually changes.
#[derive(Clone, Default)]
struct NavPublishCfg {
    settings: Arc<NavSettings>,
    generation: u64,
}

/// Retire the debug-only producer on arrival or after leaving its route,
/// even while the slot is unfocused and no paint is being materialized.
fn retire_client_trail(client: &mut Client, here: Option<WorldTile>) {
    let Some(here) = here else {
        return;
    };
    let base_x = client.map_build_base_x;
    let base_z = client.map_build_base_z;
    let position = client
        .try_move_path
        .iter()
        .position(|&(x, z)| base_x + x == here.x && base_z + z == here.z);
    if position.is_none() || position.is_some_and(|i| i + 1 == client.try_move_path.len()) {
        client.try_move_path.clear();
    }
}

/// Map the client's last tryMove BFS into world tiles and trim it for
/// paint. The producer (`try_move_path`) is debug-only; retirement runs
/// independently every frame so an inactive slot cannot retain a stale
/// trail. `here == None` keeps a pending trail during startup/network wait.
fn live_client_trail(client: &mut Client, here: Option<WorldTile>) -> Vec<WorldTile> {
    let base_x = client.map_build_base_x;
    let base_z = client.map_build_base_z;
    let trail_all: Vec<WorldTile> = client
        .try_move_path
        .iter()
        .map(|&(sx, sz)| WorldTile {
            x: base_x + sx,
            z: base_z + sz,
            level: 0,
        })
        .collect();
    let trail_world = remaining_trail(&trail_all, here);
    if here.is_some() && trail_world.is_empty() {
        client.try_move_path.clear();
    }
    trail_world
}

/// Publish nav-debug scene facts for the focused GPU drawing slot. The
/// caller demand-gates this function by settings and visible nav facts, so
/// collision traversal and paint allocation occur only when the retained
/// view changes. Non-GPU, unfocused, skip-paint and renderer-off slots
/// publish `None` once when demand drops, preventing stale paint.
///
/// `world` is the baked pack, `route` contains the projected route buffers,
/// `trail_world` is the local player's last `tryMove` route buffer,
/// `run_on` its two-tone state, and `click` the
/// traveller's current walk aim. World-to-scene is
/// `(x - map_build_base_x, z - map_build_base_z)`.
#[expect(
    clippy::too_many_arguments,
    reason = "the scene painter consumes these facts at one call site"
)]
fn publish_nav_debug(
    client: &mut Client,
    world: &NavWorld,
    route: Option<SceneRoutePaint>,
    trail_world: &[WorldTile],
    run_on: bool,
    click: Option<WorldTile>,
    settings: &NavSettings,
    drawing: bool,
    // True when this call may ensure/drop the shared sidecar: the focused
    // drawer owns demand so a non-drawing peer cannot drop under it; when
    // focus is None any remaining slot (or the remove path) may release.
    flags_owner: bool,
) {
    // Flags sidecar lifecycle: decoded once while a collision paint toggle
    // is on **and** a surface actually draws it (the paint prefers the raw
    // flags — visibility bits the packed walk word drops — when the sidecar
    // is mapped). Saved prefs alone never load. The owner drops the sidecar
    // when it stops drawing or both toggles go off so the last drawer
    // releases ownership — including when focus is cleared and no slot
    // draws. Never opened for a pack the session only walks.
    if flags_owner {
        if drawing && (settings.collision_fill || settings.nsew_labels) {
            crate::picker::ensure_flags_sidecar();
        } else {
            crate::picker::drop_flags_sidecar();
        }
    }
    if !drawing {
        client.set_nav_debug_paint(None);
        return;
    }
    let base_x = client.map_build_base_x;
    let base_z = client.map_build_base_z;
    let mut colors = NavDebugColors::default();
    colors.collision = parse_html_color(&settings.color_collision, colors.collision);
    colors.nsew = parse_html_color(&settings.color_text, colors.nsew);
    colors.path = parse_html_color(&settings.color_path, colors.path);
    colors.path_hop = parse_html_color(&settings.color_transport, colors.path_hop);
    colors.trail = parse_html_color(&settings.color_client, colors.trail);
    colors.trail_run = parse_html_color(&settings.color_client_run_alt, colors.trail_run);
    colors.hull = parse_html_color(&settings.color_transport, colors.hull);
    colors.click = parse_html_color(&settings.color_click, colors.click);
    let mut paint = NavDebugPaint {
        colors,
        show_collision: settings.collision_fill,
        show_nsew: settings.nsew_labels,
        show_path: settings.show_nav_path,
        // `show_nav_path` is the master for the path / hull / trail paints
        // (spec Display row); the layer toggles opt each one in.
        show_trail: settings.show_nav_path && settings.client_trail,
        show_hulls: settings.show_nav_path && settings.hop_labels,
        ..NavDebugPaint::default()
    };
    // Collision: every tile of the loaded scene region whose
    // `collision_fill` (blocked ground) or `nsew_labels` (face letters)
    // toggle paints it, in scene coords. Tiles outside the pack grid read
    // as open — no phantom wall at the bake's edge.
    if settings.collision_fill || settings.nsew_labels {
        let level = world.collision.origin.level;
        let ox = world.collision.origin.x;
        let oz = world.collision.origin.z;
        let (ow, oh) = (world.collision.width as i32, world.collision.height as i32);
        // Prefer the decoded flags sidecar when it matches this world's
        // grid; otherwise the walk word answers (the shared world's
        // `flags` field stays `None` — no walk-grid clone).
        let side = crate::picker::flags_sidecar_for(
            world.collision.origin,
            world.collision.width,
            world.collision.height,
        );
        let side = side.as_ref().map(|v| v.as_slice());
        // The paint-only reach bitset, baked once per world. A missing
        // bitset defaults every cell to reached — no unreached tint.
        let reach_bits = crate::picker::reach_bitset(world);
        for lz in 0..SCENE_TILES {
            for lx in 0..SCENE_TILES {
                let x = base_x + lx;
                let z = base_z + lz;
                if x < ox || z < oz || x - ox >= ow || z - oz >= oh {
                    continue;
                }
                let wt = WorldTile { x, z, level };
                let fb = collision_at_with(&world.collision, wt, side);
                let mut bits = 0u8;
                if fb.n {
                    bits |= FACE_N;
                }
                if fb.s {
                    bits |= FACE_S;
                }
                if fb.e {
                    bits |= FACE_E;
                }
                if fb.w {
                    bits |= FACE_W;
                }
                if fb.ne {
                    bits |= CORNER_NE;
                }
                if fb.se {
                    bits |= CORNER_SE;
                }
                if fb.nw {
                    bits |= CORNER_NW;
                }
                if fb.sw {
                    bits |= CORNER_SW;
                }
                if (settings.collision_fill && (fb.blocked || bits != 0))
                    || (settings.nsew_labels && bits != 0)
                {
                    paint.collision.push(NavDebugCell {
                        lx,
                        lz,
                        bits,
                        // The client fills only blocked ground; a face-only
                        // cell keeps its NSEW letters without a fill quad.
                        blocked: fb.blocked,
                        // True when the transport network reaches the tile
                        // (or the bitset is missing): the client's reach
                        // fill draws only `show_collision && !reach`.
                        reach: reach_bits
                            .as_deref()
                            .is_none_or(|bits| reached(bits, &world.collision, wt)),
                    });
                }
            }
        }
    }
    if let Some(route) = route {
        // Path: the remaining route subsampled to the 3D draw budget.
        // Full density near, stride after, always keeping the transport
        // hops and the terminal (the pack map keeps the full path).
        if settings.show_nav_path {
            let tiles = &route.path;
            let force: Vec<usize> = tiles
                .iter()
                .enumerate()
                .filter_map(|(i, p)| p.transport.then_some(i))
                .collect();
            paint.path = select_draw_indices(0, tiles.len(), &force)
                .into_iter()
                .map(|i| {
                    let p = tiles[i];
                    (p.tile.x - base_x, p.tile.z - base_z, p.transport)
                })
                .collect();
        }
        // Hulls: the loc-backed transport hops ahead, scene coords.
        // `show_nav_path` masters them; the hop-labels toggle also
        // strokes the transport loc hulls (the client's transport colour
        // family covers both).
        if settings.show_nav_path && settings.hop_labels {
            paint.hulls = route
                .hulls
                .into_iter()
                .map(|h| NavDebugHull {
                    loc_id: h.loc_id,
                    scene_x: h.at.x - base_x,
                    scene_z: h.at.z - base_z,
                })
                .collect();
            paint.labels = route
                .captions
                .into_iter()
                .map(|c| (c.at.x - base_x, c.at.z - base_z, c.text))
                .collect();
        }
    }
    // Trail: client trail tones (two-tone while running). `show_nav_path`
    // is the master (spec Display row); `client_trail` opts the layer in.
    if settings.show_nav_path && settings.client_trail {
        paint.trail = trail_tones(trail_world, run_on)
            .into_iter()
            .map(|(t, tone)| (t.x - base_x, t.z - base_z, tone == TrailTone::RunAlt))
            .collect();
    }
    // Click: the traveller's current walk aim, scene coords. `show_nav_path`
    // masters it like the hulls (no nav path, no walk-target paint).
    if settings.show_nav_path {
        if let Some(aim) = click {
            paint.click = Some((aim.x - base_x, aim.z - base_z));
        }
    }
    client.set_nav_debug_paint(Some(paint));
}

/// Ease orbit yaw toward the remaining path (host-write, no client opcode).
/// Desired heading is held until it moves ≥ `TARGET_RETARGET_MIN` so a
/// corridor does not twitch; ease velocity is ours, not the keycam field
/// (`follow_camera` would double-integrate it).
fn apply_path_camera(client: &mut Client, tiles: &[PathTile], here: Option<WorldTile>) {
    if client.shell.key_held[1] == 1 || client.shell.key_held[2] == 1 {
        return;
    }
    let Some(here) = here else {
        return;
    };
    if tiles.is_empty() {
        return;
    }
    let world: Vec<_> = tiles.iter().map(|p| p.tile).collect();
    let hops: Vec<_> = tiles.iter().map(|p| p.transport).collect();
    let Some(sampled) = nav_planner::camera::path_facing_yaw(here, &world, &hops, 12) else {
        return;
    };
    let mut hold = PATH_CAM.lock().unwrap();
    hold.desired = Some(nav_planner::camera::hold_desired(hold.desired, sampled));
    let Some(target) = hold.desired else {
        return;
    };
    let (yaw, v) = nav_planner::camera::ease_yaw(client.orbit_camera_yaw, target, hold.vel);
    hold.vel = v;
    client.orbit_camera_yaw = yaw;
    // Keycam must not also integrate our ease velocity.
    client.orbit_camera_yaw_velocity = 0;
}

struct PathCamHold {
    desired: Option<i32>,
    vel: f32,
}

static PATH_CAM: Mutex<PathCamHold> = Mutex::new(PathCamHold {
    desired: None,
    vel: 0.0,
});

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalkSendMode {
    Focused,
    Group,
}

/// One bot of the Send list. Whether it is checked is read from the shared
/// marked-bots set (the one the rail, Grid and Fleet use), never stored here.
#[derive(Debug, Clone)]
pub struct WalkSendRow {
    pub name: String,
    identity: frontend_core::ProfileIdentity,
    pub status: WalkSlotStatus,
}

impl WalkSendRow {
    /// Profile name for this send row.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Marked and able to walk.
    pub fn checked(&self, marks: &frontend_core::MarkedSelection) -> bool {
        self.status.is_eligible() && marks.contains(self.identity)
    }
}

/// Send control state. Eligibility rows are rebuilt on open/refresh, not
/// every frame; the checks and the Walk label follow the shared marks live.
/// The bot list is the rail membership when MultiBox has any.
#[derive(Debug, Clone)]
pub struct WalkSendState {
    pub mode: WalkSendMode,
    /// The Send window is open.
    pub open: bool,
    rows: Vec<WalkSendRow>,
    walk_label: String,
    /// The mode and count `walk_label` was built for.
    labelled: (WalkSendMode, usize),
}

impl Default for WalkSendState {
    fn default() -> Self {
        Self {
            mode: WalkSendMode::Focused,
            open: false,
            rows: Vec::new(),
            walk_label: "Walk".into(),
            labelled: (WalkSendMode::Focused, 0),
        }
    }
}

impl WalkSendState {
    pub fn rows(&self) -> &[WalkSendRow] {
        &self.rows
    }

    pub fn walk_label(&self) -> &str {
        &self.walk_label
    }

    pub fn checked_count(&self, marks: &frontend_core::MarkedSelection) -> usize {
        self.rows.iter().filter(|row| row.checked(marks)).count()
    }

    /// Bring the Walk label up to date with the marks. Formats only when the
    /// mode or the count changed, so an idle frame allocates nothing.
    pub fn sync_walk_label(&mut self, marks: &frontend_core::MarkedSelection) {
        let count = match self.mode {
            WalkSendMode::Focused => 0,
            WalkSendMode::Group => self.checked_count(marks),
        };
        if self.labelled == (self.mode, count) {
            return;
        }
        self.labelled = (self.mode, count);
        self.walk_label.clear();
        match self.mode {
            WalkSendMode::Focused => self.walk_label.push_str("Walk"),
            WalkSendMode::Group => {
                use std::fmt::Write as _;
                let _ = write!(self.walk_label, "Walk {count} bots");
            }
        }
    }
}
/// One accepted panel-owned write: the request that carries it (its
/// identity, unique in this session), the member it changes, the value it
/// asks for (`None` removes the member) and the view its feedback belongs to.
#[derive(Clone)]
struct PanelPreferenceMember {
    request: u64,
    path: Vec<String>,
    value: Option<serde_json::Value>,
    owner: FeedbackOwner,
}

/// The view where a panel-owned write shows its "Saving…" and its failure.
#[derive(Clone, PartialEq, Eq)]
enum FeedbackOwner {
    /// Every view: a setting of the panel itself.
    Panel,
    /// The raster or memory choice made for this bot (`None`: no bot was
    /// focused). Its feedback shows only while that bot is focused.
    Bot(Option<String>),
}

impl FeedbackOwner {
    fn of(effect: Option<&LaunchEffect>) -> Self {
        match effect {
            Some(LaunchEffect::Raster { profile, .. } | LaunchEffect::Lowmem { profile, .. }) => {
                Self::Bot(profile.clone())
            }
            Some(LaunchEffect::ServerRevision(_)) | None => Self::Panel,
        }
    }

    fn shows_for(&self, focused: Option<&str>) -> bool {
        match self {
            Self::Panel => true,
            Self::Bot(bot) => bot.as_deref() == focused,
        }
    }
}

/// Whether the member at `path` lies under `prefix`.
fn under_prefix(path: &[String], prefix: &[&str]) -> bool {
    path.len() >= prefix.len() && path.iter().zip(prefix).all(|(a, b)| a == b)
}

struct PanelPreferenceWrite {
    update: Update,
    members: Vec<PanelPreferenceMember>,
    /// What a remembered launch choice does once this write is durable.
    effect: Option<LaunchEffect>,
}

/// What a panel-owned write became on submit.
enum PanelSubmit {
    /// Durable already: there was nothing to wait for.
    Done,
    /// Queued under this request identity.
    Pending(u64),
}

/// A remembered launch choice and what it does once its write is durable.
/// `profile` is the bot that was focused when the choice was made.
#[derive(Clone)]
enum LaunchEffect {
    /// The Game-pane raster: the profile's saved raster, the focused
    /// client's renderer and the slot's GPU/CPU latch.
    Raster {
        raster: RasterMode,
        profile: Option<String>,
    },
    /// The memory mode of the profile's next login.
    Lowmem {
        lowmem: bool,
        profile: Option<String>,
    },
    /// The server revision the next session binds with.
    ServerRevision(u16),
}

impl LaunchEffect {
    /// Whether this (newer) durable choice replaces `older`: the same
    /// setting for the same bot.
    fn supersedes(&self, older: &Self) -> bool {
        match (self, older) {
            (Self::Raster { profile, .. }, Self::Raster { profile: other, .. })
            | (Self::Lowmem { profile, .. }, Self::Lowmem { profile: other, .. }) => {
                profile == other
            }
            (Self::ServerRevision(_), Self::ServerRevision(_)) => true,
            _ => false,
        }
    }
}

/// What the launch controls show while their save is pending: each pending
/// request's choice under its identity (a result ends only its own
/// preview). A raster or memory choice shows only while the bot it was made
/// for is focused; the newest one for that bot wins.
#[derive(Default)]
struct LaunchPreviews(Vec<(u64, LaunchEffect)>);

impl LaunchPreviews {
    fn show(&mut self, request: u64, effect: LaunchEffect) {
        self.0.push((request, effect));
    }

    /// The request ended (durable or failed): its preview goes.
    fn settle(&mut self, request: u64) {
        self.0.retain(|(shown, _)| *shown != request);
    }

    /// The pending raster choice made for `focused`.
    fn raster(&self, focused: Option<&str>) -> Option<RasterMode> {
        self.0.iter().rev().find_map(|(_, effect)| match effect {
            LaunchEffect::Raster { raster, profile } if profile.as_deref() == focused => {
                Some(*raster)
            }
            _ => None,
        })
    }

    /// The pending memory choice made for `focused`.
    fn lowmem(&self, focused: Option<&str>) -> Option<bool> {
        self.0.iter().rev().find_map(|(_, effect)| match effect {
            LaunchEffect::Lowmem { lowmem, profile } if profile.as_deref() == focused => {
                Some(*lowmem)
            }
            _ => None,
        })
    }

    /// The pending server revision choice.
    fn server_revision(&self) -> Option<u16> {
        self.0.iter().rev().find_map(|(_, effect)| match effect {
            LaunchEffect::ServerRevision(revision) => Some(*revision),
            _ => None,
        })
    }
}

/// A failed panel-owned write kept at its setting, in the view it belongs
/// to: the request that failed (a later request's success ends it, an older
/// one's never does).
struct PanelWriteError {
    path: Vec<String>,
    owner: FeedbackOwner,
    request: u64,
    text: String,
    since: Instant,
}

/// The operator's unsettled change of each saved operational setting
/// (`None`: the control shows the saved value). Set when the command is
/// submitted; dropped once the core no longer holds a write for the setting
/// pending, whichever way it ended, so the control then shows the saved
/// value ([`Session::settle_setting_drafts`]).
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct SettingDrafts {
    pub(crate) capture: Option<bool>,
    pub(crate) sidecar_50: Option<bool>,
    pub(crate) focused_50: Option<bool>,
    pub(crate) quester_enabled: Option<bool>,
    pub(crate) map_bake: Option<MapBakeChoice>,
    pub(crate) session_log: Option<bool>,
}

enum StartupFocus {
    Restore,
    SavedProfile(String),
}

/// What a refusal on the banner waits for: the action that supplies it
/// clears the message at once ([`Session::clear_resolved_error`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ErrorNeed {
    /// A bot has the focus.
    FocusedBot,
    /// The focused bot has a script card.
    FocusedBotCard,
    /// The first save of the profile called this has finished.
    ProfileSaved(String),
}

pub struct Session {
    /// Shared focus policy; slot threads read it every frame (observe) to
    /// apply `client.set_draw(draw_for_slot(&focus, name))`, so only the
    /// focused slot rasters.
    pub focus: Arc<Mutex<crate::focus::Focus>>,
    /// Shared operator lifecycle: vault, play, fleet membership and latch,
    /// the selected bot (mirrored into `focus.focused` for slot threads),
    /// per-slot [`SlotIo`], removals and operation results.
    pub core: OperatorSession<SlotIo>,
    /// Last vault/connection error shown in the banner.
    pub error: Option<String>,
    /// The banner message's expiry clock ([`Self::expire_error`]).
    error_expiry: frontend_core::ErrorExpiry<ErrorNeed>,
    /// BOT_MAINLAND=1 / host-play --mainland; not a panel checkbox.
    pub mainland: Arc<AtomicBool>,
    /// The log section's filtered view and controls over the shared
    /// `frontend_core::log` store.
    pub log_pane: crate::log_pane::LogPane,
    /// Whether the dockable Log window is open; this is session-local.
    pub log_window_open: bool,
    /// Whether the Log body ran this frame; transient and not serialized.
    pub log_window_body_drawn: bool,
    /// Vault passphrase scratch buffer for the in-panel unlock prompt
    /// (zeroed when cleared or dropped).
    pub pass_scratch: Passphrase,
    /// Display copy of the core's last status poll, with queued walk
    /// destinations applied.
    pub statuses: frontend_core::StatusBuf,
    /// The Profiles edit form (core-owned): its target (`Some("")` a new
    /// profile), its staged draft (the password is zeroed when cleared or
    /// dropped), the Discard / Keep editing prompt and the save feedback.
    /// An existing profile's auto-login, random and lamp settings write
    /// through the vault at once; the form's generation keys its widgets.
    pub profile_editor: frontend_core::ProfileEditor,
    /// One-shot focus request from [`Session::open_window`] (or a staged
    /// Profiles leave prompt, which must come to the front even from a
    /// hidden tab): the named window's next `Begin` takes it once.
    pub window_focus: Option<PanelWindow>,
    /// WalkTo picker open flag; the picker window lands in Task 10.
    pub walkto_open: bool,
    /// WalkTo-only opt-out, reset when the picker opens; never persisted.
    pub route_through_zones: bool,
    /// Shared durable walk-policy projection; `ui.nav` is only an edit/render buffer.
    pub(crate) walk_permissions: frontend_core::WalkGlobalsView,
    /// The operator preferences (`panel-ui.json`) through the process
    /// local-state service this session opened: the one handle every reader
    /// and writer of the panel's preferences goes through.
    pub(crate) preferences: UiPreferences,
    /// Whether the process writer has published the first readable cache.
    preferences_ready: bool,
    /// Automatic unlock or saved-profile focus deferred to a preference poll
    /// after cache adoption. Explicit selection cancels it.
    startup_focus_pending: Option<StartupFocus>,
    /// Service-owned writes remain watched by the panel even when the
    /// settings surface that submitted them is closed.
    panel_preference_writes: Vec<PanelPreferenceWrite>,
    /// The panel-owned writes accepted and not yet settled, oldest first:
    /// each previews its value until its own result arrives.
    pending_panel_members: Vec<PanelPreferenceMember>,
    /// The last request identity handed out.
    panel_request_seq: u64,
    /// The durable version the panel-owned members last followed, and
    /// whether a bot was focused then. The raster and memory controls show
    /// the focused bot's modes or, with no bot focused, the saved document:
    /// a change of either part rebases them.
    panel_rebased: (Option<api::local_state::StoreVersion>, bool),
    /// What the launch controls show while their save is pending.
    launch_previews: LaunchPreviews,
    /// Shared Nav values changed by an accepted command but not yet durable.
    pending_nav_members: HashMap<&'static str, bool>,
    /// Failed panel-owned writes, one per member path, each kept at its
    /// setting until a later write of it lands or the error timeout passes.
    preference_write_errors: Vec<PanelWriteError>,
    /// Panel-owned setting transitions retained for the headed recorder.
    setting_events: Vec<frontend_core::SettingEvent>,
    /// Set when the bounded event queue drops at least one transition.
    setting_events_overflowed: bool,
    /// Whether setting transitions are recorded.
    record_setting_events: bool,
    /// Scratch the frame composes a labelled feedback line in (`label:
    /// text`), kept so a pending or failed row builds no string per frame.
    pub(crate) feedback_scratch: String,
    /// The operator's unsettled change of a saved operational setting: what
    /// its control shows while the write is pending. The saved value (the
    /// draw mirror, the Quester controller, the bake gate) follows the
    /// durable snapshot only, so a failed write shows the saved value again.
    pub(crate) setting_drafts: SettingDrafts,
    /// What the capture-input devices were last set up for; they follow the
    /// saved capture preference ([`Self::sync_capture_devices`]).
    capture_devices: Option<bool>,
    /// Shared Quester Paths settings and reload lifecycle.
    pub(crate) quester_paths: frontend_core::QuesterPathsController,
    pub(crate) quester_paths_folder_edit: String,
    /// Preserve uncommitted text while the durable controller is adopted.
    pub(crate) quester_paths_folder_dirty: bool,
    /// The folder text the last committed write carries while it is pending;
    /// the field keeps it until the write ends.
    quester_folder_submitted: Option<String>,
    /// A pending special-areas preview was dropped (its write ended).
    special_areas_dropped: bool,
    /// The durable version the shared Nav members last followed.
    nav_rebased: Option<api::local_state::StoreVersion>,
    /// Separate Fleet window open.
    pub fleet_open: bool,
    /// The one fleet model (rows, marks, filter, order) the Fleet window,
    /// the MultiBox rail and the Grid all read; changed by commands only.
    pub fleet_model: frontend_core::FleetModel,
    /// The draw policy the render mirror (`focus`) follows; changed by
    /// `SetDraw` only.
    pub(crate) draw: frontend_core::DrawPolicy,
    /// The open card drop-down: the toolbar site that owns it, its search
    /// text and the entries the search narrows (`None` while every
    /// drop-down is closed, so each opening starts with an empty search).
    pub(crate) fleet_card_filter:
        Option<(crate::fleet_toolbar::Site, crate::fleet_toolbar::CardSearch)>,
    /// The fleet toolbar's filter field buffer, shared by every toolbar on
    /// screen and edited in place by ImGui (no allocation per frame).
    pub(crate) fleet_filter_edit: dear_imgui_rs::ImString,
    /// The fleet toolbar's count-bearing labels (one set for every site that
    /// draws the toolbar), rebuilt only when the fleet model's generation
    /// moves.
    pub(crate) fleet_labels: crate::fleet_toolbar::FleetLabels,
    /// The one world picker every Switch world entry opens (a world marker,
    /// the Fleet window's World / login cell and the toolbar's Switch world
    /// button); closed until one asks.
    pub(crate) world_picker: crate::world_picker::WorldPicker,
    /// Core-owned map custody: staged actions, selected destination, cache
    /// demand, bake consent and panel input channel.
    pub(crate) map_desk: frontend_core::MapDesk,
    /// One application-owned map view/catalogue, never a copy on each bot.
    pub map_model: frontend_core::walk_map::WalkMap,
    pub map_catalogue: Option<Arc<frontend_core::walk_map::WalkCatalogue>>,
    pub map_images: Option<Arc<frontend_core::walk_map::WalkReadyImages>>,
    pub observed_map_services: Vec<frontend_core::walk_map::WalkObservedService>,
    map_catalogue_named: bool,
    /// Last published-catalogue probe while the map demand is not ready;
    /// probes run at [`MAP_CATALOGUE_PROBE`], never every frame.
    map_catalogue_probe: Option<Instant>,
    /// WalkTo Send: focused vs group checklist. Recomputed on open/refresh.
    pub walk_send: WalkSendState,
    /// Nav config window open flag (non-modal, same as General config).
    pub nav_settings_open: bool,
    /// Non-modal settings window (renderer / capture / mem).
    pub global_settings_open: bool,
    pub debug_panel: crate::debug_panel::DebugPanelState,
    pub debug_panel_open: bool,
    pub debug_open_teleports: bool,
    /// Usernames we already sent `getvar tutorial` for this session.
    tutorial_getvar_sent: HashSet<String>,
    /// Subset of `tutorial_getvar_sent` whose pending-text box we already
    /// clicked away (see `maybe_drain_getvar_dialogue`). Cleared when the
    /// slot's box goes away so the next box still drains exactly once.
    /// A slot that is unfocused or has no capture channel is never added,
    /// so its box waits for a later pump instead of being lost.
    tutorial_getvar_drained: HashSet<String>,
    /// Forgotten-password confirm: delete the vault file (locked only).
    pub vault_reset_open: bool,
    pub vault_reset_understood: bool,
    /// Chooser ✕ waiting on the same confirm popup; `None` not pending.
    pub pending_profile_delete: Option<String>,
    pub delete_understood: bool,
    #[cfg(feature = "js")]
    /// The rs2b0t profile import in the Profiles window: whether it is
    /// open, its flow, the text it shows while writes settle, and the
    /// delete-file confirm.
    pub profile_import_open: bool,
    #[cfg(feature = "js")]
    pub profile_import: frontend_core::rs2b0t_import::ProfileImport,
    #[cfg(feature = "js")]
    pub profile_import_lines: Vec<String>,
    #[cfg(feature = "js")]
    pub profile_import_settled: bool,
    #[cfg(feature = "js")]
    pub import_delete_understood: bool,
    /// Where the Profiles divider sits, as a fraction of the split region:
    /// `[0]` the list's width side by side, `[1]` its height stacked.
    pub profiles_split: [f32; 2],
    /// Live-harness overlay: the scenario's `NavSettings` for this session
    /// without writing prefs. `None` = operator `ui.nav`.
    pub nav_overlay: Option<NavSettings>,
    /// Optional headed live paint override; never persisted with operator UI.
    nav_paints_override: Option<bool>,
    /// Optional headed/live memory override; never persisted with operator UI.
    memory_override: Option<bool>,
    /// Per-frame nav-paint mirror the slot threads publish from each
    /// observe (see [`publish_nav_debug`]); `pump_status` re-copies it
    /// from `ui.nav` / [`Session::nav_overlay`] every UI frame.
    nav_publish: Arc<Mutex<NavPublishCfg>>,
    /// Overlay generation: bumped whenever the focused traveller's route
    /// can change (a new arm, or the focused profile switching). The path
    /// overlay rebuilds immediately on a bump instead of waiting for its
    /// 1 s raster cadence.
    route_gen: u64,
    mainland_sent: Arc<Mutex<HashSet<String>>>,
    /// Flat-model scatter: after scene 2, `::tele` each slot to a shuffled
    /// walkable tile (every slot is a full Client and seeds itself).
    scatter: Arc<AtomicBool>,
    /// The server-profile binding: launch inputs, prepared template and its
    /// validation, and the play connection derived from the profile.
    pub(crate) launcher: ProfileLauncher,
    /// Interactive Unlock clicked before preparation/final validation finished.
    requested_unlock: Option<Passphrase>,
    /// Multibox wall membership (chooser / latch / bulk ops). The UI reads
    /// it for the chooser and rail; [`Session`] methods drive it.
    pub wall: Wall,
    /// MultiBox toggle: rail (or grid) policy is up. `Focus.wall_open`
    /// mirrors this so extra rasters only run while the wall is visible.
    pub multibox: bool,
    /// Persisted panel prefs (last focus + per-profile collapsed sections).
    pub ui: crate::ui_state::PanelUiState,
    /// The script picked in Browse (compiled id or loaded JS card);
    /// `None` until one is selected. Selecting never Starts — Start is the
    /// section button. This is the focused heading; per-profile pending
    /// Browse lives in the shared [`frontend_core::Scripts`].
    pub script_sel: Option<ScriptChoice>,
    /// Shared script coordination (card library and catalog, assignment,
    /// per-profile parameters, Start/Stop all, reload, Apply to all).
    pub scripts: frontend_core::Scripts,
    /// Browse picker open flag (the Scripts window in `app.rs`).
    pub script_browse_open: bool,
    #[cfg(feature = "js")]
    /// Load file browser open flag (the Load window in `app.rs`).
    pub script_load_open: bool,
    #[cfg(feature = "js")]
    /// Current directory in the out-of-tree Load file browser.
    pub script_load_dir: PathBuf,
    #[cfg(feature = "js")]
    /// Selected row in the Load file browser.
    pub script_load_sel: usize,
    #[cfg(feature = "js")]
    /// Search box in the shared file dialog (Load or Import catalog).
    pub script_dialog_search: String,
    #[cfg(feature = "js")]
    /// Shared file-dialog sort column.
    pub script_dialog_sort: crate::script_picker::DialogSort,
    #[cfg(feature = "js")]
    pub script_dialog_sort_desc: bool,
    #[cfg(feature = "js")]
    /// First-run rs2b0t clone-root folder picker (when `$RS2B0T` unset).
    pub rs2b0t_catalog_open: bool,
    #[cfg(feature = "js")]
    /// Show **Not now** (first-run only; later Import catalog… does not defer).
    pub rs2b0t_catalog_defer_ok: bool,
    #[cfg(feature = "js")]
    /// Browse already asked for a clone root this session (no root known):
    /// a later Browse only brings its window forward.
    pub(crate) rs2b0t_browse_prompted: bool,
    #[cfg(feature = "js")]
    /// Current directory in the rs2b0t folder picker.
    pub rs2b0t_catalog_dir: PathBuf,
    /// Script Browse category filter (`None` = all categories).
    pub browse_category_filter: Option<String>,
    /// Parameters editor modal open.
    /// Modal typed editors moved to Script prefs window (0.1.6).
    pub script_prefs_open: bool,
    /// Search input for a searchable script-parameter combo while it is open.
    pub script_parameter_filter: String,
    /// Setting id that owns `script_parameter_filter` while its combo is open.
    pub script_parameter_filter_id: Option<String>,
    /// Retained copy of the selected card's schema the parameter editors
    /// walk while they change the session (copied only when it changes).
    pub(crate) script_param_schema: frontend_core::scripts::OwnedSchema,
    /// Reused range and draft-detail text, so warm prefs frames do not allocate it.
    pub(crate) script_parameter_label: String,
    /// Loadouts CRUD window (non-modal).
    pub loadouts_open: bool,
    /// Item search box for the active slot or supply row.
    pub loadouts_search: String,
    /// Equipment slot currently picking an item.
    pub loadouts_search_slot: Option<frontend_core::scripts::WornSlot>,
    /// Supply row currently picking an item.
    pub loadouts_search_supply: Option<usize>,
    /// Weak selected-data/query key for cached loadout search results.
    pub(crate) loadouts_search_cache_key: Option<(
        Option<std::sync::Weak<api::game_data::SelectedGameData>>,
        String,
        Option<frontend_core::scripts::WornSlot>,
    )>,
    /// Search results and formatted rows stay stable between query/data changes.
    pub(crate) loadouts_search_hits: Vec<api::game_data::ItemSearchHit>,
    pub(crate) loadouts_picker_rows: crate::name_picker::PickerRows,
    /// Per-supply quantity text buffers (stable across frames while editing).
    pub loadouts_qty_bufs: Vec<String>,
    /// Process-wide equipment/inventory presets: the rows, the selected
    /// one, its draft and the status line the loadout rows of the command
    /// seam act on.
    pub loadouts: LoadoutForm,
    catalog_core_enabled: bool,
    pair_core_enabled: bool,
    pub(super) live_script_stop_wait_started: Option<Instant>,
    #[cfg(test)]
    catalog_core_watch: Arc<Mutex<Option<host_play::catalog_core::CoreWatch>>>,
    #[cfg(test)]
    paired_core_watch: Arc<Mutex<Option<host_play::paired_core::PairWatch>>>,
    #[cfg(all(test, feature = "js"))]
    external_core_watch: Arc<Mutex<Option<host_play::external_loader::ExternalWatch>>>,
    #[cfg(feature = "js")]
    external_core_enabled: bool,
    #[cfg(feature = "js")]
    external_ts: Option<PathBuf>,
    /// Focused-slot speaker, reconciled from the client's applied memory mode,
    /// never from the queued setting.
    audio: Arc<AudioGate<AudioOut>>,
    /// Mem-popup Relog-now armed for this bot: the first click only arms
    /// (a running script would be interrupted), the second click relogs.
    /// Cleared when the relog starts or the button targets another bot.
    mem_relog_armed: Option<String>,
    /// Whether focus/multibox writes land on disk prefs. `true` in
    /// `Session::new`; every `live_prepare_*` flips it off so an ephemeral
    /// live boot never touches the operator's `last_focus`.
    pub persist_ui: bool,
    /// One-time notice that other profiles keep running after MultiBox off
    /// or a single-mode switch.
    pub background_ack_open: bool,
    /// Explicit PREPARE / RUN-PREPARED fixture path (default live is unchanged).
    pub fixture_mode: frontend_core::live::FixtureMode,
    /// Optional identity receipt path; default `~/.274bot/fixtures/<scenario>.json`.
    pub fixture_path: Option<PathBuf>,
}

/// A panel-owned line (audio, seeding) for `name` on the shared log.
fn slot_log(name: &str, level: Level, message: &str) {
    frontend_core::log::global().slot_line(name, Source::Host, level, message);
}

/// A panel-owned line no slot owns (vault, profile binding).
fn process_log(level: Level, message: &str) {
    frontend_core::log::global().process_line(Source::Host, level, message);
}

#[cfg(test)]
impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

/// The panel's per-frame client work on every slot: draw gating, nav-debug
/// scene paint, audio, first-world seeding and the `--live` runner.
struct PanelFrameHook {
    focus: Arc<Mutex<crate::focus::Focus>>,
    mainland: Arc<AtomicBool>,
    mainland_sent: Arc<Mutex<HashSet<String>>>,
    scatter: Arc<AtomicBool>,
    live_runner: frontend_core::live::RunnerHandle,
    audio: Arc<AudioGate<AudioOut>>,
    nav_publish: Arc<Mutex<NavPublishCfg>>,
    nav_paint_cache: Mutex<crate::nav_paint_cache::Cache>,
    audio_fail: Mutex<Option<(String, Instant)>>,
    scatter_template: Option<ProfileTemplate>,
}

impl frontend_core::frames::FrameHook for PanelFrameHook {
    fn frame(
        &self,
        client: &mut dyn std::any::Any,
        name: &str,
        frame: &frontend_core::frames::FrameContext<'_>,
    ) {
        let c = client
            .downcast_mut::<Client>()
            .expect("panel slots run the client this panel links");
        let Self {
            focus,
            mainland,
            mainland_sent,
            scatter,
            audio,
            live_runner,
            nav_publish,
            nav_paint_cache,
            audio_fail,
            scatter_template,
        } = self;
        if frame.walk_cancelled() {
            nav_paint_cache.lock().unwrap().invalidate(name);
        }
        let session_boundary = frame.session_boundary();
        if session_boundary && focus.lock().unwrap().focused.as_deref() == Some(name) {
            frame.mark_walk_clear();
        }
        if session_boundary {
            nav_paint_cache.lock().unwrap().invalidate(name);
        }
        // Flat model: every slot is a full Client; draw gates the
        // slot's renderer per the wall policy (focused always,
        // members when only-render-selected is off).
        let (focused, draw) = {
            let f = focus.lock().unwrap();
            (f.focused.clone(), draw_for_slot(&f, name))
        };
        c.set_draw(draw);
        // Nav-debug scene paint: only the focused drawing slot
        // publishes; a slot that stops drawing stores None so a
        // stale paint cannot linger. Flags demand is owned solely by
        // the focused slot so a non-drawing peer never loads or drops
        // the shared sidecar out from under the drawer.
        let (layers, settings_generation) = {
            let publish = nav_publish.lock().unwrap();
            (Arc::clone(&publish.settings), publish.generation)
        };
        let is_focused = focused.as_deref() == Some(name);
        let drawing = is_focused && draw;
        let walk = frame.walk_route();
        match crate::picker::pack() {
            Some(world) => {
                let here = c.local_player.as_ref().map(|lp| WorldTile {
                    x: c.map_build_base_x + lp.route_x[0],
                    z: c.map_build_base_z + lp.route_z[0],
                    level: 0,
                });
                retire_client_trail(c, here);
                // Run orb (varp 173 / 274 overlay), not the run animation.
                let run_on = c.run_enabled();
                let active = drawing && c.nav_debug_drawable();
                let camera = drawing
                    && layers.camera_follow
                    && here.is_some()
                    && c.shell.key_held[1] != 1
                    && c.shell.key_held[2] != 1;
                let previous = nav_paint_cache.lock().unwrap().last_frame(name);
                let base_facts = crate::nav_paint_cache::Facts {
                    settings_generation,
                    active,
                    base_x: c.map_build_base_x,
                    base_z: c.map_build_base_z,
                    here,
                    route: 0,
                    click: None,
                    trail: crate::nav_paint_cache::trail_fingerprint(&c.try_move_path),
                    run_on,
                };
                let (facts, changed, outputs) =
                    live_runner.with_route_for(name, |driven, live_route, live_aim| {
                        frame.with_script_route(|script_route| {
                            let (route, click) = live_or_walk_paint(
                                driven,
                                (live_route, live_aim),
                                (walk.0.as_ref(), walk.1),
                                script_route,
                            );
                            let facts = crate::nav_paint_cache::Facts {
                                route: scene_route_fingerprint(route.as_ref()),
                                click,
                                ..base_facts
                            };
                            let changed = previous != Some(facts);
                            let path = camera || (changed && active && layers.show_nav_path);
                            let hops =
                                changed && active && layers.show_nav_path && layers.hop_labels;
                            let outputs = if path || hops {
                                SceneRoutePaint::build(
                                    &world,
                                    route.as_ref().map(ScenePaintRoute::projection),
                                    here,
                                    path,
                                    hops,
                                )
                            } else {
                                SceneRoutePaint::default()
                            };
                            (facts, changed, outputs)
                        })
                    });
                // The route borrow and the process-wide navs lock end above.
                if camera {
                    apply_path_camera(c, &outputs.path, here);
                }
                if changed {
                    // Materialize the full tryMove trail only for an active
                    // GPU consumer, after releasing the route owner.
                    let trail_world = if active {
                        live_client_trail(c, here)
                    } else {
                        Vec::new()
                    };
                    let flags_owner = is_focused || focused.is_none();
                    publish_nav_debug(
                        c,
                        &world,
                        Some(outputs),
                        &trail_world,
                        run_on,
                        facts.click,
                        &layers,
                        active,
                        flags_owner,
                    );
                    let recorded = crate::nav_paint_cache::Facts {
                        trail: crate::nav_paint_cache::trail_fingerprint(&c.try_move_path),
                        ..facts
                    };
                    nav_paint_cache.lock().unwrap().record(name, recorded);
                }
            }
            None => {
                nav_paint_cache.lock().unwrap().invalidate(name);
                c.set_nav_debug_paint(None);
            }
        }
        // Sound follows the applied client mode, not the queued setting.
        // The open closure runs on this slot's client-owning thread.
        let change = audio.frame(name, focused.as_deref(), !c.config.lowmem, || {
            let now = Instant::now();
            if let Some((who, at)) = audio_fail.lock().unwrap().as_ref() {
                if who == name && now.duration_since(*at) < AUDIO_OPEN_RETRY {
                    return None;
                }
            }
            match AudioOut::try_open(c.midi.clone(), c.waves.clone(), c.fade.clone()) {
                Ok(out) => {
                    *audio_fail.lock().unwrap() = None;
                    slot_log(
                        name,
                        Level::Info,
                        &format!("audio: speaker open ({} Hz)", out.sample_rate),
                    );
                    Some(out)
                }
                Err(e) => {
                    *audio_fail.lock().unwrap() = Some((name.to_string(), now));
                    slot_log(name, Level::Warn, &format!("audio: {e}"));
                    None
                }
            }
        });
        if change == AudioChange::Closed {
            slot_log(name, Level::Info, "audio: speaker closed");
        }
        if c.ingame
            && c.scene_state == 2
            && seed_on_first_world(c.last_login_reconnect)
            && mainland_sent.lock().unwrap().insert(name.to_string())
        {
            if scatter.load(Ordering::Relaxed) {
                let t = scatter_template.as_ref().map_or_else(
                    || frontend_core::launch::default_scatter_tile_for(c.login_uid),
                    |template| template.scatter_tile_for(c.login_uid),
                );
                api::interact::seed_at(c, t.level, t.x, t.z);
                slot_log(
                    name,
                    Level::Info,
                    &format!("scatter seed {} {} {}", t.level, t.x, t.z),
                );
            } else if mainland.load(Ordering::Relaxed) {
                api::interact::mainland_hop(c);
                slot_log(name, Level::Info, "mainland hop queued");
            }
        }
    }
}

/// The saved setting a shared Nav member belongs to.
fn nav_member_setting(key: &str) -> frontend_core::Setting {
    use frontend_core::{NavPreference as N, Setting};
    match key {
        "allow_teleports" => Setting::Nav(N::AllowTeleports),
        "allow_wilderness" => Setting::Nav(N::AllowWilderness),
        "allow_bank_fetch" => Setting::Nav(N::AllowBankFetch),
        "allow_danger_zones" | "survivable_routing" => Setting::DangerRouting,
        "script_scope_notice_ack" => Setting::Nav(N::ScriptScopeNoticeAck),
        "survivable_routing_notice_ack" => Setting::Nav(N::SurvivableRoutingNoticeAck),
        "show_special_areas" => Setting::Nav(N::ShowSpecialAreas),
        "pause_script_on_manual_walk_abort" => Setting::Nav(N::PauseScriptOnManualWalkAbort),
        _ => unreachable!("only shared Nav members are retained"),
    }
}

fn apply_walk_permissions_to_nav(
    nav: &mut NavSettings,
    view: frontend_core::WalkGlobalsView,
    pending: &mut HashMap<&'static str, bool>,
) {
    let globals = view.globals();
    apply_walk_permission(nav, "allow_teleports", globals.allow_teleports(), pending);
    apply_walk_permission(nav, "allow_wilderness", globals.allow_wilderness(), pending);
    apply_walk_permission(nav, "allow_bank_fetch", globals.allow_bank_fetch(), pending);
    apply_walk_permission(
        nav,
        "allow_danger_zones",
        globals.allow_danger_zones(),
        pending,
    );
    apply_walk_permission(
        nav,
        "survivable_routing",
        globals.survivable_routing(),
        pending,
    );
    apply_walk_permission(
        nav,
        "script_scope_notice_ack",
        view.script_scope_notice_ack(),
        pending,
    );
    apply_walk_permission(
        nav,
        "survivable_routing_notice_ack",
        view.survivable_routing_notice_ack(),
        pending,
    );
}

fn apply_walk_permission(
    nav: &mut NavSettings,
    key: &'static str,
    durable: bool,
    pending: &mut HashMap<&'static str, bool>,
) {
    let value = match pending.get(key).copied() {
        Some(value) if value == durable => {
            pending.remove(key);
            durable
        }
        Some(value) => value,
        None => durable,
    };
    *nav_member_mut(nav, key) = value;
}

fn nav_member_mut<'a>(nav: &'a mut NavSettings, key: &'static str) -> &'a mut bool {
    match key {
        "allow_teleports" => &mut nav.allow_teleports,
        "allow_wilderness" => &mut nav.allow_wilderness,
        "allow_bank_fetch" => &mut nav.allow_bank_fetch,
        "allow_danger_zones" => &mut nav.allow_danger_zones,
        "survivable_routing" => &mut nav.survivable_routing,
        "script_scope_notice_ack" => &mut nav.script_scope_notice_ack,
        "survivable_routing_notice_ack" => &mut nav.survivable_routing_notice_ack,
        _ => unreachable!("only shared Nav members are refreshed"),
    }
}

fn apply_pending_nav_members(nav: &mut NavSettings, pending: &HashMap<&'static str, bool>) {
    for (key, value) in pending {
        match *key {
            "allow_teleports" => nav.allow_teleports = *value,
            "allow_wilderness" => nav.allow_wilderness = *value,
            "allow_bank_fetch" => nav.allow_bank_fetch = *value,
            "allow_danger_zones" => nav.allow_danger_zones = *value,
            "survivable_routing" => nav.survivable_routing = *value,
            "script_scope_notice_ack" => nav.script_scope_notice_ack = *value,
            "survivable_routing_notice_ack" => nav.survivable_routing_notice_ack = *value,
            "show_special_areas" => nav.show_special_areas = *value,
            "pause_script_on_manual_walk_abort" => {
                nav.pause_script_on_manual_walk_abort = *value;
            }
            _ => unreachable!("only shared Nav members are retained"),
        }
    }
}

#[cfg(test)]
std::thread_local! {
    /// `ReorderProfiles` submits made on this test thread.
    static REORDERS_SUBMITTED: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// How many reorders this test thread has submitted.
#[cfg(test)]
pub(crate) fn reorders_submitted() -> u32 {
    REORDERS_SUBMITTED.get()
}

impl Session {
    /// Empty session for tests. Product startup uses [`Self::with_instance`].
    #[cfg(test)]
    pub fn new() -> Self {
        Self::with_instance(frontend_core::launch::InstancePermit::skip())
    }

    /// Interactive startup must pass the lock outcome so prefs/vault cannot
    /// load without a decision. Harness boots pass [`InstancePermit::SkipLock`].
    pub fn with_instance(instance: frontend_core::launch::InstancePermit) -> Self {
        #[cfg(test)]
        frontend_core::test_support::IsolatedStores::ensure_thread();
        // The composition root: the one place that resolves HOME for state.
        // The process local-state service opened here is the one writer of
        // the operator preferences and loadouts for everything below, and
        // the Play this session starts adopts it too.
        let mut core = OperatorSession::open(instance);
        core.open_process_state();
        let preferences = core
            .preferences()
            .expect("the session's local state was bound just now");
        let preferences_ready = preferences.ready();
        let mut ui = crate::ui_state::load(&preferences);
        let walk_permissions = core.adopt_operator_preferences();
        let mut quester_paths = frontend_core::QuesterPathsController::default();
        quester_paths.restore(&preferences);
        let quester_paths_folder_edit = quester_paths
            .settings()
            .folder()
            .to_string_lossy()
            .into_owned();
        let mut pending_nav_members = HashMap::new();
        apply_walk_permissions_to_nav(&mut ui.nav, walk_permissions, &mut pending_nav_members);
        let capture_pref = ui.capture;
        let (sidecar_50_pref, focused_50_pref) = (ui.sidecar_50, ui.focused_50);
        let map_desk = frontend_core::MapDesk::new(ui.map_bake);

        // Loaded now through the service. A file it refused leaves the form
        // empty with the reason, which the Loadouts window shows as its status.
        let loadouts = LoadoutForm::operator_of(&core);
        let (scripts, library_error) = frontend_core::Scripts::open_default();
        // A missing library store is a first run; a refused or corrupt one is
        // said out loud (and nothing is saved over a refused one).
        if let Some(error) = library_error {
            eprintln!("panel: {error}");
            process_log(Level::Error, &error);
        }
        Self {
            focus: Arc::new(Mutex::new(crate::focus::Focus {
                focused: None,
                renderer: true,
                game_pane_open: true,
                capture: capture_pref,
                only_render_selected: true,
                sidecar_50: sidecar_50_pref,
                live_full_rate: false,
                focused_50: focused_50_pref,
                wall_open: false,
                wall: Vec::new(),
                renderer_by: HashMap::new(),
            })),
            core,
            error: None,
            error_expiry: frontend_core::ErrorExpiry::default(),
            mainland: Arc::new(AtomicBool::new(
                env::var("BOT_MAINLAND").as_deref() == Ok("1"),
            )),
            log_pane: crate::log_pane::LogPane::default(),
            log_window_open: false,
            log_window_body_drawn: false,
            pass_scratch: Passphrase::with_capacity(256),
            statuses: frontend_core::StatusBuf::default(),
            profile_editor: frontend_core::ProfileEditor::new(),
            window_focus: None,
            #[cfg(feature = "js")]
            profile_import_open: false,
            #[cfg(feature = "js")]
            profile_import: frontend_core::rs2b0t_import::ProfileImport::new(),
            #[cfg(feature = "js")]
            profile_import_lines: Vec::new(),
            #[cfg(feature = "js")]
            profile_import_settled: false,
            #[cfg(feature = "js")]
            import_delete_understood: false,
            profiles_split: [0.4, 0.35],
            fleet_open: false,
            fleet_model: frontend_core::FleetModel::new(),
            draw: frontend_core::DrawPolicy::new(
                capture_pref,
                sidecar_50_pref,
                focused_50_pref,
                true,
            ),
            fleet_card_filter: None,
            fleet_filter_edit: dear_imgui_rs::ImString::with_capacity(64),
            fleet_labels: crate::fleet_toolbar::FleetLabels::default(),
            world_picker: crate::world_picker::WorldPicker::default(),
            walkto_open: false,
            route_through_zones: false,
            walk_permissions,
            preferences,
            preferences_ready,
            startup_focus_pending: None,
            panel_preference_writes: Vec::new(),
            pending_panel_members: Vec::new(),
            panel_request_seq: 0,
            pending_nav_members,
            preference_write_errors: Vec::new(),
            setting_events: Vec::new(),
            setting_events_overflowed: false,
            record_setting_events: false,
            feedback_scratch: String::new(),
            setting_drafts: SettingDrafts::default(),
            capture_devices: Some(capture_pref),
            quester_paths,
            quester_paths_folder_edit,
            quester_paths_folder_dirty: false,
            quester_folder_submitted: None,
            panel_rebased: (None, false),
            launch_previews: LaunchPreviews::default(),
            special_areas_dropped: false,
            nav_rebased: None,
            map_desk,
            map_model: frontend_core::walk_map::WalkMap::new(),
            map_catalogue: None,
            map_images: None,
            observed_map_services: Vec::new(),
            map_catalogue_named: false,
            map_catalogue_probe: None,
            walk_send: WalkSendState::default(),
            nav_settings_open: false,
            global_settings_open: false,
            debug_panel: crate::debug_panel::DebugPanelState::default(),
            debug_panel_open: false,
            debug_open_teleports: false,
            tutorial_getvar_sent: HashSet::new(),
            tutorial_getvar_drained: HashSet::new(),
            vault_reset_open: false,
            vault_reset_understood: false,
            pending_profile_delete: None,
            delete_understood: false,
            nav_overlay: None,
            nav_paints_override: None,
            memory_override: None,
            nav_publish: Arc::new(Mutex::new(NavPublishCfg::default())),
            route_gen: 0,
            mainland_sent: Arc::new(Mutex::new(HashSet::new())),
            scatter: Arc::new(AtomicBool::new(false)),
            wall: Wall::default(),
            multibox: false,
            ui,
            script_sel: None,
            scripts,
            script_browse_open: false,
            #[cfg(feature = "js")]
            script_load_open: false,
            #[cfg(feature = "js")]
            script_load_dir: crate::script_picker::default_load_browse_dir(None),
            #[cfg(feature = "js")]
            script_load_sel: 0,
            #[cfg(feature = "js")]
            script_dialog_search: String::new(),
            #[cfg(feature = "js")]
            script_dialog_sort: crate::script_picker::DialogSort::Name,
            #[cfg(feature = "js")]
            script_dialog_sort_desc: false,
            #[cfg(feature = "js")]
            rs2b0t_catalog_open: false,
            #[cfg(feature = "js")]
            rs2b0t_catalog_defer_ok: false,
            #[cfg(feature = "js")]
            rs2b0t_browse_prompted: false,
            #[cfg(feature = "js")]
            rs2b0t_catalog_dir: crate::script_picker::default_load_browse_dir(None),
            browse_category_filter: None,
            script_prefs_open: false,
            script_parameter_filter: String::new(),
            script_parameter_filter_id: None,
            script_param_schema: frontend_core::scripts::OwnedSchema::default(),
            script_parameter_label: String::new(),
            loadouts_open: false,
            loadouts_search: String::new(),
            loadouts_search_slot: None,
            loadouts_search_supply: None,
            loadouts_search_cache_key: None,
            loadouts_search_hits: Vec::new(),
            loadouts_picker_rows: crate::name_picker::PickerRows::default(),
            loadouts_qty_bufs: Vec::new(),
            loadouts,
            live_script_stop_wait_started: None,
            catalog_core_enabled: false,
            pair_core_enabled: false,
            #[cfg(test)]
            catalog_core_watch: Arc::new(Mutex::new(None)),
            #[cfg(test)]
            paired_core_watch: Arc::new(Mutex::new(None)),
            #[cfg(all(test, feature = "js"))]
            external_core_watch: Arc::new(Mutex::new(None)),
            #[cfg(feature = "js")]
            external_core_enabled: false,
            #[cfg(feature = "js")]
            external_ts: None,
            audio: Arc::new(AudioGate::default()),
            mem_relog_armed: None,
            persist_ui: true,
            background_ack_open: false,
            fixture_mode: frontend_core::live::FixtureMode::Default,
            fixture_path: None,
            launcher: ProfileLauncher::legacy_local(),
            requested_unlock: None,
        }
    }

    /// Refresh the durable policy before rendering or admitting a manual walk.
    pub(crate) fn refresh_walk_permissions(&mut self) {
        self.adopt_panel_preferences_if_ready();
        self.prune_pending_nav_members();
        self.walk_permissions = self.core.refresh_walk_globals();
        apply_walk_permissions_to_nav(
            &mut self.ui.nav,
            self.walk_permissions,
            &mut self.pending_nav_members,
        );
        self.route_through_zones = self
            .walk_permissions
            .danger_this_walk(self.route_through_zones);
    }

    /// One reconciliation of the saved operator settings: the core settles
    /// every accepted write by its request and lets every lent model follow
    /// the latest durable snapshot ([`OperatorSession::poll_preferences`]),
    /// calling back through the surface when a saved draw switch changes;
    /// the panel's own mirrors, drafts and capture devices then follow.
    fn follow_core_preferences(&mut self) {
        {
            let (core, mut surface) = self.core_and_surface();
            core.poll_preferences(&mut surface);
        }
        self.settle_setting_drafts();
        self.sync_capture_devices();
    }

    /// Drop each unsettled draft whose write is no longer pending (it landed
    /// or failed: the control then shows the saved value), and mirror the
    /// saved values the panel keeps in its own view model.
    pub(crate) fn settle_setting_drafts(&mut self) {
        use frontend_core::{Setting, SettingStatus};
        let pending = |core: &OperatorSession<SlotIo>, setting| {
            matches!(core.setting_status(setting), SettingStatus::Pending)
        };
        let core = &self.core;
        let drafts = &mut self.setting_drafts;
        if !pending(core, Setting::Draw(DrawSwitch::CaptureInput)) {
            drafts.capture = None;
        }
        if !pending(core, Setting::Draw(DrawSwitch::SidecarFullRate)) {
            drafts.sidecar_50 = None;
        }
        if !pending(core, Setting::Draw(DrawSwitch::FocusedFullRate)) {
            drafts.focused_50 = None;
        }
        if !pending(core, Setting::QuesterPaths) {
            drafts.quester_enabled = None;
        }
        if !pending(core, Setting::MapBake) {
            drafts.map_bake = None;
        }
        if !pending(core, Setting::SessionLog) {
            drafts.session_log = None;
        }
        self.prune_pending_nav_members();
        if !matches!(
            self.core.setting_status(Setting::QuesterPaths),
            SettingStatus::Pending
        ) {
            // The folder write ended: the field shows the saved folder
            // (what landed, or the prior one when it failed) unless the
            // operator has typed something newer.
            if let Some(submitted) = self.quester_folder_submitted.take() {
                if self.quester_paths_folder_edit == submitted {
                    self.quester_paths_folder_dirty = false;
                    self.quester_paths_folder_edit = self
                        .quester_paths
                        .settings()
                        .folder()
                        .to_string_lossy()
                        .into_owned();
                }
            }
        }
        if !self.quester_paths_folder_dirty && self.quester_folder_submitted.is_none() {
            // A clean field shows the saved folder: another process's save
            // reaches it. Typed or submitted text stays.
            let folder = self.quester_paths.settings().folder();
            if Path::new(&self.quester_paths_folder_edit) != folder {
                self.quester_paths_folder_edit = folder.to_string_lossy().into_owned();
            }
        }
        self.ui.capture = self.draw.capture();
        self.ui.sidecar_50 = self.draw.sidecar_50();
        self.ui.focused_50 = self.draw.focused_50();
        self.ui.map_bake = self.map_desk.bake_gate().choice();
        if let Ok(document) = self.preferences.document() {
            self.ui.session_log_file = document
                .get(frontend_core::log_file::SESSION_LOG_KEY)
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            self.ui.background_bots_ack = document
                .get("background_bots_ack")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            self.ui.render_all_warning_ack = document
                .get(frontend_core::session::RENDER_ALL_WARNING_ACK_KEY)
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            // The two shared Nav members the panel holds beside the walk
            // grants follow the saved value (when a new version arrived)
            // unless the operator's own change of them is still pending.
            let version = self
                .preferences
                .snapshot()
                .ok()
                .map(|snapshot| snapshot.version());
            if version != self.nav_rebased {
                self.nav_rebased = version;
                self.rebase_shared_nav_members();
            }
        }
        self.settle_notice_windows();
    }

    /// A notice window that waits for its durable acknowledgement closes
    /// when the saved value holds it; a failed write leaves it open with
    /// the failure beside "Don't show again" / "Got it". The render-all
    /// warning's "Don't show again" renders every wall member only then: a
    /// pending or failed acknowledgement changes nothing.
    fn settle_notice_windows(&mut self) {
        use frontend_core::{OperatorNotice, Setting, SettingStatus};
        if self.background_ack_open
            && self.ui.background_bots_ack
            && !matches!(
                self.core
                    .setting_status(Setting::Notice(OperatorNotice::BackgroundBots)),
                SettingStatus::Pending
            )
        {
            self.background_ack_open = false;
        }
        if self.wall.render_all_confirming
            && !matches!(
                self.core
                    .setting_status(Setting::Notice(OperatorNotice::RenderAllWarning)),
                SettingStatus::Pending
            )
        {
            self.wall.render_all_confirming = false;
            if self.ui.render_all_warning_ack {
                self.wall.render_all_warn_open = false;
                self.set_render_all(true);
            }
        }
    }

    /// The capture-input devices follow the saved capture preference: on
    /// attaches the focused slot's drain, off detaches user input. They
    /// change when the draw mirror changes, never at a submit.
    fn sync_capture_devices(&mut self) {
        let capture = self.focus.lock().unwrap().capture;
        if self.capture_devices == Some(capture) {
            return;
        }
        self.capture_devices = Some(capture);
        let name = self.focused_name();
        if capture {
            match name.as_deref() {
                Some(name) => self.capture_on(name),
                None => {
                    self.map_desk.set_input_sender(None);
                }
            }
        } else {
            self.capture_off();
        }
        // The channel is attached after the core woke the slot; kick it
        // again so the drain change lands within a frame.
        if let Some(name) = name {
            self.core.wake_slot(&name);
        }
    }

    /// Poll accepted panel writes and adopt the first cached snapshot.
    pub(crate) fn poll_panel_preferences(&mut self) {
        self.follow_core_preferences();
        self.expire_panel_write_errors(Instant::now());
        // Completed writes end in request order (the order the writer made
        // them durable), whatever order their results are read in. Only the
        // newest durable choice of each launch setting takes effect.
        let mut durable_effects = Vec::new();
        let mut index = 0;
        while index < self.panel_preference_writes.len() {
            let outcome = match &mut self.panel_preference_writes[index].update {
                Update::Durable => Some(Ok(())),
                Update::Queued(update) => update.poll(),
            };
            let Some(outcome) = outcome else {
                index += 1;
                continue;
            };
            let write = self.panel_preference_writes.remove(index);
            match outcome {
                Ok(()) => {
                    self.clear_succeeded_panel_members(&write.members);
                    for member in &write.members {
                        self.note_panel_setting(
                            member,
                            frontend_core::SettingEventState::Saved,
                            None,
                        );
                    }
                    if let Some(effect) = write.effect {
                        durable_effects.retain(|older: &LaunchEffect| !effect.supersedes(older));
                        durable_effects.push(effect);
                    }
                }
                Err(error) => self.fail_panel_members(&write.members, &error.to_string()),
            }
        }
        for effect in durable_effects {
            self.apply_launch_effect(effect);
        }

        if let Ok(document) = self.preferences.document() {
            self.clear_cached_nav_members(&document);
            self.adopt_panel_preferences(&document);
            // Every later durable version (another process's save, or this
            // panel's own landing) rebases the panel-owned members over the
            // new document, keeping the previews of requests still pending.
            // So does the end (or start) of focus: with no bot focused, the
            // raster and memory controls show the saved document at once,
            // without a later version.
            if self.preferences_ready {
                let version = self
                    .preferences
                    .snapshot()
                    .ok()
                    .map(|snapshot| snapshot.version());
                let context = (version, self.core.selected().is_some());
                if context != self.panel_rebased {
                    self.panel_rebased = context;
                    self.rebase_panel_members();
                }
            }
            // Startup selection is a between-command effect. Lazy policy
            // reads may adopt preferences, but must not replace a command's
            // captured profile or map context.
            if self.startup_focus_pending.is_some() && self.core.prepare_startup() {
                if let Some(focus) = self.startup_focus_pending.take() {
                    self.apply_startup_focus(focus);
                }
            }
        }
    }

    pub(crate) fn panel_preferences_ready(&self) -> bool {
        self.preferences_ready
    }

    #[cfg(test)]
    pub(crate) fn panel_preference_writes_pending(&self) -> usize {
        self.panel_preference_writes.len()
    }

    /// Whether a core-owned or panel-owned preference write is still pending.
    pub(crate) fn preferences_pending(&self) -> bool {
        self.core.preferences_pending() || !self.panel_preference_writes.is_empty()
    }

    /// The newest panel-write failure of the focused view (the global config
    /// shows it).
    pub(crate) fn panel_preference_write_error(&self) -> Option<&str> {
        let focused = self.core.selected();
        self.preference_write_errors
            .iter()
            .rev()
            .find(|error| error.owner.shows_for(focused))
            .map(|error| error.text.as_str())
    }

    /// The retained failure of the panel-owned setting whose members lie
    /// under `prefix`, shown at that setting (a bot's raster or memory
    /// failure only while that bot is focused).
    pub(crate) fn panel_member_error(&self, prefix: &[&str]) -> Option<&str> {
        let focused = self.core.selected();
        self.preference_write_errors
            .iter()
            .rev()
            .find(|error| error.owner.shows_for(focused) && under_prefix(&error.path, prefix))
            .map(|error| error.text.as_str())
    }

    /// Whether a panel-owned write of a member under `prefix` is accepted
    /// and not yet settled: the setting's control says "Saving…" (a bot's
    /// raster or memory choice only while that bot is focused).
    pub(crate) fn panel_member_pending(&self, prefix: &[&str]) -> bool {
        let focused = self.core.selected();
        self.pending_panel_members
            .iter()
            .any(|member| member.owner.shows_for(focused) && under_prefix(&member.path, prefix))
    }

    /// Record saved-setting transitions (the core's and this session's
    /// panel-owned members) for the headed recorder's settings sidecar.
    /// Called once when the recorder is enabled; while not called, a
    /// transition costs one bool check and records nothing.
    pub(crate) fn watch_setting_events(&mut self) {
        self.record_setting_events = true;
        self.core.watch_setting_events(true);
    }

    /// The panel-owned members' transitions and overflow status since the
    /// last call.
    pub(crate) fn take_panel_setting_events(&mut self) -> (Vec<frontend_core::SettingEvent>, bool) {
        (
            std::mem::take(&mut self.setting_events),
            std::mem::take(&mut self.setting_events_overflowed),
        )
    }

    /// Record one panel-owned member's transition. The queue is bounded
    /// like the core's; overflow is reported with the next drained batch.
    fn note_panel_setting(
        &mut self,
        member: &PanelPreferenceMember,
        state: frontend_core::SettingEventState,
        error: Option<&str>,
    ) {
        self.note_panel_setting_for(&member.path, &member.owner, state, error);
    }

    fn note_panel_setting_for(
        &mut self,
        path: &[String],
        owner: &FeedbackOwner,
        state: frontend_core::SettingEventState,
        error: Option<&str>,
    ) {
        if !self.record_setting_events {
            return;
        }
        if self.setting_events.len() >= PANEL_SETTING_EVENTS_CAP {
            self.setting_events_overflowed = true;
            return;
        }
        let path = path.join(".");
        let setting = match owner {
            FeedbackOwner::Panel => path,
            FeedbackOwner::Bot(Some(profile)) => format!("bot:{profile}:{path}"),
            FeedbackOwner::Bot(None) => format!("bot:unfocused:{path}"),
        };
        self.setting_events.push(frontend_core::SettingEvent::new(
            setting,
            state,
            error.map(str::to_owned),
        ));
    }

    /// Submit one exact panel-owned preference member without waiting.
    pub(crate) fn persist_panel_member(
        &mut self,
        path: &[&str],
        value: Option<serde_json::Value>,
    ) -> Result<(), String> {
        if !self.persist_ui {
            return Ok(());
        }
        self.submit_panel_member(path, value, None).map(|_| ())
    }

    /// Submit one remembered launch choice: it takes effect (`effect`) only
    /// when its write is durable, and the control shows it meanwhile. A run
    /// that saves nothing has no write to wait for: the effect is applied
    /// at once. `false` when the write was refused before it was accepted
    /// (the failure shows at the setting).
    fn choose_launch(
        &mut self,
        path: &[&str],
        value: Option<serde_json::Value>,
        effect: LaunchEffect,
    ) -> bool {
        if !self.persist_ui {
            self.apply_launch_effect(effect);
            return true;
        }
        match self.submit_panel_member(path, value, Some(effect.clone())) {
            Ok(PanelSubmit::Done) => true,
            Ok(PanelSubmit::Pending(request)) => {
                self.launch_previews.show(request, effect);
                true
            }
            Err(_) => false,
        }
    }

    /// Apply what a remembered launch choice does now that it is durable.
    fn apply_launch_effect(&mut self, effect: LaunchEffect) {
        match effect {
            LaunchEffect::ServerRevision(revision) => {
                // A profile bound while the save was pending keeps what it
                // bound with: the revision changes only for the next session.
                if self.launcher.bound().is_none() {
                    self.ui.server_revision = revision;
                    self.launcher.bump_generation();
                }
            }
            LaunchEffect::Raster { raster, profile } => self.apply_raster_choice(raster, profile),
            LaunchEffect::Lowmem { lowmem, profile } => self.apply_lowmem_choice(lowmem, profile),
        }
    }

    /// The request a panel-owned write became: durable already (nothing was
    /// left to wait for), or queued under its identity.
    fn submit_panel_member(
        &mut self,
        path: &[&str],
        value: Option<serde_json::Value>,
        effect: Option<LaunchEffect>,
    ) -> Result<PanelSubmit, String> {
        self.panel_request_seq += 1;
        let member = PanelPreferenceMember {
            request: self.panel_request_seq,
            path: path.iter().map(|part| (*part).to_owned()).collect(),
            value: value.clone(),
            owner: FeedbackOwner::of(effect.as_ref()),
        };
        // A new write of the member is the action that resolves its failure.
        let failures_before = self.preference_write_errors.len();
        self.preference_write_errors
            .retain(|error| error.path != member.path || error.owner != member.owner);
        if self.preference_write_errors.len() != failures_before {
            self.note_panel_setting(&member, frontend_core::SettingEventState::Cleared, None);
        }
        self.pending_panel_members.push(member.clone());
        let patch = match PreferencePatch::member(path, value) {
            Ok(patch) => patch,
            Err(error) => {
                let message = error.to_string();
                self.fail_panel_members(std::slice::from_ref(&member), &message);
                return Err(message);
            }
        };
        if patch.is_empty() {
            // The document already holds the value: there is nothing to wait for.
            self.pending_panel_members
                .retain(|pending| pending.request != member.request);
            if let Some(effect) = effect {
                self.apply_launch_effect(effect);
            }
            return Ok(PanelSubmit::Done);
        }
        match self.preferences.begin_patch(patch) {
            Ok(Update::Durable) => {
                self.clear_succeeded_panel_members(std::slice::from_ref(&member));
                self.note_panel_setting(&member, frontend_core::SettingEventState::Saved, None);
                if let Some(effect) = effect {
                    self.apply_launch_effect(effect);
                }
                Ok(PanelSubmit::Done)
            }
            Ok(update @ Update::Queued(_)) => {
                let request = member.request;
                self.note_panel_setting(&member, frontend_core::SettingEventState::Pending, None);
                self.panel_preference_writes.push(PanelPreferenceWrite {
                    update,
                    members: vec![member],
                    effect,
                });
                Ok(PanelSubmit::Pending(request))
            }
            Err(error) => {
                let message = error.to_string();
                self.fail_panel_members(std::slice::from_ref(&member), &message);
                Err(message)
            }
        }
    }
    pub(crate) fn persist_panel_serializable<T: serde::Serialize + ?Sized>(
        &mut self,
        path: &[&str],
        value: &T,
    ) -> Result<(), String> {
        match serde_json::to_value(value) {
            Ok(value) => self.persist_panel_member(path, Some(value)),
            Err(error) => {
                let member = PanelPreferenceMember {
                    request: 0,
                    path: path.iter().map(|part| (*part).to_owned()).collect(),
                    value: None,
                    owner: FeedbackOwner::Panel,
                };
                let message = error.to_string();
                self.set_preference_write_error(&member, &message, Instant::now());
                self.note_panel_setting(
                    &member,
                    frontend_core::SettingEventState::Failed,
                    Some(&message),
                );
                Err(message)
            }
        }
    }

    /// Retain a shared Nav command's value over refreshes until its durable
    /// view catches up. The panel does not save these core-owned members.
    pub(crate) fn remember_nav_preference(&mut self, key: &'static str, value: bool) {
        self.pending_nav_members.insert(key, value);
    }

    /// A shared Nav value the operator changed stays over refreshes only
    /// while its command's write is pending; a write that failed or never
    /// started leaves the durable value showing again.
    fn prune_pending_nav_members(&mut self) {
        let core = &self.core;
        let special_areas = &mut self.special_areas_dropped;
        let mut dropped_any = false;
        self.pending_nav_members.retain(|key, _| {
            let pending = matches!(
                core.setting_status(nav_member_setting(key)),
                frontend_core::SettingStatus::Pending
            );
            if !pending {
                dropped_any = true;
                if *key == "show_special_areas" {
                    *special_areas = true;
                }
            }
            pending
        });
        if dropped_any {
            self.rebase_shared_nav_members();
        }
    }

    /// The two shared Nav members the panel holds beside the walk grants (the
    /// special-areas overlay and the manual-walk pause) take the saved value,
    /// except where the operator's own change is still pending.
    fn rebase_shared_nav_members(&mut self) {
        let Ok(document) = self.preferences.document() else {
            return;
        };
        let saved = crate::ui_state::load_snapshot(&document).nav;
        if !self.pending_nav_members.contains_key("show_special_areas") {
            self.ui.nav.show_special_areas = saved.show_special_areas;
        }
        if !self
            .pending_nav_members
            .contains_key("pause_script_on_manual_walk_abort")
        {
            self.ui.nav.pause_script_on_manual_walk_abort = saved.pause_script_on_manual_walk_abort;
        }
    }

    /// Whether a pending special-areas preview was dropped since the last
    /// call (its write ended): the map's own toggle then follows the saved
    /// value instead of submitting its stale choice again.
    pub(crate) fn take_special_areas_dropped(&mut self) -> bool {
        std::mem::take(&mut self.special_areas_dropped)
    }

    fn adopt_panel_preferences_if_ready(&mut self) {
        if self.preferences_ready {
            return;
        }
        // The core adopts the durable snapshot into the draw policy, the
        // bake gate and the Quester source before the panel's own view does.
        self.follow_core_preferences();
        let Ok(document) = self.preferences.document() else {
            return;
        };
        self.adopt_panel_preferences(&document);
    }

    fn adopt_panel_preferences(&mut self, document: &serde_json::Map<String, serde_json::Value>) {
        if self.preferences_ready {
            return;
        }
        self.ui = crate::ui_state::apply_pending_members(
            crate::ui_state::load_snapshot(document),
            self.pending_panel_members
                .iter()
                .filter(|member| !crate::ui_state::is_launch_choice(&member.path))
                .map(|member| (member.path.as_slice(), &member.value)),
        );
        apply_pending_nav_members(&mut self.ui.nav, &self.pending_nav_members);
        self.preferences_ready = true;

        let preserve_folder_draft = self.quester_paths_folder_dirty;
        if !preserve_folder_draft {
            self.quester_paths_folder_edit = self
                .quester_paths
                .settings()
                .folder()
                .to_string_lossy()
                .into_owned();
        }
        self.ui.capture = self.draw.capture();
        self.ui.sidecar_50 = self.draw.sidecar_50();
        self.ui.focused_50 = self.draw.focused_50();
        self.ui.map_bake = self.map_desk.bake_gate().choice();
        #[cfg(feature = "js")]
        {
            self.script_load_dir = crate::script_picker::default_load_browse_dir(
                self.ui.script_load_last_dir.as_deref(),
            );
            self.rs2b0t_catalog_dir = crate::script_picker::default_load_browse_dir(
                self.ui.script_catalog_last_dir.as_deref(),
            );
        }

        self.clear_cached_nav_members(document);
    }

    /// A panel-owned write became durable: its request ends (the document
    /// holds its value, and a newer request of the same member keeps
    /// previewing its own) and an older request's failure shown at the
    /// setting is resolved; a newer request's failure stays. Requests are
    /// matched by identity, never by the value they carry.
    fn clear_succeeded_panel_members(&mut self, members: &[PanelPreferenceMember]) {
        for member in members {
            self.launch_previews.settle(member.request);
            self.pending_panel_members
                .retain(|pending| pending.request != member.request);
            #[cfg(test)]
            crate::ui_state::commit_test_member(&member.path, &member.value);
            let mut cleared = false;
            self.preference_write_errors.retain(|error| {
                let keep = error.path != member.path
                    || error.owner != member.owner
                    || error.request > member.request;
                cleared |= !keep;
                keep
            });
            if cleared {
                self.note_panel_setting(member, frontend_core::SettingEventState::Cleared, None);
            }
        }
        self.rebase_panel_members();
    }

    /// A panel-owned write was refused or failed: its request ends, the
    /// control shows the latest durable value (and the preview of any other
    /// request still pending) again, and the failure stays at the setting
    /// until a later write of it lands or the error timeout passes. Every
    /// request has its own outcome, whatever the others carried.
    fn fail_panel_members(&mut self, members: &[PanelPreferenceMember], error: &str) {
        let now = Instant::now();
        for member in members {
            self.launch_previews.settle(member.request);
            self.pending_panel_members
                .retain(|pending| pending.request != member.request);
            self.set_preference_write_error(member, error, now);
            self.note_panel_setting(
                member,
                frontend_core::SettingEventState::Failed,
                Some(error),
            );
        }
        self.rebase_panel_members();
    }

    /// The panel view after a request settled: the latest durable document
    /// with the previews of the requests still pending laid over it
    /// ([`crate::ui_state::rebased`]). A run that saves nothing, and a
    /// panel whose first snapshot has not arrived, have nothing to rebase.
    fn rebase_panel_members(&mut self) {
        if !self.persist_ui || !self.preferences_ready {
            return;
        }
        let Ok(document) = self.preferences.document() else {
            return;
        };
        self.ui = crate::ui_state::rebased(
            &self.ui,
            &document,
            self.pending_panel_members
                .iter()
                .map(|member| (member.path.as_slice(), &member.value)),
            self.core.selected().is_some(),
        );
        // The server revision the next session binds with follows a durable
        // value another process saved (while this panel has no choice of its
        // own pending and no profile is bound).
        let durable = document
            .get("server_revision")
            .and_then(serde_json::Value::as_u64)
            .and_then(|revision| u16::try_from(revision).ok())
            .filter(|revision| matches!(revision, 274 | 289));
        if let Some(durable) = durable {
            if self.launch_previews.server_revision().is_none()
                && self.launcher.bound().is_none()
                && durable != self.ui.server_revision
            {
                self.ui.server_revision = durable;
                self.launcher.bump_generation();
            }
        }
    }

    fn set_preference_write_error(
        &mut self,
        member: &PanelPreferenceMember,
        error: &str,
        now: Instant,
    ) {
        let request = member.request;
        let text = format!("Preferences not saved: {error}");
        match self
            .preference_write_errors
            .iter_mut()
            .find(|shown| shown.path == member.path && shown.owner == member.owner)
        {
            // A newer request's failure already shows: it stays.
            Some(shown) if shown.request > request => {}
            Some(shown) => {
                shown.request = request;
                shown.text = text;
                shown.since = now;
            }
            None => self.preference_write_errors.push(PanelWriteError {
                path: member.path.clone(),
                owner: member.owner.clone(),
                request,
                text,
                since: now,
            }),
        }
    }

    /// Drop every panel-write failure that has stood for the error timeout.
    pub(crate) fn expire_panel_write_errors(&mut self, now: Instant) {
        let mut index = 0;
        while index < self.preference_write_errors.len() {
            if now.saturating_duration_since(self.preference_write_errors[index].since)
                >= frontend_core::ERROR_MESSAGE_TIMEOUT
            {
                let error = self.preference_write_errors.remove(index);
                self.note_panel_setting_for(
                    &error.path,
                    &error.owner,
                    frontend_core::SettingEventState::Cleared,
                    None,
                );
            } else {
                index += 1;
            }
        }
    }

    fn clear_cached_nav_members(&mut self, document: &serde_json::Map<String, serde_json::Value>) {
        for key in ["show_special_areas", "pause_script_on_manual_walk_abort"] {
            let stored = document
                .get("nav")
                .and_then(serde_json::Value::as_object)
                .and_then(|nav| nav.get(key))
                .and_then(serde_json::Value::as_bool);
            if self
                .pending_nav_members
                .get(key)
                .is_some_and(|pending| Some(*pending) == stored)
            {
                self.pending_nav_members.remove(key);
            }
        }
    }

    /// Enable full-core proof only for the dedicated catalog_watch entry.
    pub fn set_catalog_core_enabled(&mut self, enabled: bool) {
        self.catalog_core_enabled = enabled;
    }

    #[cfg(test)]
    pub fn install_catalog_core_watch(&self, watch: Option<host_play::catalog_core::CoreWatch>) {
        *self.catalog_core_watch.lock().unwrap() = watch.clone();
        self.core
            .live_runner()
            .install_catalog_watch_for_test(watch);
    }

    #[cfg(test)]
    pub fn install_paired_core_watch(&self, watch: Option<host_play::paired_core::PairWatch>) {
        self.core.live_runner().install_pair_watch_for_test(watch);
    }

    /// Apply a session-only memory choice to every slot, including companions.
    pub fn set_memory_override(&mut self, memory_override: Option<bool>) {
        self.memory_override = memory_override;
    }

    /// Enable pair proof only for the dedicated pair_watch entry.
    pub fn set_pair_core_enabled(&mut self, enabled: bool) {
        self.pair_core_enabled = enabled;
    }

    #[cfg(feature = "js")]
    pub fn set_external_core_enabled(&mut self, enabled: bool) {
        self.external_core_enabled = enabled;
    }

    #[cfg(feature = "js")]
    pub fn external_core_enabled(&self) -> bool {
        self.external_core_enabled
    }

    #[cfg(feature = "js")]
    pub fn set_external_ts(&mut self, path: Option<PathBuf>) {
        self.external_ts = path;
    }

    /// Configure the optional prepare / run-prepared fixture path for the next live boot.
    pub fn set_fixture_boot(
        &mut self,
        mode: frontend_core::live::FixtureMode,
        path: Option<PathBuf>,
    ) {
        self.fixture_mode = mode;
        self.fixture_path = path;
    }

    /// Resolve the identity receipt path for `scenario`.
    pub fn fixture_identity_path(&self, scenario: &str) -> PathBuf {
        self.fixture_path
            .clone()
            .unwrap_or_else(|| frontend_core::live::default_fixture_path(scenario))
    }

    #[cfg(all(test, feature = "js"))]
    pub fn external_core_watch(&self) -> Option<host_play::external_loader::ExternalWatch> {
        self.external_core_watch.lock().unwrap().clone()
    }

    #[cfg(all(test, feature = "js"))]
    pub(crate) fn install_external_core_watch(
        &self,
        watch: Option<host_play::external_loader::ExternalWatch>,
    ) {
        *self.external_core_watch.lock().unwrap() = watch.clone();
        self.core
            .live_runner()
            .install_external_watch_for_test(watch);
    }

    #[cfg(all(test, feature = "js"))]
    pub fn pump_external_loader(&mut self) {
        self.core.pump_live_external_loader(
            &mut self.scripts,
            &mut self.script_sel,
            &self.statuses,
            Instant::now(),
        );
    }

    /// Install production launch inputs while the locked panel still exposes
    /// its saved revision selector. Resolution is deferred until unlock/live boot.
    pub fn configure_profile(&mut self, args: ProfileArgs) -> Result<(), ProfileError> {
        self.configure_profile_with_env(args, ProfileEnv::capture())
    }

    /// Install launch inputs with an explicit environment. Tests and embedders
    /// use this path so profile resolution never consults an ambient home.
    pub fn configure_profile_with_env(
        &mut self,
        args: ProfileArgs,
        environment: ProfileEnv,
    ) -> Result<(), ProfileError> {
        self.launcher.configure(args, environment)
    }

    pub(crate) fn profile_generation(&self) -> u64 {
        self.launcher.generation()
    }

    pub(crate) fn profile_preparation(&self) -> Result<ProfilePreparation, ProfileError> {
        self.launcher.preparation(self.ui.server_revision)
    }

    pub(crate) fn finish_profile_preparation(
        &mut self,
        generation: u64,
        result: Result<ProfileTemplate, ProfileError>,
    ) -> PreparationCompletion {
        let (completion, error) = self.launcher.finish_preparation(generation, result);
        match completion {
            PreparationCompletion::Installed => self.on_profile_installed(),
            PreparationCompletion::Failed => self.error = error.map(|error| error.to_string()),
            PreparationCompletion::Stale => {}
        }
        completion
    }

    /// The panel's side of a newly bound profile: walk-map and Debug state
    /// from the previous binding are released and the nav bindings follow
    /// the new profile.
    fn on_profile_installed(&mut self) {
        self.release_walk_map();
        crate::picker::bind_profile_nav(self.launcher.bound());
        self.release_debug_catalog();
        self.error = None;
    }

    /// Bind `template` as a finished preparation would (tests).
    #[cfg(test)]
    pub(crate) fn install_prepared_template(&mut self, template: ProfileTemplate) {
        self.launcher.install_template(template);
        self.on_profile_installed();
    }

    pub(crate) fn template_for_validation(&self) -> Result<ProfileTemplate, ProfileError> {
        self.launcher.template_for_validation()
    }

    pub(crate) fn install_validated_template(
        &mut self,
        validated: ValidatedProfile,
    ) -> Result<(), ProfileError> {
        self.launcher.install_validated(validated)
    }

    pub(crate) fn set_profile_preparing(&mut self, preparing: bool) {
        self.launcher.set_preparing(preparing);
    }

    pub(crate) fn profile_preparing(&self) -> bool {
        self.launcher.preparing()
    }

    pub(crate) fn request_unlock(&mut self, pass: Passphrase) {
        self.requested_unlock = Some(pass);
    }

    pub(crate) fn take_requested_unlock(&mut self) -> Option<Passphrase> {
        self.requested_unlock.take()
    }

    /// The bound server profile.
    pub(crate) fn server_profile(&self) -> Option<&BoundProfile> {
        self.launcher.bound()
    }

    /// Resolve and freeze the panel's process profile before vault mutation.
    pub fn bind_profile(&mut self) -> Result<(), ProfileError> {
        self.launcher.bind(self.ui.server_revision)?;
        self.on_profile_installed();
        Ok(())
    }

    pub fn ensure_profile_bound(&mut self) -> Result<(), ProfileError> {
        if self.launcher.bound().is_none() && self.launcher.configured() {
            self.bind_profile()?;
        }
        Ok(())
    }

    pub fn require_bot_operation(&self) -> Result<(), ProfileError> {
        self.launcher
            .bound()
            .map_or(Ok(()), BoundProfile::require_bot_operation)
    }

    /// Choose the server revision for the next session. It is a remembered
    /// launch choice, so it takes effect only when its write is durable: the
    /// preparation and the binding read the durable value
    /// ([`Self::apply_launch_effect`]), and the revision control shows the
    /// choice meanwhile ([`Self::revision_choice_label`]).
    pub fn set_server_revision(&mut self, revision: u16) -> Result<(), String> {
        if self.launcher.bound().is_some() {
            return Err("server profile is already bound; restart to change revision".into());
        }
        if !matches!(revision, 274 | 289) {
            return Err(format!("unsupported revision {revision}; use 274 or 289"));
        }
        self.choose_launch(
            &["server_revision"],
            Some(serde_json::Value::from(revision)),
            LaunchEffect::ServerRevision(revision),
        );
        Ok(())
    }

    pub fn profile_bound(&self) -> bool {
        self.launcher.bound().is_some()
    }

    pub fn server_label(&self) -> String {
        self.launcher.server_label(self.ui.server_revision)
    }

    pub fn app_title(&self) -> String {
        self.effective_revision_label()
            .map_or_else(|_| "bot".into(), |revision| format!("{revision}bot"))
    }

    pub fn effective_revision_label(&self) -> Result<String, ProfileError> {
        self.launcher.revision_label(self.ui.server_revision)
    }

    /// The revision the revision control shows: the operator's choice while
    /// its save is pending, else the effective one.
    pub fn revision_choice_label(&self) -> Result<String, ProfileError> {
        let shown = self
            .launch_previews
            .server_revision()
            .unwrap_or(self.ui.server_revision);
        self.launcher.revision_label(shown)
    }

    pub(crate) fn profile_class(&self) -> ProfileClass {
        self.launcher.class(self.core.has_play())
    }

    fn vault_path(&self) -> Result<PathBuf, ProfileError> {
        self.launcher
            .vault_path(self.ui.server_revision, default_vault_path())
    }

    pub fn play_options(&self) -> &PlayConfig {
        self.launcher.play_config()
    }

    /// Unlock (or first-run create) the default vault and start the play.
    /// A vault path that cannot be resolved refuses the Create the prompt
    /// then offers (`vault_unlock_prompt` treats it as no vault file).
    pub fn unlock(&mut self, pass: &str) -> bool {
        let path = match self.vault_path() {
            Ok(path) => path,
            Err(error) => {
                self.refuse_command(
                    OperatorCommand::CreateVault,
                    &[],
                    None,
                    &format!("server profile: {error}"),
                );
                return false;
            }
        };
        self.unlock_at(&path, pass)
    }

    /// Unlock (or first-run create) the vault at `path` and start the play.
    /// Only the focused profile is spawned as a slot; other vault rows stay
    /// parked until selected (select keeps a slot once it has run).
    pub fn unlock_at(&mut self, path: &Path, pass: &str) -> bool {
        if self.start_vault(path, pass) {
            self.startup_focus_pending = Some(StartupFocus::Restore);
            true
        } else {
            false
        }
    }

    /// Whether the default vault file exists (locked prompt: Unlock vs Create).
    pub fn default_vault_exists(&self) -> Result<bool, ProfileError> {
        self.vault_path().map(|path| path.is_file())
    }

    /// Delete `path` while locked. Refuses if a vault is open so a running
    /// session cannot clobber the file. Forgotten-password recovery.
    pub fn reset_vault_at(&mut self, path: &Path) -> bool {
        if self.core.vault_open() {
            self.refuse_command(
                OperatorCommand::ResetVault,
                &[],
                None,
                "reset vault: unlock / close the session first",
            );
            return false;
        }
        self.core.stage_vault_reset(path.to_path_buf());
        let Some(submitted) = self.submit(OperatorCommand::ResetVault, Targets::Application) else {
            return false;
        };
        match self.core.command_failure(submitted.request()) {
            None => {
                self.error = None;
                self.vault_reset_open = false;
                self.vault_reset_understood = false;
                true
            }
            Some(error) => {
                self.error = Some(error.to_string());
                false
            }
        }
    }

    pub fn reset_vault(&mut self) -> bool {
        let path = match self.vault_path() {
            Ok(path) => path,
            Err(error) => {
                self.refuse_command(
                    OperatorCommand::ResetVault,
                    &[],
                    None,
                    &format!("server profile: {error}"),
                );
                return false;
            }
        };
        self.reset_vault_at(&path)
    }

    /// Open the vault and attach an empty [`Play`]. Does **not** spawn a
    /// slot — the boot spawns the focused profile after this; MultiBox
    /// spawns the wall members.
    fn start_vault(&mut self, path: &Path, pass: &str) -> bool {
        if let Err(msg) = self
            .ensure_profile_bound()
            .and_then(|()| self.require_bot_operation())
        {
            let command = if path.is_file() {
                OperatorCommand::UnlockVault
            } else {
                OperatorCommand::CreateVault
            };
            self.refuse_command(command, &[], None, &msg.to_string());
            return false;
        }
        match self.start_play(path, pass) {
            Ok(()) => {
                self.error = None;
                true
            }
            Err((stage, msg)) => {
                let prefix = match stage {
                    UnlockStage::Vault => "vault",
                    UnlockStage::Launch => "profile",
                };
                process_log(Level::Error, &format!("{prefix}: {msg}"));
                self.error = Some(msg);
                false
            }
        }
    }

    /// Live `null_raster` setup: temp vault with `test`/`test2`, multibox
    /// wall of both, only-render-selected + focus `test`, renderer on,
    /// then `login_all`. Slot threads keep using real `Focus` → `set_draw`.
    pub fn live_prepare_null_raster(&mut self) -> Result<(), String> {
        self.persist_ui = false;
        self.draw.set_persist(false);
        let profiles = frontend_core::live::LiveProfiles::null_raster();
        let path = profiles.panel_vault(274_000_001, true);
        if !self.unlock_at(&path, profiles.passphrase()) {
            return Err(self
                .error
                .clone()
                .unwrap_or_else(|| "unlock_at failed".into()));
        }
        self.set_multibox(true);
        self.scatter.store(false, Ordering::Relaxed);
        // First MultiBox-on opens the chooser; live already loaded both
        // names. Leave the window usable (operator may click the rail).
        self.wall.chooser_open = false;
        self.load("test");
        self.load("test2");
        self.set_render_all(false);
        self.select("test");
        self.set_renderer(true);
        self.login_all();
        Ok(())
    }

    #[cfg(feature = "memory-profile")]
    pub fn prepare_memory(&mut self, run: &frontend_core::bench::BenchRun) -> Result<(), String> {
        use frontend_core::bench::RenderPolicy;
        self.persist_ui = false;
        self.draw.set_persist(false);
        self.mainland.store(true, Ordering::Relaxed);
        let policy = run.render_policy();
        let stress_wall = matches!(policy, RenderPolicy::Stress50 | RenderPolicy::Stress50Full);
        self.scatter.store(stress_wall, Ordering::Relaxed);
        if !self.start_vault(run.vault(), run.passphrase()) {
            return Err(self
                .error
                .clone()
                .unwrap_or_else(|| "benchmark vault failed".into()));
        }
        run.bind_seed_nav(&self.core)
            .map_err(|error| error.to_string())?;
        self.set_multibox(true);
        for name in run.names() {
            self.submit_reporting(OperatorCommand::Load, Targets::Profile(name));
        }
        self.sync_wall_focus();
        self.wall.chooser_open = false;
        self.memory_focus(run);
        if stress_wall {
            self.set_live_full_rate(matches!(policy, RenderPolicy::Stress50Full));
            self.sync_sidecar_cadence();
        }
        self.login_all();
        Ok(())
    }

    #[cfg(feature = "memory-profile")]
    pub fn memory_focus(&mut self, run: &frontend_core::bench::BenchRun) {
        let names = run.names();
        let name = &names[run.focus_index()];
        if self.focus.lock().unwrap().focused.as_ref() != Some(name) {
            self.select(name);
        }
        let mut focus = self.focus.lock().unwrap();
        crate::focus::memory_draw_policy(&mut focus, names, run.render_policy());
    }

    /// Live `stress50` RAM watch: temp vault `s00`…`s49` (password =
    /// username, uids `274_000_100 + i`). Every member is a full `Client`
    /// (flat model — no lean extras / channel-head). `s00` is FIFO head +
    /// focus, MultiBox rail with only-render-selected (cap-only: Game
    /// paints at focused 50 fps, rail skip-paint so 50 blits do not melt
    /// RAM/GPU), scatter-seed after scene 2, chooser closed, `login_all`.
    /// Run headed with `cargo run --release` — debug 50-heads spike RAM.
    pub fn live_prepare_stress50(&mut self) -> Result<(), String> {
        warn_stress50_debug();
        self.live_prepare_stress(50, false)
    }

    /// Live `stress50_full`: same 50-head wall as [`live_prepare_stress50`],
    /// but every wall member draws (only-render-selected off) at the live
    /// full-rate overlay (Game + sidecar 50 fps). RAM/GPU explosion check
    /// after `stress50` holds. Still `--release`.
    pub fn live_prepare_stress50_full(&mut self) -> Result<(), String> {
        warn_stress50_debug();
        self.live_prepare_stress(50, true)
    }

    /// Headed flat wall of `n` profiles (`s00`…`s{n-1}`). `full_rate`
    /// paints every member at 50 fps; otherwise cap-only RAM watch.
    fn live_prepare_stress(&mut self, n: usize, full_rate: bool) -> Result<(), String> {
        self.persist_ui = false;
        self.draw.set_persist(false);
        let profiles = frontend_core::live::LiveProfiles::stress(n);
        let path = profiles.panel_vault(274_000_100, true);
        // Empty Play first: do not spawn last_focus before s00 focuses.
        if !self.start_vault(&path, profiles.passphrase()) {
            return Err(self
                .error
                .clone()
                .unwrap_or_else(|| "start_vault failed".into()));
        }
        // RAM watch: only s00 (Game pane) may grow a GPU head. Rail members
        // stay raster Off so a flipped only-render-selected cannot attach
        // 49 extra RenderWorlds (~1 GB of loc Model clones each). Full-rate
        // keeps Gpu on every member on purpose.
        self.core
            .prepare_live_stress_profiles(profiles.entries(), full_rate);
        // Harness setup, not a frame path: the spawns below read the durable
        // rows, so the raster prefs must be on disk first.
        self.core.flush_writes();
        self.set_multibox(true);
        self.scatter.store(true, Ordering::Relaxed);
        self.wall.chooser_open = false;
        self.set_render_all(full_rate);
        // Spawn every member onto the wall without applying focus on each
        // load (that would demote/promote raster/mem and join `maininit`
        // on the UI thread — the window never presents). Raster switches
        // reattach the renderer; memory stays queued until the next login.
        // s00 is focused last so it is FIFO head.
        for name in profiles.names() {
            self.submit_reporting(OperatorCommand::Load, Targets::Profile(name));
        }
        self.sync_wall_focus();
        self.wall.chooser_open = false;
        self.select(&profiles.names()[0]);
        // NEVER assign sidecar_50 — it stays the operator knob. The live
        // overlay raises every *drawing* slot, Game included.
        self.set_live_full_rate(full_rate);
        self.sync_sidecar_cadence();
        self.login_all();
        Ok(())
    }

    #[cfg(feature = "js")]
    /// Live setup runs before the interactive event loop. Keep its catalogue
    /// receipt off the UI path's synchronous scan, then consume it before the
    /// scenario selects a card from that catalogue.
    fn fill_live_catalog(&mut self) -> Result<(), String> {
        let root = self
            .catalog_root()
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "$RS2B0T catalog root unavailable".to_string())?;
        let path_file = self.rs2b0t_path_file();
        self.scripts
            .fill_catalog_blocking(&root, &path_file)
            .map_err(|error| error.to_string())
    }

    fn stage_live_starts(&mut self, plan: frontend_core::live::StartPlan) {
        self.core.live_runner().stage(plan);
    }

    /// Live `script_<name>` setup with a shared frontend-core scenario runner.
    pub fn live_prepare_script(
        &mut self,
        scenario: impl Into<frontend_core::live::Scenario>,
    ) -> Result<(), String> {
        let scenario = scenario.into();
        self.persist_ui = false;
        self.draw.set_persist(false);
        self.core.live_runner().clear();
        #[cfg(test)]
        {
            self.catalog_core_watch.lock().unwrap().take();
            self.paired_core_watch.lock().unwrap().take();
            #[cfg(feature = "js")]
            self.external_core_watch.lock().unwrap().take();
        }
        if self.fixture_mode == frontend_core::live::FixtureMode::Prepare {
            return Err(
                "FixtureMode::Prepare is offline-only; use panel --prepare-fixture (no live boot)"
                    .into(),
            );
        }
        let mut scenario = scenario.fixture_mode(self.fixture_mode)?;
        if self.fixture_mode == frontend_core::live::FixtureMode::RunPrepared
            && scenario.run_prepared_has_setup_cheats()
        {
            return Err(format!(
                "run-prepared scenario {:?} still has setup cheats after rewrite",
                scenario.view().name()
            ));
        }

        let mut inject = scenario.settings_inject();
        if let Some(overrides) = scenario.take_settings_overrides() {
            inject
                .get_or_insert_with(serde_json::Map::new)
                .extend(overrides);
        }
        let view = scenario.view();
        let scenario_name = view.name().to_owned();
        let profile_count = view.profile_count();
        let mainland = view.mainland();
        let renderer = view.renderer();
        let only_render_selected = view.only_render_selected();
        let capture = view.capture();
        let full_rate = view.full_rate();
        let nav = view.nav();
        #[cfg(feature = "js")]
        let inject_companion_as = view.inject_companion_as().map(str::to_owned);
        let start_script = view.start_script().map(str::to_owned);
        let start_file = view.start_file().map(str::to_owned);
        let fleet_start = view.fleet_start();

        #[cfg(not(feature = "js"))]
        if fleet_start.is_some()
            || start_file.is_some()
            || start_script
                .as_deref()
                .is_some_and(|name| frontend_core::scripts::CompiledCardId::lookup(name).is_none())
        {
            return Err(JS_SURFACE_REFUSAL.into());
        }

        let profiles = match self.fixture_mode {
            frontend_core::live::FixtureMode::RunPrepared => {
                let path = self.fixture_identity_path(&scenario_name);
                let identity = frontend_core::live::FixtureIdentity::read_from(&path)?;
                identity.validate_for(&scenario_name, profile_count)?;
                if let Ok(players) = std::env::var("BOT_PLAYERS_DIR") {
                    let installed =
                        identity.install_into_players_dir(&PathBuf::from(players), true)?;
                    for path in &installed {
                        println!("[panel] installed fixture save -> {}", path.display());
                    }
                } else {
                    let save_paths = identity
                        .accounts()
                        .into_iter()
                        .map(|(_, path, _, _)| path)
                        .collect::<Vec<_>>();
                    println!(
                        "[panel] run-prepared: BOT_PLAYERS_DIR unset; ensure engine already has {}",
                        save_paths.join(", ")
                    );
                }
                frontend_core::live::LiveProfiles::from_fixture_identity(identity)
            }
            frontend_core::live::FixtureMode::Default => {
                frontend_core::live::LiveProfiles::mint(profile_count)
            }
            frontend_core::live::FixtureMode::Prepare => unreachable!("prepare rejected above"),
        };
        let names = profiles.names();
        #[cfg(feature = "js")]
        if self.external_core_enabled && (self.catalog_core_enabled || self.pair_core_enabled) {
            return Err(
                "external loader watch is mutually exclusive with catalog/pair core".into(),
            );
        }
        let live_core = self.core.resolve_live_core(
            &scenario_name,
            names.len(),
            self.catalog_core_enabled,
            self.pair_core_enabled,
        )?;
        #[cfg(not(feature = "js"))]
        let _ = live_core;
        #[cfg(feature = "js")]
        if live_core != frontend_core::live::LiveCoreMode::Pair {
            if let Some(key) = inject_companion_as.as_deref() {
                if names.len() > 1 {
                    inject.get_or_insert_with(serde_json::Map::new).insert(
                        key.to_string(),
                        serde_json::Value::String(frontend_core::live::screen_name(&names[1])),
                    );
                }
            }
        }
        self.set_script_settings_inject(inject);
        let path = profiles.panel_vault(274_000_001, true);
        if !self.unlock_at(&path, profiles.passphrase()) {
            return Err(self
                .error
                .clone()
                .unwrap_or_else(|| "unlock_at failed".into()));
        }
        self.core.configure_live_core(names)?;
        #[cfg(feature = "js")]
        if self.external_core_enabled {
            self.core
                .configure_live_external_loader(names[0].clone(), self.external_ts.as_deref())?;
        }

        self.mainland.store(mainland, Ordering::Relaxed);
        self.scatter.store(false, Ordering::Relaxed);
        if profile_count > 1 {
            self.set_multibox(true);
        }
        self.wall.chooser_open = false;
        for name in names {
            self.load(name);
        }
        self.select(&names[0]);
        self.set_renderer(renderer);
        self.set_render_all(!only_render_selected);
        self.set_capture(capture);
        self.set_live_full_rate(full_rate);
        self.nav_overlay = Some(from_scenario(&nav, &self.ui.nav));
        self.sync_sidecar_cadence();

        #[cfg(feature = "js")]
        let fixture_loadouts = scenario.fixture_loadouts();
        #[cfg(feature = "js")]
        let fleet_cards = scenario.fleet_cards(names)?;
        let play_config = self.launcher.play_config();
        let budget = frontend_core::live::budget_s_from_env();
        let runner = self.core.live_scenario_runner(
            scenario,
            names,
            play_config.host(),
            play_config.port(),
            budget,
        );

        self.core.live_runner().install(runner);
        self.live_script_stop_wait_started = None;

        if let Some(fleet) = fleet_start {
            #[cfg(feature = "js")]
            {
                self.fill_live_catalog()?;
                for card in &fleet_cards {
                    if !self.scripts.card_is_prepared(CardSource::Catalog, card) {
                        self.scripts
                            .transpile_blocking(CardSource::Catalog, card)
                            .map_err(|error| error.to_string())?;
                    }
                }
                let plan = frontend_core::live::StartPlan::fleet_catalog(
                    &mut self.scripts,
                    fleet,
                    names,
                    &fixture_loadouts,
                )?;
                self.stage_live_starts(plan);
            }
            #[cfg(not(feature = "js"))]
            {
                let _ = fleet;
                return Err(JS_SURFACE_REFUSAL.into());
            }
        } else if let Some(file_name) = start_file.as_deref() {
            #[cfg(feature = "js")]
            {
                let identity = self.scripts.load_live_example_identity(file_name)?;
                self.script_sel = Some(ScriptChoice::loaded(CardSource::File, &identity));
                let bag = self.pending_card_settings_bag(CardSource::File, &identity);
                let choice = self
                    .script_sel
                    .as_ref()
                    .expect("live loaded-card selection is set");
                let plan = self.core.live_runner().stage_loaded_card(
                    &mut self.scripts,
                    names,
                    choice,
                    bag,
                    inject_companion_as.as_deref(),
                    fixture_loadouts,
                );
                plan?;
            }
            #[cfg(not(feature = "js"))]
            {
                let _ = file_name;
                return Err(JS_SURFACE_REFUSAL.into());
            }
        } else if let Some(card_name) = start_script.as_deref() {
            if let Some(card) = frontend_core::scripts::CompiledCardId::lookup(card_name) {
                self.script_sel = Some(ScriptChoice::compiled(card));
                let bag = self.scripts.compiled_bag(&self.core, &names[0], card)?;
                let plan = frontend_core::live::StartPlan::compiled(names[0].clone(), card, bag);
                self.stage_live_starts(plan);
            } else {
                #[cfg(feature = "js")]
                {
                    if let Some((fixture, stem)) = frontend_core::live::live_file_fixture(card_name)
                    {
                        self.scripts.stage_load_file(fixture.clone());
                        let submitted = self
                            .submit_script_command(
                                OperatorCommand::LoadScriptFile,
                                Targets::Application,
                            )
                            .ok_or_else(|| {
                                format!("load {card_name} fixture: command was not submitted")
                            })?;
                        if let Some(error) = self.core.command_failure(submitted.request()) {
                            return Err(format!("load {card_name} fixture: {error}"));
                        }
                        if self.scripts.card(CardSource::File, &stem).is_none() {
                            return Err(format!("file fixture {card_name} missing after load"));
                        }
                        self.script_sel = Some(ScriptChoice::loaded(CardSource::File, &stem));
                        let bag = self.pending_card_settings_bag(CardSource::File, &stem);
                        let choice = self
                            .script_sel
                            .as_ref()
                            .expect("live loaded-card selection is set");
                        self.core.live_runner().stage_loaded_card(
                            &mut self.scripts,
                            names,
                            choice,
                            bag,
                            inject_companion_as.as_deref(),
                            fixture_loadouts,
                        )?;
                    } else {
                        self.fill_live_catalog()?;
                        self.scripts
                            .transpile_blocking(CardSource::Catalog, card_name)
                            .map_err(|error| format!("transpile {card_name}: {error}"))?;
                        if self.scripts.card(CardSource::Catalog, card_name).is_none() {
                            return Err(format!(
                                "$RS2B0T catalog has no {card_name} card (is $RS2B0T set?)"
                            ));
                        }
                        self.script_sel =
                            Some(ScriptChoice::loaded(CardSource::Catalog, card_name));
                        let bag = self.pending_card_settings_bag(CardSource::Catalog, card_name);
                        let choice = self
                            .script_sel
                            .as_ref()
                            .expect("live loaded-card selection is set");
                        self.core.live_runner().stage_loaded_card(
                            &mut self.scripts,
                            names,
                            choice,
                            bag,
                            inject_companion_as.as_deref(),
                            fixture_loadouts,
                        )?;
                    }
                }
                #[cfg(not(feature = "js"))]
                {
                    return Err(JS_SURFACE_REFUSAL.into());
                }
            }
        }
        self.login_all();
        Ok(())
    }

    /// Build the hook with the panel's per-session state and launch template.
    fn panel_frame_hook(&self) -> Arc<dyn frontend_core::frames::FrameHook> {
        Arc::new(PanelFrameHook {
            focus: Arc::clone(&self.focus),
            mainland: Arc::clone(&self.mainland),
            mainland_sent: Arc::clone(&self.mainland_sent),
            scatter: Arc::clone(&self.scatter),
            audio: Arc::clone(&self.audio),
            nav_publish: Arc::clone(&self.nav_publish),
            nav_paint_cache: Mutex::new(crate::nav_paint_cache::Cache::default()),
            // Last failed device-open `(slot, when)`; a machine without an
            // audio device must not re-open cpal (or re-log) every frame.
            audio_fail: Mutex::new(None),
            live_runner: self.core.live_runner().share(),
            scatter_template: self.launcher.template().map(ProfileTemplate::share),
        })
    }

    /// Empty `Play` (shared cache + FIFO + per-frame hook) then spawn the
    /// first focused profile only. Parked names are started from [`select`].
    ///
    /// The vault opens through `UnlockVault` (or `CreateVault` on first
    /// run, when no vault file exists yet).
    fn start_play(&mut self, path: &Path, pass: &str) -> Result<(), (UnlockStage, String)> {
        let hook = self.panel_frame_hook();
        let launch = self.launcher.take_launch().keeping_world_state();
        let row = if path.is_file() {
            self.core
                .stage_vault_unlock(path.to_path_buf(), pass, launch);
            OperatorCommand::UnlockVault
        } else {
            self.core
                .stage_vault_create(path.to_path_buf(), pass, launch);
            OperatorCommand::CreateVault
        };
        let (core, mut surface) = self.core_and_surface();
        surface.frame_hook = Some(hook);
        let submitted = core
            .submit(row, Targets::Application, &mut surface)
            .map_err(|error| (UnlockStage::Vault, error.to_string()))?;
        let failed = self
            .core
            .commands()
            .report(submitted.request())
            .and_then(|report| report.members().first())
            .and_then(|member| match member.terminal() {
                Some(api::identity::Settlement::Failed(failure)) => Some((
                    match failure {
                        frontend_core::command::CommandFailure::Failed(_) => UnlockStage::Launch,
                        _ => UnlockStage::Vault,
                    },
                    member
                        .detail()
                        .unwrap_or("vault: unlock failed")
                        .to_string(),
                )),
                _ => None,
            });
        if let Some(failed) = failed {
            return Err(failed);
        }
        crate::picker::set_pack(self.core.nav_world());
        self.statuses.refresh(&self.core);
        Ok(())
    }

    /// A durable profile save can select immediately once shared startup is ready.
    fn focus_when_ready(&mut self, focus: StartupFocus) {
        if self.preferences_ready && self.core.prepare_startup() {
            self.apply_startup_focus(focus);
        } else {
            self.startup_focus_pending = Some(focus);
        }
    }

    fn apply_startup_focus(&mut self, focus: StartupFocus) {
        match focus {
            StartupFocus::Restore => self.focus_first_profile(),
            StartupFocus::SavedProfile(name) => self.select(&name),
        }
    }

    /// Restore the saved focus when it still exists, or use the first profile.
    /// MultiBox restores it onto the rail through the usual chooser path.
    fn focus_first_profile(&mut self) {
        let names = self.profile_names();
        if let Some(name) = frontend_core::pick_focus(&names, self.ui.last_focus.as_deref()) {
            self.pick_profile(&name);
        }
    }

    /// Clear the banner message once it has stood for
    /// [`frontend_core::ERROR_MESSAGE_TIMEOUT`] with nothing resolving or
    /// replacing it (the same rule as the terminal app's message line).
    pub(crate) fn expire_error(&mut self, now: Instant) {
        self.error_expiry
            .observe(&mut self.error, self.core.refusals(), now);
    }

    /// Refuse `command` before the panel can submit it (no focused bot, a bot
    /// with no card). The refusal logs at the shared command boundary like
    /// every other refused command, shows on the banner, and the banner clears
    /// when the action that supplies `need` happens
    /// ([`Self::clear_resolved_error`]).
    pub(crate) fn refuse_command(
        &mut self,
        command: OperatorCommand,
        bots: &[&str],
        need: Option<ErrorNeed>,
        reason: &str,
    ) {
        self.core.refuse_unsubmitted(command, bots, reason);
        self.error = Some(reason.to_owned());
        if let Some(need) = need {
            self.error_expiry.waits_for(need, reason);
        }
    }

    /// Clear the banner message when the need it waits for now holds: the
    /// action that resolves a message clears it at once, whichever action it
    /// was (a focus change, a focused bot's card, a finished save).
    /// Called after a focus change and once per frame.
    pub(crate) fn clear_resolved_error(&mut self) {
        let Some(need) = self.error_expiry.waiting_need() else {
            return;
        };
        let met = match need {
            ErrorNeed::FocusedBot => self.focused_name().is_some(),
            ErrorNeed::FocusedBotCard => self
                .focused_name()
                .is_some_and(|name| self.scripts.heading(&self.core, &name).is_some()),
            ErrorNeed::ProfileSaved(name) => !self.core.profile_saving(name),
        };
        self.error_expiry.resolve(&mut self.error, |_| met);
    }

    /// Poll slot statuses and append log lines for transitions (slot up,
    /// login errors, ingame, scene changes). Call once per UI frame.
    pub fn pump_status(&mut self) {
        self.expire_error(Instant::now());
        self.clear_resolved_error();
        self.core.advance_removals(Instant::now());
        // The fleet model follows the core's rows and the vault at this
        // sync, whether or not a fleet surface is drawn (two integer
        // comparisons when nothing moved).
        self.fleet_model.refresh(&self.core);
        if self.background_ack_open && self.background_bot_count() == 0 {
            self.background_ack_open = false;
        }
        // Per-frame mirrors that must not lag a focus/renderer/wall change:
        // the sidecar-50 cadence latch, and the speaker teardown when the
        // owning slot is no longer running.
        self.sync_sidecar_cadence();
        self.sync_nav_publish();
        if let Some(owner) = self.audio.owner() {
            if !self.core.slots().contains_key(&owner) {
                self.audio.release(&owner);
            }
        }
        if !self.core.has_play() {
            // A locked-down session still settles its profile writes.
            self.core.poll_host();
            self.surface_write_failures();
            self.settle_profile_save();
            return;
        }
        // Start/Stop return before the isolate is up or reaped: the core
        // resolves them for every slot (an offline or queued slot has no
        // observe of its own), refreshes rows and records transitions; the
        // panel then commits the Starts that settled.
        self.core.poll_host();
        self.core.poll_map_demand(&mut self.map_desk);
        self.surface_write_failures();
        self.settle_profile_save();
        self.poll_scripts();
        let mut current = std::mem::take(&mut self.statuses);
        current.refresh(&self.core);
        self.ingest_tutorial_chat(&current);
        self.maybe_getvar_tutorial(&current);
        self.maybe_drain_getvar_dialogue(&current);
        // Script lifecycle lines also feed the JS external proof.
        #[cfg(feature = "js")]
        self.core
            .live_runner()
            .note_external_loader_logs(self.core.script_lines());
        self.statuses = current;
        self.settle_walk_dest();
    }

    /// Clear [`Session::walk_dest`] once the focused walk ended (the core's
    /// slot frame flags it when a route arrives, stalls or is refused).
    fn settle_walk_dest(&mut self) {
        if !self.core.take_walk_clear() {
            return;
        }
        let still_walking = self
            .core
            .selected()
            .is_some_and(|name| self.core.walk_route_armed(name));
        if !still_walking {
            self.map_desk.clear_walk_dest();
        }
    }

    /// Surface failed durable profile writes without touching the client.
    fn surface_write_failures(&mut self) {
        let mut failed = false;
        for failure in self.core.take_write_failures() {
            failed = true;
            self.error = Some(failure.to_string());
        }
        if failed {
            if let Some(lowmem) = self
                .focused_name()
                .and_then(|name| self.core.profile(&name).map(|profile| profile.lowmem()))
            {
                self.ui.lowmem = lowmem;
            }
        }
    }

    /// The raw status rows for adapter-boundary tests.
    #[cfg(test)]
    pub fn statuses(&self) -> &[SlotStatus] {
        self.statuses.live_rows()
    }

    /// Vault usernames plus any running slot outside the vault.
    pub fn profile_names(&self) -> Vec<String> {
        self.core.profile_names()
    }

    pub fn focused_name(&self) -> Option<String> {
        self.core.selected().map(str::to_owned)
    }

    /// Focused bot and each member's [`draw_for_slot`]. Tests and headless
    /// Load-all checks; not a per-frame path.
    #[cfg(test)]
    pub(crate) fn focus_draw_state(&self) -> (Option<String>, Vec<(String, bool)>) {
        let members = self.core.members().to_vec();
        let focus = self.focus.lock().unwrap();
        let focused = focus.focused.clone();
        let draws = members
            .into_iter()
            .map(|name| {
                let draw = draw_for_slot(&focus, &name);
                (name, draw)
            })
            .collect();
        (focused, draws)
    }

    /// Queue a host-owned rail cheat on the focused slot through the
    /// `DebugCommand` row; these controls are not Debug-tab reply probes. The
    /// rail draws every cheat button disabled while
    /// [`Self::debug_send_target`] has no bot to give (no focus, no play
    /// session), so a click always gets one and the early return is not
    /// reachable by a click.
    pub fn cheat_focused(&mut self, cmd: &str) {
        let Ok(name) = self.debug_send_target().map(str::to_owned) else {
            return;
        };
        self.map_desk.stage_internal_debug_command(cmd);
        self.submit(OperatorCommand::DebugCommand, Targets::Profile(&name));
    }

    /// Attach the opt-in Debug catalog from the immutable server profile.
    /// Normal selected game data is deliberately not used as a fallback.
    pub fn debug_catalog(&self) -> Result<Arc<api::debug_commands::DebugCatalog>, ProfileError> {
        let profile = self.server_profile().ok_or_else(ProfileError::not_bound)?;
        profile.debug_catalog()
    }

    /// Decode and retain the catalog for an open Debug window. A failed
    /// attachment is retained as exact text until the app releases this
    /// session's Debug state, so a bad provenance check is not retried every
    /// frame.
    pub(crate) fn ensure_debug_catalog(&mut self) -> Result<(), String> {
        let profile_key = self.server_profile().map(BoundProfile::key);
        if self.debug_panel.catalog_profile() != profile_key {
            self.debug_panel.release_catalog();
            self.debug_panel.set_catalog_profile(profile_key);
        }
        if self.debug_panel.catalog().is_some() {
            return Ok(());
        }
        if let Some(error) = self.debug_panel.catalog_error() {
            return Err(error.to_string());
        }
        match self.debug_catalog() {
            Ok(catalog) => {
                self.debug_panel.attach_catalog(catalog);
                Ok(())
            }
            Err(error) => {
                let error = error.to_string();
                self.debug_panel.set_catalog_error(error.clone());
                Err(error)
            }
        }
    }

    /// Release the opt-in catalog when the Debug window closes or its bound
    /// server profile changes.
    pub(crate) fn release_debug_catalog(&mut self) {
        self.debug_panel.release_catalog();
    }

    /// Send one Debug command to the displayed focused bot. Marked sends use
    /// [`send_debug_command_marked_snapshot`] so every shared mark is counted.
    /// The host queue and client encoder retain final cheat admission. The
    /// Debug panel draws Send disabled with the reason
    /// [`Self::debug_send_target`] gives, so a click never gets an `Err` here.
    pub fn send_debug_command(&mut self, cmd: &str) -> Result<(), String> {
        let name = self
            .debug_send_target()
            .map(str::to_owned)
            .map_err(String::from)?;
        self.map_desk.stage_debug_command(cmd, None);
        let submitted = self
            .submit(OperatorCommand::DebugCommand, Targets::Profile(&name))
            .ok_or_else(|| {
                self.error
                    .clone()
                    .unwrap_or_else(|| "Debug command submission was refused".into())
            })?;
        if let Some(error) = self.core.command_failure(submitted.request()) {
            return Err(error.to_string());
        }
        Ok(())
    }

    /// Snapshot every visible profile row with its stable marked-selection
    /// identity. The owned snapshot keeps a bulk send from retargeting after
    /// a profile rename/removal or a status poll.
    pub(crate) fn debug_target_snapshot(&self) -> Vec<(frontend_core::ProfileIdentity, String)> {
        self.core
            .profile_names()
            .into_iter()
            .map(|name| {
                let identity = self.core.profile_identity_or_synthetic(&name);
                (identity, name)
            })
            .collect()
    }

    /// Admit one formatted command against the supplied marked snapshot
    /// through the same host admission path used by focused sends. The shared
    /// selection stays identity-keyed; the adapter reports marks whose row
    /// disappeared instead of silently dropping them.
    pub(crate) fn send_debug_command_marked_snapshot(
        &mut self,
        cmd: &str,
        snapshot: Vec<(frontend_core::ProfileIdentity, String)>,
    ) -> frontend_core::MarkedCommandReport {
        self.map_desk.stage_debug_command(cmd, Some(snapshot));
        let Some(mut submitted) = self.submit(OperatorCommand::DebugCommand, Targets::Marked)
        else {
            return frontend_core::MarkedCommandReport::from_parts(0, Vec::new());
        };
        submitted
            .take_marked()
            .unwrap_or_else(|| frontend_core::MarkedCommandReport::from_parts(0, Vec::new()))
    }

    /// True when the resolved launch profile is Local.
    pub fn debug_ui(&self) -> bool {
        self.profile_class() == ProfileClass::Local
    }

    /// The focused bot a Debug command goes to, or why it has none: a Local
    /// profile, a focused bot and a play session. The Debug controls draw
    /// disabled with that reason (the rail's TutSkip, Lumbridge and Teles
    /// destinations, the Debug panel's Send and the maxme button), and their
    /// handlers ([`Self::cheat_focused`], [`Self::send_debug_command`]) take
    /// the bot from this same answer, so a click never reaches their early
    /// exits.
    pub(crate) fn debug_send_target(&self) -> Result<&str, &'static str> {
        if !self.debug_ui() {
            return Err("Debug commands require a Local profile");
        }
        let name = self.core.selected().ok_or("Pick a focused bot")?;
        if !self.core.has_play() {
            return Err("No active play session");
        }
        Ok(name)
    }

    /// WalkTo Teleport uses the host's Local+loopback rule on this session's
    /// play connection when present, otherwise the bound target and play host.
    pub fn map_teleport_authorized(&self) -> bool {
        if self.core.has_play() {
            self.core.walk_teleport_authorized()
        } else {
            frontend_core::launch::debug_teleport_authorized(
                self.profile_class(),
                self.launcher.play_config().host(),
            )
        }
    }

    #[cfg(test)]
    pub(crate) fn set_map_host(&mut self, host: &str) {
        self.launcher.set_play_host(host);
    }

    /// Cached TutSkip for the focused profile: `None` unknown, `Some(true)`
    /// skipped, `Some(false)` still in tutorial.
    pub fn focused_tutorial_skipped(&self) -> Option<bool> {
        let name = self.focused_name()?;
        self.core.profile(&name).and_then(|p| p.tutorial_skipped())
    }

    /// Persist TutSkip on the focused vault profile (debugprefs). The button
    /// is drawn only for a focused profile ([`Self::focused_tutorial_skipped`]
    /// reads `Some(false)` only with one), so a click always has one and the
    /// early return is not reachable by a click.
    pub fn mark_tutorial_skipped(&mut self) {
        let Some(name) = self.focused_name() else {
            return;
        };
        if let Err(error) = self.submit_profile_write(OperatorCommand::MarkTutorialSkipped, &name) {
            self.error = Some(error);
        }
    }

    /// Cache the engine's `getvar tutorial` read on the profile.
    /// Cache the engine's `getvar tutorial` answer as the profile's TutSkip
    /// state (an owner result the core adopts; the operator's TutSkip is
    /// `MarkTutorialSkipped`).
    fn cache_tutorial(&mut self, name: &str, skipped: bool) {
        if let Err(e) = self.core.adopt_tutorial_reading(name, skipped) {
            self.error = Some(e.to_string());
        }
    }

    fn ingest_tutorial_chat(&mut self, statuses: &frontend_core::StatusBuf) {
        for s in statuses.iter() {
            if let Some((var, value)) = parse_getvar_line(s.chat_head()) {
                if var == "tutorial" {
                    self.cache_tutorial(s.username(), value >= 1000);
                }
            }
        }
    }

    fn maybe_getvar_tutorial(&mut self, statuses: &frontend_core::StatusBuf) {
        if !self.debug_ui() {
            return;
        }
        let Some(name) = self.focused_name() else {
            return;
        };
        if self.focused_tutorial_skipped().is_some() {
            return;
        }
        let ready = statuses
            .get(&name)
            .is_some_and(|s| s.ingame() && s.scene_state() == 2);
        if !ready || !self.tutorial_getvar_sent.insert(name.clone()) {
            return;
        }
        if self.core.has_play() {
            let _ = self.core.debug_command_internal(&name, "getvar tutorial");
        }
    }

    /// Applet point for the getvar-reply drain click, inside the chat
    /// surface. Position carries no safety: the click fires only while the
    /// clicked slot's own pending box is present, and
    /// `handle_chat_if_clicks` consumes any left click to ack it (zeroing
    /// the button before the minimenu pass), so the ack can never select a
    /// chat option, a trade/duel line, walk, bank, or flip a mode.
    const GETVAR_DRAIN_CLICK: (i32, i32) = (250, 400);

    /// Click away our own `getvar tutorial` reply out of the 289 tutorial
    /// pending-text box, once per box. The engine answers `getvar` with
    /// `messageGame("get tutorial: N")`, and the 289 client captures every
    /// type-0 line into `tut_com_message` while the tutorial overlay is up;
    /// `draw_chat` then paints the reply with "Click to continue" until a
    /// left click clears it (or a fresh login resets it). It is not a chat
    /// modal — there is no `BUTTON_CONTINUE`, so `Continue` refuses and
    /// cannot drain it. The click reuses the panel's own capture channel
    /// (the exact `Move`/`Down`/`Up` events a user click produces), gated
    /// on the focused slot's own pending-box fact from the snapshot — never
    /// on the chat-ring head, so chat turnover cannot hide the box and an
    /// absent box can never be clicked into the chat minimenu. Slots are
    /// re-armed when their box goes away; a missing capture channel or an
    /// unfocused slot only defers (never marks), so the drain retries on a
    /// later pump, still at most once per box.
    fn maybe_drain_getvar_dialogue(&mut self, statuses: &frontend_core::StatusBuf) {
        if !self.debug_ui() {
            return;
        }
        // Bookkeeping needs no focus or capture: a slot whose box is gone
        // is ready to drain its next box exactly once.
        for s in statuses.iter() {
            if !s.tutorial_pending() {
                self.tutorial_getvar_drained.remove(s.username());
            }
        }
        let Some(name) = self.focused_name() else {
            return;
        };
        if !self.tutorial_getvar_sent.contains(&name) {
            return;
        }
        // The capture channel feeds the focused slot's client only, so the
        // click is gated on that same slot's box: the client consumes it
        // and it can never reach the chat menu. Another slot's box waits
        // until that slot is focused with capture on.
        let pending = statuses.get(&name).is_some_and(|s| s.tutorial_pending());
        if !pending {
            return;
        }
        if !self.map_desk.input_connected() {
            return;
        }
        if self.tutorial_getvar_drained.contains(&name) {
            return;
        }
        let (x, y) = Self::GETVAR_DRAIN_CLICK;
        let staged = self
            .map_desk
            .stage_game_input(frontend_core::GameInputEvent::Captured(InputEv::Move {
                x,
                y,
            }))
            && self
                .map_desk
                .stage_game_input(frontend_core::GameInputEvent::Captured(InputEv::Down {
                    button: 1,
                    x,
                    y,
                }))
            && self
                .map_desk
                .stage_game_input(frontend_core::GameInputEvent::Captured(InputEv::Up));
        if !staged {
            return;
        }
        self.tutorial_getvar_drained.insert(name.clone());
        // Rare (once per box) and operator-visible both in the app log and
        // on stderr, so live runs can pin the drain instant against shots.
        slot_log(
            &name,
            Level::Info,
            "getvar drain: acked the tutorial reply's pending-text box",
        );
        api::host_log!(
            stderr;
            Category::Echo,
            Level::Info,
            slot = name.as_str(),
            "getvar drain: acked the tutorial reply's pending-text box",
        );
    }

    /// Frames for the Game pane (the focused slot's mailbox). Every
    /// wall member owns its own `FrameBuf` in the flat model; fall back to
    /// the first spawned slot when nothing is focused.
    pub fn focused_pixels(&self) -> Option<Arc<FrameBuf>> {
        if let Some(slot) = self.focused_slot() {
            return Some(Arc::clone(&slot.pixels));
        }
        self.core
            .slots()
            .values()
            .next()
            .map(|s| Arc::clone(&s.pixels))
    }

    /// Username of the focused slot (the sampled one, the old TV). Falls
    /// back to the first spawned slot when the focus has no slot yet.
    pub fn tv_name(&self) -> Option<String> {
        self.focused_name()
            .filter(|n| self.core.slots().contains_key(n))
            .or_else(|| self.core.slots().keys().next().cloned())
    }

    fn focused_slot(&self) -> Option<&SlotIo> {
        self.core.slot_io(self.core.selected()?)
    }

    /// Switch focus only onto an already-live slot. Capture sequencing uses
    /// this instead of [`Self::select`] so a missing actor cannot spawn.
    pub fn focus_existing(&mut self, name: &str) -> Result<(), String> {
        if !self.core.slots().contains_key(name) {
            return Err(format!("pair capture actor {name} is not a live slot"));
        }
        self.apply_focus(name);
        self.clear_resolved_error();
        Ok(())
    }

    /// Switch the focused profile. A parked vault name is spawned on first
    /// select (login FIFO); already-running slots stay up so the picker can
    /// change focus. Capture follows the new focus when the single capture
    /// toggle is on (never two keyboards). The profile editor's fields do
    /// not follow: they stay on the profile being edited.
    /// New slots inherit the vault profile's auto-login (and logout latch).
    ///
    /// Flat model: clicking a member is pure focus — the Game pane samples
    /// that slot's `FrameBuf`. No socket is swapped (the channel-head baton
    /// is gone); every slot keeps running.
    pub fn select(&mut self, name: &str) {
        if self.refuse_saving(OperatorCommand::Select, name) {
            return;
        }
        self.core.stage_select_opens_slot();
        if let Some(submitted) = self.submit(OperatorCommand::Select, Targets::Profile(name)) {
            if let Some(error) = self.core.command_failure(submitted.request()) {
                self.error = Some(error.to_string());
            }
        }
        self.apply_focus(name);
        self.restore_script_heading(name);
        self.clear_resolved_error();
        if !self.multibox {
            self.maybe_offer_background_ack();
        }
    }

    pub fn background_bot_count(&self) -> usize {
        self.core.background_bot_count()
    }

    fn maybe_offer_background_ack(&mut self) {
        if !self.persist_ui
            || self.ui.background_bots_ack
            || frontend_core::resources::background_bots_acked(&self.preferences)
        {
            return;
        }
        if self.background_bot_count() == 0 {
            return;
        }
        self.background_ack_open = true;
    }

    /// Submit "Got it, don't show again" for the background-bots notice.
    /// The acknowledgement is durable-only: the dialog stays, saying
    /// "Saving…" or the failure beside the button, and closes when the saved
    /// value holds it ([`Self::settle_notice_windows`]).
    pub fn ack_background_bots(&mut self) {
        if self.persist_ui {
            self.core
                .stage_notice(frontend_core::OperatorNotice::BackgroundBots);
            self.submit(OperatorCommand::AcknowledgeNotice, Targets::Application);
            frontend_core::resources::clear_background_bots_ack_error(&mut self.error);
        } else {
            self.background_ack_open = false;
        }
    }

    /// Dismiss the notice this time; it can appear again on the next trigger.
    /// Closing it resolves a failed save the dialog showed.
    pub fn dismiss_background_ack(&mut self) {
        self.background_ack_open = false;
        self.core
            .dismiss_setting_failure(frontend_core::Setting::Notice(
                frontend_core::OperatorNotice::BackgroundBots,
            ));
    }

    fn cancel_startup_focus(&mut self) {
        self.startup_focus_pending = None;
    }

    fn cancel_startup_focus_for_removed_profile(&mut self, name: &str) {
        // Before hydration, Restore can still resolve to any vault profile.
        // A named save only depends on that one profile.
        if self.startup_focus_pending.as_ref().is_some_and(|focus| {
            matches!(focus, StartupFocus::Restore)
                || matches!(focus, StartupFocus::SavedProfile(target) if target == name)
        }) {
            self.cancel_startup_focus();
        }
    }

    pub(crate) fn apply_focus(&mut self, name: &str) {
        // Every profile-selection path converges here, including a repeated
        // selection of the current bot. Operator intent supersedes startup.
        self.cancel_startup_focus();
        if self.persist_ui {
            let focus = name.to_string();
            self.ui.last_focus = Some(focus.clone());
            let _ =
                self.persist_panel_member(&["last_focus"], Some(serde_json::Value::String(focus)));
        }
        // The core owns the selection (and mirrors it onto the play, waking
        // the outgoing and incoming slots); `focus.focused` is the render
        // mirror the slot threads read. Destination is not a bot: switching
        // focus keeps the pending tile.
        let (old, capture) = {
            let focus = self.focus.lock().unwrap();
            if focus.focused.as_deref() == Some(name) {
                drop(focus);
                self.submit(OperatorCommand::Select, Targets::Profile(name));
                return;
            }
            (focus.focused.clone(), focus.capture)
        };
        self.debug_panel.clear_target_feedback();
        let mut focus = self.focus.lock().unwrap();
        focus.focused = Some(name.to_string());
        drop(focus);
        // The overlay follows the focused traveller: switching focus may
        // show a different (or no) route, so force a rebuild.
        self.route_gen += 1;
        if capture {
            if let Some(old) = old.clone() {
                if let Some(slot) = self.core.slot_io(&old) {
                    slot.input.set_enabled(false);
                }
            }
            self.capture_on(name);
        } else {
            self.map_desk.set_input_sender(None);
        }
        // Detach before waking the outgoing worker: its first disabled
        // drain releases held input even if the UI has not run ownership.
        self.submit(OperatorCommand::Select, Targets::Profile(name));
        // The General config mirrors the profile's raster/mem so the pane
        // shows what the slot actually runs (display only — no write-back,
        // no re-role). The profile editor's fields stay on the profile
        // being edited: a focus change never rewrites them.
        if let Some(p) = self.core.profile(name) {
            self.ui.raster = p.raster();
            self.ui.lowmem = p.lowmem();
        }
    }

    /// Renderer checkbox: the focused member's game renderer
    /// ([`DrawSwitch::GameRenderer`]); the surface mirrors it onto
    /// `Focus.renderer` and `renderer_by[focused]`, which the slot threads
    /// read in their per-frame hook, and the core wakes the focused slot.
    pub fn set_renderer(&mut self, on: bool) {
        self.submit(
            OperatorCommand::SetDraw(DrawSwitch::GameRenderer, on),
            Targets::Application,
        );
    }

    /// Sidecar-50 pref: wall/grid members render at 50 fps instead of the
    /// 1 fps watch cadence (a render-cadence knob, not the idle park).
    /// `pump_status` mirrors it onto each slot's frame-loop latch within a
    /// frame; the core wakes the parked members so the raise is not held
    /// up by the 1 s watch bound, and saves the preference.
    pub fn set_sidecar_50(&mut self, on: bool) {
        self.submit_draw(DrawSwitch::SidecarFullRate, on);
    }

    /// Game-pane 50 fps for whoever is focused. Does not follow that
    /// client onto the rail.
    pub fn set_focused_50(&mut self, on: bool) {
        self.submit_draw(DrawSwitch::FocusedFullRate, on);
    }

    /// Submit a saved draw switch. The control shows the operator's choice
    /// while the write is pending ([`Self::setting_drafts`]); the policy and
    /// the render mirror change only when the saved value does.
    fn submit_draw(&mut self, switch: DrawSwitch, on: bool) {
        match switch {
            DrawSwitch::CaptureInput => self.setting_drafts.capture = Some(on),
            DrawSwitch::SidecarFullRate => self.setting_drafts.sidecar_50 = Some(on),
            DrawSwitch::FocusedFullRate => self.setting_drafts.focused_50 = Some(on),
            DrawSwitch::GameRenderer | DrawSwitch::LiveFullRate => {}
        }
        self.submit(OperatorCommand::SetDraw(switch, on), Targets::Application);
        self.settle_setting_drafts();
    }

    /// What a saved draw switch's checkbox shows: the operator's choice
    /// while its write is pending, else the saved value.
    pub(crate) fn draw_shown(&self, switch: DrawSwitch) -> bool {
        let focus = self.focus.lock().unwrap();
        match switch {
            DrawSwitch::CaptureInput => self.setting_drafts.capture.unwrap_or(focus.capture),
            DrawSwitch::SidecarFullRate => {
                self.setting_drafts.sidecar_50.unwrap_or(focus.sidecar_50)
            }
            DrawSwitch::FocusedFullRate => {
                self.setting_drafts.focused_50.unwrap_or(focus.focused_50)
            }
            DrawSwitch::GameRenderer => focus.renderer,
            DrawSwitch::LiveFullRate => focus.live_full_rate,
        }
    }

    /// Confirm the render-all warning. Without `remember` (OK), wall members
    /// draw from now on (this launch only) and the confirm closes. With
    /// `remember` ("Don't show again"), submit the acknowledgement, after
    /// which unchecking "only render focused" applies at once. That is
    /// durable-only, and so is its effect: the confirm stays open saying
    /// "Saving…" or the failure beside the button, the box and the render
    /// policy stay at "only render focused", and only when the saved
    /// acknowledgement holds it do the members draw and the confirm close
    /// ([`Self::settle_notice_windows`]).
    pub fn confirm_render_all(&mut self, remember: bool) {
        if remember && self.persist_ui {
            self.core
                .stage_notice(frontend_core::OperatorNotice::RenderAllWarning);
            self.submit(OperatorCommand::AcknowledgeNotice, Targets::Application);
            self.wall.render_all_confirming = true;
            self.settle_notice_windows();
        } else {
            self.set_render_all(true);
            self.wall.render_all_warn_open = false;
            self.core
                .dismiss_setting_failure(frontend_core::Setting::Notice(
                    frontend_core::OperatorNotice::RenderAllWarning,
                ));
            if remember {
                self.ui.render_all_warning_ack = true;
            }
        }
    }

    /// Render every wall member (`all`) or only the focused one: the core
    /// records the policy (the capacity warning reads it); the panel's
    /// render mirror follows and every slot is woken.
    pub(crate) fn set_render_all(&mut self, all: bool) {
        self.submit(OperatorCommand::SetRenderAll(all), Targets::Application);
        self.focus.lock().unwrap().only_render_selected = !all;
        self.core.wake_all_slots();
    }

    /// Ephemeral live overlay: every drawing slot at 50 fps, focused
    /// included (the scenario's `full_rate` knob). Not sidecar-50, not
    /// persisted; the core wakes every slot so a parked member re-reads
    /// it within a frame.
    pub fn set_live_full_rate(&mut self, on: bool) {
        self.submit_draw(DrawSwitch::LiveFullRate, on);
    }

    /// Mirror the sidecar-50 pref onto every slot's frame-cadence latch
    /// (`SlotInput::set_full_rate`). Runs every UI frame so a focus,
    /// renderer, or wall-policy change lands within a frame.
    fn sync_sidecar_cadence(&mut self) {
        let focus = self.focus.lock().unwrap();
        for (name, slot) in self.core.slots() {
            slot.input.set_full_rate(full_rate_for(&focus, name));
        }
    }

    /// Mirror effective nav settings onto slot threads. Repeated UI frames
    /// retain the same Arc and generation so they do not invalidate the
    /// focused slot's materialized nav view.
    fn sync_nav_publish(&self) {
        let next = self.effective_nav();
        let mut publish = self.nav_publish.lock().unwrap();
        if *publish.settings != next {
            publish.settings = Arc::new(next);
            publish.generation = publish.generation.wrapping_add(1);
        }
    }

    /// Live overlay when a scenario armed one, else the operator prefs.
    pub fn effective_nav(&self) -> NavSettings {
        let settings = self
            .nav_overlay
            .clone()
            .unwrap_or_else(|| self.ui.nav.clone());
        crate::nav_settings::apply_paint_override(&settings, self.nav_paints_override)
    }

    /// Set the headed live paint choice without changing saved preferences.
    pub fn set_nav_paints_override(&mut self, value: Option<bool>) {
        self.nav_paints_override = value;
        self.sync_nav_publish();
    }

    /// Game window `.build()` Some/None. Closing the pane drops the live
    /// drain but leaves the capture pref (`should_capture` is draw-gated).
    /// Reopening with capture on re-attaches the channel.
    pub fn set_game_pane_open(&mut self, open: bool) {
        let mut focus = self.focus.lock().unwrap();
        let was = focus.game_pane_open;
        focus.game_pane_open = open;
        let name = focus.focused.clone();
        let capture = focus.capture;
        drop(focus);
        if was && !open {
            if capture {
                self.capture_off();
            }
        } else if !was && open && capture {
            if let Some(name) = name.as_deref() {
                self.capture_on(name);
            }
        }
        // draw_for_slot gates on the pane; kick the focused slot so a
        // parked thread sees the change within a frame.
        if let Some(name) = name {
            self.core.wake_slot(&name);
        }
    }

    /// Capture checkbox ([`DrawSwitch::CaptureInput`]): submits the saved
    /// choice. The input devices follow the saved value
    /// ([`Self::sync_capture_devices`]): on attaches a fresh channel and
    /// enables the focused slot's drain, off detaches user input (the
    /// consumer releases its own held keys/buttons even without queued ups
    /// or before reattachment).
    pub fn set_capture(&mut self, on: bool) {
        self.submit_draw(DrawSwitch::CaptureInput, on);
    }

    fn capture_on(&mut self, name: &str) {
        if let Some(slot) = self.core.slot_io(name) {
            let tx = InputSender::connect(&slot.input);
            slot.input.set_enabled(true);
            self.map_desk.set_input_sender(Some(tx));
        } else {
            self.map_desk.set_input_sender(None);
        }
    }

    fn capture_off(&mut self) {
        if let Some(slot) = self.focused_slot() {
            slot.input.set_enabled(false);
        }
        self.map_desk.set_input_sender(None);
    }

    /// Test fixture: select `name` in the core and mirror it onto the render
    /// focus, without spawn, persistence or capture effects.
    #[cfg(test)]
    pub(crate) fn set_focus_for_test(&mut self, name: &str) {
        self.submit(OperatorCommand::Select, Targets::Profile(name));
        self.focus.lock().unwrap().focused = Some(name.to_string());
    }

    /// A profile whose first save is still being written cannot be focused
    /// or started yet; say so instead of acting on staged credentials.
    fn refuse_saving(&mut self, command: OperatorCommand, name: &str) -> bool {
        let saving = self.core.profile_saving(name);
        if saving {
            self.refuse_command(
                command,
                &[name],
                Some(ErrorNeed::ProfileSaved(name.to_owned())),
                &format!("{name}: profile is still saving"),
            );
        }
        saving
    }

    /// Split borrow: the core plus the panel adapter over the other fields,
    /// with every operator model the panel holds lent to the command.
    fn core_and_surface(&mut self) -> (&mut OperatorSession<SlotIo>, PanelSurface<'_>) {
        let models = frontend_core::Models::none()
            .with_scripts(&mut self.scripts)
            .with_fleet(&mut self.fleet_model)
            .with_editor(&mut self.profile_editor)
            .with_loadouts(&mut self.loadouts)
            .with_quester(&mut self.quester_paths)
            .with_walk_globals(&mut self.walk_permissions)
            .with_draw(&mut self.draw);
        #[cfg(feature = "js")]
        let models = models.with_import(&mut self.profile_import);
        (
            &mut self.core,
            PanelSurface {
                focus: &self.focus,
                audio: &self.audio,
                memory_override: self.memory_override,
                map_desk: &mut self.map_desk,
                models,
                frame_hook: None,
            },
        )
    }

    /// Issue one operator command through the core's command seam (the
    /// entry point the TUI and the headless driver use). A refused command
    /// shows on the banner.
    pub(crate) fn submit(
        &mut self,
        command: OperatorCommand,
        targets: Targets<'_>,
    ) -> Option<frontend_core::Submitted> {
        let (core, mut surface) = self.core_and_surface();
        match core.submit(command, targets, &mut surface) {
            Ok(submitted) => Some(submitted),
            Err(error) => {
                self.error = Some(error.to_string());
                None
            }
        }
    }

    /// [`Self::submit`], then show the command's failed operation on the
    /// banner (today's lifecycle reporting).
    fn submit_reporting(
        &mut self,
        command: OperatorCommand,
        targets: Targets<'_>,
    ) -> Option<frontend_core::Submitted> {
        let submitted = self.submit(command, targets)?;
        if let Some(op) = submitted.operation() {
            self.report_failure(op);
        }
        Some(submitted)
    }

    /// Show a failed lifecycle operation on the banner.
    fn report_failure(&mut self, op: frontend_core::OperationId) {
        if let Some(error) = self.core.failure(op) {
            self.error = Some(error);
        }
    }

    /// Reorder the saved Load order: stage `op`, then submit
    /// `ReorderProfiles` (the core decides; it refuses outside Load order).
    /// A refusal or a failed write shows on the banner.
    pub(crate) fn reorder_profiles(&mut self, op: frontend_core::ReorderOp) {
        #[cfg(test)]
        REORDERS_SUBMITTED.set(REORDERS_SUBMITTED.get() + 1);
        self.core.stage_reorder(op);
        let Some(submitted) = self.submit(OperatorCommand::ReorderProfiles, Targets::Application)
        else {
            return;
        };
        if let Some(reason) = self.core.command_failure(submitted.request()) {
            self.error = Some(reason.to_owned());
        } else if let Some(op) = submitted.operation() {
            self.report_failure(op);
        }
    }

    /// Credentials Log in: clear the logout latch, arm an explicit one-shot
    /// handshake, then select (spawn if needed).
    pub fn login(&mut self, name: &str) {
        self.submit_reporting(OperatorCommand::Login, Targets::Profile(name));
        self.select(name);
    }

    /// Log out one member (the credentials Log out button): latch it so
    /// auto-login is blocked until [`Session::login_all`], then arm a clean
    /// IF logout. The slot stays up and focused; only the login intent
    /// changes.
    pub fn logout(&mut self, name: &str) {
        self.scripts.stage_cancel_queued("logged out");
        self.submit(OperatorCommand::CancelQueuedStart, Targets::Profile(name));
        self.apply_script_notice();
        self.submit(OperatorCommand::Logout, Targets::Profile(name));
    }

    /// Persist the focused profile's auto-login checkbox to the vault
    /// (`ProfileSettings.auto_login`) and mirror it onto a running slot's
    /// `arm.auto_login`. Never spawns or stops a slot.
    pub fn set_auto_login(&mut self, name: &str, on: bool) -> bool {
        let result = self.submit_profile_write(OperatorCommand::SetAutoLogin(on), name);
        self.apply_result(result)
    }

    /// Submit a profile-write row for `name`; a dispatch refusal comes back
    /// as its owner's text (today's `Err` of the core write).
    fn submit_profile_write(&mut self, command: OperatorCommand, name: &str) -> Result<(), String> {
        let (core, mut surface) = self.core_and_surface();
        let submitted = core
            .submit(command, Targets::Profile(name), &mut surface)
            .map_err(|error| error.to_string())?;
        match self.core.command_failure(submitted.request()) {
            Some(reason) => Err(reason.to_string()),
            None => Ok(()),
        }
    }

    fn apply_result(&mut self, result: Result<(), String>) -> bool {
        match result {
            Ok(()) => {
                self.error = None;
                true
            }
            Err(error) => {
                self.error = Some(error);
                false
            }
        }
    }

    /// Persist the focused profile's guardian settings (`random_events`,
    /// `lamp_skill`, `lamp_auto`) to the vault — the same upsert path as
    /// auto-login — and mirror `random_events`, `lamp_auto`, and
    /// `lamp_skill` onto a running slot's arm so toggle-off never
    /// acts/holds without a respawn. Never spawns or stops a slot.
    pub fn set_random_settings(
        &mut self,
        name: &str,
        random_events: bool,
        lamp_skill: &str,
        lamp_auto: bool,
    ) -> bool {
        self.core
            .stage_guardian(GuardianEdit::fields(random_events, lamp_skill, lamp_auto));
        let result = self.submit_profile_write(OperatorCommand::SetGuardian, name);
        self.apply_result(result)
    }

    pub fn set_random_event_policy(
        &mut self,
        name: &str,
        event: &str,
        policy: GuardianPolicy,
    ) -> bool {
        self.core
            .stage_guardian(GuardianEdit::policy(event, policy));
        let result = self.submit_profile_write(OperatorCommand::SetGuardian, name);
        self.apply_result(result)
    }

    /// Persist the focused profile's Cure poison setting to the vault
    /// and mirror it onto a running slot's arm once the write is
    /// durable, so the slot sees the change within one frame. Never
    /// spawns or stops a slot.
    pub fn set_survival(&mut self, name: &str, cure_poison: CurePoison) -> bool {
        self.core
            .stage_survival(SurvivalEdit::cure_poison(cure_poison));
        let result = self.submit_profile_write(OperatorCommand::SetSurvival, name);
        self.apply_result(result)
    }

    /// What the Game pane's memory control shows: the operator's choice for
    /// the focused bot while its save is pending, else the queued mode for
    /// the focused bot's next login (including a session override). The mode
    /// itself changes only when the choice is durable
    /// ([`Self::apply_lowmem_choice`]). Another bot's pending choice never
    /// shows here.
    pub fn focused_lowmem(&self) -> bool {
        let focused = self.core.selected();
        self.launch_previews
            .lowmem(focused)
            .or_else(|| focused.and_then(|name| self.core.memory_mode(name)))
            .unwrap_or(self.ui.lowmem)
    }

    /// What the Game-pane none/GPU/CPU control shows (General config): the
    /// operator's choice for the focused bot while its save is pending, else
    /// the applied mode. Another bot's pending choice never shows here.
    /// Rail members stay GPU (CPU/none only as fallback via `set_draw` /
    /// `prefer_cpu`).
    pub fn focused_raster(&self) -> RasterMode {
        self.launch_previews
            .raster(self.core.selected())
            .unwrap_or(self.ui.raster)
    }

    /// Choose the Game-pane raster. A remembered launch choice takes effect
    /// only when its write is durable ([`Self::apply_raster_choice`]); until
    /// then the control shows the choice and nothing else changes, and a
    /// failed write changes nothing. Returns whether the choice was accepted.
    pub fn set_focused_raster(&mut self, raster: RasterMode) -> bool {
        let effect = LaunchEffect::Raster {
            raster,
            profile: self.focused_name(),
        };
        self.choose_launch(&["raster"], serde_json::to_value(raster).ok(), effect)
    }

    /// Apply a durable Game-pane raster. Off is `set_draw` on the focused
    /// client only. Gpu↔Cpu flips the slot's `prefer_cpu` latch; the host drops
    /// the `Renderer` and reattaches the right backend on the **same**
    /// client — never a restart. Rail members are not touched. `profile` is
    /// the bot that was focused when the choice was made: its saved raster
    /// and latch change, and the focused client follows only while it is
    /// still the focused one.
    fn apply_raster_choice(&mut self, raster: RasterMode, profile: Option<String>) {
        let focused = self.focused_name() == profile;
        if focused {
            self.ui.raster = raster;
        }
        if let Some(name) = &profile {
            if let Err(error) = self.submit_profile_write(OperatorCommand::SetRaster(raster), name)
            {
                self.error = Some(error);
            }
        }
        if focused {
            self.set_renderer(raster != RasterMode::Off);
        }
        let Some(name) = profile else {
            return;
        };
        if raster == RasterMode::Off {
            return;
        }
        if let Some(slot) = self.core.slot_io(&name) {
            slot.input.set_prefer_cpu(raster == RasterMode::Cpu);
        }
    }

    /// Choose the Game pane's memory mode for its next login. Durable-only
    /// like [`Self::set_focused_raster`] ([`Self::apply_lowmem_choice`]).
    pub fn set_focused_lowmem(&mut self, lowmem: bool) -> bool {
        self.error = None;
        let effect = LaunchEffect::Lowmem {
            lowmem,
            profile: self.focused_name(),
        };
        self.choose_launch(&["lowmem"], Some(serde_json::Value::Bool(lowmem)), effect)
    }

    /// Apply a durable memory choice: the shared command queues the next
    /// login mode of `profile`, the bot that was focused when it was made.
    /// Neither the client nor its speaker changes while this setting is
    /// saved.
    fn apply_lowmem_choice(&mut self, lowmem: bool, profile: Option<String>) {
        if self.focused_name() == profile {
            self.ui.lowmem = lowmem;
        }
        let Some(name) = profile else {
            return;
        };
        let mode = if lowmem {
            api::lifecycle::MemoryMode::Low
        } else {
            api::lifecycle::MemoryMode::High
        };
        if let Err(error) = self.submit_profile_write(OperatorCommand::SetMemoryMode(mode), &name) {
            self.error = Some(error);
        }
    }

    /// Login-time vs current memory mode for the focused slot: drives the
    /// mem notice and the Relog-now button. `None` without a focused slot
    /// that has both a recorded login and a vault row.
    pub fn focused_memory_notice(&self) -> Option<frontend_core::MemoryNotice> {
        let name = self.focused_name()?;
        self.core.memory_status(&name)
    }

    /// Whether the focused slot has work a Relog-now would interrupt or
    /// discard, including a Start still waiting for admission.
    pub fn focused_memory_relog_warning(&self) -> bool {
        let Some(name) = self.focused_name() else {
            return false;
        };
        self.core.memory_relog_warning(&name) || self.scripts.start_waiting(&name)
    }

    /// Explicit Relog-now through the login FIFO. Running or queued script
    /// work requires a second confirming click. The button is drawn only for a
    /// focused bot with a memory notice ([`Self::focused_memory_notice`]
    /// needs the focus), so a click always has one and the early return is
    /// not reachable by a click.
    pub fn request_focused_memory_relog(&mut self) -> bool {
        let Some(name) = self.focused_name() else {
            return false;
        };
        let armed = self.mem_relog_armed.as_deref() == Some(name.as_str());
        if self.focused_memory_relog_warning() && !armed {
            self.mem_relog_armed = Some(name);
            return false;
        }
        self.mem_relog_armed = None;
        self.scripts.stage_cancel_queued("logged out");
        self.submit(OperatorCommand::CancelQueuedStart, Targets::Profile(&name));
        self.apply_script_notice();
        self.submit_reporting(OperatorCommand::RelogNow, Targets::Profile(&name));
        true
    }

    /// Whether the mem-popup Relog-now is armed for the focused bot (a
    /// running script needs the second confirming click).
    pub fn mem_relog_armed(&self) -> bool {
        let focused = self.focused_name();
        focused.is_some_and(|name| self.mem_relog_armed.as_deref() == Some(name.as_str()))
    }

    /// GPU↔CPU (not Off) reattaches the renderer on the same client,
    /// never a restart or logout.
    pub fn raster_switch_needs_confirm(
        _next: RasterMode,
        _prefer_cpu: bool,
        _slot_spawned: bool,
    ) -> bool {
        false
    }

    /// Pick a raster. Off is `set_draw`. GPU↔CPU on the Game-pane client
    /// drops + reattaches the renderer (never a logout). Rail members stay
    /// GPU/lowmem.
    pub fn request_focused_raster(&mut self, raster: RasterMode) {
        if self.focused_raster() == raster {
            return;
        }
        let _ = self.set_focused_raster(raster);
    }

    /// Queue the Game pane's memory mode for its next login.
    pub fn request_focused_lowmem(&mut self, lowmem: bool) {
        if self.focused_lowmem() == lowmem {
            return;
        }
        let _ = self.set_focused_lowmem(lowmem);
    }

    /// Load a wall member: ensure its slot and select it. Auto-login
    /// follows the vault profile setting unless the member's logout latch
    /// blocks it (`SlotArm::new(should_auto_login)`); a latched member is
    /// spawned holding the title screen until [`Session::login_all`].
    /// Returns whether the name was newly added to the wall.
    pub fn load(&mut self, name: &str) -> bool {
        if self.refuse_saving(OperatorCommand::Load, name) {
            return false;
        }
        let added = self
            .submit_reporting(OperatorCommand::Load, Targets::Profile(name))
            .is_some_and(|submitted| submitted.added() > 0);
        // Load all / chooser rows spawn onto the rail and focus the member
        // (the flat model's "click" — the Game pane samples this slot). The
        // script card follows the focused profile, as in `select`.
        self.apply_focus(name);
        self.restore_script_heading(name);
        self.sync_wall_focus();
        added
    }

    /// Load every profile (vault plus running slots) that is not already a
    /// wall member — the chooser's "Load all". Returns how many were newly
    /// added. Login intent still follows each profile's auto-login setting.
    pub fn load_all(&mut self) -> usize {
        let had_focus = self.core.selected().is_some();
        let last = if !had_focus && self.persist_ui {
            crate::ui_state::load(&self.preferences).last_focus
        } else {
            None
        };
        self.core.set_focus_hint(last.as_deref());
        // Nothing left to load is not an error for Load all, as before.
        let added = {
            let (core, mut surface) = self.core_and_surface();
            core.submit(OperatorCommand::Load, Targets::Everyone, &mut surface)
        };
        let added = match added {
            Ok(submitted) => {
                if let Some(op) = submitted.operation() {
                    self.report_failure(op);
                }
                submitted.added()
            }
            Err(frontend_core::SubmitError::NoTarget(_)) => 0,
            Err(error) => {
                self.error = Some(error.to_string());
                0
            }
        };
        // Never steal an existing focus. When nothing was focused, the core
        // already picked with `pick_focus`; mirror that onto the Game pane
        // so the chosen bot draws. Applying last-member focus here used to
        // turn A's `set_draw` off.
        if !had_focus {
            if let Some(name) = self.core.selected().map(str::to_owned) {
                self.select(&name);
            }
        }
        self.sync_wall_focus();
        if !had_focus && self.core.selected().is_some() {
            self.wake_all_slots();
        }
        added
    }

    /// Chooser row ✕: delete the vault profile only. A live wall member is
    /// **not** logged out or dropped; the row just disappears from the
    /// chooser (credentials Save re-creates it). Returns whether a row was
    /// removed; failures set [`Session::error`].
    pub fn vault_remove(&mut self, name: &str) -> bool {
        let Some(submitted) = self.submit(OperatorCommand::DeleteProfile, Targets::Profile(name))
        else {
            return false;
        };
        let terminal = self
            .core
            .commands()
            .report(submitted.request())
            .and_then(|report| report.members().first())
            .and_then(|member| member.terminal());
        match terminal {
            Some(api::identity::Settlement::Completed(
                frontend_core::command::Completion::Skipped(
                    frontend_core::command::SkipReason::ProfileUnavailable,
                ),
            )) => false,
            Some(api::identity::Settlement::Failed(_)) => {
                self.error = self
                    .core
                    .command_failure(submitted.request())
                    .map(str::to_string);
                false
            }
            Some(api::identity::Settlement::Completed(
                frontend_core::command::Completion::Done,
            ))
            | None
                if submitted.operation().is_some() =>
            {
                self.cancel_startup_focus_for_removed_profile(name);
                self.error = None;
                true
            }
            Some(api::identity::Settlement::Completed(
                frontend_core::command::Completion::Done,
            )) => {
                self.cancel_startup_focus_for_removed_profile(name);
                self.error = None;
                true
            }
            _ => false,
        }
    }

    /// Mirror `wall.members` into `Focus.wall` so `draw_for_slot` can paint
    /// unfocused tiles when only-render-selected is off. Call whenever
    /// membership changes: load, load_all, rail_remove, or the seed path.
    fn sync_wall_focus(&mut self) {
        let members = self.core.members().to_vec();
        self.focus.lock().unwrap().wall = members;
    }

    /// Kick every slot thread after a wall-policy change (`only render
    /// selected` toggling flips every member's draw state; a parked thread
    /// must re-read it within a frame, not at the game-tick timeout).
    pub fn wake_all_slots(&self) {
        self.core.wake_all_slots();
    }

    /// Log in every wall member: clear their latches, cancel pending removal,
    /// and arm a login so title-screen slots handshake. A finished worker is
    /// reaped and recreated with its retained IO, matching explicit Log in.
    /// One-shot unless the profile's auto-login is set (which keeps the arm
    /// armed after the handshake). Queue membership follows worker arrival at
    /// Queueing; the focused slot is the sole priority exception.
    pub fn login_all(&mut self) {
        // An empty fleet logs in nobody; that is not an error, as before.
        let submitted = {
            let (core, mut surface) = self.core_and_surface();
            core.submit(OperatorCommand::Login, Targets::Everyone, &mut surface)
        };
        match submitted {
            Ok(submitted) => {
                if let Some(op) = submitted.operation() {
                    self.report_failure(op);
                }
            }
            Err(frontend_core::SubmitError::NoTarget(_)) => {}
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    /// Log out every wall member: record the latch (blocks auto-login
    /// until the next [`Session::login_all`]) and arm a clean IF logout.
    /// `want_login` is cleared too so a title-screen member does not
    /// handshake right back in.
    pub fn logout_all(&mut self) {
        let (core, mut surface) = self.core_and_surface();
        // Nobody to log out is not an error.
        let _ = core.submit(OperatorCommand::Logout, Targets::Everyone, &mut surface);
    }

    /// MultiBox toggle. On: seed the wall with every already-running slot
    /// (first on this process opens the chooser) and open the wall draw
    /// policy (`Focus.wall_open`), which stays true for rail **or** grid.
    /// Off: clear the grid and any open chooser and stop extra rasters
    /// (`wall_open = false`) without logging anyone out. Off closes Profiles
    /// and its form, so while the form's save has not settled it waits: the
    /// Discard / Keep editing prompt is staged and MultiBox stays on.
    /// Returns whether the toggle was applied.
    pub fn set_multibox(&mut self, on: bool) -> bool {
        let Some(submitted) = self.submit(OperatorCommand::SetMultiBox(on), Targets::Application)
        else {
            return false;
        };
        let staged = self
            .core
            .commands()
            .report(submitted.request())
            .and_then(|report| report.members().first())
            .and_then(|member| member.terminal())
            .is_some_and(|terminal| {
                matches!(
                    terminal,
                    api::identity::Settlement::Completed(
                        frontend_core::command::Completion::Skipped(
                            frontend_core::command::SkipReason::DecisionStaged
                        )
                    )
                )
            });
        if staged {
            self.request_window_focus(PanelWindow::Profiles);
            self.wall.chooser_open = true;
            return false;
        }
        self.apply_multibox(on);
        true
    }

    fn apply_multibox(&mut self, on: bool) {
        let turning_off = self.multibox && !on;
        self.multibox = on;
        if on {
            // `SetMultiBox(true)` re-seeded the fleet with every live
            // lifetime (the core cancels their rail teardown and restores
            // any retained IO) before this presentation step.
            self.wall.on_multibox_on();
            // After seed: if focus is missing or not a wall member, restore
            // last_focus when it is on the wall, else the first member.
            let focused = self.focused_name();
            let need = match focused.as_deref() {
                None => true,
                Some(f) => !self.core.fleet().contains(f),
            };
            if need {
                // Live boots never restore the operator's disk last_focus;
                // the first wall member / later `select(&names[0])` wins.
                let last = if self.persist_ui {
                    crate::ui_state::load(&self.preferences).last_focus
                } else {
                    None
                };
                if let Some(name) = frontend_core::pick_focus(self.core.members(), last.as_deref())
                {
                    self.select(&name);
                }
            }
        } else {
            self.wall.on_multibox_off();
            self.cancel_edit_profile();
        }
        self.focus.lock().unwrap().wall_open = on;
        self.sync_wall_focus();
        // The wall policy change flips every member's draw state; kick all
        // so parked threads re-read it within a frame.
        self.core.wake_all_slots();
        if turning_off {
            self.maybe_offer_background_ack();
        }
    }

    /// Grid submode of MultiBox: hides the rail in the Game pane. A no-op
    /// while MultiBox is off.
    pub fn set_grid(&mut self, on: bool) {
        if self.multibox {
            self.wall.grid = on;
        }
    }

    /// The MultiBox rail window is on screen: MultiBox is on and Grid is off.
    pub fn rail_shown(&self) -> bool {
        self.multibox && !self.wall.grid
    }

    /// Remove a member from the rail and return immediately. Connected slots
    /// get a bounded clean-logout window advanced by [`Session::pump_status`];
    /// disconnected workers are stopped asynchronously. Neither path sleeps
    /// or joins on the UI call stack.
    pub fn rail_remove(&mut self, name: &str) {
        self.rail_remove_at(name, Instant::now());
    }

    fn rail_remove_at(&mut self, name: &str, now: Instant) {
        self.scripts.stage_cancel_queued("removed");
        self.submit(OperatorCommand::CancelQueuedStart, Targets::Profile(name));
        self.apply_script_notice();
        if self.mem_relog_armed.as_deref() == Some(name) {
            self.mem_relog_armed = None;
        }
        let submitted = {
            let (core, mut surface) = self.core_and_surface();
            core.submit_at(
                OperatorCommand::Remove,
                Targets::Profile(name),
                now,
                &mut surface,
            )
        };
        let submitted = match submitted {
            Ok(submitted) => submitted,
            Err(error) => {
                self.error = Some(error.to_string());
                return;
            }
        };
        let Some(removal) = submitted.removal() else {
            return;
        };
        let (reselected, cleared) = (
            removal.reselected().map(str::to_string),
            removal.selection_cleared(),
        );
        self.cancel_startup_focus_for_removed_profile(name);
        if reselected.is_some() || cleared {
            self.cancel_startup_focus();
        }
        self.sync_wall_focus();
        if let Some(next) = reselected.as_deref() {
            self.select(next);
        } else if cleared {
            self.focus.lock().unwrap().focused = None;
            self.map_desk.set_input_sender(None);
            // Last focused slot gone and no neighbour: no remaining drawer
            // will publish, so release the flags sidecar here.
            crate::picker::drop_flags_sidecar();
        }
    }

    pub fn sync_walk_map(&mut self, world: Option<Arc<NavWorld>>) {
        let Some(profile) = self.server_profile().map(BoundProfile::share) else {
            return;
        };
        if self.map_desk.map_demand().is_none() {
            self.map_desk
                .stage_open_demand(profile.share(), WalkMapDemand::Images);
            self.submit(OperatorCommand::OpenMapDemand, Targets::Application);
        }
        self.core.poll_map_demand(&mut self.map_desk);
        let demand = self.map_desk.map_demand().map(|handle| {
            (
                handle.demand(),
                handle.is_ready(),
                handle.ready_images().ok().flatten(),
                handle.ready_catalogue().ok(),
            )
        });
        let Some((kind, ready, images, catalogue)) = demand else {
            return;
        };
        if ready {
            if self.map_images.is_none() {
                if let Some(images) = images {
                    self.map_images = Some(images);
                } else {
                    self.map_desk.note_ready_without_terrain(kind);
                }
            }
            if let Some(catalogue) = catalogue {
                self.bind_ready_catalogue(world.clone(), catalogue);
            }
        } else if self.map_catalogue.is_none() || !self.map_catalogue_named {
            // The catalogue publishes before the terrain bake ends; look
            // for it at a coarse cadence (descriptor + directory probe),
            // not on every frame of the bake.
            let now = Instant::now();
            if self
                .map_catalogue_probe
                .is_none_or(|at| now.duration_since(at) >= MAP_CATALOGUE_PROBE)
            {
                self.map_catalogue_probe = Some(now);
                if let Some(ready) = frontend_core::walk_map::peek_walk_catalogue(&profile) {
                    self.bind_ready_catalogue(world.clone(), ready);
                }
            }
        }
        self.refresh_observed_map_services(world);
    }

    /// Bake now (optionally remembering "always") through the command seam.
    pub fn accept_map_bake(&mut self, remember: bool) {
        if remember {
            self.set_map_bake_choice(MapBakeChoice::Always);
        }
        let Some(profile) = self.server_profile().map(BoundProfile::share) else {
            return;
        };
        self.map_desk.stage_bake_now(profile);
        if let Some(submitted) = self.submit(OperatorCommand::BakeMapNow, Targets::Application) {
            if let Some(error) = self.core.command_failure(submitted.request()) {
                self.error = Some(error.to_string());
            }
        }
    }

    /// Not now: retain the catalogue-only demand through the command seam.
    pub fn decline_map_bake(&mut self) {
        self.submit(OperatorCommand::DeclineMapBake, Targets::Application);
    }

    /// Remembered bake choice. Durable-only: the bake gate takes it from the
    /// saved value, the control shows the choice while its write is pending
    /// ([`Self::map_bake_shown`]), and a failed write shows the saved choice
    /// again with the failure beside the control.
    pub fn set_map_bake_choice(&mut self, choice: MapBakeChoice) {
        self.setting_drafts.map_bake = Some(choice);
        self.submit(
            OperatorCommand::SetMapBakeChoice(choice),
            Targets::Application,
        );
        self.settle_setting_drafts();
    }

    /// The remembered bake choice a control shows: the operator's choice
    /// while its write is pending, else the saved choice.
    pub fn map_bake_shown(&self) -> MapBakeChoice {
        self.setting_drafts
            .map_bake
            .unwrap_or_else(|| self.map_desk.bake_gate().choice())
    }

    pub fn release_walk_map(&mut self) {
        if self.map_desk.needs_release() {
            self.submit(OperatorCommand::ReleaseMapDemand, Targets::Application);
        }
        self.map_catalogue_probe = None;
        self.map_images = None;
        self.map_catalogue = None;
        self.observed_map_services.clear();
        self.map_catalogue_named = false;
        self.map_model.close();
    }

    pub fn map_bind_status(&self) -> Option<String> {
        let status = self.map_desk.map_demand()?.status();
        match status.state() {
            WalkMapJobState::Ready => None,
            WalkMapJobState::Queued => Some(String::from("map queued")),
            WalkMapJobState::Running => match status.progress() {
                Some(progress) if !progress.message().is_empty() => {
                    Some(progress.message().to_owned())
                }
                Some(progress) => {
                    let stage = match progress.stage() {
                        WalkMapStage::ReadingCache => "reading cache",
                        WalkMapStage::DerivingPois => "deriving POIs",
                        WalkMapStage::BakingPlane => "baking terrain",
                        WalkMapStage::BuildingZoomLevels => "building zoom",
                        WalkMapStage::Publishing => "publishing",
                    };
                    Some(format!(
                        "{stage} · {}/{}",
                        progress.completed(),
                        progress.total()
                    ))
                }
                None => Some(String::from("map running")),
            },
            WalkMapJobState::Paused => Some(String::from("map bake paused")),
            WalkMapJobState::Cancelled => Some(String::from("map bake cancelled")),
            WalkMapJobState::Failed => Some(format!(
                "map bake failed: {}",
                status.failure().unwrap_or("unknown error")
            )),
        }
    }

    fn bind_ready_catalogue(
        &mut self,
        world: Option<Arc<NavWorld>>,
        ready: Arc<frontend_core::walk_map::WalkReadyCatalogue>,
    ) {
        let profile = self.server_profile().map(BoundProfile::share);
        let Some(world) = world
            .or_else(|| self.core.nav_world())
            .or_else(|| profile.as_ref().and_then(BoundProfile::world))
            .or_else(crate::picker::pack)
        else {
            return;
        };
        let identity = ready.identity();
        let nav = self.map_nav_digest_for(Some(world.as_ref()));
        let data = profile
            .as_ref()
            .and_then(BoundProfile::game_data)
            .or_else(|| self.core.game_data());
        let same = self.map_catalogue.as_ref().is_some_and(|catalogue| {
            catalogue.identity() == identity
                && catalogue.nav_identity() == nav
                && catalogue.client_status() == WalkSourceStatus::Available
        });
        if same && (self.map_catalogue_named || data.is_none()) {
            return;
        }
        let Some(profile) = profile.as_ref() else {
            return;
        };
        let Ok((catalogue, profile_named)) =
            self.core.walk_catalogue_from_ready(profile, world, ready)
        else {
            return;
        };
        let has_data = data.is_some();
        let catalogue = match data {
            Some(data) if !profile_named => match catalogue.with_game_data(data) {
                Ok(named) => named,
                Err(_) => return,
            },
            _ => catalogue,
        };
        self.map_catalogue_named = profile_named || has_data;
        self.map_catalogue = Some(Arc::new(catalogue));
    }

    fn refresh_observed_map_services(&mut self, world: Option<Arc<NavWorld>>) {
        self.observed_map_services.clear();
        let Some(world) = world.or_else(crate::picker::pack) else {
            return;
        };
        let Some(name) = self.focused_name() else {
            return;
        };
        let context = self.picker_context(world.as_ref());
        self.observed_map_services = self.core.observed_walk_services(&name, context);
    }

    pub fn map_nav_digest(&self) -> nav_planner::map::identity::Digest {
        self.map_nav_digest_for(None)
    }

    fn map_nav_digest_for(&self, world: Option<&NavWorld>) -> nav_planner::map::identity::Digest {
        self.server_profile()
            .and_then(BoundProfile::nav_identity)
            .or_else(|| {
                self.map_catalogue.as_ref().and_then(|catalogue| {
                    if world.is_some_and(|world| !std::ptr::eq(catalogue.world().as_ref(), world)) {
                        None
                    } else {
                        Some(catalogue.nav_identity())
                    }
                })
            })
            .unwrap_or_else(|| {
                nav_planner::map::identity::Digest::of(&self.launcher.generation().to_be_bytes())
            })
    }

    /// Current map command binding. Legacy/grid-only views use the profile
    /// generation, never a pointer address or a client/content compatibility proof.
    pub fn picker_context(&self, world: &NavWorld) -> frontend_core::walk_map::WalkMapContext {
        let name = self.focused_name();
        let nav = self.map_nav_digest_for(Some(world));
        let overlay = self.map_catalogue.as_ref().map(|catalogue| catalogue.key());
        self.core
            .walk_map_context(name.as_deref(), nav, overlay, self.launcher.generation())
    }

    pub fn select_picker_tile(&mut self, world: &NavWorld, requested: Tile) -> Option<Tile> {
        let context = self.picker_context(world);
        self.map_model.bind(context);
        self.map_model.select_tile(world, requested)
    }

    pub fn select_picker_coordinates(
        &mut self,
        world: &NavWorld,
        input: &str,
    ) -> Result<Option<Tile>, frontend_core::walk_map::WalkActionError> {
        let requested = frontend_core::walk_map::WalkMap::parse_coordinates(input)?;
        let target = self.select_picker_tile(world, requested);
        self.map_model
            .set_center([f64::from(requested.x) + 0.5, f64::from(requested.z) + 0.5]);
        Ok(target)
    }

    pub fn select_picker_poi(&mut self, index: usize) -> bool {
        let Some(catalogue) = self.map_catalogue.as_ref() else {
            return false;
        };
        let context = self.picker_context(catalogue.world().as_ref());
        self.map_model.bind(context);
        match self.map_model.select_poi(catalogue, index) {
            Ok(()) => true,
            Err(error) => {
                self.error = Some(error.to_string());
                false
            }
        }
    }

    pub fn picker_action_available(
        &self,
        world: &NavWorld,
        kind: frontend_core::walk_map::WalkActionKind,
    ) -> Result<Tile, frontend_core::walk_map::WalkActionError> {
        self.core.walk_availability(
            &self.map_model,
            kind,
            self.picker_context(world),
            self.focused_name().as_deref(),
        )
    }

    fn walk_find_options(&mut self) -> FindOptions {
        self.refresh_walk_permissions();
        self.walk_permissions
            .manual_options(self.route_through_zones)
    }

    /// Consume the pending map selection through the typed WalkTo row.
    pub fn confirm_picker_walk(&mut self, world: &NavWorld) -> bool {
        let context = self.picker_context(world);
        let name = self.focused_name();
        let options = self.walk_find_options();
        let origin = name
            .as_deref()
            .and_then(|name| self.core.walk_slot_status(name).origin());
        self.map_desk
            .stage_walk_to(&mut self.map_model, context, origin, options);
        let targets = name
            .as_deref()
            .map_or(Targets::Application, Targets::Profile);
        let Some(submitted) = self.submit(OperatorCommand::WalkTo, targets) else {
            return false;
        };
        if let Some(error) = self.core.command_failure(submitted.request()) {
            self.error = Some(error.to_string());
            return false;
        }
        let applied = self
            .core
            .commands()
            .report(submitted.request())
            .is_some_and(|report| {
                report.members().iter().any(|member| {
                    matches!(
                        member.terminal(),
                        Some(api::identity::Settlement::Completed(
                            frontend_core::command::Completion::Done
                        ))
                    )
                })
            });
        if applied {
            self.route_gen = self.route_gen.wrapping_add(1);
            self.error = None;
        } else {
            self.error = Some(String::from("map: WalkTo was not applied"));
        }
        applied
    }

    fn walk_send_names(&self) -> Vec<String> {
        if !self.core.members().is_empty() {
            return self.core.members().to_vec();
        }
        if let Some(name) = self.focused_name() {
            return vec![name];
        }
        self.statuses
            .iter()
            .map(|s| s.username().to_owned())
            .collect()
    }

    fn fleet_identity(&self, name: &str) -> frontend_core::ProfileIdentity {
        self.core.profile_identity_or_synthetic(name)
    }

    pub fn refresh_walk_send(&mut self) {
        let names = self.walk_send_names();
        let mut rows = Vec::with_capacity(names.len());
        for name in names {
            let status = self.core.walk_slot_status(&name);
            let identity = self.fleet_identity(&name);
            rows.push(WalkSendRow {
                name,
                identity,
                status,
            });
        }
        self.walk_send.rows = rows;
        self.walk_send.sync_walk_label(self.fleet_model.marks());
    }

    /// Focused or Group. Entering Group with no walker marked marks the
    /// bots a fleet action covers while nothing is marked (the loaded,
    /// shown ones) that can walk; a bot the fleet filter hides is not
    /// added. "All eligible" ([`Self::walk_send_all_eligible`]) marks every
    /// row of the Send list.
    pub fn set_walk_send_mode(&mut self, mode: WalkSendMode) {
        self.walk_send.mode = mode;
        if mode == WalkSendMode::Group
            && self.walk_send.checked_count(self.fleet_model.marks()) == 0
        {
            let identities = self
                .walk_send
                .rows
                .iter()
                .filter(|row| {
                    row.status.is_eligible() && self.fleet_model.in_unmarked_scope(row.identity)
                })
                .map(|row| row.identity)
                .collect::<Vec<_>>();
            self.core.stage_mark_all(identities);
            self.submit(OperatorCommand::MarkAll, Targets::Application);
        }
        self.walk_send.sync_walk_label(self.fleet_model.marks());
    }

    pub fn walk_send_all_eligible(&mut self) {
        let identities = self
            .walk_send
            .rows
            .iter()
            .filter(|row| row.status.is_eligible())
            .map(|row| row.identity)
            .collect::<Vec<_>>();
        self.core.stage_mark_all(identities);
        self.submit(OperatorCommand::MarkAll, Targets::Application);
        self.walk_send.sync_walk_label(self.fleet_model.marks());
    }

    /// The fleet toolbar's Walk: open (or bring forward) the WalkTo picker in
    /// Group mode over the toolbar's scope. With bots marked, the Send list
    /// is checked from the marks and no mark is added. With nothing marked,
    /// Group mode marks the loaded, shown bots that can walk (the button
    /// counts every loaded, shown bot; a bot that cannot walk is skipped),
    /// so the Send list names who walks before the operator picks a tile; a
    /// bot the fleet filter hides is not added.
    pub fn open_walkto_for_fleet(&mut self) {
        self.open_window(PanelWindow::WalkTo);
        self.refresh_walk_send();
        if self.fleet_model.marks().is_empty() {
            self.set_walk_send_mode(WalkSendMode::Group);
        } else {
            self.walk_send.mode = WalkSendMode::Group;
            self.walk_send.sync_walk_label(self.fleet_model.marks());
        }
    }

    pub fn walk_send_none(&mut self) {
        self.submit(OperatorCommand::ClearMarks, Targets::Application);
        self.walk_send.sync_walk_label(self.fleet_model.marks());
    }

    /// Mark or unmark the Send list's bot at `index` in the shared set. A bot
    /// that cannot walk stays as it is.
    pub fn set_walk_send_checked(&mut self, index: usize, checked: bool) {
        let name = self
            .walk_send
            .rows
            .get(index)
            .filter(|row| row.status.is_eligible())
            .map(|row| row.name.clone());
        if let Some(name) = name {
            self.submit(OperatorCommand::SetMark(checked), Targets::Profile(&name));
        }
        self.walk_send.sync_walk_label(self.fleet_model.marks());
    }

    /// Walk every marked bot to the confirmed destination through the typed
    /// GroupWalk row. Each bot routes from its own published state.
    pub fn confirm_picker_group_walk(&mut self, world: &NavWorld) -> bool {
        let context = self.picker_context(world);
        let options = self.walk_find_options();
        self.map_desk
            .stage_group_walk(&mut self.map_model, context, options);
        let Some(mut submitted) = self.submit(OperatorCommand::GroupWalk, Targets::Marked) else {
            return false;
        };
        let Some(report) = submitted.take_bulk() else {
            self.error = Some(String::from("map: GroupWalk produced no result"));
            return false;
        };
        let walking = report.done_count();
        if walking > 0 {
            self.route_gen = self.route_gen.wrapping_add(1);
        }
        self.error = Some(report.summary());
        walking > 0
    }

    pub fn confirm_picker_teleport(&mut self, world: &NavWorld) -> bool {
        let context = self.picker_context(world);
        let name = self.focused_name();
        let origin = name
            .as_deref()
            .and_then(|name| self.core.walk_slot_status(name).origin());
        self.map_desk
            .stage_teleport(&mut self.map_model, context, origin, FindOptions::default());
        let targets = name.as_deref().map_or(Targets::Selected, Targets::Profile);
        let Some(submitted) = self.submit(OperatorCommand::Teleport, targets) else {
            return false;
        };
        if let Some(error) = self.core.command_failure(submitted.request()) {
            self.error = Some(error.to_string());
            return false;
        }
        let applied = self
            .core
            .commands()
            .report(submitted.request())
            .is_some_and(|report| {
                report.members().iter().any(|member| {
                    matches!(
                        member.terminal(),
                        Some(api::identity::Settlement::Completed(
                            frontend_core::command::Completion::Done
                        ))
                    )
                })
            });
        if applied {
            self.error = None;
        } else {
            self.error = Some(String::from("map: Teleport was not applied"));
        }
        applied
    }

    /// The focused slot's valid observed tile `(x, z, level)`. A disconnected
    /// or not-ready slot and negative/out-of-plane producer sentinels have no
    /// routing origin.
    pub fn focused_tile(&self) -> Option<(i32, i32, i32)> {
        let name = self.focused_name()?;
        self.statuses.get(&name).and_then(|s| s.ready_tile())
    }

    /// The named slot's login-queue place from the shared fleet row (a
    /// member's, or the selected slot's), else `None`: a grant, a missing
    /// row or an invalid published place clears that slot's card.
    pub fn queue_for(&self, name: &str) -> Option<frontend_core::QueuePlace> {
        self.core.fleet_view().row(name).and_then(|row| row.queue())
    }

    /// Whether Log out acts on the focused slot: the shared projection's
    /// answer, the same one Fleet Log out and the TUI use. `None` without a
    /// focused slot.
    pub fn focused_logout_eligibility(&self) -> Option<frontend_core::LogoutEligibility> {
        let name = self.focused_name()?;
        Some(
            self.core
                .fleet_view()
                .row(&name)
                .map_or(frontend_core::LogoutEligibility::NotLoggedIn, |row| {
                    row.logout_eligibility()
                }),
        )
    }

    /// Whether the focused slot has a scene/player observation safe for game
    /// actions.
    pub fn focused_ingame(&self) -> bool {
        let Some(name) = self.focused_name() else {
            return false;
        };
        self.statuses.get(&name).is_some_and(|s| s.ingame())
    }

    /// Generated facts for the bound profile, when the cache identity matches.
    pub fn selected_game_data(&self) -> Option<std::sync::Arc<api::game_data::SelectedGameData>> {
        let profile = self.server_profile()?;
        profile.game_data()
    }

    /// Submit a Quest Paths form change. The folder field is used only in
    /// the frame the operator commits it; a checkbox toggle keeps the folder
    /// last submitted (or saved). Durable-only: the controller applies and
    /// reloads the source when the saved value holds it, the checkbox shows
    /// the choice and the field keeps its text while the write is pending,
    /// and a failed write shows the saved source again with the failure.
    pub(crate) fn persist_quester_paths_change(
        &mut self,
        enabled: bool,
        folder_committed: bool,
    ) -> Result<(), String> {
        let folder = if folder_committed {
            PathBuf::from(self.quester_paths_folder_edit.as_str())
        } else if let Some(submitted) = &self.quester_folder_submitted {
            PathBuf::from(submitted.as_str())
        } else {
            self.quester_paths.settings().folder().to_path_buf()
        };
        if folder_committed {
            self.quester_folder_submitted = Some(self.quester_paths_folder_edit.clone());
        }
        self.setting_drafts.quester_enabled = Some(enabled);
        let result =
            self.persist_quester_paths(frontend_core::QuesterPathsView::new(enabled, folder));
        self.settle_setting_drafts();
        result
    }

    pub(crate) fn persist_quester_paths(
        &mut self,
        after: frontend_core::QuesterPathsView,
    ) -> Result<(), String> {
        let selected = self.selected_game_data();
        self.core.stage_quester_paths(after, true, selected);
        match self.submit(OperatorCommand::SetQuesterPaths, Targets::Application) {
            Some(submitted) => match self.core.command_failure(submitted.request()) {
                Some(error) => Err(error.to_string()),
                None => Ok(()),
            },
            None => Err(self.error.clone().unwrap_or_default()),
        }
    }

    /// What the Quest Paths checkbox shows: the operator's choice while its
    /// write is pending, else the saved source's switch.
    pub(crate) fn quester_enabled_shown(&self) -> bool {
        self.setting_drafts
            .quester_enabled
            .unwrap_or_else(|| self.quester_paths.settings().enabled())
    }

    pub(crate) fn start_quester_paths_reload(&mut self) {
        let selected = self.selected_game_data();
        self.core.stage_quester_reload(selected);
        // A refusal (already running, no profile yet) shows on the Paths
        // status line the controller keeps.
        self.submit(OperatorCommand::ReloadQuesterPaths, Targets::Application);
    }

    pub(crate) fn poll_quester_paths_reload(&mut self) {
        let selected = self.selected_game_data();
        self.advance_quester_paths_reload(selected);
    }

    pub(crate) fn advance_quester_paths_reload(
        &mut self,
        selected: Option<Arc<api::game_data::SelectedGameData>>,
    ) {
        self.core
            .advance_quester_paths(&mut self.quester_paths, selected);
    }

    /// The focused slot's script lifecycle state; `Idle` when nothing is
    /// focused or the slot has no script. The script section's disable
    /// rules key off this.
    pub fn focused_script_state(&self) -> ScriptRunState {
        let Some(name) = self.focused_name() else {
            return ScriptRunState::Idle;
        };
        self.core.script_state(&name)
    }

    /// Whether the focused script is Idle with the named self-stop reason.
    pub fn script_self_stop_observed(&self, needle: &str) -> bool {
        let Some(name) = self.focused_name() else {
            return false;
        };
        self.core.live_script_self_stop_observed(&name, needle)
    }

    /// The focused slot's script `last_error`; `None` when the slot has no
    /// script error (or nothing is focused).
    pub fn focused_script_last_error(&self) -> Option<String> {
        let name = self.focused_name()?;
        self.core.script_last_error(&name)
    }

    #[cfg(feature = "js")]
    /// Merged operator bag for the Browse-selected JS card (schema defaults,
    /// persisted overrides, optional scenario inject).
    pub fn merged_settings_bag(
        &self,
        source: CardSource,
        name: &str,
        schema: SchemaRef<'_>,
    ) -> serde_json::Map<String, serde_json::Value> {
        self.scripts.legacy_bag(source, name, schema)
    }

    /// Inject overrides merged last when the selected script Starts (live gold).
    pub fn set_script_settings_inject(
        &mut self,
        inject: Option<serde_json::Map<String, serde_json::Value>>,
    ) {
        self.scripts.set_inject(inject);
    }

    #[cfg(feature = "js")]
    /// Schema defaults + overrides + inject. Empty schema still keeps
    /// inject keys (Thiever `target: Guard`). `None` only when the merged
    /// bag is empty.
    fn pending_settings_bag(
        &self,
        source: CardSource,
        name: &str,
        schema: SchemaRef<'_>,
    ) -> Option<serde_json::Map<String, serde_json::Value>> {
        let merged = self.merged_settings_bag(source, name, schema);
        if merged.is_empty() {
            None
        } else {
            Some(merged)
        }
    }

    #[cfg(feature = "js")]
    /// [`Self::pending_settings_bag`] for a loaded card by lookup.
    fn pending_card_settings_bag(
        &self,
        source: CardSource,
        lookup: &str,
    ) -> Option<serde_json::Map<String, serde_json::Value>> {
        let card = self.scripts.card(source, lookup)?;
        self.pending_settings_bag(source, lookup, card.schema())
    }

    /// Start the Browse-selected script (compiled or loaded JS) on the
    /// focused slot. The rs2b0t rule is enforced here too: while the slot's
    /// script is active the call is refused (the Start button is disabled,
    /// so this is the no-call backstop). Errors set [`Session::error`].
    pub fn script_start_selected(&mut self) {
        let Some(name) = self.focused_name() else {
            self.refuse_command(
                OperatorCommand::StartScript,
                &[],
                Some(ErrorNeed::FocusedBot),
                "script: no focused profile",
            );
            return;
        };
        match self.script_start_focused(&name) {
            Ok(()) => self.error = None,
            Err(e) => self.error = Some(format!("script: {e}")),
        }
    }

    /// Request that the focused Quester skip its active queue row. The core
    /// owns acceptance, refusal logging and the one-command report.
    pub fn script_skip_quest(&mut self) {
        let Some(name) = self.focused_name() else {
            self.refuse_command(
                OperatorCommand::SkipQuest,
                &[],
                None,
                "script: no focused profile",
            );
            return;
        };
        let Some(submitted) = self.submit(OperatorCommand::SkipQuest, Targets::Profile(&name))
        else {
            return;
        };
        self.error = self
            .core
            .command_failure(submitted.request())
            .map(str::to_owned);
    }

    /// Pause the focused slot's script, or Resume when it is Paused (the
    /// button label follows [`script_pause_enabled`]). The button is enabled
    /// only for a state `script_pause_enabled` accepts, and
    /// [`Self::focused_script_state`] is `Idle` with no focused bot or no play
    /// session, so a click always has both and neither early exit is
    /// reachable by a click.
    pub fn script_toggle_pause(&mut self) {
        let Some(name) = self.focused_name() else {
            return;
        };
        if self.core.has_play() {
            let row = if self.core.script_paused(&name) {
                OperatorCommand::ResumeScript
            } else {
                OperatorCommand::PauseScript
            };
            self.submit(row, Targets::Profile(&name));
        }
    }

    /// Stop the focused slot's script (teardown hook, instance dropped). The
    /// Stop button is enabled for an active state (which needs a focused bot
    /// and a play session) or for a Start still queued for the selected bot,
    /// so a click always has a focused bot; without a play session the queued
    /// Start is still cancelled.
    pub fn script_stop(&mut self) {
        let Some(name) = self.focused_name() else {
            return;
        };
        self.scripts.stage_cancel_queued("cancelled");
        self.submit(OperatorCommand::CancelQueuedStart, Targets::Profile(&name));
        self.apply_script_notice();
        if self.core.has_play() {
            self.submit(OperatorCommand::StopScript, Targets::Profile(&name));
        }
    }

    /// The `index`-th button of the paint `frame` drawn for the focused
    /// slot. The core decides what it does: a script-local one-shot click,
    /// or a host Pause/Resume/Stop control bound to the profile and run the
    /// frame was read for (a control shown for another profile or an ended
    /// run is refused, not retargeted). The paint is drawn only for the
    /// focused bot, a button reports a hit only while it is enabled (the check
    /// `PaintView::activate` repeats on the same frame), and the index comes
    /// from that frame's own button list, so neither early return is
    /// reachable by a click.
    pub fn script_paint_activate(&mut self, frame: &PaintFrame, index: usize) {
        let Some(name) = self.focused_name() else {
            return;
        };
        let Some(input) = frame.view().activate(index) else {
            return;
        };
        self.scripts.stage_paint_input(input);
        self.submit(OperatorCommand::ScriptPaintInput, Targets::Profile(&name));
    }

    /// Persistent strip/rail/tabs selection on the focused slot. The paint is
    /// drawn only for the focused bot, so a click always has one.
    pub fn script_paint_select(&mut self, key: &str, select_name: &str, generation: u64) {
        let Some(name) = self.focused_name() else {
            return;
        };
        self.scripts
            .stage_paint_input(frontend_core::scripts::PaintInput::select(
                key.to_owned(),
                select_name.to_owned(),
                generation,
            ));
        self.submit(OperatorCommand::ScriptPaintInput, Targets::Profile(&name));
    }

    /// Overlay generation for the path overlay's rising-edge refresh.
    pub fn route_gen(&self) -> u64 {
        self.route_gen
    }

    /// The status-row walk cell: `"—"` when nothing is queued, else the
    /// queued dest as `"x z level"`.
    pub fn walk_status_text(&self) -> String {
        match self.map_desk.walk_dest() {
            Some(d) => format!("{} {} {}", d.x, d.z, d.level),
            None => "—".into(),
        }
    }
}

/// `(username, password)` pairs for live `null_raster` (`test`/`test2`).
#[cfg(test)]
fn null_raster_live_entries() -> Vec<(String, String)> {
    frontend_core::live::null_raster_entries()
}

/// `(username, password)` pairs for live stress walls (`s00`…`s{n-1}`).
#[cfg(test)]
fn stress_live_entries(n: usize) -> Vec<(String, String)> {
    frontend_core::live::stress_entries(n)
}

/// Create a throwaway encrypted vault for a headed live boot.
#[cfg(test)]
fn temp_live_vault_from<S: AsRef<str>>(
    entries: &[(S, S)],
    uid_base: i32,
    vault_pass: &str,
    auto_login: bool,
) -> PathBuf {
    let owned = entries
        .iter()
        .map(|(username, password)| (username.as_ref().to_owned(), password.as_ref().to_owned()))
        .collect::<Vec<_>>();
    frontend_core::live::panel_live_vault(&owned, uid_base, vault_pass, auto_login)
}

/// 50-head watches are a release RAM/GPU check. Debug cargo run looks
/// frozen and spikes RSS; do not FAIL unit tests that call the shared
/// helper at N=2/3.
fn warn_stress50_debug() {
    if cfg!(debug_assertions) {
        eprintln!(
            "panel-play: stress50 is a release RAM watch — run with cargo run --release -p panel --bin panel-play -- --live stress50"
        );
    }
}

/// Detach the picker's nav world only after the play's slot threads are
/// joined, so no live observe can read a cleared `pack()`.
impl Drop for Session {
    fn drop(&mut self) {
        self.release_walk_map();
        // Window close is Quit: every slot shuts down and the play closes
        // (joining its workers) before the panel's other fields drop. With
        // no slot left there is nothing to retire (`NoTarget`).
        let (core, mut surface) = self.core_and_surface();
        let _ = core.submit(OperatorCommand::Quit, Targets::Everyone, &mut surface);
        crate::picker::set_pack(None);
    }
}

mod chooser;
mod windows;
pub use frontend_core::{FormLeave, LeavePrompt};
pub use windows::PanelWindow;

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "silent_exit_tests.rs"]
mod silent_exit_tests;

#[cfg(all(test, feature = "js"))]
mod fleet_hold_tests {
    use super::*;
    use host_play::external_loader::{
        default_frozen_source, ExternalWatch, ExternalWatchStatus, Operation, BONES_COUNT,
        FROZEN_SHA256,
    };

    #[test]
    fn fleet_copy_hold_keeps_external_loader_start_pending_without_failure() {
        let iso =
            frontend_core::test_support::IsolatedStores::enter("panel-external-loader-copy-hold");
        let path = iso.dir().join("BoneBurier.ts");
        std::fs::copy(default_frozen_source(), &path).unwrap();
        let mut session = Session::with_instance(frontend_core::launch::InstancePermit::skip());
        session.persist_ui = false;
        session.draw.set_persist(false);
        session.scripts.use_library_at(
            iso.dir().join("js-scripts.json"),
            iso.dir().join("js-cache"),
        );
        let mut play = frontend_core::test_support::empty_play(&host_play::PlayOptions {
            host: "127.0.0.1".into(),
            transport: client::Transport::Tcp,
            port: 43594,
            cache_dir: "/tmp".into(),
            lowmem: true,
            mainland: false,
        });
        play.attach_arm("alice", host_play::SlotArm::new(1, false));
        session.core.set_play(Some(play));
        session.core.fleet_mut().add("alice");
        session.set_focus_for_test("alice");
        session.load_js(&path);
        assert_eq!(session.error, None);
        let card = session
            .scripts
            .card(CardSource::File, &path.display().to_string())
            .unwrap();
        let watch = ExternalWatch::default();
        watch.configure("alice", path.clone(), FROZEN_SHA256.into());
        watch.note_scene(true, 2);
        watch.note_inventory(Instant::now(), "alice", BONES_COUNT, 0);
        watch.note_prereq_passed();
        watch.note_load(
            1,
            card.name(),
            card.path(),
            &card.identity_key(),
            card.sha256(),
            true,
            false,
        );
        assert_eq!(watch.dispatch_operation(), Some(Operation::Start));
        session.install_external_core_watch(Some(watch.clone()));
        let hold = session
            .core
            .play_for_test()
            .unwrap()
            .hold_script_starts("waiting for settings copy confirmation".into());
        for _ in 0..3 {
            session.pump_external_loader();
            assert_eq!(watch.failure(), None);
            assert_eq!(watch.status(), ExternalWatchStatus::Ready);
            assert_eq!(watch.dispatch_operation(), Some(Operation::Start));
            assert_eq!(session.focused_script_state(), ScriptRunState::Idle);
        }
        drop(hold);
        session.pump_external_loader();
        assert_eq!(session.error, None);
        assert_eq!(watch.failure(), None);
        assert_eq!(watch.dispatch_operation(), Some(Operation::ObserveBurials));
        let deadline = Instant::now() + Duration::from_secs(60);
        while session.focused_script_state() != ScriptRunState::Running {
            assert!(
                Instant::now() < deadline,
                "external loader Start did not resume"
            );
            session.core.poll_host();
            session.poll_scripts();
            std::thread::yield_now();
        }
        assert_eq!(watch.failure(), None);
    }
}
