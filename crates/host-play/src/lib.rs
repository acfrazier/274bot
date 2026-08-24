//! `host-play`: run vaulted profiles through the host kernel. The binary
//! unlocks a vault and runs the named profiles; the `e2e` harness links
//! this library so it can poll per-slot state instead of scraping logs.

use std::collections::{HashMap, HashSet};
use std::panic::AssertUnwindSafe;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use client::client::Client;
use client::client::ClientConfig;
use client::client::LoginError;
use client::config::{Cache, IfType};
use client::io::JagFile;
pub use host::debug_enabled;
use host::lean::{Lean, LeanError};
use host::login_queue::{LoginBackoff, LoginQueue, Permit, QueuePos};
use host::prepare_client;
pub use host::set_debug;
pub use host::Host;
mod rss;
pub use rss::sample_process;
use host::{PixelBuf, SlotInput};
use vault::{Profile, Vault, VaultError};

/// Slot thread stack: 1 MiB (the Java client thread default).
const THREAD_STACK: usize = 1024 * 1024;

/// Connection settings shared by every spawned slot.
#[derive(Clone)]
pub struct PlayOptions {
    pub host: String,
    pub port: u16,
    pub cache_dir: String,
    pub lowmem: bool,
    /// After scene 2, queue rs2b0t `mainlandAccount` tele+setvar (no relog).
    pub mainland: bool,
}

/// Pollable per-slot view; the slot threads update it after each frame.
#[derive(Debug, Clone)]
pub struct SlotStatus {
    pub username: String,
    /// When the slot's first login handshake started (after its permit).
    pub login_started: Option<Instant>,
    pub ingame: bool,
    pub scene_state: i32,
    /// Last login error (code + message); cleared after a successful login.
    pub error: Option<String>,
    pub runenergy: i32,
    /// Accepted auto-run `set_run(true)` sends this slot has made.
    pub run_sends: u32,
    /// Local-player tile (filled from `local_player` in observe).
    pub tile_x: i32,
    pub tile_z: i32,
    /// Local-player name, empty until `PLAYER_INFO` lands.
    pub player: String,
    /// `Client.main_modal_id` (open modal interface, -1 when none).
    pub main_modal_id: i32,
    /// Queued walk target tile, -1 when idle (filled from the slot's
    /// traveller when one is wired; Task 8+).
    pub walk_x: i32,
    pub walk_z: i32,
    pub walk_level: i32,
    /// Place in the login FIFO while waiting for a permit: 1-based
    /// `position` of `total`; both -1 when not queued (same sentinel as
    /// the `walk_*` fields).
    pub queue_position: i32,
    pub queue_total: i32,
    /// Payload bytes from `Client.stream` (0 when no stream).
    pub bytes_in: u64,
    pub bytes_out: u64,
    /// Client draw-entry counters (honest zeros until first paint enter).
    pub game_draw_enters: u64,
    pub title_screen_draw_enters: u64,
    /// Host-stamped frame timings / paint-vs-skip counts from `Client`.
    pub loop_ns: u64,
    pub raster_ns: u64,
    pub paint_n: u64,
    pub skip_n: u64,
    /// True for a lean channel row (`host::lean::Lean`, no `Client`, no
    /// World); false for a full `Client` slot. The live channel ladder
    /// counts leanes against the one head.
    pub lean: bool,
}

/// Absolute world tile from the scene origin plus the local-player route
/// head (`route_x[0]` / `route_z[0]`). Scene pixels (`lp.x` / `lp.z`) are
/// 128× these; WalkTo and the picker need world tiles.
pub fn player_world_tile(
    map_build_base_x: i32,
    map_build_base_z: i32,
    route_x: i32,
    route_z: i32,
) -> (i32, i32) {
    (map_build_base_x + route_x, map_build_base_z + route_z)
}

/// Defaults match the derived `Default` for every field except the queued
/// walk tile, which starts `-1` (none) instead of `0`.
impl Default for SlotStatus {
    fn default() -> Self {
        Self {
            username: String::new(),
            login_started: None,
            ingame: false,
            scene_state: 0,
            error: None,
            runenergy: 0,
            run_sends: 0,
            tile_x: 0,
            tile_z: 0,
            player: String::new(),
            main_modal_id: 0,
            walk_x: -1,
            walk_z: -1,
            walk_level: -1,
            queue_position: -1,
            queue_total: -1,
            bytes_in: 0,
            bytes_out: 0,
            game_draw_enters: 0,
            title_screen_draw_enters: 0,
            loop_ns: 0,
            raster_ns: 0,
            paint_n: 0,
            skip_n: 0,
            lean: false,
        }
    }
}

/// Copy stream byte counters and draw-entry counts from `Client` onto a
/// `SlotStatus` row. No stream → bytes stay 0.
pub fn copy_stream_and_draw(c: &Client, s: &mut SlotStatus) {
    s.game_draw_enters = c.game_draw_enters;
    s.title_screen_draw_enters = c.title_screen_draw_enters;
    s.loop_ns = c.loop_ns;
    s.raster_ns = c.raster_ns;
    s.paint_n = c.paint_n;
    s.skip_n = c.skip_n;
    let (bi, bo) = c
        .stream
        .as_ref()
        .map(|st| (st.bytes_in(), st.bytes_out()))
        .unwrap_or((0, 0));
    s.bytes_in = bi;
    s.bytes_out = bo;
}

/// Per-slot control arm. The panel flips these to make a slot sit on the
/// title screen (no handshake) until login is armed, request a clean IF
/// logout, or stop the thread. A `None` arm at spawn means CLI/e2e: the
/// slot logs in immediately.
pub struct SlotArm {
    /// The profile uid this arm controls; `stop_slot` uses it to drop the
    /// slot's login-FIFO place before the thread exits. Atomic so spawn
    /// can force it from the profile even while callers hold clones.
    pub uid: AtomicI32,
    pub want_login: Arc<AtomicBool>,
    pub want_logout: Arc<AtomicBool>,
    pub stop: Arc<AtomicBool>,
    pub latch: Arc<AtomicBool>,
    /// The spawn-time auto-login intent (CLI `new(uid, true)` stays armed
    /// so an unexpected DC re-handshakes; a panel one-shot arm disarms
    /// after the handshake unless the profile's auto_login was on).
    pub auto_login: Arc<AtomicBool>,
}

impl SlotArm {
    pub fn new(uid: i32, want_login: bool) -> Arc<Self> {
        Arc::new(Self {
            uid: AtomicI32::new(uid),
            want_login: Arc::new(AtomicBool::new(want_login)),
            want_logout: Arc::new(AtomicBool::new(false)),
            stop: Arc::new(AtomicBool::new(false)),
            latch: Arc::new(AtomicBool::new(false)),
            auto_login: Arc::new(AtomicBool::new(want_login)),
        })
    }
}

/// Whether the slot may start a login handshake: on the title (not ingame)
/// and the arm wants a login that is not latched by an intentional logout.
fn should_handshake(arm: &SlotArm, ingame: bool) -> bool {
    !ingame
        && arm.want_login.load(Ordering::Relaxed)
        && !arm.latch.load(Ordering::Relaxed)
}

/// After a successful handshake: stay armed only when this slot was spawned
/// with auto-login (an unexpected DC re-handshakes); a one-shot Log in /
/// Login all disarms until the next explicit arm.
fn on_login_success(arm: &SlotArm) {
    arm.want_login.store(
        arm.auto_login.load(Ordering::Relaxed) && !arm.latch.load(Ordering::Relaxed),
        Ordering::Relaxed,
    );
}

/// Per-frame arm handling in the 20 ms body: press the CC_LOGOUT iface when
/// the panel armed a logout on an ingame slot, then report whether the
/// thread must stop (rail ✕). Probe order: logout press returns `false`
/// (keep running until `!ingame`); only then may `stop` end the body. The
/// press is the only place a clean logout can go out while the slot is
/// inside [`Host::run_client`].
fn tick_flags(client: &mut Client, ifaces: &[Option<IfType>], arm: &SlotArm) -> bool {
    if arm.want_logout.load(Ordering::Relaxed) && client.ingame {
        api::interact::logout(client, ifaces);
        arm.want_logout.store(false, Ordering::Relaxed);
        arm.latch.store(true, Ordering::Relaxed);
        arm.want_login.store(false, Ordering::Relaxed);
        // Do not honor `stop` on the same probe as the logout press — the
        // body must keep running until the client leaves the game.
        return false;
    }
    arm.stop.load(Ordering::Relaxed)
}

/// Running slots and their shared status. Slots drive `mainloop` until the
/// process exits; callers poll [`Play::statuses`] and then exit.
///
/// [`Play::spawn_slot`] can add a profile after the initial [`run_with_io`]
/// call; later slots share the same login FIFO, cache, and per-frame hook.
///
/// Channel-head (4.7): `head` is the one fat `Client` (the tuned profile)
/// and `channels` are the lean sessions for every other profile;
/// [`Play::tune`] moves the head between profiles. `profiles` keeps the
/// vault credentials every tune and lean park needs.
pub struct Play {
    /// Shared status rows; panel tests push fakes here for `pump_status`.
    pub statuses: Arc<Mutex<Vec<SlotStatus>>>,
    handles: HashMap<String, thread::JoinHandle<()>>,
    options: PlayOptions,
    cache: Arc<Cache>,
    ifaces: Vec<Option<IfType>>,
    queue: Arc<Mutex<LoginQueue>>,
    per_frame: Arc<dyn Fn(&mut Client, &str) + Send + Sync>,
    spawned: HashSet<String>,
    arms: HashMap<String, Arc<SlotArm>>,
    /// Vault profiles keyed by username (`tune` looks up the incoming
    /// password/uid; the park needs the outgoing one too).
    profiles: HashMap<String, Profile>,
    /// The tuned profile's fat `Client`; every other profile is a lean
    /// channel. `None` until the first [`Play::tune`].
    head: Option<Head>,
    /// Lean channels for every profile that is not currently the head.
    channels: HashMap<String, Lean>,
}

/// The tuned profile's fat `Client`. Exactly one head: `tune` parks the
/// previous one as a lean channel and reconnects this one (opcode 18).
struct Head {
    name: String,
    client: Client,
}

/// Errors from [`Play::tune`].
#[derive(Debug)]
pub enum TuneError {
    /// `tune` was asked for a name that is not a known profile.
    UnknownProfile(String),
    /// Parking the previous head as a lean channel (opcode 18 reconnect)
    /// failed.
    Park(LeanError),
    /// The incoming channel's reconnect login (wrapper opcode 18) failed.
    Login(LoginError),
}

impl Play {
    /// Snapshot of every slot's status.
    pub fn statuses(&self) -> Vec<SlotStatus> {
        self.statuses.lock().unwrap().clone()
    }

    /// Blocks until every slot thread exits (slot threads run forever, so
    /// this only returns if a slot panicked).
    pub fn join(self) {
        for (_, handle) in self.handles {
            let _ = handle.join();
        }
    }

    /// The control arm for a running slot, `None` when the name is not
    /// running. The panel flips the arm's flags to login/logout/stop.
    pub fn arm(&self, name: &str) -> Option<Arc<SlotArm>> {
        self.arms.get(name).cloned()
    }

    /// Stop one running slot: flag its arm `stop`, drop its login-FIFO
    /// place immediately (a queued slot must not keep later slots behind
    /// it even if the thread is still blocked in `wait_for_permit`),
    /// drop the status row and arm, then join the thread. The slot body
    /// checks `stop` every 20 ms, so the join returns within a frame when
    /// the thread is inside `run_client`. Do **not** abort the TCP link
    /// here — the caller sends a clean IF logout before calling this.
    pub fn stop_slot(&mut self, name: &str) {
        if let Some(arm) = self.arms.get(name) {
            arm.stop.store(true, Ordering::Relaxed);
            self.queue
                .lock()
                .unwrap()
                .leave(arm.uid.load(Ordering::Relaxed));
        }
        self.spawned.remove(name);
        self.statuses
            .lock()
            .unwrap()
            .retain(|s| s.username != name);
        self.arms.remove(name);
        if let Some(handle) = self.handles.remove(name) {
            let _ = handle.join();
        }
    }

    /// Register a control arm without spawning a slot thread (panel unit
    /// tests that drive login/logout flags through [`Play::arm`]).
    pub fn attach_arm(&mut self, name: &str, arm: Arc<SlotArm>) {
        self.arms.insert(name.to_string(), arm);
    }

    /// Poll until `name` reports `!ingame` (or is absent), or `timeout`
    /// elapses. Used by rail ✕ after arming a clean logout so `stop_slot`
    /// does not cut the TCP link while still ingame.
    pub fn wait_until_not_ingame(&self, name: &str, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if !self
                .statuses()
                .iter()
                .any(|s| s.username == name && s.ingame)
            {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    /// Spawn one more slot on this play's FIFO. No-op if `username` is
    /// already in the status list (already running). `None` arm behaves as
    /// [`SlotArm::new(profile.uid, true)`] — the slot logs in immediately
    /// (CLI/e2e); the panel passes a real arm so it can sit on the title.
    pub fn spawn_slot(
        &mut self,
        profile: Profile,
        input: Option<Arc<SlotInput>>,
        pixels: Option<Arc<PixelBuf>>,
        arm: Option<Arc<SlotArm>>,
    ) {
        // Keep the vault credentials on the wall: `tune` parks the
        // outgoing head and reconnects the incoming one from this map.
        self.profiles.insert(profile.username.clone(), profile.clone());
        if !self.spawned.insert(profile.username.clone()) {
            return;
        }
        let arm = arm.unwrap_or_else(|| SlotArm::new(profile.uid, true));
        // `stop_slot` leaves the FIFO by `arm.uid`; force it from the
        // profile at spawn. The store goes through the shared inner field
        // so a caller's own clone cannot keep a stale uid.
        arm.uid.store(profile.uid, Ordering::Relaxed);
        self.arms.insert(profile.username.clone(), Arc::clone(&arm));
        spawn_slot_thread(
            &self.options,
            profile,
            input,
            pixels,
            arm,
            Arc::clone(&self.cache),
            self.ifaces.clone(),
            Arc::clone(&self.queue),
            Arc::clone(&self.statuses),
            Arc::clone(&self.per_frame),
            &mut self.handles,
        );
    }

    /// Spawn one lean channel slot on this play's FIFO: `Lean::login` (cold,
    /// opcode 16, no `Client`), then pump the stream at the host cadence.
    /// The status row is marked `lean`; no-op if the name is already running.
    pub fn spawn_channel(&mut self, profile: Profile) {
        self.profiles.insert(profile.username.clone(), profile.clone());
        if !self.spawned.insert(profile.username.clone()) {
            return;
        }
        let arm = SlotArm::new(profile.uid, true);
        self.arms.insert(profile.username.clone(), Arc::clone(&arm));
        spawn_channel_thread(
            &self.options,
            profile,
            arm,
            Arc::clone(&self.queue),
            Arc::clone(&self.statuses),
            &mut self.handles,
        );
    }

    /// Tune the head to `name` (274bot channel head). Sequence:
    /// 1. drop `name`'s lean channel if one is up (the server sees that
    ///    account DC);
    /// 2. park the current head: `Client::logout` then a `Lean::login`
    ///    reconnect (opcode **18**, grant 15) so the previous account —
    ///    whose head socket just dropped, a DC — stays ingame as a
    ///    channel;
    /// 3. reconnect the head as `name` with `login(..., reconnect = true)`
    ///    (wrapper **18**, grant 15 — the server sees a valid lost_con
    ///    reconnect for the account whose lean socket just dropped);
    /// 4. wipe the previous channel's scene (`scene_state = 0`, fresh
    ///    `localPlayer`, cleared player/npc tables) — stock response 15
    ///    keeps state for a *same-session* `lost_con`, which is the wrong
    ///    story across accounts.
    ///
    /// The first tune (no head yet) skips the park; the incoming handshake
    /// is opcode 18 either way. Tuning the current head is a no-op.
    pub fn tune(&mut self, name: &str) -> Result<(), TuneError> {
        let profile = self
            .profiles
            .get(name)
            .cloned()
            .ok_or_else(|| TuneError::UnknownProfile(name.to_string()))?;
        if self.head.as_ref().is_some_and(|h| h.name == name) {
            return Ok(());
        }

        // 1. The incoming account leaves the lean wall; the server sees
        //    that connection drop, so the opcode-18 reconnect below is a
        //    valid lost_con for the same account.
        self.channels.remove(name);

        // 2. Park the current head: close its socket, then reconnect the
        //    previous account as a lean channel (opcode 18, grant 15 —
        //    the dropped head socket is a DC).
        let mut client = if let Some(mut head) = self.head.take() {
            let prev_name = head.name.clone();
            head.client.logout();
            let prev = self
                .profiles
                .get(&prev_name)
                .cloned()
                .ok_or_else(|| TuneError::UnknownProfile(prev_name.clone()))?;
            let config = bot_client_config(&self.options, &prev);
            match Lean::login(&config, &prev.username, &prev.password, prev.uid, true) {
                Ok(lean) => {
                    self.channels.insert(prev_name, lean);
                }
                Err(e) => {
                    // Park failed: restore the logged-out head so a retry
                    // re-parks (a fresh connect) instead of re-logging in.
                    self.head = Some(head);
                    return Err(TuneError::Park(e));
                }
            }
            head.client
        } else {
            prepare_client(
                bot_client_config(&self.options, &profile),
                profile.uid,
                Arc::clone(&self.cache),
                self.ifaces.clone(),
            )
        };

        // 3. Reconnect the head as `name` (wrapper opcode 18, grant 15).
        //    The Client is reused across tunes, so its RSA login block must
        //    carry `name`'s login uid, not the previous head's.
        client.login_uid = profile.uid;
        client
            .login(name, &profile.password, true)
            .map_err(TuneError::Login)?;

        // 4. Response 15 keeps the previous session's state; a channel
        //    change is a different account's scene, so wipe it.
        client.wipe_scene();

        self.head = Some(Head {
            name: name.to_string(),
            client,
        });
        Ok(())
    }
}

/// Spawn one slot thread per profile. Each slot waits for a login-queue
/// permit, sends the handshake, then drives `mainloop` at the host cadence
/// while mirroring its state into the shared status list. Slots run with no
/// input and no pixel output; [`run_with_io`] adds per-slot channels.
pub fn run(options: &PlayOptions, profiles: Vec<Profile>) -> Play {
    run_with_io(options, profiles, |_| (None, None), |_, _| {})
}

/// Like [`run`], but each slot gets the `SlotInput`/`PixelBuf` returned by
/// `per_slot` (called synchronously, keyed by username), and `per_frame`
/// runs inside the observe hook on every 20 ms frame so callers can mirror
/// panel state (e.g. `client.set_draw`) into the slot thread. The FIFO
/// login queue and mainland hop are shared by every slot.
pub fn run_with_io<F, G>(
    options: &PlayOptions,
    profiles: Vec<Profile>,
    per_slot: F,
    per_frame: G,
) -> Play
where
    F: Fn(&str) -> (Option<Arc<SlotInput>>, Option<Arc<PixelBuf>>),
    G: Fn(&mut Client, &str) + Send + Sync + 'static,
{
    let (cache, ifaces) = load_template(&options.cache_dir);
    let mut play = Play {
        statuses: Arc::new(Mutex::new(Vec::new())),
        handles: HashMap::new(),
        options: options.clone(),
        cache: Arc::new(cache),
        ifaces,
        queue: Arc::new(Mutex::new(LoginQueue::default())),
        per_frame: Arc::new(per_frame),
        spawned: HashSet::new(),
        arms: HashMap::new(),
        profiles: HashMap::new(),
        head: None,
        channels: HashMap::new(),
    };
    for profile in profiles {
        let (slot_input, slot_pixels) = per_slot(&profile.username);
        play.spawn_slot(profile, slot_input, slot_pixels, None);
    }
    play
}

/// Channel-head wall spawn: the first `heads` profiles (0 or 1) are fat
/// `Client` slots and every other profile is a lean channel — no World, no
/// ifaces, no caches (`host::lean::Lean`). The head stays draw-off like a
/// wall slot; only the live channel ladder uses this today, so the panel
/// `stress50` full-`Client` path through [`run_with_io`] is untouched.
pub fn run_channels(options: &PlayOptions, profiles: Vec<Profile>, heads: usize) -> Play {
    let (cache, ifaces) = load_template(&options.cache_dir);
    let mut play = Play {
        statuses: Arc::new(Mutex::new(Vec::new())),
        handles: HashMap::new(),
        options: options.clone(),
        cache: Arc::new(cache),
        ifaces,
        queue: Arc::new(Mutex::new(LoginQueue::default())),
        per_frame: Arc::new(|c: &mut Client, _: &str| c.set_draw(false)),
        spawned: HashSet::new(),
        arms: HashMap::new(),
        profiles: HashMap::new(),
        head: None,
        channels: HashMap::new(),
    };
    for (i, profile) in profiles.into_iter().enumerate() {
        if i < heads {
            play.spawn_slot(profile, None, None, None);
        } else {
            play.spawn_channel(profile);
        }
    }
    play
}

/// Slot client config: connection fields from `options`, memory profile
/// from the vault profile (`settings.lowmem` defaults true; panel/CLI may
/// set false for this run).
fn bot_client_config(options: &PlayOptions, profile: &Profile) -> ClientConfig {
    ClientConfig {
        host: options.host.clone(),
        port: options.port,
        cache_dir: options.cache_dir.clone(),
        members: true,
        lowmem: profile.settings.lowmem,
    }
}

fn spawn_slot_thread(
    options: &PlayOptions,
    profile: Profile,
    slot_input: Option<Arc<SlotInput>>,
    slot_pixels: Option<Arc<PixelBuf>>,
    arm: Arc<SlotArm>,
    slot_cache: Arc<Cache>,
    ifaces_template: Vec<Option<IfType>>,
    slot_queue: Arc<Mutex<LoginQueue>>,
    slot_statuses: Arc<Mutex<Vec<SlotStatus>>>,
    slot_frame: Arc<dyn Fn(&mut Client, &str) + Send + Sync>,
    handles: &mut HashMap<String, thread::JoinHandle<()>>,
) {
    let username = profile.username.clone();
    let uid = profile.uid;
    let password = profile.password.clone();
    let config = bot_client_config(options, &profile);
    let mainland = options.mainland;

    handles.insert(
        username.clone(),
        thread::Builder::new()
            .name(username.clone())
            .stack_size(THREAD_STACK)
            .spawn(move || {
            let mut client = prepare_client(config, uid, slot_cache, ifaces_template.clone());
            #[cfg(test)]
            {
                // Unit tests spawn slots with no web server on :80; shrink
                // maininit's HTTP retry so `stop_slot`'s join returns fast
                // (the client's own HTTP tests stub retries the same way).
                client.fetch_retry_wait = Duration::from_millis(1);
            }

            {
                let mut all = slot_statuses.lock().unwrap();
                all.push(SlotStatus {
                    username: username.clone(),
                    ..SlotStatus::default()
                });
            }
            if debug_enabled() {
                eprintln!("[host-play] slot {username}: thread up");
            }

            // Jag/anim/model/map prefetch (mirrors client-play; the scene
            // cannot reach scene_state 2 until the loc models are in).
            client.maininit();
            if client.error_loading {
                if debug_enabled() {
                    eprintln!("[host-play] slot {username}: maininit failed");
                }
            }

            let mut backoff = LoginBackoff::new();
            loop {
                if arm.stop.load(Ordering::Relaxed) {
                    slot_queue.lock().unwrap().leave(uid);
                    return;
                }
                if !should_handshake(&arm, client.ingame) {
                    // Sit on the title: no permit request, no handshake, until
                    // the arm wants a login (auto-login / Log in / Login all).
                    thread::sleep(Duration::from_millis(20));
                    continue;
                }
                wait_for_permit(&slot_queue, &slot_statuses, &username, uid, &arm.stop);
                if arm.stop.load(Ordering::Relaxed) {
                    slot_queue.lock().unwrap().leave(uid);
                    return;
                }
                let login_started = Instant::now();
                {
                    let mut all = slot_statuses.lock().unwrap();
                    if let Some(s) = all.iter_mut().find(|s| s.username == username) {
                        // First attempt only: retries must not move the
                        // handshake-start metric the harness asserts on.
                        if s.login_started.is_none() {
                            s.login_started = Some(login_started);
                        }
                        s.error = None;
                    }
                }
                match client.login(&username, &password, false) {
                    Ok(()) => {
                        backoff.reset();
                        // Auto-login slots stay armed so an unexpected DC
                        // re-handshakes; a one-shot Log in / Login all disarms
                        // until the next explicit arm.
                        on_login_success(&arm);
                        if debug_enabled() {
                            eprintln!("[host-play] slot {username}: ingame");
                        }
                        let mut mainland_sent = false;
                        Host::run_client(
                            &mut client,
                            &username,
                            slot_input.clone(),
                            slot_pixels.clone(),
                            |c, name, run_sends| {
                                slot_frame(c, name);
                                if !mainland_sent && mainland && c.ingame && c.scene_state == 2 {
                                    api::interact::mainland_hop(c);
                                    mainland_sent = true;
                                    if debug_enabled() {
                                        eprintln!("[host-play] slot {name}: queued mainland tele+setvar (scene 2)");
                                    }
                                }
                                let mut all = slot_statuses.lock().unwrap();
                                for s in all.iter_mut() {
                                    if s.username == name {
                                        s.ingame = c.ingame;
                                        s.scene_state = c.scene_state;
                                        s.runenergy = c.runenergy;
                                        s.run_sends = run_sends;
                                        s.main_modal_id = c.main_modal_id;
                                        copy_stream_and_draw(c, s);
                                        if let Some(lp) = &c.local_player {
                                            let (tx, tz) = player_world_tile(
                                                c.map_build_base_x,
                                                c.map_build_base_z,
                                                lp.route_x[0],
                                                lp.route_z[0],
                                            );
                                            s.tile_x = tx;
                                            s.tile_z = tz;
                                            s.player = lp.name.clone().unwrap_or_default();
                                        }
                                    }
                                }
                                drop(all);
                                if debug_enabled() && c.ingame && c.scene_state == 1 && c.loop_cycle % 100 == 0 {
                                    let od = c
                                        .on_demand
                                        .as_ref()
                                        .map(|od| {
                                            format!(
                                                "remaining={} fail={} msg={:?}",
                                                od.remaining(),
                                                od.fail_count,
                                                od.message
                                            )
                                        })
                                        .unwrap_or_else(|| "ondemand=none".into());
                                    let ground = c
                                        .map_build_ground_data
                                        .iter()
                                        .filter(|d| d.is_some())
                                        .count();
                                    let locs = c
                                        .map_build_location_data
                                        .iter()
                                        .filter(|d| d.is_some())
                                        .count();
                                    eprintln!(
                                        "[host-play] slot {name}: scene loading {od} \
                                         ground={}/{} loc={}/{} await_pi={}",
                                        ground,
                                        c.map_build_ground_data.len(),
                                        locs,
                                        c.map_build_location_data.len(),
                                        c.awaiting_player_info
                                    );
                                }
                            },
                            |c| {
                                // Leave the 20 ms body on `stop`, or once the
                                // client is back on the title (clean IF logout
                                // / DC) so this control loop decides the next
                                // handshake. `tick_flags` presses CC_LOGOUT
                                // while the arm asks for a logout.
                                tick_flags(c, &ifaces_template, &arm) || !c.ingame
                            },
                        );
                        // The 20 ms body exits as soon as the client leaves the
                        // game (clean IF logout / DC / stop); the last observe
                        // ran before the exit, so record the title state here —
                        // statuses, the rail traffic light, and the live
                        // harness must see `!ingame`.
                        {
                            let mut all = slot_statuses.lock().unwrap();
                            if let Some(s) = all.iter_mut().find(|s| s.username == username) {
                                s.ingame = client.ingame;
                                s.scene_state = client.scene_state;
                            }
                        }
                    }
                    Err(e) => {
                        let msg = format!("code {}: {}", e.code, e.mes2);
                        if debug_enabled() {
                            eprintln!("[host-play] slot {username}: login {msg}");
                        }
                        {
                            let mut all = slot_statuses.lock().unwrap();
                            if let Some(s) = all.iter_mut().find(|s| s.username == username) {
                                s.error = Some(msg);
                            }
                        }
                        // Response 16 (world full) escalates; response 5 is
                        // the engine's login-limit message ("Try again in
                        // 60 secs") and waits that long; other codes (wrong
                        // credentials, RSA mismatch, ...) retry slower.
                        let wait = match e.code {
                            16 => backoff.delay(),
                            5 => Duration::from_secs(60),
                            _ => Duration::from_secs(5),
                        };
                        thread::sleep(wait);
                    }
                }
            }
            })
            .expect("failed to spawn slot thread"),
    );
}

/// Spawn one lean channel thread. The channel waits for a login-queue
/// permit (shared FIFO with the head), cold-logins with `Lean::login`
/// (wrapper opcode 16), then pumps inbound frames at the host cadence —
/// no `maininit`, no `Client`, no pixels, no keepalive. The row mirrors
/// the thin `LeanSnapshot`; `lean` is true so the ladder counts it.
fn spawn_channel_thread(
    options: &PlayOptions,
    profile: Profile,
    arm: Arc<SlotArm>,
    slot_queue: Arc<Mutex<LoginQueue>>,
    slot_statuses: Arc<Mutex<Vec<SlotStatus>>>,
    handles: &mut HashMap<String, thread::JoinHandle<()>>,
) {
    let username = profile.username.clone();
    let uid = profile.uid;
    let password = profile.password.clone();
    let config = bot_client_config(options, &profile);

    handles.insert(
        username.clone(),
        thread::Builder::new()
            .name(format!("{username}-lean"))
            .stack_size(THREAD_STACK)
            .spawn(move || {
                {
                    let mut all = slot_statuses.lock().unwrap();
                    all.push(SlotStatus {
                        username: username.clone(),
                        lean: true,
                        ..SlotStatus::default()
                    });
                }
                loop {
                    if arm.stop.load(Ordering::Relaxed) {
                        slot_queue.lock().unwrap().leave(uid);
                        return;
                    }
                    wait_for_permit(&slot_queue, &slot_statuses, &username, uid, &arm.stop);
                    if arm.stop.load(Ordering::Relaxed) {
                        slot_queue.lock().unwrap().leave(uid);
                        return;
                    }
                    let login_started = Instant::now();
                    {
                        let mut all = slot_statuses.lock().unwrap();
                        if let Some(s) = all.iter_mut().find(|s| s.username == username) {
                            // First attempt only: retries must not move the
                            // handshake-start metric the harness polls.
                            if s.login_started.is_none() {
                                s.login_started = Some(login_started);
                            }
                            s.error = None;
                        }
                    }
                    match Lean::login(&config, &username, &password, uid, false) {
                        Ok(mut lean) => {
                            if debug_enabled() {
                                eprintln!("[host-play] channel {username}: ingame");
                            }
                            {
                                let mut all = slot_statuses.lock().unwrap();
                                if let Some(s) = all.iter_mut().find(|s| s.username == username) {
                                    s.ingame = true;
                                }
                            }
                            // Pump inbound frames at the host cadence until the
                            // connection dies or the arm stops; a dead channel
                            // falls back to the permit + login retry above.
                            loop {
                                if arm.stop.load(Ordering::Relaxed) {
                                    slot_queue.lock().unwrap().leave(uid);
                                    return;
                                }
                                let pump = lean.pump();
                                let mut died = false;
                                {
                                    let mut all = slot_statuses.lock().unwrap();
                                    if let Some(s) =
                                        all.iter_mut().find(|s| s.username == username)
                                    {
                                        match pump {
                                            Ok(()) => {
                                                let snap = lean.snapshot();
                                                s.scene_state = snap.scene_state;
                                                s.tile_x = snap.tile_x;
                                                s.tile_z = snap.tile_z;
                                            }
                                            Err(e) => {
                                                s.error = Some(match &e {
                                                    LeanError::Login(le) => format!(
                                                        "code {}: {}",
                                                        le.code, le.mes2
                                                    ),
                                                    LeanError::Io(io) => format!("io: {io}"),
                                                    LeanError::FrameTooLarge { ptype, psize } => {
                                                        format!(
                                                            "frame too large ptype={ptype} \
                                                             psize={psize}"
                                                        )
                                                    }
                                                });
                                                s.ingame = false;
                                                died = true;
                                            }
                                        }
                                    }
                                }
                                if died {
                                    break;
                                }
                                thread::sleep(Duration::from_millis(20));
                            }
                        }
                        Err(e) => {
                            let msg = match &e {
                                LeanError::Login(le) => format!("code {}: {}", le.code, le.mes2),
                                LeanError::Io(io) => format!("io: {io}"),
                                LeanError::FrameTooLarge { ptype, psize } => {
                                    format!("frame too large ptype={ptype} psize={psize}")
                                }
                            };
                            if debug_enabled() {
                                eprintln!("[host-play] channel {username}: login {msg}");
                            }
                            {
                                let mut all = slot_statuses.lock().unwrap();
                                if let Some(s) = all.iter_mut().find(|s| s.username == username) {
                                    s.error = Some(msg);
                                }
                            }
                            // A rejected lean login retries after the same
                            // codes as the fat slots (world full / login
                            // limit / wrong credentials).
                            let wait = match &e {
                                LeanError::Login(le) => match le.code {
                                    16 => Duration::from_secs(20),
                                    5 => Duration::from_secs(60),
                                    _ => Duration::from_secs(5),
                                },
                                _ => Duration::from_secs(5),
                            };
                            let deadline = Instant::now() + wait;
                            while Instant::now() < deadline {
                                if arm.stop.load(Ordering::Relaxed) {
                                    slot_queue.lock().unwrap().leave(uid);
                                    return;
                                }
                                thread::sleep(Duration::from_millis(20));
                            }
                        }
                    }
                }
            })
            .expect("failed to spawn channel thread"),
    );
}

/// Unlock `path`, or create it (and parent dirs) when missing. Any other
/// unlock error (`WrongPassphrase`, `Corrupt`, `EmptyPassphrase`) is
/// returned as-is so the CLI can print it instead of falling through to
/// `AlreadyExists`.
pub fn open_vault(path: &Path, passphrase: &str) -> Result<Vault, VaultError> {
    match Vault::unlock(path, passphrase) {
        Ok(v) => Ok(v),
        Err(VaultError::NotFound(_)) => {
            if let Some(parent) = path.parent() {
                if !parent.as_os_str().is_empty() {
                    std::fs::create_dir_all(parent)?;
                }
            }
            Vault::create(path, passphrase)
        }
        Err(e) => Err(e),
    }
}

/// Unpack the config/interface jags once and share the tables across slots
/// (the client's `load_cache` is private; this mirrors it with the same
/// public `Cache::unpack` / `IfType::unpack` entry points).
fn load_template(cache_dir: &str) -> (Cache, Vec<Option<IfType>>) {
    let cache = match std::fs::read(format!("{cache_dir}/config")) {
        Ok(bytes) => {
            std::panic::catch_unwind(AssertUnwindSafe(|| Cache::unpack(&JagFile::new(bytes))))
                .unwrap_or_default()
        }
        Err(_) => Cache::default(),
    };
    let ifaces = match std::fs::read(format!("{cache_dir}/interface")) {
        Ok(bytes) => {
            std::panic::catch_unwind(AssertUnwindSafe(|| IfType::unpack(&JagFile::new(bytes))))
                .unwrap_or_default()
        }
        Err(_) => Vec::new(),
    };
    (cache, ifaces)
}

/// Copy a login-queue snapshot onto every `SlotStatus` row named `name`;
/// `None` (granted or not queued) clears both fields back to -1.
fn apply_queue_wait(rows: &mut [SlotStatus], name: &str, pos: Option<QueuePos>) {
    let (position, total) = match pos {
        Some(p) => (p.position as i32, p.total as i32),
        None => (-1, -1),
    };
    for s in rows.iter_mut().filter(|s| s.username == name) {
        s.queue_position = position;
        s.queue_total = total;
    }
}

/// Block until the login queue grants `uid` a handshake permit, mirroring
/// the queue position onto the slot's status row while it waits. Observes
/// `stop` each iteration **before** `request_permit` so a `leave` from
/// [`Play::stop_slot`] is not undone by a re-enqueue, and returns without
/// granting when stop is set.
fn wait_for_permit(
    queue: &Arc<Mutex<LoginQueue>>,
    statuses: &Arc<Mutex<Vec<SlotStatus>>>,
    username: &str,
    uid: i32,
    stop: &AtomicBool,
) {
    loop {
        if stop.load(Ordering::Relaxed) {
            queue.lock().unwrap().leave(uid);
            let mut all = statuses.lock().unwrap();
            apply_queue_wait(&mut all, username, None);
            return;
        }
        let wait = {
            let mut q = queue.lock().unwrap();
            match q.request_permit(uid, Instant::now()) {
                Permit::Grant => {
                    drop(q);
                    let mut all = statuses.lock().unwrap();
                    apply_queue_wait(&mut all, username, None);
                    return;
                }
                Permit::Wait(wait) => {
                    let pos = q.status(uid);
                    drop(q);
                    let mut all = statuses.lock().unwrap();
                    apply_queue_wait(&mut all, username, pos);
                    drop(all);
                    wait
                }
            }
        };
        thread::sleep(wait);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::Arc;
    use std::thread;

    use client::client::{ClientConfig, ClientNpc, ClientPlayer};
    use client::config::Cache;
    use vault::ProfileSettings;

    fn tmp_vault(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("274bot-host-play-{}-{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("nested").join("vault")
    }

    #[test]
    fn spawn_config_follows_profile_lowmem() {
        let opt = PlayOptions {
            host: "127.0.0.1".into(),
            port: 1,
            cache_dir: "/tmp".into(),
            lowmem: true,
            mainland: false,
        };
        let lean = Profile {
            username: "a".into(),
            password: "a".into(),
            uid: 1,
            settings: ProfileSettings { lowmem: true, auto_login: false },
        };
        let loud = Profile {
            username: "b".into(),
            password: "b".into(),
            uid: 2,
            settings: ProfileSettings { lowmem: false, auto_login: false },
        };
        assert!(bot_client_config(&opt, &lean).lowmem);
        assert!(!bot_client_config(&opt, &loud).lowmem);
    }

    #[test]
    fn open_vault_creates_missing_parent_dirs() {
        let path = tmp_vault("create");
        assert!(!path.exists());
        let v = open_vault(&path, "bot").unwrap();
        drop(v);
        assert!(path.exists());
    }

    #[test]
    fn open_vault_wrong_pass_is_not_already_exists() {
        let path = tmp_vault("wrong");
        open_vault(&path, "bot").unwrap();
        match open_vault(&path, "nope") {
            Err(VaultError::WrongPassphrase) => {}
            Err(e) => panic!("expected WrongPassphrase, got {e}"),
            Ok(_) => panic!("expected WrongPassphrase, unlocked"),
        }
    }

    #[test]
    fn run_with_io_empty_profiles_starts_no_slots() {
        let play = run_with_io(
            &PlayOptions {
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
        assert!(play.statuses().is_empty());
    }

    #[test]
    fn run_channels_empty_profiles_starts_no_slots() {
        let play = run_channels(
            &PlayOptions {
                host: "127.0.0.1".into(),
                port: 43594,
                cache_dir: "/tmp".into(),
                lowmem: true,
                mainland: false,
            },
            vec![],
            1,
        );
        assert!(play.statuses().is_empty());
    }

    #[test]
    fn channel_rows_default_to_fat_and_channel_rows_mark_lean() {
        let mut s = SlotStatus {
            username: "t".into(),
            ..SlotStatus::default()
        };
        assert!(!s.lean, "default row is a full Client slot");
        s.lean = true;
        assert!(s.lean, "a lean channel row carries the flag");
    }

    #[test]
    fn stop_slot_sets_stop_and_forgets_name() {
        let mut play = run_with_io(
            &PlayOptions {
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
        // No real client: fake arm (uid 7) and a handle that exits only
        // once `stop_slot` flags it.
        let arm = SlotArm::new(7, false);
        play.arms.insert("alice".into(), Arc::clone(&arm));
        let watchdog = {
            let arm = Arc::clone(&arm);
            thread::spawn(move || {
                while !arm.stop.load(Ordering::Relaxed) {
                    thread::sleep(Duration::from_millis(1));
                }
            })
        };
        play.handles.insert("alice".into(), watchdog);
        play.spawned.insert("alice".into());
        // uid 7 sits queued behind a granted head; stop_slot must drop it
        // from the FIFO even though the thread is still running.
        {
            let mut q = play.queue.lock().unwrap();
            assert!(matches!(q.request_permit(1, Instant::now()), Permit::Grant));
            assert!(matches!(q.request_permit(7, Instant::now()), Permit::Wait(_)));
        }

        play.statuses.lock().unwrap().push(SlotStatus {
            username: "alice".into(),
            ..SlotStatus::default()
        });

        play.stop_slot("alice");

        assert!(arm.stop.load(Ordering::Relaxed));
        assert!(!play.spawned.contains("alice"));
        assert!(play.handles.is_empty());
        assert!(play.arms.get("alice").is_none(), "stop_slot drops the arm");
        assert!(
            play.statuses().iter().all(|s| s.username != "alice"),
            "stop_slot drops the status row"
        );
        assert!(play.queue.lock().unwrap().status(7).is_none());
    }

    #[test]
    fn stop_slot_leaves_profile_uid_when_arm_shared_at_spawn() {
        // A caller that retains its own clone makes the arm shared before
        // spawn; the uid must still be forced from the profile (an
        // `Arc::get_mut` fixup would silently no-op here).
        let arm = SlotArm::new(0, false);
        let _caller_clone = Arc::clone(&arm);
        let mut play = run_with_io(
            &PlayOptions {
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
        // The profile uid 42 sits queued behind a granted head; stopping
        // must drop 42 from the FIFO, not the arm's stale uid 0.
        {
            let mut q = play.queue.lock().unwrap();
            assert!(matches!(q.request_permit(1, Instant::now()), Permit::Grant));
            assert!(matches!(q.request_permit(42, Instant::now()), Permit::Wait(_)));
        }
        play.spawn_slot(
            Profile {
                username: "alice".into(),
                password: "pw".into(),
                uid: 42,
                settings: ProfileSettings::default(),
            },
            None,
            None,
            Some(Arc::clone(&arm)),
        );

        assert_eq!(arm.uid.load(Ordering::Relaxed), 42);
        play.stop_slot("alice");

        assert!(arm.stop.load(Ordering::Relaxed));
        assert!(play.queue.lock().unwrap().status(42).is_none());
        assert!(!play.spawned.contains("alice"));
        assert!(play.handles.is_empty());
    }

    #[test]
    fn prepare_client_from_shared_template() {
        let cache = Arc::new(Cache::default());
        let cfg = ClientConfig {
            host: "127.0.0.1".into(),
            port: 43594,
            cache_dir: "/tmp".into(),
            members: true,
            lowmem: true,
        };
        let a = prepare_client(cfg, 1, Arc::clone(&cache), vec![]);
        assert!(Arc::ptr_eq(&a.cache, &cache));
        assert!(!a.error_loading);
    }

    #[test]
    fn slot_status_walk_defaults_cleared() {
        let s = SlotStatus::default();
        assert_eq!((s.walk_x, s.walk_z, s.walk_level), (-1, -1, -1));
    }

    #[test]
    fn copy_stream_and_draw_zeros_without_stream() {
        let c = prepare_client(
            ClientConfig {
                host: "127.0.0.1".into(),
                port: 1,
                cache_dir: String::new(),
                members: true,
                lowmem: true,
            },
            1,
            Arc::new(Cache::default()),
            vec![],
        );
        let mut s = SlotStatus {
            username: "t".into(),
            ..SlotStatus::default()
        };
        copy_stream_and_draw(&c, &mut s);
        assert_eq!(s.bytes_in, 0);
        assert_eq!(s.bytes_out, 0);
        assert_eq!(s.game_draw_enters, 0);
        assert_eq!(s.title_screen_draw_enters, 0);
    }

    #[test]
    fn copy_stream_and_draw_copies_timing_fields() {
        let mut c = prepare_client(
            ClientConfig {
                host: "127.0.0.1".into(),
                port: 1,
                cache_dir: String::new(),
                members: true,
                lowmem: true,
            },
            1,
            Arc::new(Cache::default()),
            vec![],
        );
        c.loop_ns = 10;
        c.raster_ns = 3;
        c.paint_n = 2;
        c.skip_n = 8;
        let mut s = SlotStatus {
            username: "t".into(),
            ..SlotStatus::default()
        };
        copy_stream_and_draw(&c, &mut s);
        assert_eq!(s.loop_ns, 10);
        assert_eq!(s.raster_ns, 3);
        assert_eq!(s.paint_n, 2);
        assert_eq!(s.skip_n, 8);
    }

    #[test]
    fn apply_queue_wait_writes_k_of_n_and_grant_clears() {
        let mut rows = vec![
            SlotStatus { username: "a".into(), queue_position: -1, queue_total: -1, ..SlotStatus::default() },
            SlotStatus { username: "b".into(), queue_position: -1, queue_total: -1, ..SlotStatus::default() },
        ];
        apply_queue_wait(&mut rows, "b", Some(host::login_queue::QueuePos { position: 2, total: 2 }));
        assert_eq!(rows[1].queue_position, 2);
        assert_eq!(rows[1].queue_total, 2);
        apply_queue_wait(&mut rows, "b", None);
        assert_eq!(rows[1].queue_position, -1);
        assert_eq!(rows[1].queue_total, -1);
    }

    #[test]
    fn spawn_without_auto_login_does_not_handshake() {
        let arm = SlotArm::new(0, false);
        assert!(!should_handshake(&arm, false));
        arm.want_login.store(true, Ordering::Relaxed);
        assert!(should_handshake(&arm, false));
        arm.latch.store(true, Ordering::Relaxed);
        assert!(!should_handshake(&arm, false));
        arm.latch.store(false, Ordering::Relaxed);
        assert!(should_handshake(&arm, false));
        assert!(!should_handshake(&arm, true));
    }

    #[test]
    fn login_success_keeps_auto_login_armed_but_disarms_one_shot() {
        // CLI: `new(uid, true)` (auto_login true) stays armed so an unexpected
        // DC re-handshakes.
        let arm = SlotArm::new(0, true);
        on_login_success(&arm);
        assert!(should_handshake(&arm, false));

        // Panel Log in / Login all: armed explicitly, then disarmed after
        // the handshake — a DC sits on the title.
        let arm = SlotArm::new(0, false);
        arm.want_login.store(true, Ordering::Relaxed);
        on_login_success(&arm);
        assert!(!should_handshake(&arm, false));

        // The intentional-logout latch blocks even an auto-login slot.
        let arm = SlotArm::new(0, true);
        arm.latch.store(true, Ordering::Relaxed);
        on_login_success(&arm);
        assert!(!should_handshake(&arm, false));
    }

    #[test]
    fn tick_flags_presses_logout_when_ingame_and_reports_stop() {
        let cfg = ClientConfig {
            host: "127.0.0.1".into(),
            port: 43594,
            cache_dir: "/tmp".into(),
            members: true,
            lowmem: true,
        };
        let mut ifaces = vec![None; 10];
        let mut com = IfType::default();
        com.client_code = api::interact::CC_LOGOUT;
        ifaces[7] = Some(com);
        let mut client = prepare_client(cfg, 1, Arc::new(Cache::default()), ifaces.clone());
        client.ingame = true;
        let arm = SlotArm::new(0, false);
        arm.want_logout.store(true, Ordering::Relaxed);
        // Even with stop already set, the logout probe must return false so
        // the body keeps running until !ingame (no dirty disconnect).
        arm.stop.store(true, Ordering::Relaxed);

        assert!(!tick_flags(&mut client, &ifaces, &arm));
        assert!(!arm.want_logout.load(Ordering::Relaxed));
        assert!(arm.latch.load(Ordering::Relaxed));
        assert!(!arm.want_login.load(Ordering::Relaxed));
        assert_eq!(
            client.out.data()[0],
            client::io::ClientProt::IF_BUTTON.id as u8
        );

        // After the logout press, a later probe honors stop.
        assert!(tick_flags(&mut client, &ifaces, &arm));

        // A title slot never presses; `stop` still reports.
        client.ingame = false;
        arm.want_logout.store(true, Ordering::Relaxed);
        assert!(tick_flags(&mut client, &ifaces, &arm));
        assert!(
            arm.want_logout.load(Ordering::Relaxed),
            "no CC_LOGOUT press on the title; the flag stays for the panel"
        );
    }

    #[test]
    fn wait_for_permit_returns_without_reenqueue_when_stop_set() {
        let queue = Arc::new(Mutex::new(LoginQueue::default()));
        let statuses = Arc::new(Mutex::new(vec![SlotStatus {
            username: "alice".into(),
            ..SlotStatus::default()
        }]));
        let stop = AtomicBool::new(false);
        // Occupy the FIFO head so alice waits.
        assert!(matches!(
            queue.lock().unwrap().request_permit(1, Instant::now()),
            Permit::Grant
        ));
        assert!(matches!(
            queue.lock().unwrap().request_permit(7, Instant::now()),
            Permit::Wait(_)
        ));
        // Simulate stop_slot: leave then set stop; the waiter must not
        // request_permit again (which would Grant or re-queue uid 7).
        queue.lock().unwrap().leave(7);
        stop.store(true, Ordering::Relaxed);
        wait_for_permit(&queue, &statuses, "alice", 7, &stop);
        assert!(
            queue.lock().unwrap().status(7).is_none(),
            "stop must not re-enqueue after leave"
        );
    }

    #[test]
    fn wait_until_not_ingame_observes_status_flip() {
        let play = run_with_io(
            &PlayOptions {
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
        play.statuses.lock().unwrap().push(SlotStatus {
            username: "alice".into(),
            ingame: true,
            ..SlotStatus::default()
        });
        let statuses = Arc::clone(&play.statuses);
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(30));
            if let Some(s) = statuses.lock().unwrap().iter_mut().find(|s| s.username == "alice")
            {
                s.ingame = false;
            }
        });
        assert!(play.wait_until_not_ingame("alice", Duration::from_secs(1)));
    }

    #[test]
    fn player_world_tile_adds_build_base_to_route_head() {
        // Lumbridge courtyard: base 3200,3200 + route 22,20 → 3222,3220.
        assert_eq!(player_world_tile(3200, 3200, 22, 20), (3222, 3220));
        // Catherby range door from: base 2752,3392 + route 64,45 → 2816,3437.
        assert_eq!(player_world_tile(2752, 3392, 64, 45), (2816, 3437));
        assert_ne!(
            player_world_tile(3200, 3200, 22, 20),
            (22 * 128, 20 * 128),
            "must not report scene pixels"
        );
    }

    fn profile(name: &str, uid: i32) -> Profile {
        Profile {
            username: name.into(),
            password: "pw".into(),
            uid,
            settings: ProfileSettings {
                lowmem: true,
                auto_login: false,
            },
        }
    }

    /// Tune B (274bot Task 3): the head reconnects as B with wrapper opcode
    /// **18** (a valid lost_con reconnect after B's lean channel is
    /// dropped), the previous head A is parked as a **reconnect** lean
    /// channel (opcode 18 — the dropped head socket is a DC), and the
    /// response-15 grant is followed by a scene wipe (stock response 15
    /// keeps state for a same-session `lost_con`; a channel change is a
    /// different account's scene).
    #[test]
    fn tune_b_handshake_is_18_and_parks_a_as_lean() {
        // One fake server on the shared host:port; the connection order is
        // deterministic: A's tune-in (18→15), A's park lean (18→15), B's
        // tune-in (18→15). The wrapper opcode of each loginout block is
        // recorded (buf[0]) so the test can pin the 18-not-16 contract.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let wrappers = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&wrappers);
        let server = thread::spawn(move || {
            for _ in 0..3 {
                let (mut s, _) = listener.accept().unwrap();
                let mut hdr = [0u8; 2];
                s.read_exact(&mut hdr).unwrap();
                assert_eq!(hdr[0], 14); // login server probe
                for _ in 0..8 {
                    s.write_all(&[0]).unwrap();
                }
                s.write_all(&[0]).unwrap(); // response 0 → send seed
                s.write_all(&[0u8; 8]).unwrap(); // g8 seed
                let mut buf = [0u8; 512];
                let n = s.read(&mut buf).unwrap();
                assert!(n > 0);
                log.lock().unwrap().push(buf[0]);
                s.write_all(&[15]).unwrap(); // reconnect grant (15, not 2)
            }
        });

        let opts = PlayOptions {
            host: "127.0.0.1".into(),
            port: addr.port(),
            cache_dir: "/tmp".into(),
            lowmem: true,
            mainland: false,
        };
        let mut play = run_with_io(&opts, vec![], |_| (None, None), |_, _| {});
        play.profiles.insert("a".into(), profile("a", 1));
        play.profiles.insert("b".into(), profile("b", 2));

        play.tune("a").unwrap(); // establish the head (still opcode 18)
        assert_eq!(play.head.as_ref().unwrap().client.login_uid, 1);
        // A's scene is live; tune("b") must wipe it, not keep it.
        let head = play.head.as_mut().unwrap();
        head.client.scene_state = 1;
        head.client.player_count = 1;
        head.client.players[123] = Some(ClientPlayer::at(3, 4));
        head.client.npc_count = 1;
        head.client.npc[7] = Some(ClientNpc::default());
        head.client.local_player.as_mut().unwrap().y = 77;

        play.tune("b").unwrap();

        {
            let w = wrappers.lock().unwrap();
            assert_eq!(
                w.as_slice(),
                &[18, 18, 18],
                "tune-in and park are both opcode 18 reconnects"
            );
        }
        let head = play.head.as_ref().unwrap();
        assert_eq!(head.name, "b");
        assert_eq!(head.client.login_user, "b");
        assert_eq!(
            head.client.login_uid, 2,
            "the reused head Client must carry B's login uid, not A's"
        );
        assert!(head.client.ingame);
        assert_eq!(head.client.last_login_reconnect, Some(true));
        assert_eq!(
            head.client.scene_state, 0,
            "tune wipes the previous channel's scene"
        );
        assert_eq!(
            head.client.local_player.as_ref().unwrap().y, 0,
            "tune wipes the previous channel's local player"
        );
        assert!(head.client.players[123].is_none(), "previous players cleared");
        assert!(head.client.npc[7].is_none(), "previous npcs cleared");
        assert_eq!(head.client.player_count, 0);
        assert_eq!(head.client.npc_count, 0);
        assert!(
            play.channels.contains_key("a"),
            "parked A stays ingame as a lean channel"
        );
        assert!(
            !play.channels.contains_key("b"),
            "the tuned head is not a lean channel"
        );
        play.channels.get_mut("a").unwrap().pump().unwrap();
        server.join().unwrap();
    }
}
