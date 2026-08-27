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
use client::sound::output::AudioOut;
use host::{map_image_to_applet, FrameBuf, InputEv, SlotInput};
use host_play::audio::{AudioChange, AudioGate};
use host_play::{
    open_vault, run_with_io, scatter_tile_for, Play, PlayOptions, SlotArm, SlotStatus,
};
use nav::router::{find, Route};
use nav::tile::Tile;
use nav::traveller::{TravelOptions, Traveller};
use nav::world::NavWorld;
use vault::{Profile, Vault};

use crate::focus::{draw_for_slot, full_rate_for};
use crate::wall::Wall;

/// Scatter / mainland hop only on a cold world, not after a `lostCon`
/// reconnect (that would tele the re-handshaked slot on every DC).
fn seed_on_first_world(last_login_reconnect: Option<bool>) -> bool {
    last_login_reconnect != Some(true)
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
    match env::var("HOME") {
        Ok(home) => format!("{home}/experiments/Server/engine/data/pack/client"),
        Err(_) => "experiments/Server/engine/data/pack/client".into(),
    }
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
    /// Credentials-section scratch buffers (username/password fields).
    pub cred_user: String,
    pub cred_pass: String,
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
            travellers: Arc::new(Mutex::new(HashMap::new())),
            walk_dest: None,
            walk_clear: Arc::new(AtomicBool::new(false)),
            tick_latch: Arc::new(Mutex::new(HashMap::new())),
            walkto_open: false,
            picker_sel: None,
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
            options: PlayOptions {
                host: "127.0.0.1".into(),
                port: DEFAULT_PORT,
                cache_dir: default_cache_dir(),
                lowmem: true,
                // Panel per_frame queues hop from Session.mainland (env);
                // spawn-time PlayOptions.mainland stays false.
                mainland: false,
            },
        }
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

    /// Live `stress50` setup: temp vault `s00`…`s49` (password = username,
    /// uids `274_000_100 + i`), every member spawns one full `Client`
    /// (no lean extras), `s00` is the focused slot, chooser closed,
    /// only-render-selected, then `login_all`.
    pub fn live_prepare_stress50(&mut self) -> Result<(), String> {
        self.live_prepare_stress(50)
    }

    /// Headed flat wall of `n` profiles (`s00`…`s{n-1}`).
    fn live_prepare_stress(&mut self, n: usize) -> Result<(), String> {
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
        self.set_multibox(true);
        self.scatter.store(true, Ordering::Relaxed);
        self.wall.chooser_open = false;
        self.focus.lock().unwrap().only_render_selected = true;
        // s00 loads first so it is the focused slot and FIFO head.
        self.load(&names[0].0);
        for (name, _) in names.iter().skip(1) {
            self.load(name);
        }
        self.select(&names[0].0);
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
        self.mainland.store(scenario.seed.mainland, Ordering::Relaxed);
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
        if let Some(owner) = self.audio.owner() {
            if !self.slots.contains_key(&owner) {
                self.audio.release(&owner);
            }
        }
        let Some(play) = self.play.as_mut() else {
            return;
        };
        let current = play.statuses();
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
    /// select (login FIFO); already-running slots stay up so the combo can
    /// change focus. Capture follows the new focus when the single capture
    /// toggle is on (never two keyboards). The credentials fields follow.
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
        {
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
            if let Some(old) = old {
                if let Some(slot) = self.slots.get(&old) {
                    slot.input.set_enabled(false);
                }
            }
            self.capture_on(name);
        } else {
            self.capture_tx = None;
        }
        // Credentials fields follow the newly focused profile.
        if let Some(vault) = &self.vault {
            if let Some(p) = vault.get(name) {
                self.cred_user = p.username.clone();
                self.cred_pass = p.password.clone();
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

    /// Mirror the sidecar-50 pref onto every slot's frame-cadence latch
    /// (`SlotInput::set_full_rate`). Runs every UI frame so a focus,
    /// renderer, or wall-policy change lands within a frame.
    fn sync_sidecar_cadence(&mut self) {
        let focus = self.focus.lock().unwrap();
        for (name, slot) in &self.slots {
            slot.input.set_full_rate(full_rate_for(&focus, name));
        }
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
        let pixels = FrameBuf::new();
        // The speaker gate mirrors the profile's lowmem at spawn: a
        // default lowmem slot starts with Music/SFX off (no cpal).
        self.audio.set_music(username, !profile.settings.lowmem);
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

    /// Lowmem for the Music/SFX checkbox: the focused vault profile, or
    /// `true` when nothing is focused.
    pub fn focused_lowmem(&self) -> bool {
        self.focused_name()
            .and_then(|n| self.vault.as_ref().and_then(|v| v.get(&n)))
            .map(|p| p.settings.lowmem)
            .unwrap_or(true)
    }

    /// Persist the focused profile's lowmem setting to the vault
    /// (`ProfileSettings.lowmem`) and retarget the focused slot's speaker
    /// gate live: Music/SFX on (highmem) opens cpal on the focused slot,
    /// off (lowmem) tears it down. Returns whether the write landed;
    /// failures set [`Session::error`].
    pub fn set_focused_lowmem(&mut self, lowmem: bool) -> bool {
        let Some(name) = self.focused_name() else {
            self.error = Some("music/sfx: no focused profile".into());
            return false;
        };
        let Some(vault) = self.vault.as_mut() else {
            self.error = Some("music/sfx: vault locked".into());
            return false;
        };
        let Some(mut profile) = vault.get(&name).cloned() else {
            self.error = Some(format!("music/sfx: no profile {name}"));
            return false;
        };
        profile.settings.lowmem = lowmem;
        match vault.upsert(profile) {
            Ok(()) => {
                self.error = None;
                self.audio.set_music(&name, !lowmem);
                // The gate reconciles on the slot's frame loop; kick a
                // parked focused slot so the open/teardown lands within a
                // frame, not at the 1 s watch bound.
                if let Some(play) = self.play.as_ref() {
                    play.wake(&name);
                }
                true
            }
            Err(e) => {
                self.error = Some(format!("music/sfx: {e}"));
                false
            }
        }
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
                let last = crate::ui_state::load().last_focus;
                if let Some(name) = crate::ui_state::pick_focus(&self.wall.members, last.as_deref())
                {
                    self.select(&name);
                }
            }
        } else {
            self.wall.on_multibox_off();
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
    /// `error` carries a short message. Callers that do not know the
    /// player's tile fall back to [`Session::arm_walk`].
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
        match find(&world.collision, &world.graph, from_w, dest_w) {
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
    let dir = std::env::temp_dir().join(format!(
        "274bot-panel-live-{}-{}-{}",
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

#[cfg(test)]
mod tests {
    use super::{
        arm_login_all, combo_index, maybe_send_click, script_active, script_pause_enabled,
        script_status_text, script_stop_enabled, seed_on_first_world, stream_capture, Session,
        SlotIo,
    };
    use host::{FrameBuf, InputEv, SlotInput};
    use host_play::{SlotArm, SlotStatus};
    use api::snapshot::WorldTile;
    use client::dash3d::CollisionFlag;
    use nav::collision::WorldCollision;
    use nav::tile::Tile;
    use nav::transport::TransportGraph;
    use nav::world::NavWorld;
    use std::sync::atomic::Ordering;
    use std::sync::Arc;
    use vault::{Profile, ProfileSettings, Vault};

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
                flags: vec![0u32; w * h],
                walkable: vec![0u32; w * h],
            },
            graph: TransportGraph::default(),
        }
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
        s.sync_sidecar_cadence();
        assert!(!a_in.full_rate(), "the focused slot keeps its capture path");
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
    fn session_starts_with_renderer_on_capture_off() {
        let s = Session::new();
        let f = s.focus.lock().unwrap();
        assert!(f.renderer, "rail is on; host paints 1 fps until capture");
        assert!(!f.capture);
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
                walkable: nav::collision::derive_walkable(&flags),
                flags,
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
            s.play
                .as_ref()
                .unwrap()
                .arm("carol")
                .is_some(),
            "every member has a control arm"
        );
        // Focus is pure bookkeeping: selecting bob redirects the sampled
        // slot without touching a socket.
        s.select("bob");
        assert_eq!(s.focused_name().as_deref(), Some("bob"));
        assert_eq!(
            s.play.as_ref().unwrap().focused().as_deref(),
            Some("bob")
        );
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
        s.live_prepare_stress(3).expect("prepare");
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
        s.live_prepare_stress(2).expect("prepare");
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
