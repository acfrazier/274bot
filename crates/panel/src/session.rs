//! Panel session: owns the unlocked vault, the running slot map, the shared
//! `Focus`, and per-slot frame/input channels. The panel frame reads
//! `Session`; slot threads stay in `host_play` (spawned via `run_with_io`
//! with per-profile `FrameBuf` mailbox/`SlotInput`, keeping the login FIFO
//! and the mainland hop).
//!
//! Flat slot model (M2 Task 2b): every wall member is its own full `Client`
//! on its own slot thread — there is no channel head and no lean baton.
//! Clicking a member is [`Session::select`], which is pure `focus` bookkeeping:
//! the Game pane samples that slot's `FrameBuf`. The single-client boot still
//! holds: unlock spawns **one** Client (the focused profile); MultiBox spawns
//! the rest.

use std::collections::{HashMap, HashSet};
use std::env;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use api::snapshot::{GameSnapshot, WorldTile};
use client::client::Client;
use client::render::nav_debug::{
    NavDebugCell, NavDebugColors, NavDebugHull, NavDebugPaint, FACE_E, FACE_N, FACE_S, FACE_W,
};
use client::sound::output::AudioOut;
use host::{map_image_to_applet, FrameBuf, InputEv, SlotInput};
use host_play::audio::{AudioChange, AudioGate};
use host_play::{
    open_vault, run_with_io, scatter_tile_for, Play, PlayOptions, SlotArm, SlotStatus,
};
use nav::paint::{
    collision_at, hull_targets, remaining_path_tiles, remaining_trail, select_draw_indices,
    trail_tones, TrailTone,
};
use nav::router::{find_with, FindOptions, Route};
use nav::tile::Tile;
use nav::traveller::{TravelOptions, Traveller};
use nav::world::NavWorld;
use vault::{Profile, Vault};

use crate::focus::{draw_for_slot, full_rate_for};
use crate::nav_settings::{effective, parse_html_color, NavSettings};
use crate::wall::Wall;

/// Scatter / mainland hop only on a cold world, not after a `lostCon`
/// reconnect (that would tele the re-handshaked slot on every DC).
fn seed_on_first_world(last_login_reconnect: Option<bool>) -> bool {
    last_login_reconnect != Some(true)
}

/// World host for a new session: `BOT_TARGET=live` points at rs2b2t.
fn default_play_host() -> String {
    host_play::default_world_host()
}

/// Loopback hosts get the debug heading / WalkTo Teleport. Public
/// `w1.rs2b2t.com` and LAN IPs do not.
pub fn is_local_engine(host: &str) -> bool {
    matches!(
        host.trim()
            .trim_end_matches('.')
            .to_ascii_lowercase()
            .as_str(),
        "127.0.0.1" | "localhost" | "::1"
    )
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
pub fn debug_main_buttons(show_tutskip: bool) -> Vec<&'static str> {
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
    &[
        "setstat attack 99",
        "setstat defence 99",
        "setstat strength 99",
        "setstat hitpoints 99",
        "setstat ranged 99",
        "setstat prayer 99",
        "setstat magic 99",
        "setstat cooking 99",
        "setstat woodcutting 99",
        "setstat fletching 99",
        "setstat fishing 99",
        "setstat firemaking 99",
        "setstat crafting 99",
        "setstat smithing 99",
        "setstat mining 99",
        "setstat herblore 99",
        "setstat agility 99",
        "setstat thieving 99",
        "setstat runecraft 99",
    ]
}

/// Engine `::tele` body for a WalkTo tile.
pub fn walkto_tele_cmd(tile: Tile) -> String {
    api::interact::tele_args(tile.level, tile.x, tile.z)
}

/// Cooldown between cpal open retries after a device failure: a machine
/// without an audio device must not re-open (and re-log) every 20 ms frame.
const AUDIO_OPEN_RETRY: Duration = Duration::from_secs(5);

const DEFAULT_PORT: u16 = 43594;

/// Vault path used by panel-play (`~/.274bot/vault`, the same file host-play
/// uses).
pub fn default_vault_path() -> PathBuf {
    match env::var("HOME") {
        Ok(home) => PathBuf::from(format!("{home}/.274bot/vault")),
        Err(_) => PathBuf::from(".274bot/vault"),
    }
}

fn default_cache_dir() -> String {
    client::cache_dir().display().to_string()
}

/// Panel-side per-slot IO: the frame mailbox the slot stores each rendered
/// `FrameOutput` into while its renderer is on (the panel `take`s it and
/// packs the `PixMap` or reads the `Texture` back at the consume site),
/// and the input channel it drains only while capture is on.
pub struct SlotIo {
    pub input: Arc<SlotInput>,
    pub pixels: Arc<FrameBuf>,
}

/// Combo highlight: `None` when nothing is focused so the widget cannot
/// display index 0 as selected.
pub fn combo_index(focused: Option<&str>, names: &[String]) -> Option<usize> {
    focused.and_then(|n| names.iter().position(|x| x == n))
}

/// Click-through helper: maps a click inside the Game Image (local coords,
/// Image widget size) to applet coords and enqueues `InputEv::Down`. No-op
/// when the capture channel has been dropped (capture off) or the point is
/// outside the Image.
pub fn maybe_send_click(tx: &Option<Sender<InputEv>>, lx: f32, ly: f32, w: f32, h: f32) {
    let Some(tx) = tx else {
        return;
    };
    let Some((x, y)) = map_image_to_applet(lx, ly, w, h) else {
        return;
    };
    let _ = tx.send(InputEv::Down { button: 1, x, y });
}

/// Stream one hovered capture frame: `Move` first, then `Down` (left=1,
/// right=2), then `Up`, then keys. No-op when `tx` is `None` (capture off).
// All capture state arrives flattened from the applet; a param struct would
// only shuffle names across the one call site.
#[allow(clippy::too_many_arguments)]
pub fn stream_capture(
    tx: &Option<Sender<InputEv>>,
    lx: f32,
    ly: f32,
    w: f32,
    h: f32,
    left_down: bool,
    right_down: bool,
    left_up: bool,
    right_up: bool,
    keys: &[(bool, i32)],
) {
    let Some(tx) = tx else {
        return;
    };
    if let Some((x, y)) = map_image_to_applet(lx, ly, w, h) {
        let _ = tx.send(InputEv::Move { x, y });
        if left_down {
            let _ = tx.send(InputEv::Down { button: 1, x, y });
        }
        if right_down {
            let _ = tx.send(InputEv::Down { button: 2, x, y });
        }
    }
    if left_up || right_up {
        let _ = tx.send(InputEv::Up);
    }
    for &(down, ch) in keys {
        let _ = tx.send(InputEv::Key { down, ch });
    }
}

/// rs2b0t disable rule: a script is active while it holds the slot, so
/// Start and Browse (and Load) are disabled for those states.
pub fn script_active(state: script::RunState) -> bool {
    matches!(
        state,
        script::RunState::Running | script::RunState::Paused | script::RunState::Stopping
    )
}

/// Pause/Resume enable rule: enabled only while Running (Pause) or Paused
/// (Resume); the button label switches to "Resume" when paused.
pub fn script_pause_enabled(state: script::RunState) -> bool {
    matches!(state, script::RunState::Running | script::RunState::Paused)
}

/// Stop enable rule: enabled while active, but not while already Stopping.
pub fn script_stop_enabled(state: script::RunState) -> bool {
    script_active(state) && state != script::RunState::Stopping
}

/// The script status-row text for a lifecycle state.
pub fn script_status_text(state: script::RunState) -> &'static str {
    match state {
        script::RunState::Idle => "idle",
        script::RunState::Running => "running",
        script::RunState::Paused => "paused",
        script::RunState::Stopping => "stopping",
        script::RunState::Error => "error",
    }
}

/// The loaded scene's tile count per side (the client's `BUILD_AREA_SIZE`):
/// the collision paint only covers the region the client has built,
/// `[base_x, base_x+104) × [base_z, base_z+104)` in world tiles.
const SCENE_TILES: i32 = 104;

/// Transport hops up to this many remaining tiles ahead get hull strokes;
/// the client only projects the locs inside its loaded scene anyway.
const HULL_WINDOW: usize = 48;

/// Per-frame nav-paint mirror: the slot threads read it each observe to
/// publish the focused drawing slot's scene paint.
/// [`Session::pump_status`] re-copies it from `Session::ui.nav` +
/// `nav_live_force_layers` every UI frame, so modal edits and live-overlay
/// flips land within a frame.
#[derive(Clone, Default)]
struct NavPublishCfg {
    settings: NavSettings,
    live_force_layers: bool,
}

/// Publish the nav-debug scene paint for the focused drawing slot each
/// observe. `drawing` is the gate: only the focused slot with its renderer
/// on publishes; unfocused / skip-paint / renderer-off slots store `None`
/// so a stale paint never lingers. `world` is the baked pack, `route` the
/// armed walk route, `here` the player's observed world tile, `trail_world`
/// the local player's last `tryMove` route buffer (world tiles), `run_on`
/// the local player's run state (two-tone trail), and `click` the
/// traveller's current walk aim.
///
/// World → scene: `x - map_build_base_x`, `z - map_build_base_z`.
/// Collision covers every tile of the loaded [`SCENE_TILES`]² region the
/// `collision_fill` / `nsew_labels` toggles warrant; the path is the
/// remaining route subsampled to the 3D draw budget (the pack map keeps
/// the full path).
// The brief fixes this signature; a param struct would only shuffle names
// across the one call site.
#[allow(clippy::too_many_arguments)]
fn publish_nav_debug(
    client: &mut Client,
    world: &NavWorld,
    route: Option<&Route>,
    here: Option<WorldTile>,
    trail_world: &[WorldTile],
    run_on: bool,
    click: Option<WorldTile>,
    settings: &NavSettings,
    drawing: bool,
) {
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
        for lz in 0..SCENE_TILES {
            for lx in 0..SCENE_TILES {
                let x = base_x + lx;
                let z = base_z + lz;
                if x < ox || z < oz || x - ox >= ow || z - oz >= oh {
                    continue;
                }
                let fb = collision_at(&world.collision, WorldTile { x, z, level });
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
                if (settings.collision_fill && fb.blocked) || (settings.nsew_labels && bits != 0) {
                    paint.collision.push(NavDebugCell {
                        lx,
                        lz,
                        bits,
                        // The client fills only blocked ground; a face-only
                        // cell keeps its NSEW letters without a fill quad.
                        blocked: fb.blocked,
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
            let tiles = remaining_path_tiles(route, here);
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
            paint.hulls = hull_targets(route, here, HULL_WINDOW)
                .into_iter()
                .map(|h| NavDebugHull {
                    loc_id: h.loc_id,
                    scene_x: h.at.x - base_x,
                    scene_z: h.at.z - base_z,
                })
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

pub struct Session {
    /// Shared focus policy; slot threads read it every frame (observe) to
    /// apply `client.set_draw(draw_for_slot(&focus, name))`, so only the
    /// focused slot rasters.
    pub focus: Arc<Mutex<crate::focus::Focus>>,
    pub vault: Option<Vault>,
    /// Last vault/connection error shown in the banner.
    pub error: Option<String>,
    /// Running slot threads and their shared statuses (created at unlock).
    pub play: Option<Play>,
    /// Per-username slot IO.
    pub slots: HashMap<String, SlotIo>,
    /// The focused slot's live capture sender; `None` while capture is off,
    /// so UI send paths no-op.
    pub capture_tx: Option<Sender<InputEv>>,
    /// BOT_MAINLAND=1 / host-play --mainland; not a panel checkbox.
    pub mainland: Arc<AtomicBool>,
    /// Per-username panel log lines (status transitions), each capped at
    /// [`LOG_CAP`]. Vault / no-username lines use [`PROCESS`].
    pub log_by: Arc<Mutex<HashMap<String, Vec<String>>>>,
    /// Vault passphrase scratch buffer for the in-panel unlock prompt.
    pub pass_scratch: String,
    /// Last status poll (delta source for the log).
    pub statuses: Vec<SlotStatus>,
    /// Picker edit scratch (username/password). Empty on the strip.
    pub cred_user: String,
    pub cred_pass: String,
    /// Profile picker edit: `None` not editing, `Some("")` new profile,
    /// `Some(name)` editing that vault row.
    pub chooser_edit: Option<String>,
    /// Per-username walk arms; the focused slot's arm carries the armed
    /// whole-world route (polled from `start_play` `per_frame` via
    /// [`Traveller::follow`]).
    pub travellers: Arc<Mutex<HashMap<String, Arc<Mutex<WalkArm>>>>>,
    /// The tile the user last picked for WalkTo; `None` until armed. Read
    /// by [`Session::walk_status_text`] so the status row stays honest even
    /// when no route could be found.
    pub walk_dest: Option<Tile>,
    /// Slot threads set this when a traveller returns Arrived/Budget so
    /// [`Session::pump_status`] can clear [`Session::walk_dest`].
    walk_clear: Arc<AtomicBool>,
    /// Last `(gens.player, here)` ticked per username; skip until either
    /// changes so we do not re-send walk every 20 ms frame.
    tick_latch: Arc<Mutex<HashMap<String, (u64, Tile)>>>,
    /// WalkTo picker open flag; the picker window lands in Task 10.
    pub walkto_open: bool,
    /// Tile highlighted in the WalkTo picker; armed only on confirm.
    pub picker_sel: Option<Tile>,
    /// Nav config window open flag (non-modal, same as General config).
    pub nav_settings_open: bool,
    /// Non-modal settings window (renderer / capture / mem).
    pub global_settings_open: bool,
    /// Usernames we already sent `getvar tutorial` for this session.
    tutorial_getvar_sent: HashSet<String>,
    /// Forgotten-password confirm: delete the vault file (locked only).
    pub vault_reset_open: bool,
    pub vault_reset_understood: bool,
    /// Chooser ✕ waiting on the same confirm popup; `None` not pending.
    pub pending_profile_delete: Option<String>,
    pub delete_understood: bool,
    /// Live-harness overlay: force the paint-layer toggles on for this
    /// session without writing prefs (`NavSettings::effective`).
    pub nav_live_force_layers: bool,
    /// Per-frame nav-paint mirror the slot threads publish from each
    /// observe (see [`publish_nav_debug`]); `pump_status` re-copies it
    /// from `ui.nav` + `nav_live_force_layers` every UI frame.
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
    options: PlayOptions,
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
    /// section button.
    pub script_sel: Option<script::ScriptSel>,
    /// Browse picker open flag (the modal window in `app.rs`).
    pub script_browse_open: bool,
    /// Load modal open flag (the path modal in `app.rs`).
    pub script_load_open: bool,
    /// The out-of-tree JS library (`~/.274bot/js-scripts.json`). Loaded
    /// cards appear in Browse and Start spawns their isolate.
    pub js: script::JsLibrary,
    /// The Load modal's path scratch buffer.
    pub load_scratch: String,
    /// Shared `--live script_*` harness runner (Task 6): the slot thread
    /// ticks it from the per-frame hook (sends go through the slot's own
    /// `Client`), the UI frame reads its status/evidence. `None` when no
    /// scenario is live.
    pub scenario: Arc<Mutex<Option<scenario::ScenarioRunner>>>,
    /// Focused-slot speaker gate: at most one cpal speaker, owned by the
    /// focused slot while its Music/SFX toggle is on. `lowmem` (toggle
    /// off) never opens cpal; slot threads reconcile on their frame loop.
    audio: Arc<AudioGate<AudioOut>>,
    /// Whether focus/multibox writes land on disk prefs. `true` in
    /// `Session::new`; every `live_prepare_*` flips it off so an ephemeral
    /// live boot never touches the operator's `last_focus`.
    pub persist_ui: bool,
}

/// Keep each per-name panel log bounded.
const LOG_CAP: usize = 200;

/// Per-username walk arm: the whole-world [`Traveller`] plus the [`Route`]
/// it is following. [`Session::arm_walk_on`] stores the route (found over
/// the shared `NavWorld`); the slot hook polls [`Traveller::follow`] with a
/// clone of it one step per player-info tick. `route` being set is the
/// "armed" gate the status row and the overlay read; any terminal outcome
/// clears it (arrival and stall alike).
#[derive(Default)]
pub struct WalkArm {
    pub traveller: Traveller,
    pub route: Option<Route>,
}

impl WalkArm {
    /// The armed route's dest as a panel tile, `None` when idle.
    fn queued_tile(&self) -> Option<Tile> {
        self.route.as_ref().map(|r| Tile {
            x: r.dest.x,
            z: r.dest.z,
            level: r.dest.level,
        })
    }
}

/// Log bucket for vault errors and lines with no username.
pub const PROCESS: &str = "*";

/// Append `line` under `name`, dropping from the front past [`LOG_CAP`].
fn push_log(map: &mut HashMap<String, Vec<String>>, name: &str, line: String) {
    if host::debug_enabled() {
        eprintln!("[panel] {name}: {line}");
    }
    let vec = map.entry(name.to_string()).or_default();
    vec.push(line);
    while vec.len() > LOG_CAP {
        vec.remove(0);
    }
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    /// Empty session: no vault, no slots, default `PlayOptions` (same engine
    /// defaults as the host-play CLI). Unlock via [`Session::unlock`].
    pub fn new() -> Self {
        Self {
            focus: Arc::new(Mutex::new(crate::focus::Focus {
                focused: None,
                renderer: true,
                game_pane_open: true,
                capture: false,
                only_render_selected: true,
                sidecar_50: false,
                live_full_rate: false,
                focused_50: true,
                wall_open: false,
                wall: Vec::new(),
                renderer_by: HashMap::new(),
            })),
            vault: None,
            error: None,
            play: None,
            slots: HashMap::new(),
            capture_tx: None,
            mainland: Arc::new(AtomicBool::new(
                env::var("BOT_MAINLAND").as_deref() == Ok("1"),
            )),
            log_by: Arc::new(Mutex::new(HashMap::new())),
            pass_scratch: String::new(),
            statuses: Vec::new(),
            cred_user: String::new(),
            cred_pass: String::new(),
            chooser_edit: None,
            travellers: Arc::new(Mutex::new(HashMap::new())),
            walk_dest: None,
            walk_clear: Arc::new(AtomicBool::new(false)),
            tick_latch: Arc::new(Mutex::new(HashMap::new())),
            walkto_open: false,
            picker_sel: None,
            nav_settings_open: false,
            global_settings_open: false,
            tutorial_getvar_sent: HashSet::new(),
            vault_reset_open: false,
            vault_reset_understood: false,
            pending_profile_delete: None,
            delete_understood: false,
            nav_live_force_layers: false,
            nav_publish: Arc::new(Mutex::new(NavPublishCfg::default())),
            route_gen: 0,
            mainland_sent: Arc::new(Mutex::new(HashSet::new())),
            scatter: Arc::new(AtomicBool::new(false)),
            wall: Wall::default(),
            multibox: false,
            ui: crate::ui_state::load(),
            script_sel: None,
            script_browse_open: false,
            script_load_open: false,
            js: {
                let mut js = script::JsLibrary::new(script::default_js_store());
                let _ = js.restore(); // missing/broken store is not fatal here
                js
            },
            load_scratch: String::new(),
            scenario: Arc::new(Mutex::new(None)),
            audio: Arc::new(AudioGate::new()),
            persist_ui: true,
            options: PlayOptions {
                host: default_play_host(),
                port: DEFAULT_PORT,
                cache_dir: default_cache_dir(),
                lowmem: true,
                // Panel per_frame queues hop from Session.mainland (env);
                // spawn-time PlayOptions.mainland stays false.
                mainland: false,
            },
        }
    }

    pub fn play_options(&self) -> &PlayOptions {
        &self.options
    }

    /// Unlock (or first-run create) the default vault and start the play.
    pub fn unlock(&mut self, pass: &str) -> bool {
        self.unlock_at(&default_vault_path(), pass)
    }

    /// Unlock (or first-run create) the vault at `path` and start the play.
    /// Only the focused profile is spawned as a slot; other vault rows stay
    /// parked until selected (select keeps a slot once it has run).
    pub fn unlock_at(&mut self, path: &Path, pass: &str) -> bool {
        if self.start_vault(path, pass) {
            self.focus_first_profile();
            true
        } else {
            false
        }
    }

    /// Whether the default vault file exists (locked prompt: Unlock vs Create).
    pub fn default_vault_exists() -> bool {
        default_vault_path().is_file()
    }

    /// Delete `path` while locked. Refuses if a vault is open so a running
    /// session cannot clobber the file. Forgotten-password recovery.
    pub fn reset_vault_at(&mut self, path: &Path) -> bool {
        if self.vault.is_some() {
            self.error = Some("reset vault: unlock / close the session first".into());
            return false;
        }
        match Vault::reset_file(path) {
            Ok(()) => {
                self.error = None;
                self.vault_reset_open = false;
                self.vault_reset_understood = false;
                true
            }
            Err(e) => {
                self.error = Some(format!("reset vault: {e}"));
                false
            }
        }
    }

    pub fn reset_vault(&mut self) -> bool {
        self.reset_vault_at(&default_vault_path())
    }

    /// Open the vault and attach an empty [`Play`]. Does **not** spawn a
    /// slot — the boot spawns the focused profile after this; MultiBox
    /// spawns the wall members.
    fn start_vault(&mut self, path: &Path, pass: &str) -> bool {
        match open_vault(path, pass) {
            Ok(vault) => {
                self.error = None;
                self.start_play(vault);
                true
            }
            Err(e) => {
                let msg = e.to_string();
                push_log(
                    &mut self.log_by.lock().unwrap(),
                    PROCESS,
                    format!("vault: {msg}"),
                );
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
        let path = temp_live_vault(&[("test", "test"), ("test2", "test2")]);
        if !self.unlock_at(&path, "bot") {
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
        self.focus.lock().unwrap().only_render_selected = true;
        self.select("test");
        self.set_renderer(true);
        self.login_all();
        Ok(())
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
        let n = n.max(1);
        let names: Vec<(String, String)> = (0..n)
            .map(|i| {
                let name = format!("s{i:02}");
                (name.clone(), name)
            })
            .collect();
        let entries: Vec<(&str, &str)> = names
            .iter()
            .map(|(u, p)| (u.as_str(), p.as_str()))
            .collect();
        let path = temp_live_vault_from(&entries, 274_000_100);
        // Empty Play first: do not spawn last_focus before s00 focuses.
        if !self.start_vault(&path, "bot") {
            return Err(self
                .error
                .clone()
                .unwrap_or_else(|| "start_vault failed".into()));
        }
        // RAM watch: only s00 (Game pane) may grow a GPU head. Rail members
        // stay raster Off so a flipped only-render-selected cannot attach
        // 49 extra RenderWorlds (~1 GB of loc Model clones each). Full-rate
        // keeps Gpu on every member on purpose.
        if let Some(vault) = self.vault.as_mut() {
            for (i, (name, _)) in names.iter().enumerate() {
                if let Some(mut p) = vault.get(name).cloned() {
                    p.settings.lowmem = true;
                    p.settings.raster = if full_rate || i == 0 {
                        vault::RasterMode::Gpu
                    } else {
                        vault::RasterMode::Off
                    };
                    let _ = vault.upsert(p);
                }
            }
        }
        self.set_multibox(true);
        self.scatter.store(true, Ordering::Relaxed);
        self.wall.chooser_open = false;
        {
            let mut f = self.focus.lock().unwrap();
            f.only_render_selected = !full_rate;
        }
        // Spawn every member onto the wall without applying focus on each
        // load (that would demote/promote raster/mem and join `maininit`
        // on the UI thread — the window never presents). Raster/mem flips
        // are drop+reattach now, but focus apply still joins handshakes.
        // s00 is focused last so it is FIFO head.
        for (name, _) in &names {
            let _ = self.wall.load(name);
            self.ensure_slot(name, self.arm_for_profile(name));
        }
        self.sync_wall_focus();
        self.wall.chooser_open = false;
        self.select(&names[0].0);
        // NEVER assign sidecar_50 — it stays the operator knob. The live
        // overlay raises every *drawing* slot, Game included.
        self.set_live_full_rate(full_rate);
        self.sync_sidecar_cadence();
        self.login_all();
        Ok(())
    }

    /// Live `script_<name>` setup: temp vault with the scenario's seed
    /// profiles, mainland hop per the seed, single-client boot (the
    /// MultiBox wall for a fleet — more than one seed profile — so the
    /// sidecar rail pops out and every bot is visible), and the shared
    /// [`scenario::ScenarioRunner`] installed for the slot thread's
    /// per-frame hook. The UI frame reads the runner's status/evidence.
    pub fn live_prepare_script(&mut self, scenario: scenario::Scenario) -> Result<(), String> {
        // Ephemeral boot: never persist focus/last_focus from a live run.
        self.persist_ui = false;
        // Copy the view knobs before `scenario` moves into the runner.
        let view = scenario.settings.clone();
        let entries: Vec<(&str, &str)> = scenario
            .seed
            .profiles
            .iter()
            .map(|(u, p)| (*u, *p))
            .collect();
        let path = temp_live_vault(&entries);
        if !self.unlock_at(&path, "bot") {
            return Err(self
                .error
                .clone()
                .unwrap_or_else(|| "unlock_at failed".into()));
        }
        self.mainland
            .store(scenario.seed.mainland, Ordering::Relaxed);
        self.scatter.store(false, Ordering::Relaxed);
        // Fleet scenario (2+ seed profiles): open the MultiBox wall like
        // `live_prepare_null_raster`/`live_prepare_stress`, so the rail
        // pops out and every bot is visible.
        if scenario.seed.profiles.len() > 1 {
            self.set_multibox(true);
        }
        self.wall.chooser_open = false;
        let names: Vec<String> = scenario
            .seed
            .profiles
            .iter()
            .map(|(u, _)| u.to_string())
            .collect();
        for name in &names {
            self.load(name);
        }
        self.select(&names[0]);
        // Apply the scenario's view knobs now the wall is up; the slot
        // threads re-read the focus within a frame.
        self.set_renderer(view.renderer);
        self.focus.lock().unwrap().only_render_selected = view.only_render_selected;
        self.set_capture(view.capture);
        self.set_live_full_rate(view.full_rate);
        // A nav_debug scenario forces the paint-layer toggles on for the
        // run. The force lives on the session only — never ui_state::save'd
        // — so a live boot cannot clobber the operator's nav prefs.
        self.nav_live_force_layers = view.nav_debug;
        // NEVER assign sidecar_50 — it stays the operator knob.
        self.sync_sidecar_cadence();
        let mut runner = scenario::ScenarioRunner::new(scenario);
        if let Some(play) = &self.play {
            runner.set_obj_names(play.obj_names());
        }
        *self.scenario.lock().unwrap() = Some(runner);
        self.login_all();
        Ok(())
    }

    /// Empty `Play` (shared cache + FIFO + per-frame hook) then spawn the
    /// first focused profile only. Parked names are started from [`select`].
    fn start_play(&mut self, vault: Vault) {
        let focus = Arc::clone(&self.focus);
        let log_by = Arc::clone(&self.log_by);
        let mainland = Arc::clone(&self.mainland);
        let mainland_sent = Arc::clone(&self.mainland_sent);
        let scatter = Arc::clone(&self.scatter);
        let travellers = Arc::clone(&self.travellers);
        let tick_latch = Arc::clone(&self.tick_latch);
        let walk_clear = Arc::clone(&self.walk_clear);
        let scenario = Arc::clone(&self.scenario);
        let audio = Arc::clone(&self.audio);
        let nav_publish = Arc::clone(&self.nav_publish);
        // Last failed device-open `(slot, when)`; a machine without an
        // audio device must not re-open cpal (or re-log) every frame.
        let audio_fail: Arc<Mutex<Option<(String, Instant)>>> = Arc::new(Mutex::new(None));
        let options = self.options.clone();
        let play = run_with_io(
            &options,
            Vec::new(),
            |_| (None, None),
            move |c, name| {
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
                // stale paint cannot linger.
                let (nav_settings, live_force) = {
                    let cfg = nav_publish.lock().unwrap();
                    (cfg.settings.clone(), cfg.live_force_layers)
                };
                let layers = effective(&nav_settings, live_force);
                let drawing = focused.as_deref() == Some(name) && draw;
                let (route, click) = match travellers.lock().unwrap().get(name).cloned() {
                    Some(arm) => {
                        let arm = arm.lock().unwrap();
                        (arm.route.clone(), arm.traveller.current_aim())
                    }
                    None => (None, None),
                };
                match crate::picker::pack() {
                    Some(world) => {
                        let here = c.local_player.as_ref().map(|lp| WorldTile {
                            x: c.map_build_base_x + lp.route_x[0],
                            z: c.map_build_base_z + lp.route_z[0],
                            level: 0,
                        });
                        // Run orb (varp 173 / 274 overlay), not the run
                        // animation — the anim is only true while a run
                        // cycle plays.
                        let run_on = c.run_enabled();
                        // Full tryMove BFS (every scene tile, src→dest),
                        // not the entity walk buffer (capped at 9) or the
                        // MOVE waypoint list (capped at 25).
                        let base_x = c.map_build_base_x;
                        let base_z = c.map_build_base_z;
                        let trail_all: Vec<WorldTile> = c
                            .try_move_path
                            .iter()
                            .map(|&(sx, sz)| WorldTile {
                                x: base_x + sx,
                                z: base_z + sz,
                                level: 0,
                            })
                            .collect();
                        let trail_world = remaining_trail(&trail_all, here);
                        publish_nav_debug(
                            c,
                            &world,
                            route.as_ref(),
                            here,
                            &trail_world,
                            run_on,
                            click,
                            &layers,
                            drawing,
                        );
                    }
                    None => c.set_nav_debug_paint(None),
                }
                // Focused-slot speaker: at most one cpal speaker, fed by
                // this slot's Client audio state (midi/waves/fade), gated
                // on focus + the Music/SFX toggle — `lowmem` (toggle off)
                // never opens cpal. The gate reconciles every frame; the
                // open closure runs on this slot's thread.
                let change = audio.frame(name, focused.as_deref(), || {
                    let now = Instant::now();
                    if let Some((who, at)) = audio_fail.lock().unwrap().as_ref() {
                        if who == name && now.duration_since(*at) < AUDIO_OPEN_RETRY {
                            return None;
                        }
                    }
                    match AudioOut::try_open(c.midi.clone(), c.waves.clone(), c.fade.clone()) {
                        Ok(out) => {
                            *audio_fail.lock().unwrap() = None;
                            push_log(
                                &mut log_by.lock().unwrap(),
                                name,
                                format!("audio: speaker open ({} Hz)", out.sample_rate),
                            );
                            Some(out)
                        }
                        Err(e) => {
                            *audio_fail.lock().unwrap() = Some((name.to_string(), now));
                            push_log(&mut log_by.lock().unwrap(), name, format!("audio: {e}"));
                            None
                        }
                    }
                });
                if change == AudioChange::Closed {
                    push_log(
                        &mut log_by.lock().unwrap(),
                        name,
                        "audio: speaker closed".into(),
                    );
                }
                // Reconcile the client's actual `lowmem` mode to the
                // Music/SFX gate (toggle on = highmem): a lowmem spawn
                // skipped the sound load, so flipping the toggle
                // mid-session must re-run it live, not on the next
                // respawn. `set_lowmem` is idempotent — per-frame is cheap.
                c.set_lowmem(!audio.music_on(name));
                if c.ingame
                    && c.scene_state == 2
                    && seed_on_first_world(c.last_login_reconnect)
                    && mainland_sent.lock().unwrap().insert(name.to_string())
                {
                    if scatter.load(Ordering::Relaxed) {
                        let t = scatter_tile_for(c.login_uid);
                        api::interact::seed_at(c, t.level, t.x, t.z);
                        push_log(
                            &mut log_by.lock().unwrap(),
                            name,
                            format!("{name}: scatter seed {} {} {}", t.level, t.x, t.z),
                        );
                    } else if mainland.load(Ordering::Relaxed) {
                        api::interact::mainland_hop(c);
                        push_log(
                            &mut log_by.lock().unwrap(),
                            name,
                            format!("{name}: mainland hop queued"),
                        );
                    }
                }

                // Shared `--live script_*` runner: tick the scenario's
                // driven slot and its companion slots, before the
                // local-player gate (seeding must observe frames with no
                // player decode yet). The slot thread drives sends through
                // its own `Client`; the UI frame only reads the runner's
                // status/evidence.
                if let Some(runner) = scenario.lock().unwrap().as_mut() {
                    if runner.drives(name) {
                        runner.tick(c);
                    } else if let Some(index) = runner.companion_for(name) {
                        runner.companion_tick(index, c);
                    }
                }

                let (rx, rz) = match &c.local_player {
                    Some(lp) => (lp.route_x[0], lp.route_z[0]),
                    None => return,
                };
                let here = Tile {
                    x: c.map_build_base_x + rx,
                    z: c.map_build_base_z + rz,
                    level: 0,
                };
                let Some(arm) = travellers.lock().unwrap().get(name).cloned() else {
                    return;
                };
                {
                    let mut latch = tick_latch.lock().unwrap();
                    if latch.get(name) == Some(&(c.gens.player, here)) {
                        return;
                    }
                    latch.insert(name.to_string(), (c.gens.player, here));
                }
                let finished = {
                    let mut arm = arm.lock().unwrap();
                    let Some(route) = arm.route.clone() else {
                        return;
                    };
                    // The follow surface reads the canonical base + route-head
                    // tile from a snapshot rebuilt off the same client; the
                    // run is polled one step per player-info tick.
                    let mut snapshot = GameSnapshot::new();
                    snapshot.rebuild(c);
                    let mut options = TravelOptions {
                        // Exact arrival: the armed dest must be stood on
                        // before the route clears (the v1 traveller arrived
                        // the same way).
                        close_enough: 0,
                        ..TravelOptions::default()
                    };
                    let outcome = arm.traveller.follow(c, &snapshot, route, &mut options);
                    if outcome.is_some() {
                        arm.route = None;
                        true
                    } else {
                        false
                    }
                };
                if finished {
                    walk_clear.store(true, Ordering::Relaxed);
                }
            },
        );
        self.play = Some(play);
        crate::picker::set_pack(self.play.as_ref().and_then(|p| p.world()));
        self.statuses = self.play.as_ref().map(|p| p.statuses()).unwrap_or_default();
        self.vault = Some(vault);
    }

    /// After unlock/`spawn_all`: restore `last_focus` when it is still a
    /// vault/slot name; otherwise focus the first so the combo and renderer
    /// are not stuck on `None`.
    fn focus_first_profile(&mut self) {
        let names = self.profile_names();
        let last = crate::ui_state::load().last_focus;
        if let Some(name) = crate::ui_state::pick_focus(&names, last.as_deref()) {
            self.select(&name);
        }
    }

    /// Poll slot statuses and append log lines for transitions (slot up,
    /// login errors, ingame, scene changes). Call once per UI frame.
    pub fn pump_status(&mut self) {
        // Per-frame mirrors that must not lag a focus/renderer/wall change:
        // the sidecar-50 cadence latch, and the speaker teardown when the
        // owning slot is no longer running.
        self.sync_sidecar_cadence();
        self.sync_nav_publish();
        if let Some(owner) = self.audio.owner() {
            if !self.slots.contains_key(&owner) {
                self.audio.release(&owner);
            }
        }
        let Some(current) = self.play.as_ref().map(|p| p.statuses()) else {
            return;
        };
        self.ingest_tutorial_chat(&current);
        self.maybe_getvar_tutorial(&current);
        {
            let mut log_by = self.log_by.lock().unwrap();
            for s in &current {
                let name = s.username.as_str();
                let prev = self.statuses.iter().find(|p| p.username == s.username);
                match prev {
                    None => {
                        push_log(&mut log_by, name, format!("{name}: slot up"));
                        if let Some(e) = &s.error {
                            push_log(&mut log_by, name, format!("{name}: login {e}"));
                        }
                    }
                    Some(p) => {
                        if p.error.is_none() && s.error.is_some() {
                            push_log(
                                &mut log_by,
                                name,
                                format!("{name}: login {}", s.error.as_deref().unwrap_or_default()),
                            );
                        }
                        if !p.ingame && s.ingame {
                            push_log(&mut log_by, name, format!("{name}: ingame"));
                        }
                        if p.scene_state != s.scene_state {
                            push_log(
                                &mut log_by,
                                name,
                                format!("{name}: scene {}", s.scene_state),
                            );
                        }
                    }
                }
            }
        }
        self.statuses = current;
        self.sync_walk_status();
    }

    /// Copy each slot's walk-arm dest into `walk_*` (−1 if none) and
    /// clear [`Session::walk_dest`] after Arrived.
    fn sync_walk_status(&mut self) {
        for s in &mut self.statuses {
            let queued = self
                .travellers
                .lock()
                .unwrap()
                .get(&s.username)
                .and_then(|a| a.lock().unwrap().queued_tile());
            apply_queued_walk(s, queued);
        }
        if self.walk_clear.swap(false, Ordering::Relaxed) {
            let keep = self.focused_name().and_then(|n| {
                self.travellers
                    .lock()
                    .unwrap()
                    .get(&n)
                    .and_then(|a| a.lock().unwrap().queued_tile())
            });
            if keep.is_none() {
                self.walk_dest = None;
            }
        }
    }

    /// Snapshot of every slot's status (for the status section).
    pub fn statuses(&self) -> Vec<SlotStatus> {
        self.statuses.clone()
    }

    /// Vault usernames plus any running slot outside the vault.
    pub fn profile_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .vault
            .as_ref()
            .map(|v| v.profiles().map(|p| p.username.clone()).collect())
            .unwrap_or_default();
        if let Some(play) = &self.play {
            for s in play.statuses() {
                if !names.contains(&s.username) {
                    names.push(s.username);
                }
            }
        }
        names
    }

    pub fn focused_name(&self) -> Option<String> {
        self.focus.lock().unwrap().focused.clone()
    }

    /// Queue a `CLIENT_CHEAT` on the focused slot. No-op without play/focus.
    pub fn cheat_focused(&self, cmd: &str) {
        let Some(play) = self.play.as_ref() else {
            return;
        };
        let Some(name) = self.focused_name() else {
            return;
        };
        play.cheat(&name, cmd);
    }

    /// True when this session's world host is a local engine.
    pub fn debug_ui(&self) -> bool {
        is_local_engine(&self.options.host)
    }

    /// Cached TutSkip for the focused profile: `None` unknown, `Some(true)`
    /// skipped, `Some(false)` still in tutorial.
    pub fn focused_tutorial_skipped(&self) -> Option<bool> {
        let name = self.focused_name()?;
        self.vault
            .as_ref()
            .and_then(|v| v.get(&name))
            .and_then(|p| p.settings.tutorial_skipped)
    }

    /// Persist TutSkip on the focused vault profile (debugprefs).
    pub fn mark_tutorial_skipped(&mut self) {
        let Some(name) = self.focused_name() else {
            return;
        };
        self.cache_tutorial(&name, true);
    }

    fn cache_tutorial(&mut self, name: &str, skipped: bool) {
        let Some(vault) = self.vault.as_mut() else {
            return;
        };
        let Some(mut profile) = vault.get(name).cloned() else {
            return;
        };
        if profile.settings.tutorial_skipped == Some(skipped) {
            return;
        }
        profile.settings.tutorial_skipped = Some(skipped);
        let _ = vault.upsert(profile);
    }

    fn ingest_tutorial_chat(&mut self, statuses: &[SlotStatus]) {
        for s in statuses {
            if let Some((var, value)) = parse_getvar_line(&s.chat_head) {
                if var == "tutorial" {
                    self.cache_tutorial(&s.username, value >= 1000);
                }
            }
        }
    }

    fn maybe_getvar_tutorial(&mut self, statuses: &[SlotStatus]) {
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
            .iter()
            .any(|s| s.username == name && s.ingame && s.scene_state == 2);
        if !ready || !self.tutorial_getvar_sent.insert(name.clone()) {
            return;
        }
        if let Some(play) = self.play.as_ref() {
            play.cheat(&name, "getvar tutorial");
        }
    }

    /// Frames for the Game pane (the focused slot's mailbox). Every
    /// wall member owns its own `FrameBuf` in the flat model; fall back to
    /// the first spawned slot when nothing is focused.
    pub fn focused_pixels(&self) -> Option<Arc<FrameBuf>> {
        if let Some(slot) = self.focused_slot() {
            return Some(Arc::clone(&slot.pixels));
        }
        self.slots.values().next().map(|s| Arc::clone(&s.pixels))
    }

    /// Username of the focused slot (the sampled one, the old TV). Falls
    /// back to the first spawned slot when the focus has no slot yet.
    pub fn tv_name(&self) -> Option<String> {
        self.focused_name()
            .filter(|n| self.slots.contains_key(n))
            .or_else(|| self.slots.keys().next().cloned())
    }

    fn focused_slot(&self) -> Option<&SlotIo> {
        let name = self.focused_name()?;
        self.slots.get(&name)
    }

    /// Switch the focused profile. A parked vault name is spawned on first
    /// select (login FIFO); already-running slots stay up so the picker can
    /// change focus. Capture follows the new focus when the single capture
    /// toggle is on (never two keyboards). The picker edit fields follow.
    /// New slots inherit the vault profile's auto-login (and logout latch).
    ///
    /// Flat model: clicking a member is pure focus — the Game pane samples
    /// that slot's `FrameBuf`. No socket is swapped (the channel-head baton
    /// is gone); every slot keeps running.
    pub fn select(&mut self, name: &str) {
        let arm = self.arm_for_profile(name);
        self.ensure_slot(name, arm);
        self.apply_focus(name);
    }

    fn apply_focus(&mut self, name: &str) {
        if self.persist_ui {
            // Reload so an injected/disk collapsed map is not clobbered.
            let mut ui = crate::ui_state::load();
            ui.last_focus = Some(name.to_string());
            crate::ui_state::save(&ui);
            self.ui = ui;
        }
        let mut focus = self.focus.lock().unwrap();
        if focus.focused.as_deref() == Some(name) {
            return;
        }
        let old = focus.focused.clone();
        focus.focused = Some(name.to_string());
        let capture = focus.capture;
        drop(focus);
        // Mirror onto the play: which slot the panel samples (host-play
        // keeps it as pure bookkeeping — no socket adopt/park).
        if let Some(play) = self.play.as_mut() {
            play.focus(name);
            // The draw state of both the outgoing and incoming slot can
            // change (draw_for_slot follows the focus); kick both so a
            // parked thread re-reads it within a frame, not at the next
            // game-tick park timeout.
            if let Some(old) = old.as_deref() {
                play.wake(old);
            }
        }
        // The overlay follows the focused traveller: switching focus may
        // show a different (or no) route, so force a rebuild.
        self.route_gen += 1;
        if capture {
            if let Some(old) = old.clone() {
                if let Some(slot) = self.slots.get(&old) {
                    slot.input.set_enabled(false);
                }
            }
            self.capture_on(name);
        } else {
            self.capture_tx = None;
        }
        // Credentials fields follow the newly focused profile; the General
        // config mirrors the profile's raster/mem so the pane shows what the
        // slot actually runs (display only — no write-back, no re-role).
        if let Some(vault) = &self.vault {
            if let Some(p) = vault.get(name) {
                self.cred_user = p.username.clone();
                self.cred_pass = p.password.clone();
                self.ui.raster = p.settings.raster;
                self.ui.lowmem = p.settings.lowmem;
            }
        }
    }

    /// Renderer checkbox. Writes both the focused checkbox (`Focus.renderer`)
    /// and `renderer_by[focused]` so per-slot draw policy stays in sync.
    /// Slot threads apply `set_draw` from the focus in their per-frame hook.
    pub fn set_renderer(&mut self, on: bool) {
        let mut focus = self.focus.lock().unwrap();
        focus.renderer = on;
        let name = focus.focused.clone();
        if let Some(name) = &name {
            focus.renderer_by.insert(name.clone(), on);
        }
        drop(focus);
        // The focused slot's draw state flips with the checkbox; kick it so
        // a parked thread applies `set_draw` within a frame.
        if let Some(name) = name {
            if let Some(play) = self.play.as_ref() {
                play.wake(&name);
            }
        }
    }

    /// Sidecar-50 pref: wall/grid members render at 50 fps instead of the
    /// 1 fps watch cadence (a render-cadence knob, not the idle park).
    /// `pump_status` mirrors it onto each slot's frame-loop latch within a
    /// frame; kick the parked members so the raise is not held up by the
    /// 1 s watch bound.
    pub fn set_sidecar_50(&mut self, on: bool) {
        self.focus.lock().unwrap().sidecar_50 = on;
        self.wake_all_slots();
    }

    /// Game-pane 50 fps for whoever is focused. Does not follow that
    /// client onto the rail.
    pub fn set_focused_50(&mut self, on: bool) {
        self.focus.lock().unwrap().focused_50 = on;
        self.wake_all_slots();
    }

    /// Ephemeral live overlay: every drawing slot at 50 fps, focused
    /// included (the scenario's `full_rate` knob). Not sidecar-50, not
    /// persisted; raising the frame cadence needs a kick so a parked
    /// member re-reads it within a frame.
    pub fn set_live_full_rate(&mut self, on: bool) {
        self.focus.lock().unwrap().live_full_rate = on;
        self.wake_all_slots();
    }

    /// Mirror the sidecar-50 pref onto every slot's frame-cadence latch
    /// (`SlotInput::set_full_rate`). Runs every UI frame so a focus,
    /// renderer, or wall-policy change lands within a frame.
    fn sync_sidecar_cadence(&mut self) {
        let focus = self.focus.lock().unwrap();
        for (name, slot) in &self.slots {
            slot.input.set_full_rate(full_rate_for(&focus, name));
        }
    }

    /// Mirror the effective nav-paint config onto the slot threads (they
    /// publish the focused drawing slot's paint every observe). Runs every
    /// UI frame so a modal edit or live-overlay flip lands within a frame.
    fn sync_nav_publish(&self) {
        *self.nav_publish.lock().unwrap() = NavPublishCfg {
            settings: self.ui.nav.clone(),
            live_force_layers: self.nav_live_force_layers,
        };
    }

    /// Game window `.build()` Some/None. Closing the pane turns capture off
    /// (`set_enabled(false)` + drop tx); reopening does not re-enable it.
    pub fn set_game_pane_open(&mut self, open: bool) {
        let mut focus = self.focus.lock().unwrap();
        let was = focus.game_pane_open;
        focus.game_pane_open = open;
        let name = focus.focused.clone();
        drop(focus);
        if was && !open {
            self.set_capture(false);
        }
        // draw_for_slot gates on the pane; kick the focused slot so a
        // parked thread sees the change within a frame.
        if let Some(name) = name {
            if let Some(play) = self.play.as_ref() {
                play.wake(&name);
            }
        }
    }

    /// Capture checkbox. On: attach a fresh channel and enable the focused
    /// slot's drain. Off: disable the drain and drop the sender so the UI
    /// cannot enqueue (the slot thread does no `try_recv` while disabled).
    pub fn set_capture(&mut self, on: bool) {
        self.focus.lock().unwrap().capture = on;
        let name = self.focused_name();
        if on {
            match name.as_deref() {
                Some(name) => self.capture_on(name),
                None => self.capture_tx = None,
            }
        } else {
            self.capture_off();
        }
        // Capture flips the slot's idle classification (capture → frame
        // loop); kick it so the change lands within a frame.
        if let Some(name) = name {
            if let Some(play) = self.play.as_ref() {
                play.wake(&name);
            }
        }
    }

    fn capture_on(&mut self, name: &str) {
        if let Some(slot) = self.slots.get(name) {
            let (tx, rx) = mpsc::channel();
            slot.input.connect_rx(rx);
            slot.input.set_enabled(true);
            self.capture_tx = Some(tx);
        } else {
            self.capture_tx = None;
        }
    }

    fn capture_off(&mut self) {
        if let Some(slot) = self.focused_slot() {
            slot.input.set_enabled(false);
        }
        self.capture_tx = None;
    }

    /// Save the credentials fields as a vault profile: the username field
    /// is the key, the password field the secret, and an existing profile's
    /// uid/settings are kept. Does not require a focused profile (first-run
    /// empty vault). After a successful upsert, spawns the slot via the
    /// existing FIFO if it is not running, then selects it. Returns whether
    /// the write landed; failures set [`Session::error`].
    pub fn save_credentials(&mut self) -> bool {
        if self.vault.is_none() {
            self.error = Some("credentials: vault locked".into());
            return false;
        }
        let username = self.cred_user.trim().to_string();
        if username.is_empty() {
            self.error = Some("credentials: username required".into());
            return false;
        }
        let profile = {
            let vault = self.vault.as_mut().expect("vault checked");
            let existing = vault.get(&username).cloned();
            Profile {
                uid: existing
                    .as_ref()
                    .map(|p| p.uid)
                    .unwrap_or_else(|| fresh_uid(vault)),
                username: username.clone(),
                password: self.cred_pass.clone(),
                settings: existing.map(|p| p.settings).unwrap_or_default(),
            }
        };
        match self.vault.as_mut().expect("vault checked").upsert(profile) {
            Ok(()) => self.error = None,
            Err(e) => {
                self.error = Some(format!("credentials: {e}"));
                return false;
            }
        }
        // `select` builds the arm from the vault auto-login setting.
        self.select(&username);
        true
    }

    /// Control arm for a vault profile: `SlotArm::new(uid, auto_login)` with
    /// `want_login` cleared when the wall logout latch blocks auto-login.
    fn arm_for_profile(&self, name: &str) -> Option<Arc<SlotArm>> {
        let profile = self.vault.as_ref().and_then(|v| v.get(name))?;
        let auto_login = profile.settings.auto_login;
        let arm = SlotArm::new(profile.uid, auto_login);
        if !self.wall.should_auto_login(name, auto_login) {
            arm.want_login.store(false, Ordering::Relaxed);
        }
        Some(arm)
    }

    /// Register per-slot IO and spawn via [`Play::spawn_slot`] when a play
    /// is live. Without `play` (unit tests / pre-unlock) only the IO map is
    /// filled so focus can attach. `arm` carries the spawn's login intent:
    /// `None` logs in immediately (CLI/e2e); panel paths pass
    /// [`Session::arm_for_profile`] so auto-login / latch are respected.
    ///
    /// Flat model: every profile spawns **one** full `Client` slot with its
    /// own input + framebuffer (no lean channel, no render-all guard — a
    /// headless member just has its draw off).
    fn ensure_slot(&mut self, username: &str, arm: Option<Arc<SlotArm>>) {
        if self.slots.contains_key(username) {
            return;
        }
        if self
            .play
            .as_ref()
            .is_some_and(|p| p.arm(username).is_some())
        {
            return;
        }
        let Some(profile) = self.vault.as_ref().and_then(|v| v.get(username)).cloned() else {
            return;
        };
        let input = SlotInput::new();
        // Raster/mem come from the vault profile (the same source as
        // `bot_client_config`); a focus change never re-roles a live slot.
        let raster = profile.settings.raster;
        let lowmem = profile.settings.lowmem;
        input.set_prefer_cpu(raster == vault::RasterMode::Cpu);
        {
            let mut f = self.focus.lock().unwrap();
            f.renderer_by
                .insert(username.to_string(), raster != vault::RasterMode::Off);
        }
        let pixels = FrameBuf::new();
        self.audio.set_music(username, !lowmem);
        if let Some(play) = &mut self.play {
            play.spawn_slot(
                profile,
                Some(Arc::clone(&input)),
                Some(Arc::clone(&pixels)),
                arm,
            );
        }
        self.slots
            .insert(username.to_string(), SlotIo { input, pixels });
    }

    /// Credentials Log in: clear the logout latch, arm a handshake the same
    /// way as Login all (`arm_login_all`), then select (spawn if needed).
    pub fn login(&mut self, name: &str) {
        self.wall.clear_latch(name);
        if let Some(arm) = self.play.as_ref().and_then(|p| p.arm(name)) {
            arm_login_all(&arm);
        }
        self.select(name);
    }

    /// Empty the credentials-section fields. The vault entry is untouched.
    pub fn clear_credentials(&mut self) {
        self.cred_user.clear();
        self.cred_pass.clear();
    }

    /// Open the profile picker on `name` (or a blank new row).
    pub fn begin_edit_profile(&mut self, name: Option<&str>) {
        match name {
            Some(n) => {
                if let Some(p) = self.vault.as_ref().and_then(|v| v.get(n)) {
                    self.cred_user = p.username.clone();
                    self.cred_pass = p.password.clone();
                }
                self.chooser_edit = Some(n.to_string());
            }
            None => {
                self.cred_user.clear();
                self.cred_pass.clear();
                self.chooser_edit = Some(String::new());
            }
        }
        self.wall.chooser_open = true;
    }

    pub fn cancel_edit_profile(&mut self) {
        self.chooser_edit = None;
    }

    /// Log out one member (the credentials Logout button): latch it so
    /// auto-login is blocked until [`Session::login_all`], then arm a clean
    /// IF logout. The slot stays up and focused; only the login intent
    /// changes.
    pub fn logout(&mut self, name: &str) {
        self.wall.latch_logout(name);
        if let Some(arm) = self.play.as_ref().and_then(|p| p.arm(name)) {
            arm.want_login.store(false, Ordering::Relaxed);
            arm.want_logout.store(true, Ordering::Relaxed);
        }
        // The logout press lives in the probe (per-tick); kick a parked
        // slot so the clean logout goes out within a frame.
        if let Some(play) = self.play.as_ref() {
            play.wake(name);
        }
    }

    /// Persist the focused profile's auto-login checkbox to the vault
    /// (`ProfileSettings.auto_login`) and mirror it onto a running slot's
    /// `arm.auto_login`. Never spawns or stops a slot.
    pub fn set_auto_login(&mut self, name: &str, on: bool) -> bool {
        let Some(vault) = self.vault.as_mut() else {
            self.error = Some("auto-login: vault locked".into());
            return false;
        };
        let Some(mut profile) = vault.get(name).cloned() else {
            self.error = Some(format!("auto-login: no profile {name}"));
            return false;
        };
        profile.settings.auto_login = on;
        match vault.upsert(profile) {
            Ok(()) => self.error = None,
            Err(e) => {
                self.error = Some(format!("auto-login: {e}"));
                return false;
            }
        }
        if let Some(arm) = self.play.as_ref().and_then(|p| p.arm(name)) {
            arm.auto_login.store(on, Ordering::Relaxed);
        }
        true
    }

    /// Game-pane lowmem (General config). Rail members stay lowmem.
    pub fn focused_lowmem(&self) -> bool {
        self.ui.lowmem
    }

    /// Game-pane none/GPU/CPU (General config). Rail members stay GPU
    /// (CPU/none only as fallback via `set_draw` / `prefer_cpu`).
    pub fn focused_raster(&self) -> vault::RasterMode {
        self.ui.raster
    }

    fn persist_game_render_prefs(&mut self) {
        if self.persist_ui {
            crate::ui_state::save(&self.ui);
        }
        let Some(name) = self.focused_name() else {
            return;
        };
        let Some(vault) = self.vault.as_mut() else {
            return;
        };
        if let Some(mut p) = vault.get(&name).cloned() {
            p.settings.raster = self.ui.raster;
            p.settings.lowmem = self.ui.lowmem;
            let _ = vault.upsert(p);
        }
    }

    /// Apply Game-pane raster. Off is `set_draw` on the focused client
    /// only. Gpu↔Cpu flips the slot's `prefer_cpu` latch; the host drops
    /// the `Renderer` and reattaches the right backend on the **same**
    /// client — never a restart. Rail members are not touched.
    pub fn set_focused_raster(&mut self, raster: vault::RasterMode) -> bool {
        self.ui.raster = raster;
        self.persist_game_render_prefs();
        self.set_renderer(raster != vault::RasterMode::Off);
        let Some(name) = self.focused_name() else {
            return true;
        };
        if raster == vault::RasterMode::Off {
            return true;
        }
        let want_cpu = raster == vault::RasterMode::Cpu;
        if let Some(slot) = self.slots.get(&name) {
            slot.input.set_prefer_cpu(want_cpu);
        }
        true
    }

    pub fn set_focused_lowmem(&mut self, lowmem: bool) -> bool {
        self.ui.lowmem = lowmem;
        self.persist_game_render_prefs();
        self.error = None;
        let Some(name) = self.focused_name() else {
            return true;
        };
        // The audio gate is the slot threads' lowmem channel: each frame
        // the slot applies `c.set_lowmem(!audio.music_on(name))`, and the
        // host drops the `Renderer` when `config.lowmem` changes so the
        // next paint attaches with the new mode. No restart.
        self.audio.set_music(&name, !lowmem);
        true
    }

    /// Status-row copy for the focused profile's mem mode.
    pub fn mem_status_text(lowmem: bool) -> &'static str {
        if lowmem {
            "lowmem"
        } else {
            "highmem"
        }
    }

    /// GPU↔CPU (not Off) on a spawned slot is a drop+reattach — the
    /// `Client` and its socket stay up, so no logout/restart confirm is
    /// ever required (Off is `set_draw`; mem flips are live too).
    pub fn raster_switch_needs_confirm(
        _next: vault::RasterMode,
        _prefer_cpu: bool,
        _slot_spawned: bool,
    ) -> bool {
        false
    }

    /// Pick a raster. Off is `set_draw`. GPU↔CPU on the Game-pane client
    /// drops + reattaches the renderer (never a logout). Rail members stay
    /// GPU/lowmem.
    pub fn request_focused_raster(&mut self, raster: vault::RasterMode) {
        if self.focused_raster() == raster {
            return;
        }
        let _ = self.set_focused_raster(raster);
    }

    /// Pick highmem/lowmem for the Game pane. The live `Client` flips mem
    /// (the host drops + reattaches the renderer); never a restart. Rail
    /// members stay lowmem.
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
        let newly = self.wall.load(name);
        let auto_login = self
            .vault
            .as_ref()
            .and_then(|v| v.get(name))
            .map(|p| p.settings.auto_login)
            .unwrap_or(false);
        let want_login = self.wall.should_auto_login(name, auto_login);
        if let Some(arm) = self.play.as_ref().and_then(|p| p.arm(name)) {
            // Already running (re-click): re-apply the login intent so a
            // latched logout stays on the title.
            arm.want_login.store(want_login, Ordering::Relaxed);
            arm.auto_login.store(auto_login, Ordering::Relaxed);
        } else {
            self.ensure_slot(name, self.arm_for_profile(name));
        }
        // Load all / chooser rows spawn onto the rail and focus the member
        // (the flat model's "click" — the Game pane samples this slot).
        self.apply_focus(name);
        self.sync_wall_focus();
        newly
    }

    /// Load every profile (vault plus running slots) that is not already a
    /// wall member — the chooser's "Load all". Returns how many were newly
    /// added. Login intent still follows each profile's auto-login setting.
    pub fn load_all(&mut self) -> usize {
        let names = self.profile_names();
        let mut added = 0;
        for name in names {
            if self.load(&name) {
                added += 1;
            }
        }
        added
    }

    /// Chooser row ✕: delete the vault profile only. A live wall member is
    /// **not** logged out or dropped; the row just disappears from the
    /// chooser (credentials Save re-creates it). Returns whether a row was
    /// removed; failures set [`Session::error`].
    pub fn vault_remove(&mut self, name: &str) -> bool {
        let Some(vault) = self.vault.as_mut() else {
            self.error = Some("chooser: vault locked".into());
            return false;
        };
        match vault.remove(name) {
            Ok(removed) => {
                if removed {
                    self.error = None;
                }
                removed
            }
            Err(e) => {
                self.error = Some(format!("chooser: {e}"));
                false
            }
        }
    }

    /// Mirror `wall.members` into `Focus.wall` so `draw_for_slot` can paint
    /// unfocused tiles when only-render-selected is off. Call whenever
    /// membership changes: load, load_all, rail_remove, or the seed path.
    fn sync_wall_focus(&mut self) {
        let members = self.wall.members.clone();
        self.focus.lock().unwrap().wall = members;
    }

    /// Kick every slot thread after a wall-policy change (`only render
    /// selected` toggling flips every member's draw state; a parked thread
    /// must re-read it within a frame, not at the game-tick timeout).
    pub fn wake_all_slots(&self) {
        if let Some(play) = self.play.as_ref() {
            play.wake_all();
        }
    }

    /// Log in every wall member: clear their latches and arm a login so
    /// title-screen slots handshake. One-shot unless the profile's
    /// auto-login is set (which keeps the arm armed after the handshake).
    /// The focused slot is moved to the front of the login FIFO so it is
    /// not stuck behind members that queued first.
    pub fn login_all(&mut self) {
        // Prefer the focused SlotIo (pixels), not the status row: the
        // focused slot can still be inside `maininit` when Login all runs,
        // so the status row is missing and prefer would be skipped.
        let head = self.tv_name();
        if let (Some(play), Some(h)) = (self.play.as_ref(), head.as_ref()) {
            if let Some(arm) = play.arm(h) {
                play.prefer_login(arm.uid.load(Ordering::Relaxed));
            }
        }
        let mut names = self.wall.members.clone();
        if let Some(h) = &head {
            names.retain(|n| n != h);
            names.insert(0, h.clone());
        }
        for name in names {
            self.wall.clear_latch(&name);
            if let Some(arm) = self.play.as_ref().and_then(|p| p.arm(&name)) {
                arm_login_all(&arm);
            }
        }
        if let Some(play) = self.play.as_ref() {
            play.wake_all();
        }
    }

    /// Log out every wall member: record the latch (blocks auto-login
    /// until the next [`Session::login_all`]) and arm a clean IF logout.
    /// `want_login` is cleared too so a title-screen member does not
    /// handshake right back in.
    pub fn logout_all(&mut self) {
        let mut names = self.wall.members.clone();
        if let Some(play) = &self.play {
            for s in play.statuses() {
                if !names.iter().any(|n| n == &s.username) {
                    names.push(s.username);
                }
            }
        }
        for name in names {
            self.wall.latch_logout(&name);
            if let Some(arm) = self.play.as_ref().and_then(|p| p.arm(&name)) {
                arm.want_logout.store(true, Ordering::Relaxed);
                arm.want_login.store(false, Ordering::Relaxed);
            }
        }
        if let Some(play) = self.play.as_ref() {
            play.wake_all();
        }
    }

    /// MultiBox toggle. On: seed the wall with every already-running slot
    /// (first on this process opens the chooser) and open the wall draw
    /// policy (`Focus.wall_open`), which stays true for rail **or** grid.
    /// Off: clear the grid and any open chooser and stop extra rasters
    /// (`wall_open = false`) without logging anyone out.
    pub fn set_multibox(&mut self, on: bool) {
        self.multibox = on;
        if on {
            let running: Vec<String> = self
                .play
                .as_ref()
                .map(|p| p.statuses().iter().map(|s| s.username.clone()).collect())
                .unwrap_or_default();
            self.wall.on_multibox_on(&running);
            // After seed: if focus is missing or not a wall member, restore
            // last_focus when it is on the wall, else the first member.
            let focused = self.focused_name();
            let need = match focused.as_deref() {
                None => true,
                Some(f) => !self.wall.members.iter().any(|m| m == f),
            };
            if need {
                // Live boots never restore the operator's disk last_focus;
                // the first wall member / later `select(&names[0])` wins.
                let last = if self.persist_ui {
                    crate::ui_state::load().last_focus
                } else {
                    None
                };
                if let Some(name) = crate::ui_state::pick_focus(&self.wall.members, last.as_deref())
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
        if let Some(play) = self.play.as_ref() {
            play.wake_all();
        }
    }

    /// Grid submode of MultiBox: hides the rail in the Game pane. A no-op
    /// while MultiBox is off.
    pub fn set_grid(&mut self, on: bool) {
        if self.multibox {
            self.wall.grid = on;
        }
    }

    /// Remove a member from the rail: focus a neighbour if this name was
    /// focused, drop it from the wall, clear its logout latch, arm a clean
    /// logout when ingame (without `stop`), wait until `!ingame` or ~10 s,
    /// then `stop_slot` and forget its IO. Not-ingame members stop immediately.
    pub fn rail_remove(&mut self, name: &str) {
        let focused = self.focused_name();
        let neighbour = self.wall.focus_neighbour(name, focused.as_deref());
        self.wall.rail_remove(name);
        self.wall.clear_latch(name);
        if let Some(play) = &self.play {
            let ingame = play
                .statuses()
                .iter()
                .any(|s| s.username == name && s.ingame);
            if ingame {
                if let Some(arm) = play.arm(name) {
                    // Clean logout only — do not set stop until !ingame.
                    arm.want_logout.store(true, Ordering::Relaxed);
                }
                // The logout press lives in the probe; kick a parked slot
                // so the clean logout is pressed instead of waiting on the
                // game-tick park timeout.
                play.wake(name);
                play.wait_until_not_ingame(name, Duration::from_secs(10));
            }
        }
        if let Some(play) = &mut self.play {
            play.stop_slot(name);
        }
        // Flat model: each member owns its own framebuffer; stop means drop.
        self.slots.remove(name);
        self.audio.release(name);
        self.sync_wall_focus();
        if focused.as_deref() == Some(name) {
            match neighbour {
                Some(n) => self.select(&n),
                None => {
                    self.focus.lock().unwrap().focused = None;
                    self.capture_tx = None;
                }
            }
        }
    }

    /// Arm a walk to `dest`. The picked dest is always stored so the status
    /// row shows what the user asked for even when no route could be found.
    /// Routing needs the player's observed tile and a loaded pack; the
    /// picker routes via [`Session::arm_walk_on`] when it has both.
    pub fn arm_walk(&mut self, dest: Tile) {
        self.walk_dest = Some(dest);
        self.walk_clear.store(false, Ordering::Relaxed);
    }

    /// Arm a walk to `dest` and route it on `world` from `from` (the
    /// player's observed tile). On `Ok(route)` the focused username's walk
    /// arm stores the route so the observe tick can step it via
    /// [`Traveller::follow`]; on `NoPath` only the dest is stored and
    /// `error` carries a short message. The Nav settings' [`FindOptions`]
    /// apply: `ui.nav.allow_teleports` unions the any-tile teleport layer
    /// in and `ui.nav.allow_wilderness` allows entering the wilderness.
    /// Callers that do not know the player's tile fall back to
    /// [`Session::arm_walk`].
    pub fn arm_walk_on(&mut self, world: &NavWorld, from: Tile, dest: Tile) {
        self.walk_dest = Some(dest);
        self.walk_clear.store(false, Ordering::Relaxed);
        let from_w = WorldTile {
            x: from.x,
            z: from.z,
            level: from.level,
        };
        let dest_w = WorldTile {
            x: dest.x,
            z: dest.z,
            level: dest.level,
        };
        let routed = find_with(
            &world.collision,
            &world.graph,
            from_w,
            dest_w,
            FindOptions {
                allow_teleports: self.ui.nav.allow_teleports,
                allow_wilderness: self.ui.nav.allow_wilderness,
            },
        );
        match routed {
            Ok(route) => {
                self.error = None;
                if let Some(name) = self.focused_name() {
                    let arm = self
                        .travellers
                        .lock()
                        .unwrap()
                        .entry(name.clone())
                        .or_insert_with(|| Arc::new(Mutex::new(WalkArm::default())))
                        .clone();
                    let mut arm = arm.lock().unwrap();
                    // A fresh arm replaces any in-flight follow run.
                    arm.traveller.clear();
                    arm.route = Some(route);
                    drop(arm);
                    self.tick_latch.lock().unwrap().remove(&name);
                    // Rising edge: the overlay must paint the new route on
                    // this frame, not after the 1 s raster cadence.
                    self.route_gen += 1;
                }
            }
            Err(_) => {
                self.error = Some(format!("no path to {} {} {}", dest.x, dest.z, dest.level));
            }
        }
    }

    /// Arm the current [`Session::picker_sel`] on `world`. Returns false
    /// when nothing is selected. Clears the selection either way so a
    /// second confirm does not re-fire.
    pub fn confirm_picker_walk(&mut self, world: &NavWorld) -> bool {
        let Some(tile) = self.picker_sel.take() else {
            return false;
        };
        match self.focused_tile() {
            Some((fx, fz)) => {
                let from = Tile {
                    x: fx,
                    z: fz,
                    level: tile.level,
                };
                self.arm_walk_on(world, from, tile);
            }
            None => self.arm_walk(tile),
        }
        true
    }

    /// The focused slot's observed tile, `None` when nothing is focused or
    /// the slot has not reported a position yet (both coordinates zero).
    pub fn focused_tile(&self) -> Option<(i32, i32)> {
        let name = self.focused_name()?;
        self.statuses()
            .iter()
            .find(|s| s.username == name)
            .filter(|s| s.tile_x != 0 || s.tile_z != 0)
            .map(|s| (s.tile_x, s.tile_z))
    }

    /// The focused slot's login-FIFO place `(position, total)` while it
    /// waits for a permit (`position >= 1`), else `None`. The status row
    /// and the queue card read this; grant clears it to `None`.
    pub fn focused_queue(&self) -> Option<(i32, i32)> {
        let name = self.focused_name()?;
        self.statuses()
            .iter()
            .find(|s| s.username == name)
            .filter(|s| s.queue_position >= 1)
            .map(|s| (s.queue_position, s.queue_total))
    }

    /// Queue card place: the focused slot if it is waiting, else the FIFO
    /// head among every waiting wall member. Keeps showing *k of n* once
    /// the focused slot has granted and later members remain queued.
    pub fn queue_place(&self) -> Option<(i32, i32)> {
        if let Some(q) = self.focused_queue() {
            return Some(q);
        }
        self.statuses
            .iter()
            .filter(|s| s.queue_position >= 1)
            .min_by_key(|s| s.queue_position)
            .map(|s| (s.queue_position, s.queue_total))
    }

    /// Whether the focused slot is ingame — the Logout button's enable
    /// gate (a queued or title-screen slot has nothing to log out).
    pub fn focused_ingame(&self) -> bool {
        let Some(name) = self.focused_name() else {
            return false;
        };
        self.statuses()
            .iter()
            .any(|s| s.username == name && s.ingame)
    }

    /// The focused slot's script lifecycle state; `Idle` when nothing is
    /// focused or the slot has no script. The script section's disable
    /// rules key off this.
    pub fn focused_script_state(&self) -> script::RunState {
        let Some(name) = self.focused_name() else {
            return script::RunState::Idle;
        };
        self.play
            .as_ref()
            .map(|p| p.script_state(&name))
            .unwrap_or(script::RunState::Idle)
    }

    /// The focused slot's script `last_error`; `None` when the slot has no
    /// script error (or nothing is focused).
    pub fn focused_script_last_error(&self) -> Option<String> {
        let name = self.focused_name()?;
        self.play.as_ref()?.script_last_error(&name)
    }

    /// Start the Browse-selected script (compiled or loaded JS) on the
    /// focused slot. The rs2b0t rule is enforced here too: while the slot's
    /// script is active the call is refused (the Start button is disabled,
    /// so this is the no-call backstop). Errors set [`Session::error`].
    pub fn script_start_selected(&mut self) {
        let Some(name) = self.focused_name() else {
            self.error = Some("script: no focused profile".into());
            return;
        };
        let Some(sel) = self.script_sel.clone() else {
            self.error = Some("script: browse to pick one first".into());
            return;
        };
        if script_active(self.focused_script_state()) {
            return;
        }
        let result = match (self.play.as_ref(), sel) {
            (Some(play), script::ScriptSel::Compiled(id)) => play.script_start(&name, id),
            (Some(play), script::ScriptSel::Loaded(card_name)) => match self.js.get(&card_name) {
                Some(card) => play.script_start_load(&name, card.source.clone(), card.shape),
                None => Err(format!("no loaded script: {card_name}")),
            },
            (None, _) => Err("no play".to_string()),
        };
        match result {
            Ok(()) => self.error = None,
            Err(e) => self.error = Some(format!("script: {e}")),
        }
    }

    /// Load a local JS file into the library (registers a picker card,
    /// persists `~/.274bot/js-scripts.json`), select it for Start, and
    /// clear the modal scratch. Errors set [`Session::error`].
    pub fn load_js(&mut self, path: &str) {
        let trimmed = path.trim();
        if trimmed.is_empty() {
            self.error = Some("script: path required".into());
            return;
        }
        match self.js.load(std::path::Path::new(trimmed)) {
            Ok(card) => {
                self.error = None;
                self.script_sel = Some(script::ScriptSel::Loaded(card.name));
                self.load_scratch.clear();
            }
            Err(e) => self.error = Some(format!("load: {e}")),
        }
    }

    /// Pause the focused slot's script, or Resume when it is Paused (the
    /// button label follows [`script_pause_enabled`]).
    pub fn script_toggle_pause(&mut self) {
        let Some(name) = self.focused_name() else {
            return;
        };
        let Some(play) = self.play.as_ref() else {
            return;
        };
        if play.script_state(&name) == script::RunState::Paused {
            play.script_resume(&name);
        } else {
            play.script_pause(&name);
        }
    }

    /// Stop the focused slot's script (teardown hook, instance dropped).
    pub fn script_stop(&mut self) {
        let Some(name) = self.focused_name() else {
            return;
        };
        if let Some(play) = self.play.as_ref() {
            play.script_stop(&name);
        }
    }

    /// Overlay generation for the path overlay's rising-edge refresh.
    pub fn route_gen(&self) -> u64 {
        self.route_gen
    }

    /// The status-row walk cell: `"—"` when nothing is queued, else the
    /// queued dest as `"x z level"`.
    pub fn walk_status_text(&self) -> String {
        match self.walk_dest {
            Some(d) => format!("{} {} {}", d.x, d.z, d.level),
            None => "—".into(),
        }
    }
}

/// Throwaway encrypted vault for live prepare (e2e `temp_vault` pattern,
/// kept panel-private so panel does not depend on the e2e crate).
/// Null raster keeps base uid `274_000_001`.
fn temp_live_vault(entries: &[(&str, &str)]) -> PathBuf {
    temp_live_vault_from(entries, 274_000_001)
}

/// Same as [`temp_live_vault`] with an explicit uid base (`base + i`).
fn temp_live_vault_from(entries: &[(&str, &str)], uid_base: i32) -> PathBuf {
    // Unique per call: parallel tests boot several scenarios and must not
    // race on one temp vault path.
    static SERIAL: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let serial = SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "274bot-panel-live-{}-{}-{}-{serial}",
        std::process::id(),
        entries.len(),
        uid_base
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("vault");
    if path.exists() {
        std::fs::remove_file(&path).unwrap();
    }
    let mut vault = Vault::create(&path, "bot").unwrap();
    for (i, (user, pass)) in entries.iter().enumerate() {
        vault
            .upsert(Profile {
                username: (*user).into(),
                password: (*pass).into(),
                uid: uid_base + i as i32,
                settings: vault::ProfileSettings::default(),
            })
            .unwrap();
    }
    path
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

/// Fresh uid for a profile with no existing vault entry: one past the max
/// (host-play assigns uids from the same 274M base range).
fn fresh_uid(vault: &Vault) -> i32 {
    vault.profiles().map(|p| p.uid).max().unwrap_or(274_000_000) + 1
}

/// The flags `login_all` applies to a member's arm: clear the logout latch,
/// arm a login, and cancel any pending logout. `want_logout` only clears
/// inside the slot body when it observes the member ingame, so a
/// title-screen member keeps a stale logout that would otherwise fire on
/// the first ingame frame after Login all handshakes it back in.
fn arm_login_all(arm: &SlotArm) {
    arm.latch.store(false, Ordering::Relaxed);
    arm.want_login.store(true, Ordering::Relaxed);
    arm.want_logout.store(false, Ordering::Relaxed);
}

/// Copy a traveller dest into `SlotStatus.walk_*`; −1 when idle.
fn apply_queued_walk(status: &mut SlotStatus, queued: Option<Tile>) {
    match queued {
        Some(t) => {
            status.walk_x = t.x;
            status.walk_z = t.z;
            status.walk_level = t.level;
        }
        None => {
            status.walk_x = -1;
            status.walk_z = -1;
            status.walk_level = -1;
        }
    }
}

/// Detach the picker's nav world only after the play's slot threads are
/// joined, so no live observe can read a cleared `pack()`.
impl Drop for Session {
    fn drop(&mut self) {
        self.play = None;
        crate::picker::set_pack(None);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        arm_login_all, combo_index, debug_dest_cheats, debug_main_buttons, debug_maxme_cheats,
        is_local_engine, maybe_send_click, parse_getvar_line, publish_nav_debug, script_active,
        script_pause_enabled, script_status_text, script_stop_enabled, seed_on_first_world,
        stream_capture, walkto_tele_cmd, Session, SlotIo,
    };
    use crate::focus::draw_for_slot;
    use api::snapshot::WorldTile;
    use client::dash3d::CollisionFlag;
    use client::render::nav_debug::{FACE_N, FACE_S};
    use host::{FrameBuf, InputEv, SlotInput};
    use host_play::{SlotArm, SlotStatus};
    use nav::collision::WorldCollision;
    use nav::paint::{MAX_DRAW_TILES, NEAR_FULL_DENSITY};
    use nav::router::{Leg, Route};
    use nav::tile::Tile;
    use nav::transport::{TransportEdge, TransportGraph, TransportKind};
    use nav::world::NavWorld;
    use std::sync::atomic::Ordering;
    use std::sync::Arc;
    use std::time::Duration;
    use vault::{Profile, ProfileSettings, Vault};

    use crate::nav_settings::{effective, NavSettings};

    #[test]
    fn is_local_engine_is_loopback_only() {
        assert!(is_local_engine("127.0.0.1"));
        assert!(is_local_engine("localhost"));
        assert!(is_local_engine("::1"));
        assert!(!is_local_engine("w1.rs2b2t.com"));
        assert!(!is_local_engine("192.168.1.5"));
    }

    #[test]
    fn debug_dest_lumbridge_sends_home() {
        let d = debug_dest_cheats();
        assert!(d
            .iter()
            .any(|x| x.label == "Lumbridge" && x.cheat == "~home"));
        assert!(d.iter().any(|x| x.label == "Seers" && x.cheat == "~seers"));
        assert!(!d.iter().any(|x| x.label == "North"));
    }

    #[test]
    fn debug_dest_greenland_tooltip_is_the_script_comment() {
        let g = debug_dest_cheats()
            .iter()
            .find(|x| x.label == "Greenland")
            .expect("greenland dest");
        assert_eq!(g.cheat, "~greenland");
        assert!(
            g.tooltip.contains("Gnome Stronghold"),
            "hover must say where this is, got {:?}",
            g.tooltip
        );
    }

    #[test]
    fn walkto_tele_cmd_is_engine_tele_args() {
        let t = Tile {
            x: 3253,
            z: 3266,
            level: 0,
        };
        assert_eq!(walkto_tele_cmd(t), "tele 0,50,51,53,2");
    }

    #[test]
    fn mark_tutorial_skipped_persists_on_focused_profile() {
        let path = tmp_vault("tutskip-pref.vault");
        let mut s = Session::new();
        s.vault = Some(Vault::create(&path, "bot").unwrap());
        s.vault
            .as_mut()
            .unwrap()
            .upsert(profile("alice", "pw", 42))
            .unwrap();
        s.focus.lock().unwrap().focused = Some("alice".into());
        assert_eq!(s.focused_tutorial_skipped(), None);
        s.mark_tutorial_skipped();
        assert_eq!(s.focused_tutorial_skipped(), Some(true));
        assert_eq!(
            s.vault
                .as_ref()
                .unwrap()
                .get("alice")
                .unwrap()
                .settings
                .tutorial_skipped,
            Some(true)
        );
    }

    #[test]
    fn parse_getvar_line_reads_engine_reply() {
        assert_eq!(
            parse_getvar_line("get tutorial: 1000"),
            Some(("tutorial", 1000))
        );
        assert_eq!(parse_getvar_line("get tutorial: 0"), Some(("tutorial", 0)));
        assert_eq!(parse_getvar_line("hello"), None);
    }

    #[test]
    fn tutskip_button_omitted_until_known_open() {
        assert_eq!(
            debug_main_buttons(false),
            ["DebugPanel", "Lumbridge", "maxme", "Teles"]
        );
        assert_eq!(
            debug_main_buttons(true),
            ["DebugPanel", "TutSkip", "Lumbridge", "maxme", "Teles"]
        );
    }

    #[test]
    fn debug_maxme_is_setstat_99_not_maxme_proc() {
        let cmds = debug_maxme_cheats();
        assert!(!cmds.iter().any(|c| *c == "maxme"));
        assert!(cmds.contains(&"setstat attack 99"));
        assert_eq!(cmds.len(), 19);
    }

    fn empty_play() -> host_play::Play {
        host_play::run_with_io(
            &host_play::PlayOptions {
                host: "127.0.0.1".into(),
                port: 43594,
                cache_dir: "/tmp".into(),
                lowmem: true,
                mainland: false,
            },
            vec![],
            |_| (None, None),
            |_, _| {},
        )
    }

    fn status(name: &str, ingame: bool, scene: i32) -> SlotStatus {
        SlotStatus {
            username: name.into(),
            ingame,
            scene_state: scene,
            ..SlotStatus::default()
        }
    }

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
                walk: vec![0u16; w * h],
                flags: None,
            },
            graph: TransportGraph::default(),
        }
    }

    /// A synthetic offline client with a fake mainland scene base (same
    /// trick as the app tests — no live server, no network).
    fn paint_client() -> client::client::Client {
        let mut c = host::prepare_client(
            client::client::ClientConfig {
                host: "127.0.0.1".into(),
                port: 43594,
                cache_dir: "/tmp".into(),
                members: true,
                lowmem: true,
            },
            1,
            Arc::new(client::config::Cache::default()),
            Arc::new(vec![]),
            Vec::new(),
        );
        c.map_build_base_x = 3200;
        c.map_build_base_z = 3200;
        c
    }

    /// A 64×64 level-0 world at (3200, 3200) with a face wall and a
    /// WR_GRND ground block inside the scene region.
    fn walled_world() -> NavWorld {
        let width = 64;
        let height = 64;
        let mut flags = vec![0u32; width * height];
        flags[1 * width + 1] = CollisionFlag::W_N as u32 | CollisionFlag::W_S as u32;
        flags[2 * width + 2] = CollisionFlag::WR_GRND as u32;
        NavWorld {
            collision: WorldCollision {
                origin: WorldTile {
                    x: 3200,
                    z: 3200,
                    level: 0,
                },
                width,
                height,
                walk: nav::collision::pack_walk_u16(&flags),
                flags: None,
            },
            graph: TransportGraph::default(),
        }
    }

    #[test]
    fn focused_slot_publishes_scene_collision_for_loaded_map() {
        let mut c = paint_client();
        let world = walled_world();
        // Live-harness layers force collision_fill + nsew_labels on.
        let layers = effective(&NavSettings::default(), true);
        let route = Route {
            legs: vec![Leg::Walk {
                tiles: vec![
                    WorldTile {
                        x: 3200,
                        z: 3200,
                        level: 0,
                    },
                    WorldTile {
                        x: 3201,
                        z: 3200,
                        level: 0,
                    },
                    WorldTile {
                        x: 3202,
                        z: 3200,
                        level: 0,
                    },
                ],
            }],
            dest: WorldTile {
                x: 3202,
                z: 3200,
                level: 0,
            },
            ticks: 1.0,
        };
        let here = Some(WorldTile {
            x: 3200,
            z: 3200,
            level: 0,
        });
        // The traveller's current walk aim, world tile → scene (52, 52).
        let click = Some(WorldTile {
            x: 3252,
            z: 3252,
            level: 0,
        });
        publish_nav_debug(
            &mut c,
            &world,
            Some(&route),
            here,
            &[],
            false,
            click,
            &layers,
            true,
        );
        let paint = c.nav_debug_paint().expect("focused drawing slot publishes");
        assert!(
            !paint.collision.is_empty(),
            "the loaded scene must include the walled tiles"
        );
        assert!(
            paint
                .collision
                .iter()
                .all(|cell| (0..104).contains(&cell.lx) && (0..104).contains(&cell.lz)),
            "collision cells are scene tiles inside the loaded 104×104 map"
        );
        assert!(
            paint
                .collision
                .iter()
                .all(|cell| cell.lx < 64 && cell.lz < 64),
            "tiles outside the pack bake must not paint a phantom wall"
        );
        let nsew = paint
            .collision
            .iter()
            .find(|cell| cell.lx == 1 && cell.lz == 1);
        assert!(
            nsew.is_some_and(|cell| cell.bits & FACE_N != 0 && cell.bits & FACE_S != 0),
            "the W_N/W_S tile must pack the N and S face bits"
        );
        assert!(
            paint
                .collision
                .iter()
                .any(|cell| cell.lx == 2 && cell.lz == 2 && cell.bits == 0),
            "the WR_GRND tile blocks ground with no face bits"
        );
        // Path: world tiles convert to scene tiles (client clips).
        assert_eq!(
            paint.path,
            vec![(0, 0, false), (1, 0, false), (2, 0, false)],
            "remaining path tiles convert to scene lx,lz"
        );
        // Click: the traveller's walk aim converts to scene coords.
        assert_eq!(paint.click, Some((52, 52)));
        assert!(paint.show_collision && paint.show_nsew && paint.show_path);
    }

    #[test]
    fn unfocused_slot_clears_nav_debug_paint() {
        let mut c = paint_client();
        let world = walled_world();
        let layers = effective(&NavSettings::default(), true);
        publish_nav_debug(&mut c, &world, None, None, &[], false, None, &layers, true);
        assert!(c.nav_debug_paint().is_some());
        // Unfocused / skip-paint / renderer-off slots must not linger on a
        // stale paint.
        publish_nav_debug(&mut c, &world, None, None, &[], false, None, &layers, false);
        assert!(
            c.nav_debug_paint().is_none(),
            "a non-drawing slot stores None"
        );
    }

    #[test]
    fn focused_slot_publishes_client_trail_tones() {
        let mut c = paint_client();
        let world = walled_world();
        let layers = NavSettings {
            show_nav_path: true,
            client_trail: true,
            ..NavSettings::default()
        };
        // The local player's last tryMove path, world tiles (the route
        // buffer minus the base), run on.
        let trail_world = vec![
            WorldTile {
                x: 3200,
                z: 3200,
                level: 0,
            },
            WorldTile {
                x: 3201,
                z: 3200,
                level: 0,
            },
            WorldTile {
                x: 3202,
                z: 3200,
                level: 0,
            },
        ];
        publish_nav_debug(
            &mut c,
            &world,
            None,
            None,
            &trail_world,
            true,
            None,
            &layers,
            true,
        );
        let paint = c.nav_debug_paint().expect("focused drawing slot publishes");
        assert_eq!(
            paint.trail,
            vec![(0, 0, false), (1, 0, true), (2, 0, false)],
            "run-on trail alternates Primary / RunAlt in scene coords"
        );
        assert!(paint.show_trail);
    }

    /// A walk leg then a Door transport (loc-backed), the shape the hull
    /// and draw-budget tests need.
    fn door_route() -> Route {
        Route {
            legs: vec![
                Leg::Walk {
                    tiles: vec![
                        WorldTile {
                            x: 3200,
                            z: 3200,
                            level: 0,
                        },
                        WorldTile {
                            x: 3201,
                            z: 3200,
                            level: 0,
                        },
                    ],
                },
                Leg::Transport {
                    edge: TransportEdge {
                        kind: TransportKind::Door,
                        at: WorldTile {
                            x: 3202,
                            z: 3200,
                            level: 0,
                        },
                        to: WorldTile {
                            x: 3203,
                            z: 3200,
                            level: 0,
                        },
                        loc_id: 1530,
                        option: 1,
                        ticks: 1,
                        dir: None,
                        open_loc_id: None,
                        skill_req: vec![],
                        item_req: vec![],
                        quest_req: vec![],
                        varp_req: vec![],
                        worn_req: vec![],
                    },
                },
            ],
            dest: WorldTile {
                x: 3203,
                z: 3200,
                level: 0,
            },
            ticks: 0.0,
        }
    }

    #[test]
    fn face_only_cell_stays_in_nsew_set_without_collision_block() {
        // A W_S-only tile: standable ground, one face flag. With both
        // layers on the cell must stay in the NSEW set (letters) yet
        // report `blocked` false so the client never collision-fills it.
        let width = 64;
        let height = 64;
        let mut flags = vec![0u32; width * height];
        flags[1 * width + 1] = CollisionFlag::W_S as u32;
        let world = NavWorld {
            collision: WorldCollision {
                origin: WorldTile {
                    x: 3200,
                    z: 3200,
                    level: 0,
                },
                width,
                height,
                walk: nav::collision::pack_walk_u16(&flags),
                flags: None,
            },
            graph: TransportGraph::default(),
        };
        let mut c = paint_client();
        let layers = effective(&NavSettings::default(), true);
        publish_nav_debug(&mut c, &world, None, None, &[], false, None, &layers, true);
        let paint = c.nav_debug_paint().expect("focused drawing slot publishes");
        let face_only = paint
            .collision
            .iter()
            .find(|cell| cell.lx == 1 && cell.lz == 1)
            .expect("the W_S tile must be in the NSEW set");
        assert!(
            face_only.bits & FACE_S != 0 && !face_only.blocked,
            "face-only cell keeps its letter but is never collision-blocked"
        );
    }

    #[test]
    fn show_nav_path_masters_hulls_click_and_trail() {
        let mut c = paint_client();
        let world = walled_world();
        let route = door_route();
        let click = Some(WorldTile {
            x: 3252,
            z: 3252,
            level: 0,
        });
        let trail_world = [WorldTile {
            x: 3200,
            z: 3200,
            level: 0,
        }];

        // Master off: hulls/click/trail stay off even with their own
        // toggles on.
        let layers = NavSettings {
            hop_labels: true,
            client_trail: true,
            ..NavSettings::default()
        };
        publish_nav_debug(
            &mut c,
            &world,
            Some(&route),
            None,
            &trail_world,
            false,
            click,
            &layers,
            true,
        );
        let paint = c.nav_debug_paint().unwrap();
        assert!(
            !paint.show_hulls && paint.hulls.is_empty(),
            "show_nav_path masters the hulls"
        );
        assert!(paint.click.is_none(), "no nav path, no walk-target paint");
        assert!(
            !paint.show_trail && paint.trail.is_empty(),
            "show_nav_path masters the trail"
        );

        // Master on, layer toggles off: hulls and trail still stay off;
        // the click (which has no extra toggle) comes on.
        let layers = NavSettings {
            show_nav_path: true,
            hop_labels: false,
            client_trail: false,
            ..NavSettings::default()
        };
        publish_nav_debug(
            &mut c,
            &world,
            Some(&route),
            None,
            &trail_world,
            false,
            click,
            &layers,
            true,
        );
        let paint = c.nav_debug_paint().unwrap();
        assert!(
            !paint.show_hulls && paint.hulls.is_empty(),
            "hop_labels is the hulls' second gate"
        );
        assert!(
            !paint.show_trail && paint.trail.is_empty(),
            "client_trail is the trail's second gate"
        );
        assert_eq!(
            paint.click,
            Some((52, 52)),
            "show_nav_path masters the click paint"
        );

        // Master + toggles on: the door hop hull and the trail publish.
        let layers = NavSettings {
            show_nav_path: true,
            hop_labels: true,
            client_trail: true,
            ..NavSettings::default()
        };
        publish_nav_debug(
            &mut c,
            &world,
            Some(&route),
            None,
            &trail_world,
            true,
            click,
            &layers,
            true,
        );
        let paint = c.nav_debug_paint().unwrap();
        assert!(
            paint.show_hulls && paint.hulls.iter().any(|h| h.loc_id == 1530),
            "the door hop hull publishes with master + hop_labels"
        );
        assert!(
            paint.show_trail && paint.trail.iter().any(|&(lx, lz, _)| lx == 0 && lz == 0),
            "the trail publishes with master + client_trail"
        );
    }

    #[test]
    fn nav_path_subsamples_to_the_draw_budget_keeping_hops() {
        // 300 walk tiles + a door hop: the 3D path must stay under the
        // draw budget, full density near, keeping the transport hop and
        // the terminal.
        let mut c = paint_client();
        let world = walled_world();
        let mut tiles: Vec<WorldTile> = (0..300)
            .map(|x| WorldTile {
                x: 3200 + x,
                z: 3200,
                level: 0,
            })
            .collect();
        tiles.push(WorldTile {
            x: 3500,
            z: 3200,
            level: 0,
        });
        tiles.push(WorldTile {
            x: 3501,
            z: 3200,
            level: 0,
        });
        let route = Route {
            legs: vec![
                Leg::Walk {
                    tiles: tiles[..300].to_vec(),
                },
                Leg::Transport {
                    edge: TransportEdge {
                        kind: TransportKind::Door,
                        at: tiles[300],
                        to: tiles[301],
                        loc_id: 1530,
                        option: 1,
                        ticks: 1,
                        dir: None,
                        open_loc_id: None,
                        skill_req: vec![],
                        item_req: vec![],
                        quest_req: vec![],
                        varp_req: vec![],
                        worn_req: vec![],
                    },
                },
            ],
            dest: tiles[301],
            ticks: 0.0,
        };
        let layers = NavSettings {
            show_nav_path: true,
            ..NavSettings::default()
        };
        publish_nav_debug(
            &mut c,
            &world,
            Some(&route),
            None,
            &[],
            false,
            None,
            &layers,
            true,
        );
        let paint = c.nav_debug_paint().unwrap();
        assert!(
            paint.path.len() <= MAX_DRAW_TILES,
            "the 3D path respects the draw budget ({} tiles)",
            paint.path.len()
        );
        assert!(
            paint.path.iter().any(|&p| p == (300, 0, true)),
            "the transport hop is never subsampled away"
        );
        assert_eq!(
            paint.path.last(),
            Some(&(301, 0, true)),
            "the terminal hop tile always survives"
        );
        assert!(
            paint.path.iter().any(|&p| p == (0, 0, false))
                && paint
                    .path
                    .iter()
                    .any(|&p| p == (NEAR_FULL_DENSITY as i32 - 1, 0, false)),
            "the near path stays at full density"
        );
    }

    #[test]
    fn tv_name_follows_the_focused_slot() {
        let mut s = Session::new();
        s.play = Some(empty_play());
        s.slots.insert(
            "s00".into(),
            SlotIo {
                input: SlotInput::new(),
                pixels: FrameBuf::new(),
            },
        );
        s.slots.insert(
            "s05".into(),
            SlotIo {
                input: SlotInput::new(),
                pixels: FrameBuf::new(),
            },
        );
        s.select("s05");
        assert_eq!(
            s.tv_name().as_deref(),
            Some("s05"),
            "Login all must prefer the focused slot, not the first FrameBuf key"
        );
        assert_eq!(
            s.play.as_ref().unwrap().focused().as_deref(),
            Some("s05"),
            "select mirrors the sampled slot onto the play (pure bookkeeping)"
        );
    }

    #[test]
    fn seed_on_first_world_skips_after_reconnect() {
        assert!(seed_on_first_world(None));
        assert!(seed_on_first_world(Some(false)));
        assert!(!seed_on_first_world(Some(true)));
    }

    #[test]
    fn pump_status_log_is_per_username() {
        // two SlotStatus rows, pump twice with transitions; log_by["alice"] does not contain bob lines
        let mut s = Session::new();
        let play = empty_play();
        play.statuses
            .lock()
            .unwrap()
            .extend([status("alice", false, 0), status("bob", false, 0)]);
        s.play = Some(play);

        s.pump_status();
        {
            let log_by = s.log_by.lock().unwrap();
            let alice = log_by.get("alice").expect("alice log");
            let bob = log_by.get("bob").expect("bob log");
            assert!(alice.iter().any(|l| l.contains("slot up")));
            assert!(bob.iter().any(|l| l.contains("slot up")));
            assert!(alice.iter().all(|l| !l.contains("bob")));
            assert!(bob.iter().all(|l| !l.contains("alice")));
        }

        s.play
            .as_ref()
            .unwrap()
            .statuses
            .lock()
            .unwrap()
            .iter_mut()
            .for_each(|row| {
                if row.username == "alice" {
                    row.ingame = true;
                    row.scene_state = 2;
                } else if row.username == "bob" {
                    row.ingame = true;
                    row.scene_state = 1;
                }
            });
        s.pump_status();
        let log_by = s.log_by.lock().unwrap();
        let alice = log_by.get("alice").expect("alice log");
        let bob = log_by.get("bob").expect("bob log");
        assert!(alice.iter().any(|l| l.contains("ingame")));
        assert!(alice.iter().any(|l| l.contains("scene 2")));
        assert!(bob.iter().any(|l| l.contains("ingame")));
        assert!(bob.iter().any(|l| l.contains("scene 1")));
        assert!(
            alice
                .iter()
                .all(|l| !l.contains("bob") && !l.contains("scene 1")),
            "alice must not see bob lines: {alice:?}"
        );
        assert!(
            bob.iter()
                .all(|l| !l.contains("alice") && !l.contains("scene 2")),
            "bob must not see alice lines: {bob:?}"
        );
    }

    #[test]
    fn music_toggle_mirrors_onto_the_audio_gate_live() {
        let path = tmp_vault("audio-toggle.vault");
        let mut s = Session::new();
        s.vault = Some(Vault::create(&path, "bot").unwrap());
        s.vault
            .as_mut()
            .unwrap()
            .upsert(profile("alice", "pw", 42))
            .unwrap();
        // The default lowmem slot starts with Music/SFX off: no cpal.
        assert!(s.focused_lowmem());
        s.select("alice");
        assert!(
            !s.audio.music_on("alice"),
            "default lowmem must not arm music"
        );
        // Toggle on (highmem): the gate arms the focused slot's speaker.
        assert!(s.set_focused_lowmem(false));
        assert!(s.audio.music_on("alice"));
        assert!(!s.focused_lowmem());
        // Toggle off (lowmem): the gate tears the speaker down.
        assert!(s.set_focused_lowmem(true));
        assert!(!s.audio.music_on("alice"));
    }

    #[test]
    fn sidecar_cadence_sync_raises_members_not_focus() {
        let mut s = Session::new();
        let a_in = SlotInput::new();
        let b_in = SlotInput::new();
        s.slots.insert(
            "a".into(),
            SlotIo {
                input: Arc::clone(&a_in),
                pixels: FrameBuf::new(),
            },
        );
        s.slots.insert(
            "b".into(),
            SlotIo {
                input: Arc::clone(&b_in),
                pixels: FrameBuf::new(),
            },
        );
        {
            let mut f = s.focus.lock().unwrap();
            f.focused = Some("a".into());
            f.only_render_selected = false;
            f.wall_open = true;
            f.wall = vec!["a".into(), "b".into()];
            f.renderer_by =
                std::collections::HashMap::from([("a".into(), true), ("b".into(), true)]);
            f.sidecar_50 = true;
        }
        s.focus.lock().unwrap().focused_50 = false;
        s.sync_sidecar_cadence();
        assert!(!a_in.full_rate(), "sidecar must not raise the Game pane");
        assert!(b_in.full_rate(), "the sidecar pref raises a drawing member");
        // Pref off returns the 1 fps watch cadence.
        s.set_sidecar_50(false);
        s.sync_sidecar_cadence();
        assert!(!b_in.full_rate());
    }

    fn tmp_vault(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("274bot-panel-session-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        if p.exists() {
            std::fs::remove_file(&p).unwrap();
        }
        p
    }

    fn profile(username: &str, password: &str, uid: i32) -> Profile {
        Profile {
            username: username.into(),
            password: password.into(),
            uid,
            settings: ProfileSettings::default(),
        }
    }

    #[test]
    fn unlock_at_uses_the_given_path() {
        let path = tmp_vault("unlock-at.vault");
        let mut s = Session::new();
        assert!(s.unlock_at(&path, "bot"));
        assert!(s.vault.is_some());
    }

    #[test]
    fn wrong_pass_does_not_delete_or_replace_the_vault() {
        let path = tmp_vault("wrong-pass.vault");
        let mut s = Session::new();
        assert!(s.unlock_at(&path, "bot"));
        s.vault
            .as_mut()
            .unwrap()
            .upsert(profile("alice", "pw", 42))
            .unwrap();
        drop(s);

        let mut s = Session::new();
        assert!(!s.unlock_at(&path, "nope"));
        assert!(s.vault.is_none());
        assert!(path.is_file());
        let v = Vault::unlock(&path, "bot").unwrap();
        assert!(v.get("alice").is_some());
    }

    #[test]
    fn reset_vault_at_refuses_while_unlocked() {
        let path = tmp_vault("reset-locked.vault");
        let mut s = Session::new();
        assert!(s.unlock_at(&path, "bot"));
        assert!(!s.reset_vault_at(&path));
        assert!(path.is_file());
        assert!(s.vault.is_some());
    }

    #[test]
    fn reset_vault_at_deletes_while_locked() {
        let path = tmp_vault("reset-ok.vault");
        let mut s = Session::new();
        assert!(s.unlock_at(&path, "bot"));
        s.vault = None;
        assert!(s.reset_vault_at(&path));
        assert!(!path.exists());
        assert!(s.unlock_at(&path, "newpass"));
    }

    #[test]
    fn session_starts_with_renderer_on_capture_off() {
        let s = Session::new();
        let f = s.focus.lock().unwrap();
        assert!(f.renderer, "rail is on; Game pane 50 fps is focused_50");
        assert!(!f.capture);
        assert!(f.focused_50);
    }

    #[test]
    fn multibox_toggle_does_not_arm_scatter() {
        let mut s = Session::new();
        s.set_multibox(true);
        assert!(
            !s.scatter.load(Ordering::Relaxed),
            "MultiBox must not arm the stress50 scatter-seed"
        );
        s.set_multibox(false);
        assert!(!s.scatter.load(Ordering::Relaxed));
    }

    #[test]
    fn walk_status_is_dash_when_no_route() {
        let s = Session::new();
        assert_eq!(s.walk_status_text(), "—");
    }

    #[test]
    fn picker_select_does_not_arm_until_confirm() {
        let mut s = Session::new();
        let dest = Tile {
            x: 2,
            z: 2,
            level: 0,
        };
        s.picker_sel = Some(dest);
        assert_eq!(s.walk_status_text(), "—");
        assert!(s.confirm_picker_walk(&open_world(3, 3)));
        assert!(s.walk_status_text().contains("2"));
        assert!(s.picker_sel.is_none());
        assert!(!s.confirm_picker_walk(&open_world(3, 3)));
    }

    #[test]
    fn arm_walk_sets_queued_text() {
        let mut s = Session::new();
        s.arm_walk(Tile {
            x: 3222,
            z: 3222,
            level: 0,
        });
        assert!(s.walk_status_text().contains("3222"));
    }

    #[test]
    fn arm_walk_on_routes_and_arms_focused_traveller() {
        let mut s = Session::new();
        s.focus.lock().unwrap().focused = Some("alice".into());
        let world = open_world(3, 3);
        let dest = Tile {
            x: 2,
            z: 2,
            level: 0,
        };
        s.arm_walk_on(
            &world,
            Tile {
                x: 0,
                z: 0,
                level: 0,
            },
            dest,
        );
        assert_eq!(s.walk_dest, Some(dest), "dest stays stored on success");
        assert!(s.error.is_none(), "a found route clears the error banner");
        let queued = s
            .travellers
            .lock()
            .unwrap()
            .get("alice")
            .expect("focused walk arm exists")
            .lock()
            .unwrap()
            .queued_tile();
        assert_eq!(queued, Some(dest));
    }

    #[test]
    fn arm_walk_on_no_path_stores_dest_and_sets_error() {
        let mut s = Session::new();
        s.focus.lock().unwrap().focused = Some("alice".into());
        // Block the middle column: (1,0), (1,1), (1,2) on the 3x3 world.
        let mut flags = vec![0u32; 9];
        for z in 0..3 {
            flags[z * 3 + 1] = CollisionFlag::WALK_BLOCK_FLAGS as u32;
        }
        let world = NavWorld {
            collision: WorldCollision {
                origin: WorldTile {
                    x: 0,
                    z: 0,
                    level: 0,
                },
                width: 3,
                height: 3,
                walk: nav::collision::pack_walk_u16(&flags),
                flags: None,
            },
            graph: TransportGraph::default(),
        };
        let dest = Tile {
            x: 2,
            z: 1,
            level: 0,
        };
        s.arm_walk_on(
            &world,
            Tile {
                x: 0,
                z: 1,
                level: 0,
            },
            dest,
        );
        assert_eq!(s.walk_dest, Some(dest), "dest stays stored on NoPath");
        let err = s.error.clone().expect("no-path message set");
        assert!(
            err.contains("no path"),
            "short no-path message, got {err:?}"
        );
        assert!(
            s.travellers.lock().unwrap().get("alice").is_none_or(|a| a
                .lock()
                .unwrap()
                .route
                .is_none()),
            "no route must be armed when find fails"
        );
    }

    #[test]
    fn arm_walk_on_ignores_teles_until_allow_teleports() {
        // world: origin cannot walk to dest; a teleport edge can.
        let mut session = Session::new();
        session.focus.lock().unwrap().focused = Some("alice".into());
        // Wall splits the 5x5 between x=1 and x=2 (nav fixture shape), so
        // no walk crosses; only the any-tile teleport edge reaches (4,4).
        let mut flags = vec![0u32; 25];
        for z in 0..5 {
            flags[z * 5 + 1] |= CollisionFlag::W_E as u32;
            flags[z * 5 + 2] |= CollisionFlag::W_W as u32;
        }
        let dest_tile = Tile {
            x: 4,
            z: 4,
            level: 0,
        };
        let dest = WorldTile {
            x: 4,
            z: 4,
            level: 0,
        };
        let mut graph = TransportGraph::default();
        graph.teleports.push(TransportEdge {
            kind: TransportKind::Teleport,
            at: WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            to: dest,
            loc_id: 0,
            option: 0,
            ticks: 3,
            dir: None,
            open_loc_id: None,
            skill_req: vec![],
            item_req: vec![],
            quest_req: vec![],
            varp_req: vec![],
            worn_req: vec![],
        });
        let world = NavWorld {
            collision: WorldCollision {
                origin: WorldTile {
                    x: 0,
                    z: 0,
                    level: 0,
                },
                width: 5,
                height: 5,
                walk: nav::collision::pack_walk_u16(&flags),
                flags: None,
            },
            graph,
        };
        let origin = Tile {
            x: 0,
            z: 0,
            level: 0,
        };
        session.ui.nav.allow_teleports = false;
        session.arm_walk_on(&world, origin, dest_tile);
        assert!(
            session.error.as_ref().unwrap().contains("no path"),
            "walk-only find must not use the teleport edge"
        );
        session.ui.nav.allow_teleports = true;
        session.arm_walk_on(&world, origin, dest_tile);
        assert!(
            session.error.is_none(),
            "allow_teleports routes the teleport"
        );
        let arm = session.travellers.lock().unwrap();
        let route = arm
            .get(&session.focused_name().unwrap())
            .unwrap()
            .lock()
            .unwrap()
            .route
            .clone();
        assert!(route.unwrap().legs.iter().any(|l| matches!(
            l,
            Leg::Transport { edge } if edge.kind == TransportKind::Teleport
        )));
    }

    #[test]
    fn arm_walk_on_uses_find_with_options() {
        // The tele fixture moved to wildy-north coords, with the teleport
        // landing on a wilderness tile: the teleport edge is the only way
        // across the wall, and its landing is inside the zone. Neither
        // flag alone may route — `arm_walk_on` must pass both
        // `ui.nav.allow_teleports` and `ui.nav.allow_wilderness` through
        // to `find_with`.
        let mut session = Session::new();
        session.focus.lock().unwrap().focused = Some("alice".into());
        let mut flags = vec![0u32; 5 * 12];
        for z in 0..12 {
            flags[z * 5 + 1] |= CollisionFlag::W_E as u32;
            flags[z * 5 + 2] |= CollisionFlag::W_W as u32;
        }
        let dest_tile = Tile {
            x: 3102,
            z: 3525,
            level: 0,
        };
        let dest = WorldTile {
            x: 3102,
            z: 3525,
            level: 0,
        };
        let mut graph = TransportGraph::default();
        graph.teleports.push(TransportEdge {
            kind: TransportKind::Teleport,
            at: WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            to: dest,
            loc_id: 0,
            option: 0,
            ticks: 3,
            dir: None,
            open_loc_id: None,
            skill_req: vec![],
            item_req: vec![],
            quest_req: vec![],
            varp_req: vec![],
            worn_req: vec![],
        });
        let world = NavWorld {
            collision: WorldCollision {
                origin: WorldTile {
                    x: 3099,
                    z: 3518,
                    level: 0,
                },
                width: 5,
                height: 12,
                walk: nav::collision::pack_walk_u16(&flags),
                flags: None,
            },
            graph,
        };
        let origin = Tile {
            x: 3100,
            z: 3519,
            level: 0,
        };
        session.ui.nav.allow_teleports = false;
        session.ui.nav.allow_wilderness = false;
        session.arm_walk_on(&world, origin, dest_tile);
        assert!(
            session.error.as_ref().unwrap().contains("no path"),
            "default find must not route into the wilderness"
        );
        session.ui.nav.allow_teleports = true;
        session.arm_walk_on(&world, origin, dest_tile);
        assert!(
            session.error.as_ref().unwrap().contains("no path"),
            "a teleport landing inside the wilderness stays blocked without allow_wilderness"
        );
        session.ui.nav.allow_wilderness = true;
        session.arm_walk_on(&world, origin, dest_tile);
        assert!(
            session.error.is_none(),
            "both UI flags must route the teleport into the wilderness"
        );
        let arm = session.travellers.lock().unwrap();
        let route = arm
            .get(&session.focused_name().unwrap())
            .unwrap()
            .lock()
            .unwrap()
            .route
            .clone();
        assert!(route.unwrap().legs.iter().any(|l| matches!(
            l,
            Leg::Transport { edge } if edge.kind == TransportKind::Teleport
        )));
    }

    #[test]
    fn arm_walk_on_without_focus_skips_route_but_stores_dest() {
        let mut s = Session::new();
        let world = open_world(3, 3);
        let dest = Tile {
            x: 2,
            z: 2,
            level: 0,
        };
        s.arm_walk_on(
            &world,
            Tile {
                x: 0,
                z: 0,
                level: 0,
            },
            dest,
        );
        assert_eq!(s.walk_dest, Some(dest));
        assert!(
            s.travellers.lock().unwrap().is_empty(),
            "no focused name to key a walk arm"
        );
    }

    #[test]
    fn arm_walk_on_success_bumps_route_gen() {
        let mut s = Session::new();
        s.focus.lock().unwrap().focused = Some("alice".into());
        let world = open_world(3, 3);
        assert_eq!(s.route_gen(), 0);
        s.arm_walk_on(
            &world,
            Tile {
                x: 0,
                z: 0,
                level: 0,
            },
            Tile {
                x: 2,
                z: 2,
                level: 0,
            },
        );
        assert_ne!(s.route_gen(), 0, "a new arm must bump the overlay gen");
    }

    #[test]
    fn sync_walk_status_copies_queued_and_clears_dest_on_arrived() {
        let mut s = Session::new();
        s.focus.lock().unwrap().focused = Some("alice".into());
        s.statuses.push(SlotStatus {
            username: "alice".into(),
            ..SlotStatus::default()
        });
        let world = open_world(3, 3);
        let dest = Tile {
            x: 2,
            z: 2,
            level: 0,
        };
        s.arm_walk_on(
            &world,
            Tile {
                x: 0,
                z: 0,
                level: 0,
            },
            dest,
        );
        s.sync_walk_status();
        assert_eq!(
            (
                s.statuses[0].walk_x,
                s.statuses[0].walk_z,
                s.statuses[0].walk_level
            ),
            (2, 2, 0)
        );
        // The slot hook clears the route and flags walk_clear on Arrived.
        s.travellers
            .lock()
            .unwrap()
            .get("alice")
            .unwrap()
            .lock()
            .unwrap()
            .route = None;
        s.walk_clear
            .store(true, std::sync::atomic::Ordering::Relaxed);
        s.sync_walk_status();
        assert_eq!(s.walk_status_text(), "—");
        assert_eq!(
            (
                s.statuses[0].walk_x,
                s.statuses[0].walk_z,
                s.statuses[0].walk_level
            ),
            (-1, -1, -1)
        );
    }

    #[test]
    fn select_bumps_route_gen_only_on_focus_change() {
        let mut s = Session::new();
        assert_eq!(s.route_gen(), 0);
        s.select("alice");
        assert_eq!(s.route_gen(), 1);
        assert_eq!(s.focused_name().as_deref(), Some("alice"));
        s.select("alice");
        assert_eq!(s.route_gen(), 1, "re-selecting the focused name is a no-op");
        s.select("bob");
        assert_eq!(s.route_gen(), 2);
    }

    #[test]
    fn focused_tile_is_none_without_status() {
        let s = Session::new();
        s.focus.lock().unwrap().focused = Some("alice".into());
        assert_eq!(s.focused_tile(), None, "no status rows yet");
    }

    #[test]
    fn combo_index_is_none_when_unfocused() {
        let names = vec!["alice".into(), "bob".into()];
        assert_eq!(combo_index(None, &names), None);
        assert_eq!(combo_index(Some("alice"), &names), Some(0));
        assert_eq!(combo_index(Some("bob"), &names), Some(1));
        assert_eq!(combo_index(Some("carol"), &names), None);
    }

    #[test]
    fn focus_first_profile_selects_first_vault_name() {
        crate::ui_state::save(&crate::ui_state::PanelUiState::default());
        let path = tmp_vault("focus-first.vault");
        let mut s = Session::new();
        s.vault = Some(Vault::create(&path, "bot").unwrap());
        s.vault
            .as_mut()
            .unwrap()
            .upsert(profile("alice", "pw", 42))
            .unwrap();
        s.vault
            .as_mut()
            .unwrap()
            .upsert(profile("bob", "pw", 43))
            .unwrap();
        s.focus_first_profile();
        assert_eq!(s.focused_name().as_deref(), Some("alice"));
        assert_eq!(s.cred_user, "alice");
        assert!(s.slots.contains_key("alice"));
        assert!(
            !s.slots.contains_key("bob"),
            "parked vault rows must not start a Client"
        );
    }

    #[test]
    fn focus_first_prefers_last_focus() {
        let path = tmp_vault("focus-last.vault");
        let mut s = Session::new();
        s.vault = Some(Vault::create(&path, "bot").unwrap());
        s.vault
            .as_mut()
            .unwrap()
            .upsert(profile("alice", "pw", 42))
            .unwrap();
        s.vault
            .as_mut()
            .unwrap()
            .upsert(profile("bob", "pw", 43))
            .unwrap();
        crate::ui_state::save(&crate::ui_state::PanelUiState {
            last_focus: Some("bob".into()),
            ..Default::default()
        });
        s.focus_first_profile();
        assert_eq!(s.focused_name().as_deref(), Some("bob"));
        assert_eq!(crate::ui_state::load().last_focus.as_deref(), Some("bob"));
    }

    #[test]
    fn select_saves_last_focus() {
        crate::ui_state::save(&crate::ui_state::PanelUiState::default());
        let path = tmp_vault("select-last-focus.vault");
        let mut s = Session::new();
        s.vault = Some(Vault::create(&path, "bot").unwrap());
        s.vault
            .as_mut()
            .unwrap()
            .upsert(profile("alice", "pw", 42))
            .unwrap();
        s.select("alice");
        assert_eq!(crate::ui_state::load().last_focus.as_deref(), Some("alice"));
    }

    #[test]
    fn set_multibox_restores_last_focus_when_focus_not_on_wall() {
        let path = tmp_vault("multibox-last-focus.vault");
        let mut s = Session::new();
        s.vault = Some(Vault::create(&path, "bot").unwrap());
        s.vault
            .as_mut()
            .unwrap()
            .upsert(profile("alice", "pw", 42))
            .unwrap();
        s.vault
            .as_mut()
            .unwrap()
            .upsert(profile("bob", "pw", 43))
            .unwrap();
        s.select("alice");
        s.wall.load("bob");
        crate::ui_state::save(&crate::ui_state::PanelUiState {
            last_focus: Some("bob".into()),
            ..Default::default()
        });
        // Focused alice is not a wall member; MultiBox-on should pick bob.
        s.set_multibox(true);
        assert_eq!(s.focused_name().as_deref(), Some("bob"));
        assert!(s.wall.members.iter().any(|m| m == "bob"));
    }

    #[test]
    fn select_spawns_parked_profile_once() {
        let path = tmp_vault("select-spawn.vault");
        let mut s = Session::new();
        s.vault = Some(Vault::create(&path, "bot").unwrap());
        s.vault
            .as_mut()
            .unwrap()
            .upsert(profile("alice", "pw", 42))
            .unwrap();
        s.vault
            .as_mut()
            .unwrap()
            .upsert(profile("bob", "pw", 43))
            .unwrap();
        s.select("alice");
        assert_eq!(s.slots.len(), 1);
        s.select("bob");
        assert_eq!(s.slots.len(), 2);
        s.select("alice");
        assert_eq!(s.slots.len(), 2);
    }

    #[test]
    fn flat_model_spawns_every_member_as_a_client() {
        let path = tmp_vault("flat-spawn.vault");
        let mut s = Session::new();
        assert!(s.unlock_at(&path, "bot"));
        for (n, uid) in [("alice", 1), ("bob", 2), ("carol", 3)] {
            s.vault
                .as_mut()
                .unwrap()
                .upsert(profile(n, "pw", uid))
                .unwrap();
        }
        s.select("alice");
        s.load("bob");
        s.load("carol");
        // ensure_slot registers the IO map synchronously; wait only for the
        // slot threads to publish their status rows.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while std::time::Instant::now() < deadline {
            if s.play.as_ref().unwrap().statuses().len() == 3 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(s.slots.len(), 3, "every wall member owns a FrameBuf slot");
        assert_eq!(
            s.play.as_ref().unwrap().statuses().len(),
            3,
            "one full Client slot per profile — no lean channels"
        );
        assert!(
            s.play.as_ref().unwrap().arm("carol").is_some(),
            "every member has a control arm"
        );
        // Focus is pure bookkeeping: selecting bob redirects the sampled
        // slot without touching a socket.
        s.select("bob");
        assert_eq!(s.focused_name().as_deref(), Some("bob"));
        assert_eq!(s.play.as_ref().unwrap().focused().as_deref(), Some("bob"));
        assert_eq!(
            s.play.as_ref().unwrap().statuses().len(),
            3,
            "focus does not swap sockets; every slot stays up"
        );
        // No `stop_slot` joins here: the slot threads sit in `maininit`'s
        // bounded HTTP retry (host-play shrinks it only under its own
        // `#[cfg(test)]`), so a join would block the suite for minutes.
        // The threads are detached and die at process exit.
    }

    #[test]
    fn sidecar_select_does_not_restart_when_game_is_highmem() {
        let path = tmp_vault("select-no-restart.vault");
        let mut s = Session::new();
        s.persist_ui = false;
        s.vault = Some(Vault::create(&path, "bot").unwrap());
        s.vault
            .as_mut()
            .unwrap()
            .upsert(profile("alice", "pw", 42))
            .unwrap();
        s.vault
            .as_mut()
            .unwrap()
            .upsert(profile("bob", "pw", 43))
            .unwrap();
        s.ui.lowmem = false;
        s.select("alice");
        s.load("bob");
        let alice_px = std::sync::Arc::as_ptr(&s.slots.get("alice").unwrap().pixels);
        let bob_px = std::sync::Arc::as_ptr(&s.slots.get("bob").unwrap().pixels);
        s.select("bob");
        assert_eq!(s.focused_name().as_deref(), Some("bob"));
        assert_eq!(
            std::sync::Arc::as_ptr(&s.slots.get("alice").unwrap().pixels),
            alice_px
        );
        assert_eq!(
            std::sync::Arc::as_ptr(&s.slots.get("bob").unwrap().pixels),
            bob_px
        );
        let log = s.log_by.lock().unwrap();
        assert!(!log.values().flatten().any(|l| l.contains("slot restarted")));
    }

    #[test]
    fn sidecar_select_does_not_restart_when_game_is_cpu() {
        let path = tmp_vault("select-no-restart-cpu.vault");
        let mut s = Session::new();
        s.persist_ui = false;
        s.vault = Some(Vault::create(&path, "bot").unwrap());
        s.vault
            .as_mut()
            .unwrap()
            .upsert(profile("alice", "pw", 42))
            .unwrap();
        s.vault
            .as_mut()
            .unwrap()
            .upsert(profile("bob", "pw", 43))
            .unwrap();
        s.ui.raster = vault::RasterMode::Cpu;
        s.select("alice");
        s.load("bob");
        let alice_px = std::sync::Arc::as_ptr(&s.slots.get("alice").unwrap().pixels);
        s.select("bob");
        assert_eq!(
            std::sync::Arc::as_ptr(&s.slots.get("alice").unwrap().pixels),
            alice_px
        );
    }

    #[test]
    fn logout_all_arms_every_wall_member() {
        let path = tmp_vault("logout-all-flat.vault");
        let mut s = Session::new();
        assert!(s.unlock_at(&path, "bot"));
        for (n, uid) in [("alice", 1), ("bob", 2)] {
            s.vault
                .as_mut()
                .unwrap()
                .upsert(profile(n, "pw", uid))
                .unwrap();
        }
        s.select("alice");
        s.load("bob");
        s.wall.load("alice");
        s.wall.load("bob");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while std::time::Instant::now() < deadline {
            if s.play.as_ref().unwrap().arm("bob").is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        s.logout_all();
        assert!(
            s.play
                .as_ref()
                .unwrap()
                .arm("alice")
                .unwrap()
                .want_logout
                .load(Ordering::Relaxed),
            "the focused member must logout"
        );
        assert!(
            s.play
                .as_ref()
                .unwrap()
                .arm("bob")
                .unwrap()
                .want_logout
                .load(Ordering::Relaxed),
            "every wall member must logout"
        );
        // No `stop_slot` joins: the slot threads stay in `maininit`'s
        // bounded HTTP retry (see `flat_model_spawns_every_member_as_a_client`).
    }

    #[test]
    fn headed_stress_spawns_every_member_and_focuses_s00() {
        crate::ui_state::save(&crate::ui_state::PanelUiState {
            last_focus: Some("s02".into()),
            ..Default::default()
        });
        let mut s = Session::new();
        s.live_prepare_stress(3, false).expect("prepare");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while std::time::Instant::now() < deadline {
            if s.play.as_ref().unwrap().statuses().len() == 3 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(s.slots.len(), 3, "every member owns its own Client slot");
        assert_eq!(
            s.focused_name().as_deref(),
            Some("s00"),
            "focused slot must be s00, not last_focus s02"
        );
        assert_eq!(s.tv_name().as_deref(), Some("s00"));
        let front = s.play.as_ref().unwrap().login_queue_uids();
        assert_eq!(
            front.first().copied(),
            Some(274_000_100),
            "s00 uid must be FIFO head, got {front:?}"
        );
        assert!(
            s.play
                .as_ref()
                .unwrap()
                .arm("s00")
                .unwrap()
                .want_login
                .load(Ordering::Relaxed),
            "the focused slot arms immediately"
        );
        assert!(
            s.focus.lock().unwrap().only_render_selected,
            "RAM watch is cap-only (Game paints, rail skip-paint)"
        );
        assert!(
            !s.focus.lock().unwrap().live_full_rate,
            "RAM watch does not raise sidecar/game to 50 fps overlay"
        );
        assert!(
            s.scatter.load(Ordering::Relaxed),
            "stress wall scatter-seeds after scene 2"
        );
        // No `stop_slot` joins: the slot threads stay in `maininit`'s
        // bounded HTTP retry (see `flat_model_spawns_every_member_as_a_client`).
    }

    #[test]
    fn login_all_arms_every_wall_member() {
        crate::ui_state::save(&crate::ui_state::PanelUiState {
            last_focus: None,
            ..Default::default()
        });
        let mut s = Session::new();
        s.live_prepare_stress(2, false).expect("prepare");
        assert!(
            s.play
                .as_ref()
                .unwrap()
                .arm("s01")
                .unwrap()
                .want_login
                .load(Ordering::Relaxed),
            "login all arms every member immediately (the FIFO serializes)"
        );
        // No `stop_slot` joins (see `flat_model_spawns_every_member_as_a_client`).
    }

    #[test]
    fn headed_stress_full_paints_every_member_at_50fps() {
        crate::ui_state::save(&crate::ui_state::PanelUiState {
            last_focus: None,
            ..Default::default()
        });
        let mut s = Session::new();
        s.live_prepare_stress(2, true).expect("prepare");
        let f = s.focus.lock().unwrap();
        assert!(
            !f.only_render_selected,
            "full-rate 50 paints sidecar tiles, not cap-only"
        );
        assert!(f.live_full_rate, "full-rate overlay on Game + sidecar");
        drop(f);
        for name in ["s00", "s01"] {
            let slot = s.slots.get(name).expect("slot");
            assert!(
                slot.input.full_rate(),
                "{name} must run the 50 fps cadence"
            );
        }
    }

    /// RAM watch members must be raster Off so a flipped only-render-
    /// selected cannot attach 49 GPU heads. s00 stays GPU (the Game pane).
    #[test]
    fn stress50_rail_members_are_raster_off() {
        crate::ui_state::save(&crate::ui_state::PanelUiState {
            last_focus: None,
            ..Default::default()
        });
        let mut s = Session::new();
        s.live_prepare_stress(3, false).expect("prepare");
        let f = s.focus.lock().unwrap();
        assert_eq!(f.renderer_by.get("s00").copied(), Some(true));
        assert_eq!(
            f.renderer_by.get("s01").copied(),
            Some(false),
            "s01 must not be able to grow a GPU head"
        );
        assert_eq!(f.renderer_by.get("s02").copied(), Some(false));
        assert!(draw_for_slot(&f, "s00"));
        assert!(!draw_for_slot(&f, "s01"));
        drop(f);
        s.focus.lock().unwrap().only_render_selected = false;
        let f = s.focus.lock().unwrap();
        assert!(
            !draw_for_slot(&f, "s01"),
            "raster Off must keep rail members unheaded even with render-all"
        );
        assert!(draw_for_slot(&f, "s00"));
    }

    #[test]
    fn live_prepare_script_boots_the_seed_profile_and_installs_runner() {
        crate::ui_state::save(&crate::ui_state::PanelUiState {
            last_focus: None,
            ..Default::default()
        });
        let mut s = Session::new();
        let scenario = scenario::get("walk").expect("walk scenario in registry");
        s.live_prepare_script(scenario).expect("prepare");
        let play = s.play.as_ref().expect("play started");
        assert!(
            play.arm("test").unwrap().want_login.load(Ordering::Relaxed),
            "login all arms the seed profile's handshake"
        );
        let runner = s.scenario.lock().unwrap();
        let runner = runner.as_ref().expect("scenario runner installed");
        assert_eq!(runner.profile_name(), "test");
        assert!(
            matches!(runner.status(), scenario::RunnerStatus::Seeding),
            "a fresh runner holds in seeding until ingame scene 2"
        );
        assert!(
            runner.drives("test") && !runner.drives("test2"),
            "the runner ticks only its seed profile's slot"
        );
        // No `stop_slot` joins (see `flat_model_spawns_every_member_as_a_client`).
    }

    #[test]
    fn live_prepare_script_enables_multibox_for_a_fleet_only() {
        crate::ui_state::save(&crate::ui_state::PanelUiState {
            last_focus: None,
            ..Default::default()
        });
        let mut s = Session::new();
        let fleet = scenario::get("nav_door").expect("nav_door is registered");
        assert!(
            fleet.seed.profiles.len() > 1,
            "nav_door is a two-profile fleet"
        );
        s.live_prepare_script(fleet).expect("prepare");
        assert!(
            s.multibox,
            "a fleet (2+ seed profiles) opens the MultiBox wall"
        );
        assert!(
            s.focus.lock().unwrap().wall_open,
            "multibox mirrors onto the focus so every bot rasters"
        );
        assert!(
            s.wall.members.iter().any(|m| m == "test2"),
            "every seed profile is a wall member"
        );
        assert!(!s.wall.chooser_open, "live keeps the chooser closed");
        // No `stop_slot` joins (see `flat_model_spawns_every_member_as_a_client`).

        crate::ui_state::save(&crate::ui_state::PanelUiState {
            last_focus: None,
            ..Default::default()
        });
        let mut s = Session::new();
        let solo = scenario::get("walk").expect("walk is registered");
        assert_eq!(solo.seed.profiles.len(), 1);
        s.live_prepare_script(solo).expect("prepare");
        assert!(!s.multibox, "a solo scenario keeps the single-bot boot");
        assert!(
            !s.focus.lock().unwrap().wall_open,
            "no wall members, no extra rasters"
        );
        // No `stop_slot` joins (see `flat_model_spawns_every_member_as_a_client`).
    }

    #[test]
    fn live_prepare_script_does_not_write_last_focus() {
        crate::ui_state::save(&crate::ui_state::PanelUiState {
            last_focus: Some("alice".into()),
            ..Default::default()
        });
        let mut s = Session::new();
        let fleet = scenario::get("nav_door").expect("nav_door");
        s.live_prepare_script(fleet).expect("prepare");
        assert_eq!(
            crate::ui_state::load().last_focus.as_deref(),
            Some("alice"),
            "live boot must not clobber the operator last profile"
        );
    }

    #[test]
    fn live_prepare_nav_door_applies_full_rate_and_leaves_sidecar_off() {
        crate::ui_state::save(&crate::ui_state::PanelUiState {
            last_focus: None,
            ..Default::default()
        });
        let mut s = Session::new();
        let fleet = scenario::get("nav_door").expect("nav_door");
        s.live_prepare_script(fleet).expect("prepare");
        let f = s.focus.lock().unwrap();
        assert!(f.live_full_rate);
        assert!(!f.only_render_selected, "closer must paint");
        assert!(!f.capture);
        assert!(f.renderer);
        assert!(!f.sidecar_50, "sidecar stays the operator knob");
    }

    #[test]
    fn live_prepare_nav_full_runner_already_has_deadline_and_shot() {
        crate::ui_state::save(&crate::ui_state::PanelUiState {
            last_focus: None,
            ..Default::default()
        });
        let mut s = Session::new();
        s.live_prepare_script(scenario::get("nav_full").unwrap())
            .expect("prepare");
        let runner = s.scenario.lock().unwrap();
        let runner = runner.as_ref().expect("runner");
        assert_eq!(runner.deadline(), Duration::from_secs(360));
        assert_eq!(runner.terminal_shot(), Some("nav_full terminal"));
    }

    #[test]
    fn live_prepare_smoke_runner_already_has_the_300s_deadline() {
        crate::ui_state::save(&crate::ui_state::PanelUiState {
            last_focus: None,
            ..Default::default()
        });
        let mut s = Session::new();
        s.live_prepare_script(scenario::get("render_smoke").unwrap())
            .expect("prepare");
        let runner = s.scenario.lock().unwrap();
        let runner = runner.as_ref().expect("runner");
        assert_eq!(runner.deadline(), Duration::from_secs(300));
    }

    #[test]
    fn live_force_layers_does_not_write_panel_ui() {
        crate::ui_state::save(&crate::ui_state::PanelUiState {
            last_focus: None,
            ..Default::default()
        });
        let mut s = Session::new();
        s.nav_live_force_layers = true;
        assert!(
            crate::nav_settings::effective(&s.ui.nav, s.nav_live_force_layers).show_nav_path,
            "the live force drives the effective paint layers at runtime"
        );
        // A save/load roundtrip of the panel prefs must not persist the
        // forced layer bools: `nav_live_force_layers` is session-only.
        let ui = crate::ui_state::load();
        assert!(
            !ui.nav.show_nav_path,
            "forced layers must never reach panel-ui.json"
        );
        // And a live boot of a nav_debug scenario arms the force without
        // writing the prefs.
        let mut s = Session::new();
        s.live_prepare_script(scenario::get("nav_door").unwrap())
            .expect("prepare");
        assert!(
            s.nav_live_force_layers,
            "nav_debug scenario arms the live overlay"
        );
        let ui = crate::ui_state::load();
        assert!(
            !ui.nav.show_nav_path,
            "live boot of a nav_debug scenario never writes the prefs"
        );
    }

    #[test]
    fn live_full_rate_sync_raises_focus_and_members() {
        let mut s = Session::new();
        let a_in = SlotInput::new();
        let b_in = SlotInput::new();
        s.slots.insert(
            "a".into(),
            SlotIo {
                input: Arc::clone(&a_in),
                pixels: FrameBuf::new(),
            },
        );
        s.slots.insert(
            "b".into(),
            SlotIo {
                input: Arc::clone(&b_in),
                pixels: FrameBuf::new(),
            },
        );
        {
            let mut f = s.focus.lock().unwrap();
            f.focused = Some("a".into());
            f.only_render_selected = false;
            f.wall_open = true;
            f.wall = vec!["a".into(), "b".into()];
            f.renderer_by =
                std::collections::HashMap::from([("a".into(), true), ("b".into(), true)]);
            f.sidecar_50 = false;
            f.live_full_rate = true;
        }
        s.sync_sidecar_cadence();
        assert!(a_in.full_rate(), "focused slot is 50 fps via live overlay");
        assert!(b_in.full_rate(), "member is 50 fps via live overlay");
    }

    #[test]
    fn queue_place_falls_back_to_fifo_head_when_focus_already_granted() {
        let mut s = Session::new();
        s.focus.lock().unwrap().focused = Some("s00".into());
        s.statuses.push(SlotStatus {
            username: "s00".into(),
            queue_position: -1,
            queue_total: -1,
            ..SlotStatus::default()
        });
        s.statuses.push(SlotStatus {
            username: "s01".into(),
            queue_position: 1,
            queue_total: 49,
            ..SlotStatus::default()
        });
        s.statuses.push(SlotStatus {
            username: "s02".into(),
            queue_position: 2,
            queue_total: 49,
            ..SlotStatus::default()
        });
        assert_eq!(s.focused_queue(), None);
        assert_eq!(
            s.queue_place(),
            Some((1, 49)),
            "Game pane still shows k of n"
        );
    }

    #[test]
    fn focus_first_profile_noop_when_empty() {
        let path = tmp_vault("focus-empty.vault");
        let mut s = Session::new();
        s.vault = Some(Vault::create(&path, "bot").unwrap());
        s.focus_first_profile();
        assert!(s.focused_name().is_none());
    }

    #[test]
    fn maybe_send_click_is_noop_without_tx() {
        maybe_send_click(&None, 1.0, 1.0, 765.0, 503.0);
    }

    #[test]
    fn stream_capture_is_noop_without_tx() {
        stream_capture(
            &None,
            1.0,
            1.0,
            765.0,
            503.0,
            true,
            true,
            true,
            true,
            &[(true, b'a' as i32)],
        );
    }

    #[test]
    fn stream_capture_sends_move_then_down() {
        let (tx, rx) = std::sync::mpsc::channel();
        stream_capture(
            &Some(tx),
            0.0,
            0.0,
            765.0,
            503.0,
            true,
            false,
            false,
            false,
            &[],
        );
        match rx.try_recv() {
            Ok(InputEv::Move { x, y }) => assert_eq!((x, y), (0, 0)),
            other => panic!("{other:?}"),
        }
        match rx.try_recv() {
            Ok(InputEv::Down { button, x, y }) => assert_eq!((button, x, y), (1, 0, 0)),
            other => panic!("{other:?}"),
        }
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn stream_capture_sends_right_up_and_key() {
        let (tx, rx) = std::sync::mpsc::channel();
        stream_capture(
            &Some(tx),
            0.0,
            0.0,
            765.0,
            503.0,
            false,
            true,
            true,
            false,
            &[(true, 10)],
        );
        let evs: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert!(matches!(evs[0], InputEv::Move { x: 0, y: 0 }));
        assert!(matches!(
            evs[1],
            InputEv::Down {
                button: 2,
                x: 0,
                y: 0
            }
        ));
        assert!(matches!(evs[2], InputEv::Up));
        assert!(matches!(evs[3], InputEv::Key { down: true, ch: 10 }));
    }

    #[test]
    fn closing_game_pane_turns_capture_off() {
        let mut s = Session::new();
        s.select("alice");
        s.set_capture(true);
        assert!(s.focus.lock().unwrap().capture);
        s.set_game_pane_open(false);
        let f = s.focus.lock().unwrap();
        assert!(!f.game_pane_open);
        assert!(!f.capture);
        assert!(s.capture_tx.is_none());
    }

    #[test]
    fn opening_game_pane_sets_flag_without_capture() {
        let mut s = Session::new();
        s.set_game_pane_open(false);
        s.set_game_pane_open(true);
        let f = s.focus.lock().unwrap();
        assert!(f.game_pane_open);
        assert!(!f.capture);
    }

    #[test]
    fn maybe_send_click_sends_when_tx_present() {
        let (tx, rx) = std::sync::mpsc::channel();
        maybe_send_click(&Some(tx), 0.0, 0.0, 765.0, 503.0);
        match rx.try_recv() {
            Ok(InputEv::Down { x, y, .. }) => assert_eq!((x, y), (0, 0)),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn maybe_send_click_outside_image_sends_nothing() {
        let (tx, rx) = std::sync::mpsc::channel();
        maybe_send_click(&Some(tx), -5.0, 10.0, 765.0, 503.0);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn login_focuses_the_named_profile() {
        let mut s = Session::new();
        assert!(s.focused_name().is_none());
        s.login("alice");
        assert_eq!(s.focused_name().as_deref(), Some("alice"));
    }

    #[test]
    fn login_after_logout_rearms_handshake_on_fake_arm() {
        // Logout latches + clears want_login; Log in must call arm_login_all
        // (clear latch, want_login, cancel want_logout) then select.
        let mut s = Session::new();
        let mut play = host_play::run_with_io(
            &host_play::PlayOptions {
                host: "127.0.0.1".into(),
                port: 43594,
                cache_dir: "/tmp".into(),
                lowmem: true,
                mainland: false,
            },
            vec![],
            |_| (None, None),
            |_, _| {},
        );
        let arm = SlotArm::new(7, false);
        arm.latch.store(true, Ordering::Relaxed);
        arm.want_login.store(false, Ordering::Relaxed);
        arm.want_logout.store(true, Ordering::Relaxed);
        play.attach_arm("alice", Arc::clone(&arm));
        s.play = Some(play);
        s.wall.load("alice");
        s.logout("alice");
        assert!(s.wall.latch.contains("alice"));

        s.login("alice");

        assert!(arm.want_login.load(Ordering::Relaxed));
        assert!(!arm.want_logout.load(Ordering::Relaxed));
        assert!(!arm.latch.load(Ordering::Relaxed));
        assert!(!s.wall.latch.contains("alice"));
        assert_eq!(s.focused_name().as_deref(), Some("alice"));
    }

    #[test]
    fn arm_login_all_cancels_pending_logout() {
        // A title-screen member keeps want_logout=true (the slot body only
        // clears it when it observes ingame); Login all must cancel it or
        // the handshake would be undone on the first ingame frame.
        let arm = SlotArm::new(7, false);
        arm.latch.store(true, Ordering::Relaxed);
        arm.want_logout.store(true, Ordering::Relaxed);
        arm_login_all(&arm);
        assert!(arm.want_login.load(Ordering::Relaxed));
        assert!(!arm.want_logout.load(Ordering::Relaxed));
        assert!(!arm.latch.load(Ordering::Relaxed));
    }

    #[test]
    fn select_syncs_credentials_fields_from_focused_profile() {
        let path = tmp_vault("select-sync.vault");
        let mut s = Session::new();
        s.vault = Some(Vault::create(&path, "bot").unwrap());
        s.vault
            .as_mut()
            .unwrap()
            .upsert(profile("alice", "pw", 42))
            .unwrap();

        s.select("alice");
        assert_eq!(s.cred_user, "alice");
        assert_eq!(s.cred_pass, "pw");
    }

    #[test]
    fn save_credentials_upserts_under_username_key_keeping_uid() {
        let path = tmp_vault("save-creds.vault");
        let mut s = Session::new();
        s.vault = Some(Vault::create(&path, "bot").unwrap());
        s.vault
            .as_mut()
            .unwrap()
            .upsert(profile("alice", "oldpass", 42))
            .unwrap();

        s.cred_user = "alice".into();
        s.cred_pass = "newpass".into();
        assert!(s.save_credentials());

        let p = s.vault.as_ref().unwrap().get("alice").unwrap();
        assert_eq!(p.password, "newpass");
        assert_eq!(p.uid, 42, "save must keep the existing uid");
    }

    #[test]
    fn save_credentials_creates_new_profile_when_username_is_new() {
        let path = tmp_vault("new-user.vault");
        let mut s = Session::new();
        s.vault = Some(Vault::create(&path, "bot").unwrap());
        s.vault
            .as_mut()
            .unwrap()
            .upsert(profile("alice", "pw", 42))
            .unwrap();

        s.cred_user = "bob".into();
        s.cred_pass = "bobpass".into();
        assert!(s.save_credentials());
        assert_eq!(s.focused_name().as_deref(), Some("bob"));
        assert!(s.slots.contains_key("bob"));

        let p = s.vault.as_ref().unwrap().get("bob").unwrap();
        assert_eq!(p.password, "bobpass");
        assert_ne!(p.uid, 42, "a new profile gets a fresh uid");
        assert_eq!(
            s.vault.as_ref().unwrap().get("alice").unwrap().password,
            "pw",
            "saving a new username must not touch existing profiles"
        );
    }

    #[test]
    fn save_credentials_rejects_empty_username() {
        let path = tmp_vault("empty-user.vault");
        let mut s = Session::new();
        s.vault = Some(Vault::create(&path, "bot").unwrap());
        s.cred_user = "  ".into();
        s.cred_pass = "x".into();
        assert!(!s.save_credentials());
        assert!(s.error.is_some());
    }

    #[test]
    fn save_credentials_without_focus_upserts_spawns_and_selects() {
        let path = tmp_vault("empty-first-run.vault");
        let mut s = Session::new();
        s.vault = Some(Vault::create(&path, "bot").unwrap());
        assert!(s.focused_name().is_none());
        s.cred_user = "test".into();
        s.cred_pass = "test".into();
        assert!(s.save_credentials());
        assert!(s.vault.as_ref().unwrap().get("test").is_some());
        assert_eq!(s.focused_name().as_deref(), Some("test"));
        assert!(s.slots.contains_key("test"));
    }

    #[test]
    fn save_credentials_does_not_duplicate_running_slot() {
        let path = tmp_vault("no-dup-slot.vault");
        let mut s = Session::new();
        s.vault = Some(Vault::create(&path, "bot").unwrap());
        s.vault
            .as_mut()
            .unwrap()
            .upsert(profile("alice", "pw", 42))
            .unwrap();
        s.slots.insert(
            "alice".into(),
            SlotIo {
                input: SlotInput::new(),
                pixels: FrameBuf::new(),
            },
        );
        s.cred_user = "alice".into();
        s.cred_pass = "newpw".into();
        assert!(s.save_credentials());
        assert_eq!(s.slots.len(), 1);
        assert_eq!(s.focused_name().as_deref(), Some("alice"));
    }

    #[test]
    fn clear_credentials_empties_fields_but_keeps_vault() {
        let path = tmp_vault("clear-creds.vault");
        let mut s = Session::new();
        s.vault = Some(Vault::create(&path, "bot").unwrap());
        s.vault
            .as_mut()
            .unwrap()
            .upsert(profile("alice", "pw", 42))
            .unwrap();
        s.select("alice");
        s.cred_pass = "edited".into();

        s.clear_credentials();
        assert!(s.cred_user.is_empty());
        assert!(s.cred_pass.is_empty());
        assert!(
            s.vault.as_ref().unwrap().get("alice").is_some(),
            "clear must not delete the vault profile"
        );
    }

    #[test]
    fn begin_edit_profile_loads_fields_and_opens_chooser() {
        let path = tmp_vault("edit-profile.vault");
        let mut s = Session::new();
        s.vault = Some(Vault::create(&path, "bot").unwrap());
        s.vault
            .as_mut()
            .unwrap()
            .upsert(profile("alice", "secret", 42))
            .unwrap();

        s.begin_edit_profile(Some("alice"));
        assert!(s.wall.chooser_open);
        assert_eq!(s.chooser_edit.as_deref(), Some("alice"));
        assert_eq!(s.cred_user, "alice");
        assert_eq!(s.cred_pass, "secret");

        s.begin_edit_profile(None);
        assert_eq!(s.chooser_edit.as_deref(), Some(""));
        assert!(s.cred_user.is_empty());
        assert!(s.cred_pass.is_empty());

        s.cancel_edit_profile();
        assert!(s.chooser_edit.is_none());
        assert!(s.wall.chooser_open, "cancel leaves the picker open");
    }

    #[test]
    fn set_multibox_off_cancels_picker_edit() {
        let path = tmp_vault("edit-off.vault");
        let mut s = Session::new();
        s.vault = Some(Vault::create(&path, "bot").unwrap());
        s.vault
            .as_mut()
            .unwrap()
            .upsert(profile("alice", "pw", 42))
            .unwrap();
        s.begin_edit_profile(Some("alice"));
        s.set_multibox(true);
        s.set_multibox(false);
        assert!(s.chooser_edit.is_none());
        assert!(!s.wall.chooser_open);
    }

    #[test]
    fn set_multibox_wires_wall_open_and_off_clears_grid() {
        let mut s = Session::new();
        assert!(!s.multibox);
        assert!(!s.focus.lock().unwrap().wall_open);
        s.set_multibox(true);
        assert!(s.multibox);
        assert!(
            s.focus.lock().unwrap().wall_open,
            "rail or grid: wall is open"
        );
        s.set_grid(true);
        assert!(s.wall.grid);
        assert!(
            s.focus.lock().unwrap().wall_open,
            "grid is a submode of MultiBox; wall_open stays on"
        );
        s.set_multibox(false);
        assert!(!s.multibox);
        assert!(!s.focus.lock().unwrap().wall_open, "extra rasters stop");
        assert!(!s.wall.grid, "MultiBox off clears grid");
    }

    #[test]
    fn set_grid_is_noop_while_multibox_off() {
        let mut s = Session::new();
        s.set_grid(true);
        assert!(!s.wall.grid);
    }

    #[test]
    fn multibox_on_never_latches_a_tv_mode() {
        let mut s = Session::new();
        s.set_multibox(true);
        s.set_grid(true);
        assert!(s.wall.grid, "every member is a full Client; grid works");
        s.set_multibox(false);
        assert!(!s.wall.grid);
    }

    #[test]
    fn set_auto_login_upserts_without_spawning() {
        let path = tmp_vault("auto-login.vault");
        let mut s = Session::new();
        s.vault = Some(Vault::create(&path, "bot").unwrap());
        s.vault
            .as_mut()
            .unwrap()
            .upsert(profile("alice", "pw", 42))
            .unwrap();
        assert!(s.set_auto_login("alice", true));
        assert!(
            s.vault
                .as_ref()
                .unwrap()
                .get("alice")
                .unwrap()
                .settings
                .auto_login
        );
        assert!(s.slots.is_empty(), "set_auto_login must not spawn a slot");
        assert!(s.set_auto_login("alice", false));
        assert!(
            !s.vault
                .as_ref()
                .unwrap()
                .get("alice")
                .unwrap()
                .settings
                .auto_login
        );
    }

    #[test]
    fn music_sfx_persists_lowmem_false() {
        let path = tmp_vault("music-sfx.vault");
        let mut s = Session::new();
        assert!(s.focused_lowmem(), "no focused profile defaults to lowmem");
        s.vault = Some(Vault::create(&path, "bot").unwrap());
        s.vault
            .as_mut()
            .unwrap()
            .upsert(profile("alice", "pw", 42))
            .unwrap();
        s.select("alice");
        assert!(s.focused_lowmem(), "fresh profile defaults to lowmem");
        assert!(s.set_focused_lowmem(false));
        assert!(
            !s.vault
                .as_ref()
                .unwrap()
                .get("alice")
                .unwrap()
                .settings
                .lowmem
        );
        assert!(!s.focused_lowmem(), "focused profile reflects the setting");
        assert!(s.set_focused_lowmem(true));
        assert!(
            s.vault
                .as_ref()
                .unwrap()
                .get("alice")
                .unwrap()
                .settings
                .lowmem
        );
    }

    #[test]
    fn set_auto_login_mirrors_running_arm() {
        let path = tmp_vault("auto-login-arm.vault");
        let mut s = Session::new();
        s.vault = Some(Vault::create(&path, "bot").unwrap());
        s.vault
            .as_mut()
            .unwrap()
            .upsert(profile("alice", "pw", 42))
            .unwrap();
        let mut play = host_play::run_with_io(
            &host_play::PlayOptions {
                host: "127.0.0.1".into(),
                port: 43594,
                cache_dir: "/tmp".into(),
                lowmem: true,
                mainland: false,
            },
            vec![],
            |_| (None, None),
            |_, _| {},
        );
        let arm = SlotArm::new(42, false);
        play.attach_arm("alice", Arc::clone(&arm));
        s.play = Some(play);
        assert!(s.set_auto_login("alice", true));
        assert!(arm.auto_login.load(Ordering::Relaxed));
        assert!(s.set_auto_login("alice", false));
        assert!(!arm.auto_login.load(Ordering::Relaxed));
    }

    #[test]
    fn set_renderer_writes_renderer_by_for_focused() {
        let mut s = Session::new();
        s.focus.lock().unwrap().focused = Some("alice".into());
        s.set_renderer(false);
        let f = s.focus.lock().unwrap();
        assert!(!f.renderer);
        assert_eq!(f.renderer_by.get("alice").copied(), Some(false));
        drop(f);
        s.set_renderer(true);
        let f = s.focus.lock().unwrap();
        assert!(f.renderer);
        assert_eq!(f.renderer_by.get("alice").copied(), Some(true));
    }

    #[test]
    fn raster_persists_and_off_keeps_prefer_cpu_until_cpu() {
        let path = tmp_vault("raster-mode.vault");
        let mut s = Session::new();
        s.vault = Some(Vault::create(&path, "bot").unwrap());
        s.vault
            .as_mut()
            .unwrap()
            .upsert(profile("alice", "pw", 42))
            .unwrap();
        s.select("alice");
        assert_eq!(s.focused_raster(), vault::RasterMode::Gpu);
        assert!(!s.slots.get("alice").unwrap().input.prefer_cpu());
        assert!(s.set_focused_raster(vault::RasterMode::Off));
        assert_eq!(s.focused_raster(), vault::RasterMode::Off);
        assert_eq!(
            s.focus.lock().unwrap().renderer_by.get("alice").copied(),
            Some(false)
        );
        assert!(
            !s.slots.get("alice").unwrap().input.prefer_cpu(),
            "Off must not flip the GPU/CPU latch"
        );
        assert!(s.set_focused_raster(vault::RasterMode::Cpu));
        assert_eq!(s.focused_raster(), vault::RasterMode::Cpu);
        assert!(s.slots.get("alice").unwrap().input.prefer_cpu());
        assert_eq!(
            s.vault
                .as_ref()
                .unwrap()
                .get("alice")
                .unwrap()
                .settings
                .raster,
            vault::RasterMode::Cpu
        );
    }

    #[test]
    fn raster_switch_confirm_only_when_backend_changes_on_spawned_slot() {
        use vault::RasterMode::*;
        // GPU↔CPU on a spawned slot is a drop+reattach (the Client and its
        // socket stay), so no logout/restart confirm is ever required —
        // not even Off↔Gpu/Cpu or a spawned slot.
        assert!(!Session::raster_switch_needs_confirm(Off, false, true));
        assert!(!Session::raster_switch_needs_confirm(Gpu, false, true));
        assert!(!Session::raster_switch_needs_confirm(Cpu, false, true));
        assert!(!Session::raster_switch_needs_confirm(Gpu, true, true));
        assert!(!Session::raster_switch_needs_confirm(Cpu, true, true));
        assert!(!Session::raster_switch_needs_confirm(Cpu, false, false));
        assert_eq!(Session::mem_status_text(true), "lowmem");
        assert_eq!(Session::mem_status_text(false), "highmem");
    }

    #[test]
    fn request_raster_cpu_on_spawned_slot_applies_immediately() {
        let path = tmp_vault("raster-no-confirm.vault");
        let mut s = Session::new();
        s.vault = Some(Vault::create(&path, "bot").unwrap());
        s.vault
            .as_mut()
            .unwrap()
            .upsert(profile("alice", "pw", 42))
            .unwrap();
        s.select("alice");
        s.request_focused_raster(vault::RasterMode::Cpu);
        assert_eq!(
            s.focused_raster(),
            vault::RasterMode::Cpu,
            "GPU→CPU applies at once: drop+reattach, never a logout"
        );
        s.request_focused_raster(vault::RasterMode::Off);
        assert_eq!(s.focused_raster(), vault::RasterMode::Off);
        assert!(s.slots.contains_key("alice"), "Off must keep the slot");
    }

    #[test]
    fn request_focused_lowmem_applies_without_confirm_and_keeps_slot() {
        let path = tmp_vault("mem-no-confirm.vault");
        let mut s = Session::new();
        s.vault = Some(Vault::create(&path, "bot").unwrap());
        s.vault
            .as_mut()
            .unwrap()
            .upsert(profile("alice", "pw", 42))
            .unwrap();
        s.select("alice");
        s.request_focused_lowmem(false);
        assert!(!s.focused_lowmem(), "mem flip applies at once");
        assert!(s.slots.contains_key("alice"), "mem flip must keep the slot");
    }

    #[test]
    fn raster_switch_keeps_slot_frame_buf_and_input() {
        let path = tmp_vault("raster-no-restart.vault");
        let mut s = Session::new();
        s.vault = Some(Vault::create(&path, "bot").unwrap());
        s.vault
            .as_mut()
            .unwrap()
            .upsert(profile("alice", "pw", 42))
            .unwrap();
        s.select("alice");
        let slot = s.slots.get("alice").expect("select spawns the slot");
        let buf = Arc::clone(&slot.pixels);
        let inp = Arc::clone(&slot.input);
        assert!(s.set_focused_raster(vault::RasterMode::Cpu));
        assert!(inp.prefer_cpu(), "GPU→CPU sets the slot's prefer_cpu latch");
        assert!(s.set_focused_raster(vault::RasterMode::Gpu));
        assert!(!inp.prefer_cpu(), "CPU→GPU clears the prefer_cpu latch");
        let slot = s.slots.get("alice").expect("GPU↔CPU must keep the slot");
        assert!(
            Arc::ptr_eq(&slot.pixels, &buf),
            "a GPU↔CPU flip must keep the same FrameBuf (no restart)"
        );
        assert!(
            Arc::ptr_eq(&slot.input, &inp),
            "a GPU↔CPU flip must keep the same SlotInput (no restart)"
        );
    }

    #[test]
    fn lowmem_flip_keeps_slot_frame_buf_and_input() {
        let path = tmp_vault("mem-no-restart.vault");
        let mut s = Session::new();
        s.vault = Some(Vault::create(&path, "bot").unwrap());
        s.vault
            .as_mut()
            .unwrap()
            .upsert(profile("alice", "pw", 42))
            .unwrap();
        s.select("alice");
        let slot = s.slots.get("alice").expect("select spawns the slot");
        let buf = Arc::clone(&slot.pixels);
        let inp = Arc::clone(&slot.input);
        assert!(s.set_focused_lowmem(false));
        assert!(s.set_focused_lowmem(true));
        let slot = s.slots.get("alice").expect("a mem flip must keep the slot");
        assert!(
            Arc::ptr_eq(&slot.pixels, &buf),
            "a mem flip must keep the same FrameBuf (no restart)"
        );
        assert!(
            Arc::ptr_eq(&slot.input, &inp),
            "a mem flip must keep the same SlotInput (no restart)"
        );
        assert!(s.focused_lowmem());
    }

    #[test]
    fn arm_for_profile_respects_auto_login_and_latch() {
        let path = tmp_vault("arm-for-profile.vault");
        let mut s = Session::new();
        s.vault = Some(Vault::create(&path, "bot").unwrap());
        let mut p = profile("alice", "pw", 42);
        p.settings.auto_login = true;
        s.vault.as_mut().unwrap().upsert(p).unwrap();
        let arm = s.arm_for_profile("alice").expect("arm");
        assert!(arm.want_login.load(Ordering::Relaxed));
        assert!(arm.auto_login.load(Ordering::Relaxed));
        s.wall.latch_logout("alice");
        let arm = s.arm_for_profile("alice").expect("arm");
        assert!(
            !arm.want_login.load(Ordering::Relaxed),
            "latch blocks handshake"
        );
        assert!(
            arm.auto_login.load(Ordering::Relaxed),
            "profile auto_login stays on the arm"
        );
    }

    #[test]
    fn set_auto_login_rejects_unknown_profile_without_spawning() {
        let path = tmp_vault("auto-login-missing.vault");
        let mut s = Session::new();
        s.vault = Some(Vault::create(&path, "bot").unwrap());
        assert!(!s.set_auto_login("nobody", true));
        assert!(s.error.is_some(), "missing profile sets the banner");
        assert!(s.slots.is_empty());
    }

    #[test]
    fn logout_latches_member_until_login_all() {
        let mut s = Session::new();
        s.wall.load("alice");
        s.logout("alice");
        assert!(s.wall.latch.contains("alice"), "intentional logout latches");
        assert!(!s.wall.should_auto_login("alice", true));
        s.login_all();
        assert!(
            !s.wall.latch.contains("alice"),
            "Login all clears the latch"
        );
    }

    #[test]
    fn focused_ingame_is_false_without_status() {
        let mut s = Session::new();
        s.focus.lock().unwrap().focused = Some("alice".into());
        assert!(!s.focused_ingame());
        s.statuses.push(SlotStatus {
            username: "alice".into(),
            ingame: true,
            ..SlotStatus::default()
        });
        assert!(s.focused_ingame());
    }

    #[test]
    fn focused_queue_tracks_the_focused_status_row() {
        let mut s = Session::new();
        s.focus.lock().unwrap().focused = Some("alice".into());
        assert_eq!(s.focused_queue(), None, "not queued by default");
        s.statuses.push(SlotStatus {
            username: "alice".into(),
            queue_position: 2,
            queue_total: 3,
            ..SlotStatus::default()
        });
        assert_eq!(s.focused_queue(), Some((2, 3)));

        // A queued non-focused slot does not surface on another focus.
        let mut s2 = Session::new();
        s2.focus.lock().unwrap().focused = Some("bob".into());
        s2.statuses.push(SlotStatus {
            username: "alice".into(),
            queue_position: 1,
            queue_total: 2,
            ..SlotStatus::default()
        });
        assert_eq!(s2.focused_queue(), None);
    }

    #[test]
    fn load_and_rail_remove_sync_focus_wall() {
        let path = tmp_vault("focus-wall-sync.vault");
        let mut s = Session::new();
        s.vault = Some(Vault::create(&path, "bot").unwrap());
        s.vault
            .as_mut()
            .unwrap()
            .upsert(profile("alice", "pw", 42))
            .unwrap();
        s.vault
            .as_mut()
            .unwrap()
            .upsert(profile("bob", "pw", 43))
            .unwrap();
        s.load("alice");
        s.load("bob");
        assert_eq!(
            s.focus.lock().unwrap().wall,
            vec!["alice".to_string(), "bob".to_string()],
            "membership mirrors into Focus.wall for draw_for_slot"
        );
        assert_eq!(s.focused_name().as_deref(), Some("bob"));
        s.rail_remove("bob");
        assert_eq!(
            s.focus.lock().unwrap().wall,
            vec!["alice".to_string()],
            "rail ✕ drops the name from Focus.wall too"
        );
        assert_eq!(
            s.focused_name().as_deref(),
            Some("alice"),
            "rail ✕ focuses the neighbour when the focused member is removed"
        );
    }

    #[test]
    fn rail_remove_clears_focus_when_last_member() {
        let path = tmp_vault("rail-remove-last.vault");
        let mut s = Session::new();
        s.vault = Some(Vault::create(&path, "bot").unwrap());
        s.vault
            .as_mut()
            .unwrap()
            .upsert(profile("alice", "pw", 42))
            .unwrap();
        s.load("alice");
        assert_eq!(s.focused_name().as_deref(), Some("alice"));
        s.rail_remove("alice");
        assert!(s.focused_name().is_none());
        assert!(s.wall.members.is_empty());
    }

    #[test]
    fn set_multibox_on_syncs_focus_wall() {
        let mut s = Session::new();
        s.set_multibox(true);
        assert_eq!(
            s.focus.lock().unwrap().wall,
            s.wall.members,
            "the seed path (running slots) mirrors into Focus.wall too"
        );
        s.set_multibox(false);
        assert_eq!(s.focus.lock().unwrap().wall, s.wall.members);
    }

    #[test]
    fn load_all_loads_vault_profiles_and_syncs_focus_wall() {
        let path = tmp_vault("load-all.vault");
        let mut s = Session::new();
        s.vault = Some(Vault::create(&path, "bot").unwrap());
        s.vault
            .as_mut()
            .unwrap()
            .upsert(profile("alice", "pw", 42))
            .unwrap();
        s.vault
            .as_mut()
            .unwrap()
            .upsert(profile("bob", "pw", 43))
            .unwrap();
        s.load("alice");
        let added = s.load_all();
        assert_eq!(added, 1, "only bob is new");
        assert_eq!(s.wall.members, vec!["alice".to_string(), "bob".to_string()]);
        assert_eq!(s.focus.lock().unwrap().wall, s.wall.members);
    }

    #[test]
    fn chooser_vault_remove_keeps_wall_member_and_slot() {
        let path = tmp_vault("chooser-remove.vault");
        let mut s = Session::new();
        s.vault = Some(Vault::create(&path, "bot").unwrap());
        s.vault
            .as_mut()
            .unwrap()
            .upsert(profile("alice", "pw", 42))
            .unwrap();
        s.load("alice");
        assert!(s.vault_remove("alice"), "chooser ✕ deletes the vault row");
        assert!(
            s.vault.as_ref().unwrap().get("alice").is_none(),
            "profile row gone from the vault"
        );
        assert_eq!(
            s.wall.members,
            vec!["alice".to_string()],
            "chooser ✕ must not rail_remove a live member"
        );
        assert!(s.slots.contains_key("alice"), "slot stays up");
        assert!(
            s.focus.lock().unwrap().wall.contains(&"alice".to_string()),
            "Focus.wall still lists the member"
        );
    }

    #[test]
    fn script_active_matches_rs2b0t() {
        assert!(script_active(script::RunState::Running));
        assert!(script_active(script::RunState::Paused));
        assert!(script_active(script::RunState::Stopping));
        assert!(!script_active(script::RunState::Idle));
        assert!(!script_active(script::RunState::Error));
    }

    #[test]
    fn script_pause_resume_stop_enable_rules() {
        assert!(script_pause_enabled(script::RunState::Running));
        assert!(script_pause_enabled(script::RunState::Paused));
        assert!(!script_pause_enabled(script::RunState::Idle));
        assert!(!script_pause_enabled(script::RunState::Stopping));
        assert!(!script_pause_enabled(script::RunState::Error));
        assert!(script_stop_enabled(script::RunState::Running));
        assert!(script_stop_enabled(script::RunState::Paused));
        assert!(!script_stop_enabled(script::RunState::Stopping));
        assert!(!script_stop_enabled(script::RunState::Idle));
        assert!(!script_stop_enabled(script::RunState::Error));
    }

    #[test]
    fn script_status_text_matches_rs2b0t_labels() {
        assert_eq!(script_status_text(script::RunState::Idle), "idle");
        assert_eq!(script_status_text(script::RunState::Running), "running");
        assert_eq!(script_status_text(script::RunState::Paused), "paused");
        assert_eq!(script_status_text(script::RunState::Stopping), "stopping");
        assert_eq!(script_status_text(script::RunState::Error), "error");
    }

    #[test]
    fn script_start_selected_unported_id_reports_not_ported() {
        let mut s = Session::new();
        let mut play = empty_play();
        play.attach_arm("alice", SlotArm::new(42, false));
        s.play = Some(play);
        s.focus.lock().unwrap().focused = Some("alice".into());
        s.script_sel = Some(script::ScriptSel::Compiled(script::CompiledId(
            "BoneBurier",
        )));
        s.script_start_selected();
        let err = s.error.clone().expect("not-ported message");
        assert!(err.contains("not ported"), "{err}");
        assert_eq!(s.focused_script_state(), script::RunState::Idle);
    }

    #[test]
    fn load_js_registers_card_selects_and_persists_to_the_session_store() {
        let dir =
            std::env::temp_dir().join(format!("274bot-panel-session-load-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = dir.join("js-scripts.json");
        let path = dir.join("tickbot.js");
        std::fs::write(
            &path,
            "export function tick(api) { api._n = (api._n||0)+1 }",
        )
        .unwrap();

        let mut s = Session::new();
        s.js = script::JsLibrary::new(store.clone());
        s.load_js(path.to_str().unwrap());
        assert_eq!(s.error, None, "load should succeed: {:?}", s.error);
        assert_eq!(
            s.script_sel,
            Some(script::ScriptSel::Loaded("tickbot".to_string()))
        );
        assert_eq!(s.js.cards().len(), 1);
        assert_eq!(s.load_scratch, "", "success clears the modal scratch");
        assert!(store.exists(), "the card is persisted to the session store");

        // A path that is not a bot shape fails and keeps the error banner.
        let bad = dir.join("plain.js");
        std::fs::write(&bad, "const x = 1;").unwrap();
        s.load_js(bad.to_str().unwrap());
        assert!(s.error.as_deref().is_some_and(|e| e.contains("shape")));
    }

    #[test]
    fn script_start_selected_refuses_without_selection_or_play() {
        let mut s = Session::new();
        s.script_start_selected();
        let err = s.error.clone().expect("no-focus banner");
        assert!(err.contains("focused"), "{err}");
        s.error = None;
        s.focus.lock().unwrap().focused = Some("alice".into());
        s.script_start_selected();
        let err = s.error.clone().expect("no-selection banner");
        assert!(err.contains("browse"), "{err}");
        s.error = None;
        s.script_sel = Some(script::ScriptSel::Compiled(script::CompiledId(
            "BoneBurier",
        )));
        s.script_start_selected();
        let err = s.error.clone().expect("no-play banner");
        assert!(err.contains("play"), "{err}");
        assert_eq!(s.focused_script_state(), script::RunState::Idle);
    }
}
