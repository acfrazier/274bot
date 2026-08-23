//! Panel session: owns the unlocked vault, the running slot map, the shared
//! `Focus`, and per-slot pixel/input channels. The dear-app frame reads
//! `Session`; slot threads stay in `host_play` (spawned via `run_with_io`
//! with per-profile `PixelBuf`/`SlotInput`, keeping the login FIFO and the
//! mainland hop).

use std::collections::{HashMap, HashSet};
use std::env;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};

use api::interact::Driver;
use host::{map_image_to_applet, InputEv, PixelBuf, SlotInput};
use host_play::{open_vault, run_with_io, Play, PlayOptions, SlotArm, SlotStatus};
use nav::grid::StepGrid;
use nav::router::{find, NoPath};
use nav::tile::Tile;
use nav::traveller::{NavStatus, Traveller};
use vault::{Profile, Vault};

use crate::focus::draw_for_slot;
use crate::wall::Wall;

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

/// Panel-side per-slot IO: the pixel buffer the slot paints into while its
/// renderer is on, and the input channel it drains only while capture is on.
pub struct SlotIo {
    pub input: Arc<SlotInput>,
    pub pixels: Arc<PixelBuf>,
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
    /// Mainland checkbox; the per-frame hook queues the hop at scene 2.
    pub mainland: Arc<AtomicBool>,
    /// Panel log lines (status transitions), capped at [`LOG_CAP`].
    pub log: Arc<Mutex<Vec<String>>>,
    /// Vault passphrase scratch buffer for the in-panel unlock prompt.
    pub pass_scratch: String,
    /// Last status poll (delta source for the log).
    pub statuses: Vec<SlotStatus>,
    /// Credentials-section scratch buffers (username/password fields).
    pub cred_user: String,
    pub cred_pass: String,
    /// Per-username nav travellers; the focused slot's traveller carries
    /// the armed walk route (ticked from `start_play` `per_frame`).
    pub travellers: Arc<Mutex<HashMap<String, Arc<Mutex<Traveller>>>>>,
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
    options: PlayOptions,
    /// Multibox wall membership (chooser / latch / bulk ops). The UI reads
    /// it for the chooser and rail; [`Session`] methods drive it.
    pub wall: Wall,
    /// MultiBox toggle: rail (or grid) policy is up. `Focus.wall_open`
    /// mirrors this so extra rasters only run while the wall is visible.
    pub multibox: bool,
}

/// Keep the panel log bounded.
const LOG_CAP: usize = 200;

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
            log: Arc::new(Mutex::new(Vec::new())),
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
            wall: Wall::default(),
            multibox: false,
            options: PlayOptions {
                host: "127.0.0.1".into(),
                port: DEFAULT_PORT,
                cache_dir: default_cache_dir(),
                lowmem: true,
                // The panel queues the mainland hop itself (live checkbox),
                // so the spawn-time `PlayOptions.mainland` stays false.
                mainland: false,
            },
        }
    }

    /// Unlock (or first-run create) the vault and start the play. Only the
    /// focused profile is spawned as a slot; other vault rows stay parked
    /// until selected (channel-change keeps a slot once it has run).
    pub fn unlock(&mut self, pass: &str) -> bool {
        let path = default_vault_path();
        match open_vault(&path, pass) {
            Ok(vault) => {
                self.error = None;
                self.start_play(vault);
                true
            }
            Err(e) => {
                self.error = Some(e.to_string());
                false
            }
        }
    }

    /// Empty `Play` (shared cache + FIFO + per-frame hook) then spawn the
    /// first focused profile only. Parked names are started from [`select`].
    fn start_play(&mut self, vault: Vault) {
        let focus = Arc::clone(&self.focus);
        let log = Arc::clone(&self.log);
        let mainland = Arc::clone(&self.mainland);
        let mainland_sent = Arc::clone(&self.mainland_sent);
        let travellers = Arc::clone(&self.travellers);
        let tick_latch = Arc::clone(&self.tick_latch);
        let walk_clear = Arc::clone(&self.walk_clear);
        let options = self.options.clone();
        let play = run_with_io(
            &options,
            Vec::new(),
            |_| (None, None),
            move |c, name| {
                let draw = draw_for_slot(&focus.lock().unwrap(), name);
                c.set_draw(draw);
                if mainland.load(Ordering::Relaxed)
                    && c.ingame
                    && c.scene_state == 2
                    && mainland_sent.lock().unwrap().insert(name.to_string())
                {
                    api::interact::mainland_hop(c);
                    log.lock()
                        .unwrap()
                        .push(format!("{name}: mainland hop queued"));
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
                let Some(traveller) = travellers.lock().unwrap().get(name).cloned() else {
                    return;
                };
                {
                    let mut latch = tick_latch.lock().unwrap();
                    if latch.get(name) == Some(&(c.gens.player, here)) {
                        return;
                    }
                    latch.insert(name.to_string(), (c.gens.player, here));
                }
                let door = traveller.lock().unwrap().current_door(here);
                let door_open = match door {
                    Some((loc, closed_id)) => {
                        let (bx, bz) = Driver::build_base(c);
                        Driver::loc_typecode(c, loc.x - bx, loc.z - bz)
                            .map(|tc| (tc >> 14) & 0x7fff)
                            != Some(closed_id)
                    }
                    None => false,
                };
                let status = traveller.lock().unwrap().tick(c, here, door_open);
                if matches!(status, NavStatus::Arrived | NavStatus::Budget) {
                    walk_clear.store(true, Ordering::Relaxed);
                }
            },
        );
        self.play = Some(play);
        self.statuses = self.play.as_ref().map(|p| p.statuses()).unwrap_or_default();
        self.vault = Some(vault);
        self.focus_first_profile();
    }

    /// After unlock/`spawn_all`: if the vault (or running slots) has names,
    /// focus the first so the combo and renderer are not stuck on `None`.
    fn focus_first_profile(&mut self) {
        let names = self.profile_names();
        if !names.is_empty() {
            self.select(&names[0]);
        }
    }

    /// Poll slot statuses and append log lines for transitions (slot up,
    /// login errors, ingame, scene changes). Call once per UI frame.
    pub fn pump_status(&mut self) {
        let Some(play) = &self.play else {
            return;
        };
        let current = play.statuses();
        {
            let mut log = self.log.lock().unwrap();
            for s in &current {
                let prev = self.statuses.iter().find(|p| p.username == s.username);
                match prev {
                    None => {
                        log.push(format!("{}: slot up", s.username));
                        if let Some(e) = &s.error {
                            log.push(format!("{}: login {}", s.username, e));
                        }
                    }
                    Some(p) => {
                        if p.error.is_none() && s.error.is_some() {
                            log.push(format!(
                                "{}: login {}",
                                s.username,
                                s.error.as_deref().unwrap_or_default()
                            ));
                        }
                        if !p.ingame && s.ingame {
                            log.push(format!("{}: ingame", s.username));
                        }
                        if p.scene_state != s.scene_state {
                            log.push(format!("{}: scene {}", s.username, s.scene_state));
                        }
                    }
                }
            }
            while log.len() > LOG_CAP {
                log.remove(0);
            }
        }
        self.statuses = current;
        self.sync_walk_status();
    }

    /// Copy each slot's traveller `queued()` into `walk_*` (−1 if none) and
    /// clear [`Session::walk_dest`] after Arrived/Budget.
    fn sync_walk_status(&mut self) {
        for s in &mut self.statuses {
            let queued = self
                .travellers
                .lock()
                .unwrap()
                .get(&s.username)
                .and_then(|t| t.lock().unwrap().queued());
            apply_queued_walk(s, queued);
        }
        if self.walk_clear.swap(false, Ordering::Relaxed) {
            let keep = self.focused_name().and_then(|n| {
                self.travellers
                    .lock()
                    .unwrap()
                    .get(&n)
                    .and_then(|t| t.lock().unwrap().queued())
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

    /// The focused slot's pixel buffer (None when nothing is focused).
    pub fn focused_pixels(&self) -> Option<Arc<PixelBuf>> {
        self.focused_slot().map(|s| Arc::clone(&s.pixels))
    }

    fn focused_slot(&self) -> Option<&SlotIo> {
        let name = self.focused_name()?;
        self.slots.get(&name)
    }

    /// Switch the focused profile. A parked vault name is spawned on first
    /// select (login FIFO); already-running slots stay up so the combo can
    /// channel-change. Capture follows the new focus when the single capture
    /// toggle is on (never two keyboards). The credentials fields follow.
    pub fn select(&mut self, name: &str) {
        self.ensure_slot(name, None);
        let mut focus = self.focus.lock().unwrap();
        if focus.focused.as_deref() == Some(name) {
            return;
        }
        let old = focus.focused.clone();
        focus.focused = Some(name.to_string());
        let capture = focus.capture;
        drop(focus);
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

    /// Renderer checkbox. The slot threads apply `set_draw` from the focus
    /// in their per-frame observe hook, so no other wiring is needed.
    pub fn set_renderer(&mut self, on: bool) {
        self.focus.lock().unwrap().renderer = on;
    }

    /// Game window `.build()` Some/None. Closing the pane turns capture off
    /// (`set_enabled(false)` + drop tx); reopening does not re-enable it.
    pub fn set_game_pane_open(&mut self, open: bool) {
        let mut focus = self.focus.lock().unwrap();
        let was = focus.game_pane_open;
        focus.game_pane_open = open;
        drop(focus);
        if was && !open {
            self.set_capture(false);
        }
    }

    /// Capture checkbox. On: attach a fresh channel and enable the focused
    /// slot's drain. Off: disable the drain and drop the sender so the UI
    /// cannot enqueue (the slot thread does no `try_recv` while disabled).
    pub fn set_capture(&mut self, on: bool) {
        self.focus.lock().unwrap().capture = on;
        if on {
            let name = self.focused_name();
            match name {
                Some(name) => self.capture_on(&name),
                None => self.capture_tx = None,
            }
        } else {
            self.capture_off();
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
        self.ensure_slot(&username, None);
        self.select(&username);
        true
    }

    /// Register per-slot IO and spawn via [`Play::spawn_slot`] when a play
    /// is live. Without `play` (unit tests / pre-unlock) only the IO map is
    /// filled so focus can attach. `arm` carries the spawn's login intent:
    /// `None` logs in immediately (the pre-wall behavior); a wall `load`
    /// passes a real arm so a latched logout can hold the title screen.
    fn ensure_slot(&mut self, username: &str, arm: Option<Arc<SlotArm>>) {
        if self.slots.contains_key(username) {
            return;
        }
        let Some(profile) = self.vault.as_ref().and_then(|v| v.get(username)).cloned() else {
            return;
        };
        let input = SlotInput::new();
        let pixels = PixelBuf::new();
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

    /// Focus the credentials username. Save upserts then selects (spawn if
    /// needed). Log in is the same select path.
    pub fn login(&mut self, name: &str) {
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
    }

    /// Persist the focused profile's auto-login checkbox to the vault
    /// (`ProfileSettings.auto_login`). Slot spawns/loads read the setting;
    /// this method itself never spawns or stops a slot.
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
        true
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
            let arm = self
                .vault
                .as_ref()
                .and_then(|v| v.get(name))
                .map(|p| SlotArm::new(p.uid, want_login));
            self.ensure_slot(name, arm);
        }
        self.select(name);
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

    /// Log in every wall member: clear their latches and arm a login so
    /// title-screen slots handshake. One-shot unless the profile's
    /// auto-login is set (which keeps the arm armed after the handshake).
    pub fn login_all(&mut self) {
        for name in self.wall.members.clone() {
            self.wall.clear_latch(&name);
            if let Some(arm) = self.play.as_ref().and_then(|p| p.arm(&name)) {
                arm_login_all(&arm);
            }
        }
    }

    /// Log out every wall member: record the latch (blocks auto-login
    /// until the next [`Session::login_all`]) and arm a clean IF logout.
    /// `want_login` is cleared too so a title-screen member does not
    /// handshake right back in.
    pub fn logout_all(&mut self) {
        for name in self.wall.members.clone() {
            self.wall.latch_logout(&name);
            if let Some(arm) = self.play.as_ref().and_then(|p| p.arm(&name)) {
                arm.want_logout.store(true, Ordering::Relaxed);
                arm.want_login.store(false, Ordering::Relaxed);
            }
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
        } else {
            self.wall.on_multibox_off();
        }
        self.focus.lock().unwrap().wall_open = on;
        self.sync_wall_focus();
    }

    /// Grid submode of MultiBox: hides the rail in the Game pane (the grid
    /// cells themselves land in Task 12). A no-op while MultiBox is off.
    pub fn set_grid(&mut self, on: bool) {
        if self.multibox {
            self.wall.grid = on;
        }
    }

    /// Remove a member from the rail: drop it from the wall, clear its
    /// logout latch (so a re-added member is not blocked from auto-login),
    /// arm a clean logout when it is ingame, then stop the slot and forget
    /// its IO. The wait-until-`!ingame` between the logout and the stop is
    /// a live/UI concern; here the flags are set in the right order and the
    /// thread is joined immediately.
    pub fn rail_remove(&mut self, name: &str) {
        self.wall.rail_remove(name);
        self.wall.clear_latch(name);
        if let Some(play) = &self.play {
            let ingame = play
                .statuses()
                .iter()
                .any(|s| s.username == name && s.ingame);
            if let Some(arm) = play.arm(name) {
                if ingame {
                    arm.want_logout.store(true, Ordering::Relaxed);
                }
            }
        }
        if let Some(play) = &mut self.play {
            play.stop_slot(name);
        }
        self.slots.remove(name);
        self.sync_wall_focus();
    }

    /// Arm a walk to `dest`. The picked dest is always stored so the status
    /// row shows what the user asked for even when no route could be found.
    /// Routing needs the player's observed tile and a loaded pack; the
    /// picker routes via [`Session::arm_walk_on`] when it has both.
    pub fn arm_walk(&mut self, dest: Tile) {
        self.walk_dest = Some(dest);
        self.walk_clear.store(false, Ordering::Relaxed);
    }

    /// Arm a walk to `dest` and route it on `grid` from `from` (the player's
    /// observed tile). On `Ok(route)` the focused username's traveller is
    /// armed so the observe tick can step it; on `NoPath` only the dest is
    /// stored and `error` carries a short message. Callers that do not know
    /// the player's tile fall back to [`Session::arm_walk`].
    pub fn arm_walk_on(&mut self, grid: &StepGrid, from: Tile, dest: Tile) {
        self.walk_dest = Some(dest);
        self.walk_clear.store(false, Ordering::Relaxed);
        match find(grid, from, dest) {
            Ok(route) => {
                self.error = None;
                if let Some(name) = self.focused_name() {
                    let traveller = self
                        .travellers
                        .lock()
                        .unwrap()
                        .entry(name.clone())
                        .or_insert_with(|| Arc::new(Mutex::new(Traveller::new())))
                        .clone();
                    traveller.lock().unwrap().arm(route);
                    self.tick_latch.lock().unwrap().remove(&name);
                    // Rising edge: the overlay must paint the new route on
                    // this frame, not after the 1 s raster cadence.
                    self.route_gen += 1;
                }
            }
            Err(NoPath) => {
                self.error = Some(format!("no path to {} {} {}", dest.x, dest.z, dest.level));
            }
        }
    }

    /// Arm the current [`Session::picker_sel`] on `grid`. Returns false when
    /// nothing is selected. Clears the selection either way so a second
    /// confirm does not re-fire.
    pub fn confirm_picker_walk(&mut self, grid: &StepGrid) -> bool {
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
                self.arm_walk_on(grid, from, tile);
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
    use super::{arm_login_all, combo_index, maybe_send_click, stream_capture, Session, SlotIo};
    use host::{InputEv, PixelBuf, SlotInput};
    use host_play::{SlotArm, SlotStatus};
    use std::sync::atomic::Ordering;
    use nav::grid::StepGrid;
    use nav::tile::Tile;
    use vault::{Profile, ProfileSettings, Vault};

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
    fn session_starts_with_renderer_on_capture_off() {
        let s = Session::new();
        let f = s.focus.lock().unwrap();
        assert!(f.renderer, "rail is on; host paints 1 fps until capture");
        assert!(!f.capture);
    }

    #[test]
    fn walk_status_is_dash_when_no_route() {
        let s = Session::new();
        assert_eq!(s.walk_status_text(), "—");
    }

    #[test]
    fn picker_select_does_not_arm_until_confirm() {
        let mut s = Session::new();
        let dest = Tile { x: 2, z: 2, level: 0 };
        s.picker_sel = Some(dest);
        assert_eq!(s.walk_status_text(), "—");
        assert!(s.confirm_picker_walk(&StepGrid::fixture_open_3x3()));
        assert!(s.walk_status_text().contains("2"));
        assert!(s.picker_sel.is_none());
        assert!(!s.confirm_picker_walk(&StepGrid::fixture_open_3x3()));
    }

    #[test]
    fn arm_walk_sets_queued_text() {
        let mut s = Session::new();
        s.arm_walk(Tile { x: 3222, z: 3222, level: 0 });
        assert!(s.walk_status_text().contains("3222"));
    }

    #[test]
    fn arm_walk_on_routes_and_arms_focused_traveller() {
        let mut s = Session::new();
        s.focus.lock().unwrap().focused = Some("alice".into());
        let g = StepGrid::fixture_open_3x3();
        let dest = Tile { x: 2, z: 2, level: 0 };
        s.arm_walk_on(&g, Tile { x: 0, z: 0, level: 0 }, dest);
        assert_eq!(s.walk_dest, Some(dest), "dest stays stored on success");
        assert!(s.error.is_none(), "a found route clears the error banner");
        let queued = s
            .travellers
            .lock()
            .unwrap()
            .get("alice")
            .expect("focused traveller exists")
            .lock()
            .unwrap()
            .queued();
        assert_eq!(queued, Some(dest));
    }

    #[test]
    fn arm_walk_on_no_path_stores_dest_and_sets_error() {
        let mut s = Session::new();
        s.focus.lock().unwrap().focused = Some("alice".into());
        let mut g = StepGrid::fixture_open_3x3();
        g.set_walkable(Tile { x: 1, z: 0, level: 0 }, false);
        g.set_walkable(Tile { x: 1, z: 1, level: 0 }, false);
        g.set_walkable(Tile { x: 1, z: 2, level: 0 }, false);
        let dest = Tile { x: 2, z: 1, level: 0 };
        s.arm_walk_on(&g, Tile { x: 0, z: 1, level: 0 }, dest);
        assert_eq!(s.walk_dest, Some(dest), "dest stays stored on NoPath");
        let err = s.error.clone().expect("no-path message set");
        assert!(err.contains("no path"), "short no-path message, got {err:?}");
        assert!(
            s.travellers
                .lock()
                .unwrap()
                .get("alice")
                .is_none_or(|t| t.lock().unwrap().queued().is_none()),
            "no route must be armed when find fails"
        );
    }

    #[test]
    fn arm_walk_on_without_focus_skips_route_but_stores_dest() {
        let mut s = Session::new();
        let g = StepGrid::fixture_open_3x3();
        let dest = Tile { x: 2, z: 2, level: 0 };
        s.arm_walk_on(&g, Tile { x: 0, z: 0, level: 0 }, dest);
        assert_eq!(s.walk_dest, Some(dest));
        assert!(
            s.travellers.lock().unwrap().is_empty(),
            "no focused name to key a traveller"
        );
    }

    #[test]
    fn arm_walk_on_success_bumps_route_gen() {
        let mut s = Session::new();
        s.focus.lock().unwrap().focused = Some("alice".into());
        let g = StepGrid::fixture_open_3x3();
        assert_eq!(s.route_gen(), 0);
        s.arm_walk_on(&g, Tile { x: 0, z: 0, level: 0 }, Tile { x: 2, z: 2, level: 0 });
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
        let g = StepGrid::fixture_open_3x3();
        let dest = Tile { x: 2, z: 2, level: 0 };
        s.arm_walk_on(&g, Tile { x: 0, z: 0, level: 0 }, dest);
        s.sync_walk_status();
        assert_eq!(
            (s.statuses[0].walk_x, s.statuses[0].walk_z, s.statuses[0].walk_level),
            (2, 2, 0)
        );
        s.travellers
            .lock()
            .unwrap()
            .get("alice")
            .unwrap()
            .lock()
            .unwrap()
            .clear();
        s.walk_clear.store(true, std::sync::atomic::Ordering::Relaxed);
        s.sync_walk_status();
        assert_eq!(s.walk_status_text(), "—");
        assert_eq!(
            (s.statuses[0].walk_x, s.statuses[0].walk_z, s.statuses[0].walk_level),
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
                pixels: PixelBuf::new(),
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
        assert!(s.focus.lock().unwrap().wall_open, "rail or grid: wall is open");
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
            s.vault.as_ref().unwrap().get("alice").unwrap().settings.auto_login
        );
        assert!(s.slots.is_empty(), "set_auto_login must not spawn a slot");
        assert!(s.set_auto_login("alice", false));
        assert!(
            !s.vault.as_ref().unwrap().get("alice").unwrap().settings.auto_login
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
        assert!(!s.wall.latch.contains("alice"), "Login all clears the latch");
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
        s.rail_remove("alice");
        assert_eq!(
            s.focus.lock().unwrap().wall,
            vec!["bob".to_string()],
            "rail ✕ drops the name from Focus.wall too"
        );
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
        assert_eq!(
            s.wall.members,
            vec!["alice".to_string(), "bob".to_string()]
        );
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
}
