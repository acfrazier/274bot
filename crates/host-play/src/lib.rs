//! `host-play`: run vaulted profiles through the host kernel. The binary
//! unlocks a vault and runs the named profiles; the `e2e` harness links
//! this library so it can poll per-slot state instead of scraping logs.

use std::collections::{HashMap, HashSet, VecDeque};
use std::panic::AssertUnwindSafe;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use api::interact::Driver;
use client::client::Client;
use client::client::ClientConfig;
use client::client::LoginError;
use client::config::if_type::ComponentType;
use client::config::{Cache, IfType};
use client::io::JagFile;
pub use host::debug_enabled;
use host::lean::{Lean, LeanError};
use host::login_queue::{LoginBackoff, LoginQueue, Permit, QueuePos};
use host::prepare_client;
pub use host::set_debug;
pub use host::Host;
mod rss;
mod scatter;
use host::{should_emit_tick, PixelBuf, Pump, SlotInput};
use nav::grid::StepGrid;
use nav::router::find;
use nav::tile::Tile;
use nav::traveller::{NavStatus, Traveller};
pub use rss::sample_process;
pub use scatter::{scatter_tile_for, tele_args};

/// Per-slot hook invoked by the slot thread after every mainloop pass.
type SlotFrame = Arc<dyn Fn(&mut Client, &str) + Send + Sync>;
use script::{ScriptCtx, SlotScript};
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
    /// Queued walk target tile, -1 when idle (mirrored from the slot's
    /// traveller by the pump's per-uid nav step each observe).
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

impl SlotStatus {
    /// Wall member is online: a fat Client has built the scene (`scene_state
    /// == 2`); a lean channel only ever reaches `scene_state` 1 on
    /// `REBUILD_NORMAL`, so login-granted (`ingame`) is enough.
    pub fn is_up(&self) -> bool {
        if self.lean {
            self.ingame
        } else {
            self.ingame && self.scene_state == 2
        }
    }
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

/// Nav pack path: `$NAV_PACK`, else `~/.274bot/274bot.navpack` (same rule
/// as the panel picker; host-play must not depend on panel).
pub fn default_pack_path() -> std::path::PathBuf {
    match std::env::var("NAV_PACK") {
        Ok(p) => std::path::PathBuf::from(p),
        Err(_) => match std::env::var("HOME") {
            Ok(home) => std::path::PathBuf::from(format!("{home}/.274bot/274bot.navpack")),
            Err(_) => std::path::PathBuf::from(".274bot/274bot.navpack"),
        },
    }
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

/// One observe of a slot's script wiring (fat and lean share it): gate
/// [`SlotScript::on_is_up`], dispatch [`SlotScript::on_game_tick`] on the
/// PLAYER_INFO edge, then run any cheats the panel queued. `driver` is the
/// slot body's own `Client`/`Lean`; `here` is the local player's world tile
/// `(x, z, level)` when the body decoded one, else `None` (then the walk
/// hook refuses to arm). `travellers`/`grid` back the `ctx.walk` closure:
/// A* from `here` to the requested tile on the host grid, arming the
/// uid's traveller when a route exists (a Start that would panic on the
/// first tick must not succeed when no route can arm). Returns whether the
/// driver's out buffer was
/// written (the lean pump flushes; the fat `Client` sends on its next
/// mainloop pass). A slot whose script is Idle/Paused publishes nothing —
/// no dispatch, no flush.
// Slot threads pass the same shared handles everywhere; a context struct
// would churn every call site, so the arg count is allowed on purpose.
#[allow(clippy::too_many_arguments)]
fn script_observe(
    driver: &mut dyn Driver,
    name: &str,
    up: bool,
    tick_edge: bool,
    tick: u64,
    here: Option<(i32, i32, i32)>,
    inv: Option<&[(i32, i32)]>,
    obj_names: Option<&api::obj_names::ObjNames>,
    scripts: &Arc<Mutex<HashMap<String, SlotScript>>>,
    cheats: &Arc<Mutex<HashMap<String, VecDeque<String>>>>,
    travellers: &Arc<Mutex<HashMap<String, Traveller>>>,
    grid: &Option<Arc<StepGrid>>,
) -> bool {
    let mut wrote = false;
    {
        let mut all = scripts.lock().unwrap();
        if let Some(slot) = all.get_mut(name) {
            slot.on_is_up(up);
            // skip script snapshot unless SlotScript is Running.
            if tick_edge && slot.state() == script::RunState::Running {
                let mut walk = {
                    let grid = grid.clone();
                    let travellers = Arc::clone(travellers);
                    let name = name.to_string();
                    move |x: i32, z: i32, level: i32| -> bool {
                        let Some((hx, hz, hl)) = here else {
                            return false;
                        };
                        let Some(grid) = grid.as_deref() else {
                            return false;
                        };
                        let from = Tile { x: hx, z: hz, level: hl };
                        let to = Tile { x, z, level };
                        let Ok(route) = find(grid, from, to) else {
                            return false;
                        };
                        travellers
                            .lock()
                            .unwrap()
                            .entry(name.clone())
                            .or_default()
                            .arm(route);
                        true
                    }
                };
                slot.on_game_tick(&mut ScriptCtx {
                    driver,
                    tick,
                    here,
                    walk: Some(&mut walk),
                    inv,
                    obj_names,
                });
                wrote = true;
            }
        }
    }
    let cmds = {
        let mut all = cheats.lock().unwrap();
        all.get_mut(name).map(std::mem::take).unwrap_or_default()
    };
    for cmd in cmds {
        api::interact::cheat(driver, &cmd);
        wrote = true;
    }
    wrote
}

/// Per-slot nav latch key: the `(player gen / lean tick, here)` pair the
/// pump last stepped. The step is skipped until either half changes, so a
/// hop is sent once per server tick, not every 20 ms frame (panel
/// `tick_latch`).
type NavStepKey = (u64, Option<(i32, i32, i32)>);

/// True when `name`'s slot script is Running — the only state that builds
/// the per-observe inventory view (the observe re-checks the gate inside).
fn script_running(scripts: &Arc<Mutex<HashMap<String, SlotScript>>>, name: &str) -> bool {
    scripts
        .lock()
        .unwrap()
        .get(name)
        .is_some_and(|s| s.state() == script::RunState::Running)
}

/// One pump step of a uid's traveller: drive the armed route one hop
/// through `driver`. `here` is the player's absolute tile when the body
/// decoded one (else the traveller stands still) and `door_open` the
/// door's live state (the fat observe reads the loc typecode; the lean
/// pump passes `false` — see gaps.md). Mirrors the traveller's queued
/// dest into the status row's `walk_*` fields (`-1` when idle). Returns
/// the traveller's [`NavStatus`] so callers can tell whether the hop
/// wrote the driver.
fn step_traveller<D: Driver>(
    driver: &mut D,
    name: &str,
    here: Option<(i32, i32, i32)>,
    door_open: bool,
    travellers: &Arc<Mutex<HashMap<String, Traveller>>>,
    statuses: &Arc<Mutex<Vec<SlotStatus>>>,
) -> NavStatus {
    let Some((hx, hz, hl)) = here else {
        return NavStatus::Idle;
    };
    let here = Tile { x: hx, z: hz, level: hl };
    let (status, queued) = {
        let mut all = travellers.lock().unwrap();
        let Some(t) = all.get_mut(name) else {
            return NavStatus::Idle;
        };
        let status = t.tick(driver, here, door_open);
        (status, t.queued())
    };
    let mut rows = statuses.lock().unwrap();
    if let Some(s) = rows.iter_mut().find(|s| s.username == name) {
        match queued {
            Some(q) => {
                s.walk_x = q.x;
                s.walk_z = q.z;
                s.walk_level = q.level;
            }
            None => {
                s.walk_x = -1;
                s.walk_z = -1;
                s.walk_level = -1;
            }
        }
    }
    status
}

/// The fat Client's inventory `(obj_id, count)` slots, zipped from the
/// TYPE_INV iface's linked obj ids/numbers (the server's `UPDATE_INV_FULL`
/// fills them each frame). Short-lived: rebuilt per observe while the slot
/// script is Running; `None` when no TYPE_INV iface is loaded yet.
fn inventory_from_ifaces(ifaces: &[Option<IfType>]) -> Option<Vec<(i32, i32)>> {
    let inv = ifaces
        .iter()
        .flatten()
        .find(|f| f.r#type == ComponentType::TYPE_INV)?;
    let (Some(ids), Some(counts)) = (&inv.link_obj_type, &inv.link_obj_number) else {
        return None;
    };
    Some(ids.iter().zip(counts).map(|(id, n)| (*id, *n)).collect())
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
    /// Next handshake is opcode 18 (lost_con reconnect). First-ever online
    /// is 16; after a grant, or when parking/tuning, this is true.
    pub reconnect: Arc<AtomicBool>,
    /// Park/tune baton: the slot thread sends its live `Lean` here on stop
    /// instead of dropping the TCP.
    pub handoff: Mutex<Option<mpsc::Sender<Lean>>>,
    /// Reverse baton: spawn_slot starts from this live socket and opcode-18
    /// reconnects in place.
    pub adopt: Mutex<Option<Lean>>,
    /// In-place channel change on the fat TV thread (no second `maininit`).
    pub swap: Mutex<Option<HeadSwap>>,
    pub want_swap: Arc<AtomicBool>,
}

/// Steal the current TV socket as a lean, then opcode-18 the incoming
/// lean onto the **same** fat `Client` thread.
pub struct HeadSwap {
    pub lean: Lean,
    pub username: String,
    pub password: String,
    pub uid: i32,
    pub park: mpsc::Sender<Lean>,
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
            reconnect: Arc::new(AtomicBool::new(false)),
            handoff: Mutex::new(None),
            adopt: Mutex::new(None),
            swap: Mutex::new(None),
            want_swap: Arc::new(AtomicBool::new(false)),
        })
    }
}

fn send_handoff_lean(arm: &SlotArm, lean: Option<Lean>, uid: i32, queue: &Arc<Mutex<LoginQueue>>) {
    if let Some(tx) = arm.handoff.lock().unwrap().take() {
        if let Some(lean) = lean {
            let _ = tx.send(lean);
        }
    }
    queue.lock().unwrap().leave(uid);
}

fn send_handoff_client(
    arm: &SlotArm,
    client: &mut Client,
    uid: i32,
    queue: &Arc<Mutex<LoginQueue>>,
) {
    if let Some(tx) = arm.handoff.lock().unwrap().take() {
        if let Some(lean) = Lean::from_client(client) {
            let _ = tx.send(lean);
        }
    }
    queue.lock().unwrap().leave(uid);
}

/// Whether the slot may start a login handshake: on the title (not ingame)
/// and the arm wants a login that is not latched by an intentional logout.
fn should_handshake(arm: &SlotArm, ingame: bool) -> bool {
    !ingame && arm.want_login.load(Ordering::Relaxed) && !arm.latch.load(Ordering::Relaxed)
}

/// After a successful handshake: stay armed only when this slot was spawned
/// with auto-login (an unexpected DC re-handshakes); a one-shot Log in /
/// Login all disarms until the next explicit arm.
fn on_login_success(arm: &SlotArm) {
    arm.want_login.store(
        arm.auto_login.load(Ordering::Relaxed) && !arm.latch.load(Ordering::Relaxed),
        Ordering::Relaxed,
    );
    // A later DC / tune / park is opcode 18, not a cold 16.
    arm.reconnect.store(true, Ordering::Relaxed);
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
    /// The shared obj-id → name table every script ctx resolves `has_item`
    /// against (built once from `cache.objs`; lean channels never load
    /// their own cache).
    obj_names: Arc<api::obj_names::ObjNames>,
    ifaces: Vec<Option<IfType>>,
    queue: Arc<Mutex<LoginQueue>>,
    per_frame: SlotFrame,
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
    /// In-flight threaded retune (UI must not join slot threads).
    pending_tune: Option<PendingTune>,
    /// Per-slot compiled scripts: the slot threads drive `on_is_up` /
    /// `on_game_tick` on each drain, the panel arms them via the
    /// [`Play::script_start`] family. Keyed by username (the identity the
    /// status rows and arms use).
    scripts: Arc<Mutex<HashMap<String, SlotScript>>>,
    /// Per-slot cheat commands the panel queued; each slot thread runs
    /// `api::interact::cheat` on its own Driver and flushes the socket.
    cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>>,
    /// Per-uid nav travellers: `ctx.walk` arms a route into the uid's
    /// traveller and the slot pump steps it one hop per observe. One
    /// struct per bot on the pump — no per-bot nav thread.
    travellers: Arc<Mutex<HashMap<String, Traveller>>>,
    /// Host-scope nav grid baked from the pack at construction (see
    /// [`default_pack_path`]); `None` when no pack loads, and `ctx.walk`
    /// then refuses to arm.
    grid: Option<Arc<StepGrid>>,
}

/// Handoffs in flight for [`Play::retune`]: receivers are polled from
/// [`Play::poll_tune`] so the panel thread never `join`s a login/draw.
struct PendingTune {
    name: String,
    prev: Option<String>,
    incoming_rx: mpsc::Receiver<Lean>,
    park_rx: Option<mpsc::Receiver<Lean>>,
    incoming: Option<Lean>,
    parked: Option<Lean>,
    incoming_done: bool,
    park_done: bool,
    spawned_head: bool,
    swap_placed: bool,
    /// Latest cap-click while this hop is in flight; started when we finish.
    queued: Option<String>,
    input: Option<Arc<SlotInput>>,
    pixels: Option<Arc<PixelBuf>>,
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
    /// A previous retune is still handing off sockets.
    Busy,
}

impl Play {
    /// True while [`Play::retune`] is still handing off sockets.
    pub fn tune_pending(&self) -> bool {
        self.pending_tune.is_some()
    }

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

    /// Keep vault credentials for a later [`Play::tune`] / [`Play::retune`].
    pub fn remember_profile(&mut self, profile: Profile) {
        self.profiles.insert(profile.username.clone(), profile);
    }

    /// Move `uid` to the front of the login FIFO so the TV head handshakes
    /// before lean extras that already queued. Mirrors the place onto the
    /// status row so the queue card can show *k of n* during maininit
    /// (the slot has not entered [`wait_for_permit`] yet).
    pub fn prefer_login(&self, uid: i32) {
        let mut q = self.queue.lock().unwrap();
        q.prefer(uid);
        let pos = q.status(uid);
        drop(q);
        let name = self
            .arms
            .iter()
            .find(|(_, arm)| arm.uid.load(Ordering::Relaxed) == uid)
            .map(|(n, _)| n.clone());
        if let Some(name) = name {
            apply_queue_wait(&mut self.statuses.lock().unwrap(), &name, pos);
        }
    }

    /// Snapshot of the login FIFO (front first). Panel tests pin TV-first.
    pub fn login_queue_uids(&self) -> Vec<i32> {
        self.queue.lock().unwrap().queued_uids()
    }

    /// The unique fat `Client` on this play (the TV). `None` while every
    /// row is lean or the wall is empty.
    pub fn fat_head_name(&self) -> Option<String> {
        self.statuses()
            .into_iter()
            .find(|s| !s.lean)
            .map(|s| s.username)
    }

    /// Whether `name` is a slot this play controls (spawned or armed), so
    /// script control can never create an entry no thread drives.
    fn slot_active(&self, name: &str) -> bool {
        self.spawned.contains(name) || self.arms.contains_key(name)
    }

    /// Start a compiled script on `name`'s slot. `Err("no slot: {name}")`
    /// when no running slot owns that name, `Err("not ported: {id}")` when
    /// the picker id has no ported script yet, or `Err` when the slot
    /// already runs one. The slot thread gates it on `is_up`.
    pub fn script_start(&self, name: &str, id: script::CompiledId) -> Result<(), String> {
        if !self.slot_active(name) {
            return Err(format!("no slot: {name}"));
        }
        let make = script::factory(id).ok_or_else(|| format!("not ported: {}", id.0))?;
        self.scripts
            .lock()
            .unwrap()
            .entry(name.to_string())
            .or_default()
            .start_compiled(make())
    }

    /// Start a loaded JS bot on `name`'s slot: the isolate is spawned here,
    /// on Start (never at Load). Same slot gating as
    /// [`Play::script_start`].
    pub fn script_start_load(
        &self,
        name: &str,
        source: String,
        shape: script::LoadShape,
    ) -> Result<(), String> {
        if !self.slot_active(name) {
            return Err(format!("no slot: {name}"));
        }
        self.scripts
            .lock()
            .unwrap()
            .entry(name.to_string())
            .or_default()
            .start_load(source, shape)
    }

    /// Pause `name`'s script (operator Pause; survives login until
    /// Resume re-arms it). No-op when the slot has no script.
    pub fn script_pause(&self, name: &str) {
        if let Some(slot) = self.scripts.lock().unwrap().get_mut(name) {
            slot.pause();
        }
    }

    /// Resume `name`'s script; the next `on_is_up` re-gates it.
    pub fn script_resume(&self, name: &str) {
        if let Some(slot) = self.scripts.lock().unwrap().get_mut(name) {
            slot.resume();
        }
    }

    /// Stop `name`'s script: teardown hook, instance dropped, Idle.
    pub fn script_stop(&self, name: &str) {
        if let Some(slot) = self.scripts.lock().unwrap().get_mut(name) {
            slot.stop();
        }
    }

    /// `name`'s script lifecycle state; `Idle` when the slot has none.
    pub fn script_state(&self, name: &str) -> script::RunState {
        self.scripts
            .lock()
            .unwrap()
            .get(name)
            .map(|slot| slot.state())
            .unwrap_or(script::RunState::Idle)
    }

    /// `name`'s script `last_error`; `None` when the slot has none.
    pub fn script_last_error(&self, name: &str) -> Option<String> {
        self.scripts
            .lock()
            .unwrap()
            .get(name)
            .and_then(|slot| slot.last_error().map(str::to_string))
    }

    /// Queue `cmd` (the `::` part only) for `user`'s slot: its own thread
    /// writes `CLIENT_CHEAT` through the slot's Driver and flushes. No-op
    /// when the user is not a running slot.
    pub fn cheat(&self, user: &str, cmd: &str) {
        if let Some(q) = self.cheats.lock().unwrap().get_mut(user) {
            q.push_back(cmd.to_string());
        }
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
        self.statuses.lock().unwrap().retain(|s| s.username != name);
        self.arms.remove(name);
        self.scripts.lock().unwrap().remove(name);
        self.cheats.lock().unwrap().remove(name);
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
        self.profiles
            .insert(profile.username.clone(), profile.clone());
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
            Arc::clone(&self.scripts),
            Arc::clone(&self.cheats),
            Arc::clone(&self.travellers),
            self.grid.clone(),
            Arc::clone(&self.obj_names),
            Arc::clone(&self.per_frame),
            &mut self.handles,
        );
    }

    /// Spawn one lean channel slot on this play's FIFO: `Lean::login` (cold,
    /// opcode 16, no `Client`), then pump the stream at the host cadence.
    /// The status row is marked `lean`; no-op if the name is already running.
    /// `None` arm logs in immediately (CLI/e2e); the panel passes a real
    /// arm so extras sit on the title until Login all.
    pub fn spawn_channel(&mut self, profile: Profile) {
        self.spawn_channel_with(profile, None, false);
    }

    /// Like [`spawn_channel`], with a panel control arm (auto-login / latch).
    pub fn spawn_channel_with_arm(&mut self, profile: Profile, arm: Option<Arc<SlotArm>>) {
        self.spawn_channel_with(profile, arm, false);
    }

    /// Like [`spawn_channel`], but the handshake is opcode 18 (park after
    /// the fat head's socket dropped).
    pub fn spawn_channel_reconnect(&mut self, profile: Profile) {
        self.spawn_channel_with(profile, None, true);
    }

    fn spawn_channel_with(&mut self, profile: Profile, arm: Option<Arc<SlotArm>>, reconnect: bool) {
        self.profiles
            .insert(profile.username.clone(), profile.clone());
        if !self.spawned.insert(profile.username.clone()) {
            return;
        }
        let arm = arm.unwrap_or_else(|| SlotArm::new(profile.uid, true));
        arm.uid.store(profile.uid, Ordering::Relaxed);
        arm.reconnect.store(reconnect, Ordering::Relaxed);
        self.arms.insert(profile.username.clone(), Arc::clone(&arm));
        spawn_channel_thread(
            &self.options,
            profile,
            arm,
            Arc::clone(&self.queue),
            Arc::clone(&self.statuses),
            Arc::clone(&self.scripts),
            Arc::clone(&self.cheats),
            Arc::clone(&self.travellers),
            self.grid.clone(),
            Arc::clone(&self.obj_names),
            self.ifaces.clone(),
            &mut self.handles,
        );
    }

    /// Threaded channel-head tune: signal baton-pass (no UI `join`). The
    /// panel must call [`Play::poll_tune`] each frame to finish spawn.
    pub fn retune(
        &mut self,
        name: &str,
        input: Option<Arc<SlotInput>>,
        pixels: Option<Arc<PixelBuf>>,
    ) -> Result<(), TuneError> {
        if self.fat_head_name().as_deref() == Some(name) {
            return Ok(());
        }
        if !self.profiles.contains_key(name) {
            return Err(TuneError::UnknownProfile(name.to_string()));
        }
        if let Some(pending) = self.pending_tune.as_mut() {
            // Cap-click during a hop: keep the in-flight steal, remember
            // the latest target. Do not Busy the UI.
            if pending.name != name {
                pending.queued = Some(name.to_string());
            }
            return Ok(());
        }
        let incoming_rx = self.begin_handoff(name);
        let prev = self.fat_head_name().filter(|p| p != name);
        // Keep the TV thread: park is an in-place swap, not begin_handoff.
        self.pending_tune = Some(PendingTune {
            name: name.to_string(),
            prev,
            incoming_rx,
            park_rx: None,
            incoming: None,
            parked: None,
            incoming_done: false,
            park_done: false,
            spawned_head: false,
            swap_placed: false,
            queued: None,
            input,
            pixels,
        });
        self.poll_tune();
        Ok(())
    }

    /// Drive in-flight retune: take handed-off sockets and spawn the new
    /// fat head / parked lean. Never joins a slot thread.
    pub fn poll_tune(&mut self) {
        let (
            spawn_head,
            place_swap,
            name,
            incoming,
            input,
            pixels,
            prev,
            parked,
            rekey,
            done,
            queued,
            failed,
        ) = {
            let Some(pending) = self.pending_tune.as_mut() else {
                return;
            };
            if !pending.incoming_done {
                match pending.incoming_rx.try_recv() {
                    Ok(lean) => {
                        pending.incoming = Some(lean);
                        pending.incoming_done = true;
                    }
                    Err(mpsc::TryRecvError::Disconnected) => pending.incoming_done = true,
                    Err(mpsc::TryRecvError::Empty) => {}
                }
            }
            if !pending.park_done {
                match pending.park_rx.as_ref() {
                    None if pending.prev.is_none() || pending.swap_placed => {
                        if pending.prev.is_none() {
                            pending.park_done = true;
                        }
                    }
                    None => {}
                    Some(rx) => match rx.try_recv() {
                        Ok(lean) => {
                            pending.parked = Some(lean);
                            pending.park_done = true;
                        }
                        Err(mpsc::TryRecvError::Disconnected) => pending.park_done = true,
                        Err(mpsc::TryRecvError::Empty) => {}
                    },
                }
            }
            let in_place = pending.prev.is_some();
            let steal_failed = in_place
                && pending.incoming_done
                && pending.incoming.is_none()
                && !pending.swap_placed;
            let place_swap = in_place
                && pending.incoming_done
                && pending.incoming.is_some()
                && !pending.swap_placed;
            let spawn_head = !in_place && pending.incoming_done && !pending.spawned_head;
            if spawn_head {
                pending.spawned_head = true;
            }
            let name = pending.name.clone();
            let incoming = if place_swap || spawn_head {
                pending.incoming.take()
            } else {
                None
            };
            let input = if spawn_head {
                pending.input.take()
            } else {
                None
            };
            let pixels = if spawn_head {
                pending.pixels.take()
            } else {
                None
            };
            let spawn_park = pending.park_done && pending.parked.is_some();
            let prev = pending.prev.clone();
            let parked = if spawn_park {
                pending.parked.take()
            } else {
                None
            };
            let rekey = spawn_park;
            let done = pending.incoming_done
                && pending.park_done
                && (pending.spawned_head || pending.swap_placed)
                && (pending.prev.is_none() || spawn_park);
            let queued = if done || steal_failed {
                pending.queued.take()
            } else {
                None
            };
            let failed = if steal_failed {
                Some(pending.name.clone())
            } else {
                None
            };
            (
                spawn_head, place_swap, name, incoming, input, pixels, prev, parked, rekey, done,
                queued, failed,
            )
        };
        if place_swap {
            if let (Some(prev), Some(lean), Some(profile)) =
                (prev.clone(), incoming, self.profiles.get(&name).cloned())
            {
                if let Some(arm) = self.arms.get(&prev).cloned() {
                    let (park_tx, park_rx) = mpsc::channel();
                    if let Some(pending) = self.pending_tune.as_mut() {
                        pending.park_rx = Some(park_rx);
                    }
                    *arm.swap.lock().unwrap() = Some(HeadSwap {
                        lean,
                        username: profile.username.clone(),
                        password: profile.password.clone(),
                        uid: profile.uid,
                        park: park_tx,
                    });
                    arm.want_swap.store(true, Ordering::Relaxed);
                    if let Some(pending) = self.pending_tune.as_mut() {
                        pending.swap_placed = true;
                    }
                    if debug_enabled() {
                        eprintln!("[host-play] retune in-place {prev} -> {name}");
                    }
                } else {
                    self.spawn_channel_from_lean(profile, lean);
                    self.pending_tune = None;
                }
            }
        } else if spawn_head {
            if let Some(profile) = self.profiles.get(&name).cloned() {
                let arm = SlotArm::new(profile.uid, true);
                if incoming.is_some() {
                    arm.reconnect.store(true, Ordering::Relaxed);
                }
                if let Some(lean) = incoming {
                    *arm.adopt.lock().unwrap() = Some(lean);
                }
                self.spawn_slot(profile, input, pixels, Some(arm));
            }
        }
        if rekey {
            if let Some(prev) = prev.clone() {
                self.rekey_fat(&prev, &name);
            }
        }
        if let (Some(prev), Some(parked)) = (prev, parked) {
            if let Some(p) = self.profiles.get(&prev).cloned() {
                self.spawn_channel_from_lean(p, parked);
            }
        }
        if done || failed.is_some() {
            self.pending_tune = None;
        }
        if let Some(name) = failed {
            if let Some(p) = self.profiles.get(&name).cloned() {
                self.spawn_channel_reconnect(p);
            }
        }
        if let Some(next) = queued {
            if self.fat_head_name().as_deref() != Some(next.as_str()) {
                let _ = self.retune(&next, None, None);
            }
        }
    }

    fn rekey_fat(&mut self, prev: &str, next: &str) {
        if prev == next {
            return;
        }
        if let Some(arm) = self.arms.remove(prev) {
            self.arms.insert(next.to_string(), arm);
        }
        if let Some(handle) = self.handles.remove(prev) {
            self.handles.insert(next.to_string(), handle);
        }
        self.spawned.remove(prev);
        self.spawned.insert(next.to_string());
        // The renamed thread looks its script/cheat slots up by its new
        // username; keep the per-uid state with it across the tune.
        if let Some(slot) = self.scripts.lock().unwrap().remove(prev) {
            self.scripts.lock().unwrap().insert(next.to_string(), slot);
        }
        if let Some(q) = self.cheats.lock().unwrap().remove(prev) {
            self.cheats.lock().unwrap().insert(next.to_string(), q);
        }
        let mut all = self.statuses.lock().unwrap();
        if let Some(s) = all.iter_mut().find(|s| s.username == prev && !s.lean) {
            s.username = next.to_string();
            s.lean = false;
        }
    }

    /// Signal a slot to hand off its live socket. Does not join.
    fn begin_handoff(&mut self, name: &str) -> mpsc::Receiver<Lean> {
        if let Some(lean) = self.channels.remove(name) {
            let (tx, rx) = mpsc::channel();
            let _ = tx.send(lean);
            return rx;
        }
        let (tx, rx) = mpsc::channel();
        if let Some(arm) = self.arms.get(name).cloned() {
            *arm.handoff.lock().unwrap() = Some(tx);
            arm.stop.store(true, Ordering::Relaxed);
            self.spawned.remove(name);
            self.statuses.lock().unwrap().retain(|s| s.username != name);
            self.arms.remove(name);
            if let Some(handle) = self.handles.remove(name) {
                thread::spawn(move || {
                    let _ = handle.join();
                });
            }
        }
        rx
    }

    fn spawn_channel_from_lean(&mut self, profile: Profile, lean: Lean) {
        self.profiles
            .insert(profile.username.clone(), profile.clone());
        if !self.spawned.insert(profile.username.clone()) {
            return;
        }
        let arm = SlotArm::new(profile.uid, true);
        arm.reconnect.store(true, Ordering::Relaxed);
        *arm.adopt.lock().unwrap() = Some(lean);
        self.arms.insert(profile.username.clone(), Arc::clone(&arm));
        spawn_channel_thread(
            &self.options,
            profile,
            arm,
            Arc::clone(&self.queue),
            Arc::clone(&self.statuses),
            Arc::clone(&self.scripts),
            Arc::clone(&self.cheats),
            Arc::clone(&self.travellers),
            self.grid.clone(),
            Arc::clone(&self.obj_names),
            self.ifaces.clone(),
            &mut self.handles,
        );
    }

    /// Tune the head to `name` (274bot channel head). Sequence:
    /// 1. Take `name`'s lean socket if one is up (baton, no DC);
    /// 2. Park the current head: steal its live socket into a Lean
    ///    (throw away World/ifaces/pixmaps) — no TCP close;
    /// 3. Put `name`'s socket on the fat Client and opcode-**18** reconnect
    ///    **in place** so the server dumps region/player state. No lean
    ///    socket → fresh TCP 18 as before;
    /// 4. wipe the previous channel's scene (`scene_state = 0`, fresh
    ///    `localPlayer`, cleared player/npc tables).
    ///
    /// The first tune (no head yet) skips the park. Tuning the current
    /// head is a no-op.
    pub fn tune(&mut self, name: &str) -> Result<(), TuneError> {
        let profile = self
            .profiles
            .get(name)
            .cloned()
            .ok_or_else(|| TuneError::UnknownProfile(name.to_string()))?;
        if self.head.as_ref().is_some_and(|h| h.name == name) {
            return Ok(());
        }

        // 1. Incoming lean: keep the TCP, hand the socket to the Client.
        let incoming = self.channels.remove(name);

        // 2. Park the current head: baton-pass the live socket into Lean.
        let mut client = if let Some(mut head) = self.head.take() {
            let prev_name = head.name.clone();
            match Lean::from_client(&mut head.client) {
                Some(lean) => {
                    self.channels.insert(prev_name, lean);
                }
                None => {
                    // No stream to steal (never logged in): leave parked
                    // without a channel rather than a fake DC login.
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

        // 3. Opcode 18 on the adopted socket (or a fresh TCP if `name`
        //    was not a lean channel). Same RSA block as a lost_con.
        client.login_uid = profile.uid;
        if let Some(lean) = incoming {
            client.stream = Some(lean.into_stream());
            client.baton = true;
        }
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
    let cache = Arc::new(cache);
    let obj_names = Arc::new(api::obj_names::ObjNames::from_objs(&cache.objs));
    let mut play = Play {
        statuses: Arc::new(Mutex::new(Vec::new())),
        handles: HashMap::new(),
        options: options.clone(),
        cache,
        obj_names,
        ifaces,
        queue: Arc::new(Mutex::new(LoginQueue::default())),
        per_frame: Arc::new(per_frame),
        spawned: HashSet::new(),
        arms: HashMap::new(),
        profiles: HashMap::new(),
        head: None,
        channels: HashMap::new(),
        pending_tune: None,
        scripts: Arc::new(Mutex::new(HashMap::new())),
        cheats: Arc::new(Mutex::new(HashMap::new())),
        travellers: Arc::new(Mutex::new(HashMap::new())),
        grid: nav::pack::load_pack(&default_pack_path()).ok().map(Arc::new),
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
    let cache = Arc::new(cache);
    let obj_names = Arc::new(api::obj_names::ObjNames::from_objs(&cache.objs));
    let mut play = Play {
        statuses: Arc::new(Mutex::new(Vec::new())),
        handles: HashMap::new(),
        options: options.clone(),
        cache,
        obj_names,
        ifaces,
        queue: Arc::new(Mutex::new(LoginQueue::default())),
        per_frame: Arc::new(|c: &mut Client, _: &str| c.set_draw(false)),
        spawned: HashSet::new(),
        arms: HashMap::new(),
        profiles: HashMap::new(),
        head: None,
        channels: HashMap::new(),
        pending_tune: None,
        scripts: Arc::new(Mutex::new(HashMap::new())),
        cheats: Arc::new(Mutex::new(HashMap::new())),
        travellers: Arc::new(Mutex::new(HashMap::new())),
        grid: nav::pack::load_pack(&default_pack_path()).ok().map(Arc::new),
    };
    let tv_uid = profiles.first().map(|p| p.uid);
    for (i, profile) in profiles.into_iter().enumerate() {
        if i < heads {
            play.spawn_slot(profile, None, None, None);
        } else {
            play.spawn_channel(profile);
        }
    }
    if heads >= 1 {
        if let Some(uid) = tv_uid {
            play.prefer_login(uid);
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

fn mark_login_started(statuses: &Arc<Mutex<Vec<SlotStatus>>>, name: &str) {
    let mut all = statuses.lock().unwrap();
    if let Some(s) = all.iter_mut().find(|s| s.username == name) {
        if s.login_started.is_none() {
            s.login_started = Some(Instant::now());
        }
        s.error = None;
    }
}

fn record_login_error(statuses: &Arc<Mutex<Vec<SlotStatus>>>, name: &str, e: &LoginError) {
    let msg = format!("code {}: {}", e.code, e.mes2);
    if debug_enabled() {
        eprintln!("[host-play] slot {name}: login {msg}");
    }
    let mut all = statuses.lock().unwrap();
    if let Some(s) = all.iter_mut().find(|s| s.username == name) {
        s.error = Some(msg);
    }
}

fn login_retry_wait(backoff: &mut LoginBackoff, code: i32) -> Duration {
    match code {
        16 => backoff.delay(),
        5 => Duration::from_secs(60),
        _ => Duration::from_secs(5),
    }
}

/// Every profile spawns one slot thread; shared handles are threaded through
/// because the closure moves most of them (allowed: see `script_observe`).
#[allow(clippy::too_many_arguments)]
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
    slot_scripts: Arc<Mutex<HashMap<String, SlotScript>>>,
    slot_cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>>,
    slot_travellers: Arc<Mutex<HashMap<String, Traveller>>>,
    slot_grid: Option<Arc<StepGrid>>,
    slot_obj_names: Arc<api::obj_names::ObjNames>,
    slot_frame: SlotFrame,
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
            {
                // Publish the row before `prepare_client`/`maininit` (highmem
                // TV can stall for seconds). Login all prefers this uid;
                // lean extras must not claim the FIFO while the tube loads.
                let mut all = slot_statuses.lock().unwrap();
                all.push(SlotStatus {
                    username: username.clone(),
                    ..SlotStatus::default()
                });
            }
            slot_scripts
                .lock()
                .unwrap()
                .entry(username.clone())
                .or_default();
            slot_cheats
                .lock()
                .unwrap()
                .entry(username.clone())
                .or_default();
            let mut client = prepare_client(config, uid, slot_cache, ifaces_template.clone());
            #[cfg(test)]
            {
                // Unit tests spawn slots with no web server on :80; shrink
                // maininit's HTTP retry so `stop_slot`'s join returns fast
                // (the client's own HTTP tests stub retries the same way).
                client.fetch_retry_wait = Duration::from_millis(1);
            }
            if debug_enabled() {
                eprintln!("[host-play] slot {username}: thread up");
            }

            // Jag/anim/model/map prefetch (mirrors client-play; the scene
            // cannot reach scene_state 2 until the loc models are in).
            client.maininit();
            if client.error_loading && debug_enabled() {
                eprintln!("[host-play] slot {username}: maininit failed");
            }

            let uname = Arc::new(Mutex::new(username.clone()));
            let mut password = password;
            let mut uid = uid;
            let mut backoff = LoginBackoff::new();
            loop {
                if arm.stop.load(Ordering::Relaxed) {
                    send_handoff_client(&arm, &mut client, uid, &slot_queue);
                    return;
                }
                if let Some(swap) = arm.swap.lock().unwrap().take() {
                    arm.want_swap.store(false, Ordering::Relaxed);
                    // Park A (TCP stays up as lean). Drop B's lean so the
                    // server sees a DC, then opcode-18 on a *new* TCP —
                    // 18 on the stolen live socket is RSA code 6.
                    let parked = Lean::from_client(&mut client);
                    let new_uid = swap.uid;
                    let new_pass = swap.password.clone();
                    let new_name = swap.username.clone();
                    drop(swap.lean);
                    client.login_uid = new_uid;
                    eprintln!("[host-play] slot swap -> {new_name}: DC + opcode 18");
                    match client.login(&new_name, &new_pass, true) {
                        Ok(()) => {
                            if let Some(parked) = parked {
                                let _ = swap.park.send(parked);
                            }
                            uid = new_uid;
                            password = new_pass;
                            *uname.lock().unwrap() = new_name;
                            arm.uid.store(uid, Ordering::Relaxed);
                            client.wipe_scene();
                            backoff.reset();
                            on_login_success(&arm);
                        }
                        Err(e) => {
                            if let Some(parked) = parked {
                                client.stream = Some(parked.into_stream());
                                client.baton = true;
                            }
                            let name = uname.lock().unwrap().clone();
                            record_login_error(&slot_statuses, &name, &e);
                            thread::sleep(login_retry_wait(&mut backoff, e.code));
                            continue;
                        }
                    }
                } else if !client.ingame {
                    if !should_handshake(&arm, client.ingame) {
                        thread::sleep(Duration::from_millis(20));
                        continue;
                    }
                    let name = uname.lock().unwrap().clone();
                    wait_for_permit(&slot_queue, &slot_statuses, &name, uid, &arm.stop);
                    if arm.stop.load(Ordering::Relaxed) {
                        send_handoff_client(&arm, &mut client, uid, &slot_queue);
                        return;
                    }
                    mark_login_started(&slot_statuses, &name);
                    if let Some(lean) = arm.adopt.lock().unwrap().take() {
                        client.stream = Some(lean.into_stream());
                        client.baton = true;
                        arm.reconnect.store(true, Ordering::Relaxed);
                    }
                    let reconnect = arm.reconnect.load(Ordering::Relaxed);
                    if debug_enabled() {
                        eprintln!(
                            "[host-play] slot {name}: handshake begin reconnect={reconnect}"
                        );
                    }
                    match client.login(&name, &password, reconnect) {
                        Ok(()) => {
                            if arm.reconnect.load(Ordering::Relaxed) {
                                client.wipe_scene();
                            }
                            backoff.reset();
                            on_login_success(&arm);
                            if debug_enabled() {
                                eprintln!("[host-play] slot {name}: handshake ok");
                            }
                        }
                        Err(e) => {
                            record_login_error(&slot_statuses, &name, &e);
                            thread::sleep(login_retry_wait(&mut backoff, e.code));
                            continue;
                        }
                    }
                }
                let mut mainland_sent = false;
                let uname_obs = Arc::clone(&uname);
                let arm_obs = Arc::clone(&arm);
                let run_name = uname.lock().unwrap().clone();
                Host::run_client(
                    &mut client,
                    &run_name,
                    slot_input.clone(),
                    slot_pixels.clone(),
                    {
                        let slot_frame = Arc::clone(&slot_frame);
                        let slot_statuses = Arc::clone(&slot_statuses);
                        let slot_scripts = Arc::clone(&slot_scripts);
                        let slot_cheats = Arc::clone(&slot_cheats);
                        let slot_obj_names = Arc::clone(&slot_obj_names);
                        let slot_travellers = Arc::clone(&slot_travellers);
                        let slot_grid = slot_grid.clone();
                        let mut pump = Pump::new();
                        let mut script_tick: u64 = 0;
                        // Last `(player gen, here)` the traveller stepped:
                        // skip until either changes so the hop budget counts
                        // server ticks, not 20 ms frames (panel `tick_latch`).
                        let mut last_nav_step: Option<NavStepKey> = None;
                        move |c, _ignored, run_sends| {
                            let name = uname_obs.lock().unwrap().clone();
                            slot_frame(c, &name);
                            if !mainland_sent && mainland && c.ingame && c.scene_state == 2 {
                                api::interact::mainland_hop(c);
                                mainland_sent = true;
                                if debug_enabled() {
                                    eprintln!("[host-play] slot {name}: queued mainland tele+setvar (scene 2)");
                                }
                            }
                            // The host's own pump diffs gens inside
                            // `client_frame` (after this observe); diff the
                            // previous frame's gens here so scripts see one
                            // edge per PLAYER_INFO (same `should_emit_tick`).
                            let drain = pump.drain(c.gens);
                            let tick_edge = should_emit_tick(drain.player_info);
                            if tick_edge {
                                script_tick = script_tick.wrapping_add(1);
                            }
                            let (up, here) = {
                                let mut all = slot_statuses.lock().unwrap();
                                let mut up = false;
                                let mut here = None;
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
                                            // Tile level is not decoded on
                                            // either body yet (see gaps.md).
                                            here = Some((tx, tz, 0));
                                            s.player = lp.name.clone().unwrap_or_default();
                                        }
                                        up = s.is_up();
                                    }
                                }
                                (up, here)
                            };
                            // Inventory view: zip the TYPE_INV iface's obj
                            // ids/counts, rebuilt each observe while the
                            // script is Running (the idle-skip gate).
                            let inv = if script_running(&slot_scripts, &name) {
                                inventory_from_ifaces(&c.ifaces)
                            } else {
                                None
                            };
                            script_observe(
                                c,
                                &name,
                                up,
                                tick_edge,
                                script_tick,
                                here,
                                inv.as_deref(),
                                Some(slot_obj_names.as_ref()),
                                &slot_scripts,
                                &slot_cheats,
                                &slot_travellers,
                                &slot_grid,
                            );
                            // Per-uid nav step on the pump, gated on the
                            // player-gen/tile latch like the panel's WalkTo
                            // hook so a hop is sent once per server tick,
                            // not re-sent every 20 ms frame. Door state is
                            // read live from the fat client's loc typecode.
                            let nav_key = (c.gens.player, here);
                            if last_nav_step != Some(nav_key) {
                                last_nav_step = Some(nav_key);
                                let door_open = {
                                    let all = slot_travellers.lock().unwrap();
                                    match all.get(&name).and_then(|t| {
                                        here.and_then(|(hx, hz, hl)| {
                                            t.current_door(Tile { x: hx, z: hz, level: hl })
                                        })
                                    }) {
                                        Some((loc, closed_id)) => {
                                            let (bx, bz) = c.build_base();
                                            c.loc_typecode(loc.x - bx, loc.z - bz)
                                                .map(|tc| (tc >> 14) & 0x7fff)
                                                != Some(closed_id)
                                        }
                                        None => false,
                                    }
                                };
                                step_traveller(
                                    c,
                                    &name,
                                    here,
                                    door_open,
                                    &slot_travellers,
                                    &slot_statuses,
                                );
                            }
                        }
                    },
                    {
                        let ifaces_template = ifaces_template.clone();
                        move |c| {
                            tick_flags(c, &ifaces_template, &arm_obs)
                                || arm_obs.want_swap.load(Ordering::Relaxed)
                                || !c.ingame
                        }
                    },
                );
                if arm.stop.load(Ordering::Relaxed) {
                    send_handoff_client(&arm, &mut client, uid, &slot_queue);
                    return;
                }
                let name = uname.lock().unwrap().clone();
                let mut all = slot_statuses.lock().unwrap();
                if let Some(s) = all.iter_mut().find(|s| s.username == name) {
                    s.ingame = client.ingame;
                    s.scene_state = client.scene_state;
                }
            }
            })
            .expect("failed to spawn slot thread"),
    );
}

enum LeanPump {
    /// Arm `stop` / handoff — the thread should exit.
    Stopped,
    /// Socket died; caller may reconnect.
    Died,
}

/// Pump a live lean: mark `ingame`, host `NO_TIMEOUT` once a second, update
/// the snapshot. Adopt (parked TV) uses `seed = false`.
#[allow(clippy::too_many_arguments)]
fn run_lean_pump(
    mut lean: Lean,
    arm: &SlotArm,
    username: &str,
    uid: i32,
    slot_queue: &Arc<Mutex<LoginQueue>>,
    slot_statuses: &Arc<Mutex<Vec<SlotStatus>>>,
    slot_scripts: &Arc<Mutex<HashMap<String, SlotScript>>>,
    slot_cheats: &Arc<Mutex<HashMap<String, VecDeque<String>>>>,
    slot_travellers: &Arc<Mutex<HashMap<String, Traveller>>>,
    slot_grid: &Option<Arc<StepGrid>>,
    slot_obj_names: &Arc<api::obj_names::ObjNames>,
    ifaces: &[Option<IfType>],
    seed: bool,
) -> LeanPump {
    let mut last_tick: u64;
    {
        let snap = lean.snapshot();
        last_tick = snap.tick;
        let mut all = slot_statuses.lock().unwrap();
        if let Some(s) = all.iter_mut().find(|s| s.username == username) {
            s.ingame = true;
            s.error = None;
            s.scene_state = snap.scene_state;
            s.tile_x = snap.tile_x;
            s.tile_z = snap.tile_z;
        }
    }
    let mut seeded = !seed;
    let mut last_ka = Instant::now();
    let mut logging_out = false;
    // Last `(tick, here)` the traveller stepped: skip until either changes
    // so the hop budget counts PLAYER_INFOs, not 20 ms frames.
    let mut last_nav_step: Option<NavStepKey> = None;
    loop {
        if arm.stop.load(Ordering::Relaxed) {
            send_handoff_lean(arm, Some(lean), uid, slot_queue);
            return LeanPump::Stopped;
        }
        if arm.want_logout.load(Ordering::Relaxed) {
            let _ = api::interact::logout(&mut lean, ifaces);
            let _ = lean.flush();
            arm.want_logout.store(false, Ordering::Relaxed);
            arm.latch.store(true, Ordering::Relaxed);
            arm.want_login.store(false, Ordering::Relaxed);
            logging_out = true;
        }
        if !logging_out && last_ka.elapsed() >= Duration::from_secs(1) {
            lean.write_no_timeout();
            last_ka = Instant::now();
        }
        let pump = lean.pump();
        let mut died = false;
        let mut up = false;
        let mut tick_edge = false;
        let mut tick = 0u64;
        // The lean snapshot's `here` is the local player tile from the
        // PLAYER_INFO teleport branch; it only moves on teleport/rebuild
        // (walk-run tracking is a later gap — see gaps.md).
        let mut here = None;
        {
            let mut all = slot_statuses.lock().unwrap();
            if let Some(s) = all.iter_mut().find(|s| s.username == username) {
                match pump {
                    Ok(()) => {
                        let snap = lean.snapshot();
                        let scene_state = snap.scene_state;
                        // The snapshot's PLAYER_INFO count is the lean
                        // channel's tick edge (mirrors fat `should_emit_tick`).
                        tick_edge = snap.tick != last_tick;
                        last_tick = snap.tick;
                        tick = snap.tick;
                        here = snap.here;
                        s.scene_state = scene_state;
                        s.tile_x = snap.tile_x;
                        s.tile_z = snap.tile_z;
                        s.ingame = true;
                        up = s.is_up();
                        if !seeded && scene_state != 0 {
                            let t = scatter_tile_for(uid);
                            api::interact::seed_at(&mut lean, t.level, t.x, t.z);
                            if let Err(e) = lean.flush() {
                                s.error = Some(format!("seed: {e:?}"));
                                s.ingame = false;
                                died = true;
                            } else {
                                seeded = true;
                                if debug_enabled() {
                                    eprintln!(
                                        "[host-play] channel {username}: seed {} {} {}",
                                        t.level, t.x, t.z
                                    );
                                }
                            }
                        }
                    }
                    Err(e) => {
                        s.error = Some(match &e {
                            LeanError::Login(le) => format!("code {}: {}", le.code, le.mes2),
                            LeanError::Io(io) => format!("io: {io}"),
                            LeanError::FrameTooLarge { ptype, psize } => {
                                format!("frame too large ptype={ptype} psize={psize}")
                            }
                        });
                        s.ingame = false;
                        died = true;
                    }
                }
            }
        }
        // Script wiring runs every observe, dead or not: a downed channel
        // re-gates `on_is_up(false)` so the script pauses while it is out.
        // Inventory view: clone the snapshot's inv slots, only while the
        // script is Running (same idle-skip gate as the fat path). The
        // clone is owned so the `&mut lean` driver borrow below is free.
        let inv: Option<Vec<(i32, i32)>> = if script_running(slot_scripts, username) {
            Some(lean.snapshot().inv.clone())
        } else {
            None
        };
        let mut wrote = script_observe(
            &mut lean,
            username,
            up,
            tick_edge,
            tick,
            here,
            inv.as_deref(),
            Some(slot_obj_names.as_ref()),
            slot_scripts,
            slot_cheats,
            slot_travellers,
            slot_grid,
        );
        // Per-uid nav step on the pump, gated on the tick/tile latch like
        // the fat path. Lean has no loc typecode decode, so every door leg
        // is worked as closed (`door_open = false`; see gaps.md).
        let nav_key = (tick, here);
        if last_nav_step != Some(nav_key) {
            last_nav_step = Some(nav_key);
            if matches!(
                step_traveller(&mut lean, username, here, false, slot_travellers, slot_statuses),
                NavStatus::Walking | NavStatus::Door
            ) {
                wrote = true;
            }
        }
        if wrote && !died {
            if let Err(e) = lean.flush() {
                let mut all = slot_statuses.lock().unwrap();
                if let Some(s) = all.iter_mut().find(|s| s.username == username) {
                    s.error = Some(format!("script flush: {e:?}"));
                    s.ingame = false;
                }
                died = true;
            }
        }
        if died {
            return LeanPump::Died;
        }
        thread::sleep(Duration::from_millis(20));
    }
}

/// Spawn one lean channel thread. The channel waits for a login-queue
/// permit (shared FIFO with the head), cold-logins with `Lean::login`
/// (wrapper opcode 16), then pumps inbound frames at the host cadence —
/// no `maininit`, no `Client`, no pixels. Host writes `NO_TIMEOUT` so
/// parked extras stay logged in. The row mirrors the thin `LeanSnapshot`.
#[allow(clippy::too_many_arguments)]
fn spawn_channel_thread(
    options: &PlayOptions,
    profile: Profile,
    arm: Arc<SlotArm>,
    slot_queue: Arc<Mutex<LoginQueue>>,
    slot_statuses: Arc<Mutex<Vec<SlotStatus>>>,
    slot_scripts: Arc<Mutex<HashMap<String, SlotScript>>>,
    slot_cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>>,
    slot_travellers: Arc<Mutex<HashMap<String, Traveller>>>,
    slot_grid: Option<Arc<StepGrid>>,
    slot_obj_names: Arc<api::obj_names::ObjNames>,
    ifaces: Vec<Option<IfType>>,
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
                slot_scripts
                    .lock()
                    .unwrap()
                    .entry(username.clone())
                    .or_default();
                slot_cheats
                    .lock()
                    .unwrap()
                    .entry(username.clone())
                    .or_default();
                if let Some(lean) = arm.adopt.lock().unwrap().take() {
                    match run_lean_pump(
                        lean,
                        &arm,
                        &username,
                        uid,
                        &slot_queue,
                        &slot_statuses,
                        &slot_scripts,
                        &slot_cheats,
                        &slot_travellers,
                        &slot_grid,
                        &slot_obj_names,
                        &ifaces,
                        false,
                    ) {
                        LeanPump::Stopped => return,
                        LeanPump::Died => {}
                    }
                }
                loop {
                    if arm.stop.load(Ordering::Relaxed) {
                        send_handoff_lean(&arm, None, uid, &slot_queue);
                        return;
                    }
                    if !should_handshake(&arm, false) {
                        // Sit idle until Login all / auto-login arms a handshake.
                        thread::sleep(Duration::from_millis(20));
                        continue;
                    }
                    wait_for_permit(&slot_queue, &slot_statuses, &username, uid, &arm.stop);
                    if arm.stop.load(Ordering::Relaxed) {
                        send_handoff_lean(&arm, None, uid, &slot_queue);
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
                    match Lean::login(
                        &config,
                        &username,
                        &password,
                        uid,
                        arm.reconnect.load(Ordering::Relaxed),
                    ) {
                        Ok(lean) => {
                            arm.reconnect.store(true, Ordering::Relaxed);
                            if debug_enabled() {
                                eprintln!("[host-play] channel {username}: ingame");
                            }
                            match run_lean_pump(
                                lean,
                                &arm,
                                &username,
                                uid,
                                &slot_queue,
                                &slot_statuses,
                                &slot_scripts,
                                &slot_cheats,
                                &slot_travellers,
                                &slot_grid,
                                &slot_obj_names,
                                &ifaces,
                                true,
                            ) {
                                LeanPump::Stopped => return,
                                LeanPump::Died => {}
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
        let deadline = Instant::now() + wait;
        while Instant::now() < deadline {
            if stop.load(Ordering::Relaxed) {
                queue.lock().unwrap().leave(uid);
                let mut all = statuses.lock().unwrap();
                apply_queue_wait(&mut all, username, None);
                return;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            thread::sleep(left.min(Duration::from_millis(20)));
        }
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
            settings: ProfileSettings {
                lowmem: true,
                auto_login: false,
            },
        };
        let loud = Profile {
            username: "b".into(),
            password: "b".into(),
            uid: 2,
            settings: ProfileSettings {
                lowmem: false,
                auto_login: false,
            },
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
            assert!(matches!(
                q.request_permit(7, Instant::now()),
                Permit::Wait(_)
            ));
        }

        play.statuses.lock().unwrap().push(SlotStatus {
            username: "alice".into(),
            ..SlotStatus::default()
        });

        play.stop_slot("alice");

        assert!(arm.stop.load(Ordering::Relaxed));
        assert!(!play.spawned.contains("alice"));
        assert!(play.handles.is_empty());
        assert!(!play.arms.contains_key("alice"), "stop_slot drops the arm");
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
            assert!(matches!(
                q.request_permit(42, Instant::now()),
                Permit::Wait(_)
            ));
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
    fn slot_status_is_up_lean_ingame_without_scene_2() {
        let mut lean = SlotStatus {
            username: "s01".into(),
            ingame: true,
            scene_state: 1,
            lean: true,
            ..SlotStatus::default()
        };
        assert!(lean.is_up());
        lean.ingame = false;
        assert!(!lean.is_up());
        let fat = SlotStatus {
            username: "s00".into(),
            ingame: true,
            scene_state: 1,
            lean: false,
            ..SlotStatus::default()
        };
        assert!(!fat.is_up(), "fat Client still loading is not up");
        let fat_ready = SlotStatus {
            username: "s00".into(),
            ingame: true,
            scene_state: 2,
            ..SlotStatus::default()
        };
        assert!(fat_ready.is_up());
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
            SlotStatus {
                username: "a".into(),
                queue_position: -1,
                queue_total: -1,
                ..SlotStatus::default()
            },
            SlotStatus {
                username: "b".into(),
                queue_position: -1,
                queue_total: -1,
                ..SlotStatus::default()
            },
        ];
        apply_queue_wait(
            &mut rows,
            "b",
            Some(host::login_queue::QueuePos {
                position: 2,
                total: 2,
            }),
        );
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
        let com = IfType {
            client_code: api::interact::CC_LOGOUT,
            ..Default::default()
        };
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
            if let Some(s) = statuses
                .lock()
                .unwrap()
                .iter_mut()
                .find(|s| s.username == "alice")
            {
                s.ingame = false;
            }
        });
        assert!(play.wait_until_not_ingame("alice", Duration::from_secs(1)));
    }

    #[test]
    fn retune_unknown_profile_is_error() {
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
        assert!(matches!(
            play.retune("ghost", None, None),
            Err(TuneError::UnknownProfile(n)) if n == "ghost"
        ));
        play.remember_profile(profile("ghost", 1));
        let t0 = Instant::now();
        play.retune("ghost", None, None).unwrap();
        assert!(
            t0.elapsed() < Duration::from_millis(250),
            "retune must not join a slot thread"
        );
        play.stop_slot("ghost");
    }

    #[test]
    fn retune_queues_latest_instead_of_busy() {
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
        play.remember_profile(profile("a", 1));
        play.remember_profile(profile("b", 2));
        play.remember_profile(profile("c", 3));
        play.statuses.lock().unwrap().push(SlotStatus {
            username: "a".into(),
            ingame: true,
            scene_state: 2,
            ..SlotStatus::default()
        });
        let arm_b = SlotArm::new(2, false);
        play.attach_arm("b", Arc::clone(&arm_b));
        play.retune("b", None, None).unwrap();
        assert!(play.tune_pending(), "steal of b has no thread yet");
        play.retune("c", None, None).unwrap();
        assert!(
            play.tune_pending(),
            "a later cap-click must queue, not Busy"
        );
    }

    #[test]
    fn prefer_login_mirrors_k_of_n_onto_the_status_row() {
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
        let arm = SlotArm::new(7, false);
        play.attach_arm("alice", Arc::clone(&arm));
        play.statuses.lock().unwrap().push(SlotStatus {
            username: "alice".into(),
            ..SlotStatus::default()
        });
        play.prefer_login(7);
        let row = play
            .statuses()
            .into_iter()
            .find(|s| s.username == "alice")
            .unwrap();
        assert_eq!(row.queue_position, 1);
        assert_eq!(row.queue_total, 1);
        assert_eq!(play.login_queue_uids(), vec![7]);
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

    /// Tune B: park A is a socket baton (no extra login). B is not yet a
    /// lean channel, so the head's 18 is a fresh TCP. Wrappers: A's 18,
    /// B's 18 — not a third park-18.
    #[test]
    fn tune_b_handshake_is_18_and_parks_a_as_lean() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let wrappers = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&wrappers);
        let server = thread::spawn(move || {
            for _ in 0..2 {
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
                &[18, 18],
                "A tune-in and B tune-in are 18; park is a baton, not a third login"
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
            head.client.local_player.as_ref().unwrap().y,
            0,
            "tune wipes the previous channel's local player"
        );
        assert!(
            head.client.players[123].is_none(),
            "previous players cleared"
        );
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

    fn grant_login(s: &mut std::net::TcpStream, log: &Mutex<Vec<u8>>, code: u8) {
        let mut hdr = [0u8; 2];
        s.read_exact(&mut hdr).unwrap();
        assert_eq!(hdr[0], 14);
        for _ in 0..8 {
            s.write_all(&[0]).unwrap();
        }
        s.write_all(&[0]).unwrap();
        s.write_all(&[0u8; 8]).unwrap();
        let mut buf = [0u8; 512];
        let n = s.read(&mut buf).unwrap();
        assert!(n > 0);
        log.lock().unwrap().push(buf[0]);
        s.write_all(&[code]).unwrap();
        if code == 2 {
            s.write_all(&[0, 0]).unwrap(); // staff + mouse after grant 2
        }
    }

    /// Reverse baton: B is already a lean channel. Tune B must opcode-18
    /// on B's existing TCP (fake reconnect so the server dumps state),
    /// not open a third socket. A is parked by stealing its socket.
    #[test]
    fn tune_reverse_baton_sends_18_on_the_lean_socket() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let wrappers = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&wrappers);
        let server = thread::spawn(move || {
            let (mut a, _) = listener.accept().unwrap();
            grant_login(&mut a, &log, 15);
            let (mut b, _) = listener.accept().unwrap();
            grant_login(&mut b, &log, 2);
            grant_login(&mut b, &log, 15);
            let _ = a;
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
        play.tune("a").unwrap();

        let cfg = ClientConfig {
            host: "127.0.0.1".into(),
            port: addr.port(),
            cache_dir: "/tmp".into(),
            members: true,
            lowmem: true,
        };
        let lean = Lean::login(&cfg, "b", "pw", 2, false).unwrap();
        play.channels.insert("b".into(), lean);
        play.tune("b").unwrap();

        assert_eq!(
            wrappers.lock().unwrap().as_slice(),
            &[18, 16, 18],
            "A 18, B cold 16, B reverse-baton 18 on the same socket"
        );
        assert_eq!(play.head.as_ref().unwrap().name, "b");
        assert!(play.channels.contains_key("a"));
        assert!(!play.channels.contains_key("b"));
        assert_eq!(play.head.as_ref().unwrap().client.scene_state, 0);
        server.join().unwrap();
    }

    // --- Task 5: per-uid compiled scripts ---

    #[test]
    fn script_start_unknown_compiled_id_errors_without_v8() {
        // `script::factory` returns `None` for every picker id until the
        // script is ported (WalkTo is the first port; BoneBurier is not
        // yet); Start must surface that, never a dummy.
        let _play = run_with_io(
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
        // alice is a real (armed) slot, so the error is about the picker id.
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
        play.attach_arm("alice", SlotArm::new(7, false));
        let err = play
            .script_start("alice", script::CompiledId("BoneBurier"))
            .unwrap_err();
        assert!(err.contains("not ported"), "err was {err}");
    }

    #[test]
    fn script_start_load_spawns_isolate_only_on_start_and_refuses_when_active() {
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
        play.attach_arm("alice", SlotArm::new(7, false));
        let src = "export function tick(api) { api._n = (api._n||0)+1 }".to_string();
        play.script_start_load("alice", src.clone(), script::LoadShape::NativeTick)
            .unwrap();
        assert_eq!(play.script_state("alice"), script::RunState::Running);

        let err = play
            .script_start_load("alice", src.clone(), script::LoadShape::NativeTick)
            .unwrap_err();
        assert!(err.contains("active"), "err was {err}");

        play.script_stop("alice");
        assert_eq!(play.script_state("alice"), script::RunState::Idle);

        // Unknown slot: never creates an entry, and never a V8 runtime.
        let err = play
            .script_start_load("ghost", src, script::LoadShape::NativeTick)
            .unwrap_err();
        assert!(err.contains("no slot"), "err was {err}");
        assert!(
            !play.scripts.lock().unwrap().contains_key("ghost"),
            "an unknown uid must never get a SlotScript entry"
        );
    }

    #[test]
    fn script_start_unknown_slot_errors_without_phantom_entry() {
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
        let err = play
            .script_start("ghost", script::CompiledId("WalkTo"))
            .unwrap_err();
        assert!(err.contains("no slot"), "err was {err}");
        assert_eq!(play.script_state("ghost"), script::RunState::Idle);
        assert!(
            !play.scripts.lock().unwrap().contains_key("ghost"),
            "an unknown uid must never get a SlotScript entry"
        );
    }

    #[test]
    fn script_control_is_noop_for_unknown_slot_and_state_defaults_idle() {
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
        assert_eq!(play.script_state("ghost"), script::RunState::Idle);
        assert_eq!(play.script_last_error("ghost"), None);
        play.script_pause("ghost");
        play.script_resume("ghost");
        play.script_stop("ghost");
        assert_eq!(play.script_state("ghost"), script::RunState::Idle);
        play.cheat("ghost", "tele 0,50,50,20,20");
        assert!(
            play.cheats.lock().unwrap().is_empty(),
            "unknown uid cheat is a no-op"
        );
    }

    /// Test script that counts ticks into a shared cell (the panel cannot
    /// read a running script's internals, so the wiring tests observe the
    /// side effect instead).
    #[derive(Default)]
    struct TickCounter(Arc<Mutex<u32>>);

    impl script::Script for TickCounter {
        fn name(&self) -> &str {
            "TickCounter"
        }
        fn tick(&mut self, _ctx: &mut ScriptCtx<'_>) {
            *self.0.lock().unwrap() += 1;
        }
    }

    /// One fat/lean `script_observe` wiring rig: a started slot script for
    /// "alice" plus its (empty) cheat queue.
    struct ScriptWiring {
        scripts: Arc<Mutex<HashMap<String, SlotScript>>>,
        cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>>,
        count: Arc<Mutex<u32>>,
    }

    fn script_wiring() -> ScriptWiring {
        let scripts: Arc<Mutex<HashMap<String, SlotScript>>> = Arc::new(Mutex::new(HashMap::new()));
        let cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let count = Arc::new(Mutex::new(0));
        let mut all = scripts.lock().unwrap();
        let slot = all.entry("alice".into()).or_default();
        slot.start_compiled(Box::new(TickCounter(Arc::clone(&count))))
            .unwrap();
        drop(all);
        cheats
            .lock()
            .unwrap()
            .insert("alice".into(), VecDeque::new());
        ScriptWiring {
            scripts,
            cheats,
            count,
        }
    }

    /// Empty nav rig for observe tests that never touch `ctx.walk`: no
    /// travellers and no grid (the walk hook would refuse anyway).
    #[allow(clippy::type_complexity)]
    fn empty_nav() -> (Arc<Mutex<HashMap<String, Traveller>>>, Option<Arc<StepGrid>>) {
        (Arc::new(Mutex::new(HashMap::new())), None)
    }

    #[test]
    fn script_observe_ticks_only_on_player_edge_while_up() {
        let ScriptWiring {
            scripts,
            cheats,
            count,
        } = script_wiring();
        let (travellers, grid) = empty_nav();
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
        // Not up: the edge must not dispatch (the is_up pause gate).
        script_observe(&mut c, "alice", false, true, 1, None, None, None, &scripts, &cheats, &travellers, &grid);
        assert_eq!(*count.lock().unwrap(), 0);
        // Up + edge: exactly one tick.
        script_observe(&mut c, "alice", true, true, 2, None, None, None, &scripts, &cheats, &travellers, &grid);
        assert_eq!(*count.lock().unwrap(), 1);
        // Up but no edge: nothing.
        script_observe(&mut c, "alice", true, false, 2, None, None, None, &scripts, &cheats, &travellers, &grid);
        assert_eq!(*count.lock().unwrap(), 1);
        // A dispatched tick wrote the driver's out buffer (lean flush cue).
        assert!(script_observe(
            &mut c, "alice", true, true, 3, None, None, None, &scripts, &cheats, &travellers, &grid
        ));
        assert_eq!(*count.lock().unwrap(), 2);
    }

    #[test]
    fn script_observe_idle_slot_publishes_nothing_on_tick_edge() {
        // Task 12: an Idle SlotScript must not publish a script snapshot —
        // no dispatch and no driver write, so the lean pump skips its flush.
        let scripts = Arc::new(Mutex::new(HashMap::new()));
        let cheats = Arc::new(Mutex::new(HashMap::new()));
        let count = Arc::new(Mutex::new(0));
        let (travellers, grid) = empty_nav();
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
        // Never started: no SlotScript entry (Idle). Edge + up publishes
        // nothing — the driver's out buffer stays empty.
        assert!(!script_observe(
            &mut c, "alice", true, true, 1, None, None, None, &scripts, &cheats, &travellers, &grid
        ));
        assert_eq!(c.out.pos, 0, "no script bytes on the driver");
        // Started then stopped: Idle again, same skip.
        scripts
            .lock()
            .unwrap()
            .entry("alice".into())
            .or_default()
            .start_compiled(Box::new(TickCounter(Arc::clone(&count))))
            .unwrap();
        scripts.lock().unwrap().get_mut("alice").unwrap().stop();
        assert_eq!(
            scripts.lock().unwrap().get("alice").unwrap().state(),
            script::RunState::Idle
        );
        assert!(!script_observe(
            &mut c, "alice", true, true, 2, None, None, None, &scripts, &cheats, &travellers, &grid
        ));
        assert_eq!(*count.lock().unwrap(), 0, "Idle must not dispatch tick");
    }

    #[test]
    fn script_observe_drains_queued_cheat_onto_driver() {
        let ScriptWiring {
            scripts,
            cheats,
            count,
        } = script_wiring();
        let (travellers, grid) = empty_nav();
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
        cheats
            .lock()
            .unwrap()
            .get_mut("alice")
            .unwrap()
            .push_back("setvar tutorial 1000".into());
        let wrote = script_observe(
            &mut c, "alice", true, false, 0, None, None, None, &scripts, &cheats, &travellers, &grid,
        );
        assert!(wrote, "the cheat wrote the driver's out buffer");
        assert_eq!(
            c.out.data()[0],
            client::io::ClientProt::CLIENT_CHEAT.id as u8
        );
        assert!(
            cheats.lock().unwrap().get("alice").unwrap().is_empty(),
            "a drained queue stays for the next panel push"
        );
        assert_eq!(
            *count.lock().unwrap(),
            0,
            "no tick edge → the script must not run"
        );
    }

    /// Records what a dispatched tick's ctx exposed: whether the inventory
    /// view and the shared name table reached the script, and the resolved
    /// `has_item` answer for "Bones".
    #[derive(Default)]
    struct InvProbe(Arc<Mutex<Option<(bool, bool, bool)>>>);

    impl script::Script for InvProbe {
        fn name(&self) -> &str {
            "InvProbe"
        }
        fn tick(&mut self, ctx: &mut ScriptCtx<'_>) {
            *self.0.lock().unwrap() = Some((
                ctx.inv.is_some(),
                ctx.obj_names.is_some(),
                ctx.has_item("Bones"),
            ));
        }
    }

    #[test]
    fn script_observe_passes_inventory_when_running() {
        let mut objs = vec![client::config::ObjType::default(); 2];
        objs[1].id = 1;
        objs[1].name = "Bones".into();
        let names = api::obj_names::ObjNames::from_objs(&objs);
        let seen = Arc::new(Mutex::new(None));
        let scripts: Arc<Mutex<HashMap<String, SlotScript>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let (travellers, grid) = empty_nav();
        scripts
            .lock()
            .unwrap()
            .entry("alice".into())
            .or_default()
            .start_compiled(Box::new(InvProbe(Arc::clone(&seen))))
            .unwrap();
        let inv: Vec<(i32, i32)> = vec![(1, 3), (0, 0)];
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
        script_observe(
            &mut c,
            "alice",
            true,
            true,
            1,
            None,
            Some(&inv),
            Some(&names),
            &scripts,
            &cheats,
            &travellers,
            &grid,
        );
        assert_eq!(
            *seen.lock().unwrap(),
            Some((true, true, true)),
            "a Running script sees the inventory view and resolves names"
        );
    }

    /// Test script that queues one walk to a (mutable) target each tick and
    /// records what `ctx.walk` returned.
    struct WalkProbe(Arc<Mutex<Option<bool>>>, Arc<Mutex<(i32, i32, i32)>>);

    impl script::Script for WalkProbe {
        fn name(&self) -> &str {
            "WalkProbe"
        }
        fn tick(&mut self, ctx: &mut ScriptCtx<'_>) {
            let (x, z, level) = *self.1.lock().unwrap();
            let ok = match ctx.walk.as_mut() {
                Some(w) => w(x, z, level),
                None => false,
            };
            *self.0.lock().unwrap() = Some(ok);
        }
    }

    /// Recording driver: captures the last accepted walk target. `route`
    /// is `(0,0)` and `build_base` `(0,0)`, so absolute world tiles equal
    /// scene tiles and `api::walk` resolves a route origin.
    #[derive(Default)]
    struct NavRec {
        walked: Option<(i32, i32)>,
        sink: Sink,
    }

    impl Driver for NavRec {
        fn set_menu(&mut self, _slot: i32, _action: i32, _a: i32, _b: i32, _c: i32) {}
        fn do_action(&mut self, _slot: i32) -> bool {
            true
        }
        fn try_move(
            &mut self,
            _src_x: i32,
            _src_z: i32,
            dx: i32,
            dz: i32,
            _try_nearest: bool,
            _loc_width: i32,
            _loc_length: i32,
            _loc_angle: i32,
            _loc_shape: i32,
            _forceapproach: i32,
            _ty: i32,
        ) -> bool {
            self.walked = Some((dx, dz));
            true
        }
        fn local_route(&self) -> Option<(i32, i32)> {
            Some((0, 0))
        }
        fn build_base(&self) -> (i32, i32) {
            (0, 0)
        }
        fn loc_typecode(&self, _scene_x: i32, _scene_z: i32) -> Option<i32> {
            None
        }
        fn out(&mut self) -> &mut dyn api::prot::Out {
            &mut self.sink
        }
        fn login(&mut self, _username: &str, _password: &str, _reconnect: bool) -> bool {
            true
        }
    }

    /// Minimal outbound sink: the recording driver never writes packets.
    #[derive(Default)]
    struct Sink;

    impl api::prot::Out for Sink {
        fn p1_enc(&mut self, _opcode: i32) {}
        fn p1(&mut self, _value: i32) {}
        fn p2(&mut self, _value: i32) {}
        fn p4(&mut self, _value: i32) {}
        fn pjstr(&mut self, _s: &str) {}
    }

    /// Walk-hook rig: a started `WalkProbe` for "alice" (target `(2,0,0)`),
    /// an empty travellers map, the open-3×3 fixture grid, and a status row.
    struct NavRig {
        scripts: Arc<Mutex<HashMap<String, SlotScript>>>,
        cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>>,
        travellers: Arc<Mutex<HashMap<String, Traveller>>>,
        grid: Option<Arc<StepGrid>>,
        statuses: Arc<Mutex<Vec<SlotStatus>>>,
        walk_ret: Arc<Mutex<Option<bool>>>,
        walk_target: Arc<Mutex<(i32, i32, i32)>>,
    }

    fn nav_rig() -> NavRig {
        let scripts: Arc<Mutex<HashMap<String, SlotScript>>> = Arc::new(Mutex::new(HashMap::new()));
        let cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let travellers: Arc<Mutex<HashMap<String, Traveller>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let grid = Some(Arc::new(StepGrid::fixture_open_3x3()));
        let statuses: Arc<Mutex<Vec<SlotStatus>>> = Arc::new(Mutex::new(Vec::new()));
        let walk_ret = Arc::new(Mutex::new(None));
        let walk_target = Arc::new(Mutex::new((2, 0, 0)));
        scripts
            .lock()
            .unwrap()
            .entry("alice".into())
            .or_default()
            .start_compiled(Box::new(WalkProbe(
                Arc::clone(&walk_ret),
                Arc::clone(&walk_target),
            )))
            .unwrap();
        statuses.lock().unwrap().push(SlotStatus {
            username: "alice".into(),
            ..SlotStatus::default()
        });
        NavRig {
            scripts,
            cheats,
            travellers,
            grid,
            statuses,
            walk_ret,
            walk_target,
        }
    }

    #[test]
    fn script_observe_walk_arms_route_and_pump_steps_traveller() {
        let NavRig {
            scripts,
            cheats,
            travellers,
            grid,
            statuses,
            walk_ret,
            ..
        } = nav_rig();
        let mut d = NavRec::default();

        // The observe dispatches the script tick with the walk hook; the
        // hook A*s from the observed `here` and arms the uid's traveller.
        assert!(script_observe(
            &mut d,
            "alice",
            true,
            true,
            1,
            Some((0, 0, 0)),
            None,
            None,
            &scripts,
            &cheats,
            &travellers,
            &grid,
        ));
        assert_eq!(
            *walk_ret.lock().unwrap(),
            Some(true),
            "ctx.walk returned true for an armed route"
        );
        assert_eq!(
            travellers.lock().unwrap().get("alice").and_then(|t| t.queued()),
            Some(Tile { x: 2, z: 0, level: 0 }),
            "the armed route queues the requested dest"
        );

        // The pump's per-uid nav step sends one hop toward the dest and
        // mirrors the queued target into the status row.
        assert_eq!(
            step_traveller(&mut d, "alice", Some((0, 0, 0)), false, &travellers, &statuses),
            NavStatus::Walking,
        );
        assert_eq!(d.walked, Some((2, 0)), "the hop targets the dest tile");
        {
            let rows = statuses.lock().unwrap();
            assert_eq!(rows[0].walk_x, 2, "status mirrors the queued target");
            assert_eq!(rows[0].walk_z, 0);
            assert_eq!(rows[0].walk_level, 0);
        }

        // Standing on the dest reports Arrived and clears the route.
        assert_eq!(
            step_traveller(&mut d, "alice", Some((2, 0, 0)), false, &travellers, &statuses),
            NavStatus::Arrived,
        );
        assert_eq!(
            travellers.lock().unwrap().get("alice").and_then(|t| t.queued()),
            None,
            "arrival clears the armed route"
        );
        {
            let rows = statuses.lock().unwrap();
            assert_eq!(rows[0].walk_x, -1, "idle traveller reports no target");
        }
    }

    #[test]
    fn script_observe_walk_is_false_without_here_grid_or_path() {
        let NavRig {
            scripts,
            cheats,
            travellers,
            grid,
            walk_ret,
            walk_target,
            ..
        } = nav_rig();
        let no_grid: Option<Arc<StepGrid>> = None;
        let mut d = NavRec::default();

        // No observed tile: the hook refuses before any grid lookup.
        script_observe(
            &mut d,
            "alice",
            true,
            true,
            1,
            None,
            None,
            None,
            &scripts,
            &cheats,
            &travellers,
            &grid,
        );
        assert_eq!(*walk_ret.lock().unwrap(), Some(false), "no here → no arm");

        // No grid: nothing to route on.
        script_observe(
            &mut d,
            "alice",
            true,
            true,
            2,
            Some((0, 0, 0)),
            None,
            None,
            &scripts,
            &cheats,
            &travellers,
            &no_grid,
        );
        assert_eq!(*walk_ret.lock().unwrap(), Some(false), "no grid → no arm");

        // A dest the grid cannot reach: NoPath.
        *walk_target.lock().unwrap() = (5, 5, 0);
        script_observe(
            &mut d,
            "alice",
            true,
            true,
            3,
            Some((0, 0, 0)),
            None,
            None,
            &scripts,
            &cheats,
            &travellers,
            &grid,
        );
        assert_eq!(*walk_ret.lock().unwrap(), Some(false), "no path → no arm");
        assert!(
            travellers
                .lock()
                .unwrap()
                .get("alice")
                .and_then(|t| t.queued())
                .is_none(),
            "nothing was armed across the refusals"
        );
    }
}
