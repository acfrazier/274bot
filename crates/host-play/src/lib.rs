//! `host-play`: run vaulted profiles through the host kernel. The binary
//! unlocks a vault and runs the named profiles; the `e2e` harness links
//! this library so it can poll per-slot state instead of scraping logs.

pub mod audio;

use std::collections::{HashMap, HashSet, VecDeque};
use std::panic::AssertUnwindSafe;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use api::interact::Driver;
use client::client::Client;
use client::client::ClientConfig;
use client::client::LoginError;
use client::config::if_type::ComponentType;
use client::config::{Cache, IfType, IfTypeMut};
use client::io::JagFile;
pub use host::debug_enabled;
use host::login_queue::{LoginBackoff, LoginQueue, Permit, QueuePos};
use host::prepare_client;
pub use host::set_debug;
pub use host::Host;
mod rss;
mod scatter;
use api::snapshot::{GameSnapshot, WorldTile};
use host::{should_emit_tick, wake_channel, FrameBuf, Pump, SlotInput, SlotPark, SlotWake};
use nav::router::{find_with, FindOptions, Route};
use nav::traveller::{TravelOptions, Traveller};
use nav::world::NavWorld;
pub use rss::sample_process;
pub use scatter::{scatter_tile_for, tele_args};

/// [`client::bot_target::world_host_for`] from a `BOT_TARGET` string.
pub fn world_host_for_bot_target(target: Option<&str>) -> String {
    client::bot_target::world_host_for(client::bot_target::bot_target_from_env(target)).into()
}

/// Active world host (`BOT_TARGET` / `--prod`).
pub fn default_world_host() -> String {
    client::world_host()
}

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
    /// traveller's route dest by the pump's per-uid nav step each
    /// observe).
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
    /// Newest `MESSAGE_GAME` / chat-ring head (`chat_text[0]`). Used to
    /// parse `getvar` replies (`get tutorial: 1000`).
    pub chat_head: String,
}

impl SlotStatus {
    /// Wall member is online: every slot is a full `Client` now (no lean
    /// special case), so a slot is up when the scene is built.
    pub fn is_up(&self) -> bool {
        self.ingame && self.scene_state == 2
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
            chat_head: String::new(),
        }
    }
}

/// Copy the stream byte counters from `Client` onto a `SlotStatus` row. No
/// stream → bytes stay 0.
///
/// The old draw-entry counters and frame timings are gone from `Client`
/// (M2 Task 1): `game_draw_enters`/`title_screen_draw_enters` are
/// unmaintainable through the opaque `Renderer::mainredraw`, and the
/// loop/raster/paint/skip timings are slot-local in `host`'s private
/// `SlotLoop`. The status row keeps only what `Client.stream` still
/// exposes.
pub fn copy_stream_bytes(c: &Client, s: &mut SlotStatus) {
    let (bi, bo) = c
        .stream
        .as_ref()
        .map(|st| (st.bytes_in(), st.bytes_out()))
        .unwrap_or((0, 0));
    s.bytes_in = bi;
    s.bytes_out = bo;
}

/// One observe of a slot's script wiring: gate [`SlotScript::on_is_up`],
/// dispatch [`SlotScript::on_game_tick`] on the PLAYER_INFO edge, then run
/// any cheats the panel queued. `driver` is the slot body's own `Client`
/// (the only `Driver`); `here` is the local player's world tile
/// `(x, z, level)` when the body decoded one, else `None` (then the walk
/// hooks refuse to arm). `navs`/`world` back the `ctx.walk` and
/// `ctx.walk_with` closures — one shared arm ([`WalkArm`]) takes the
/// [`FindOptions`] (`walk` passes the defaults): the arm refuses
/// synchronously only when there is no tile, no nav world, or a route
/// already queued; `find_with` runs off-pump on a short-lived worker per
/// request, storing the route in the uid's nav bot when one exists (a walk
/// that would panic on the first follow step must not succeed when no
/// route can arm). Returns whether the driver's out buffer was written
/// (the slot's own `Client` sends on its next mainloop pass). A slot whose
/// script is Idle/Paused publishes nothing — no dispatch, no flush.
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
    navs: &Arc<Mutex<HashMap<String, NavBot>>>,
    world: &Option<Arc<NavWorld>>,
) -> bool {
    let mut wrote = false;
    {
        let mut all = scripts.lock().unwrap();
        if let Some(slot) = all.get_mut(name) {
            slot.on_is_up(up);
            // skip script snapshot unless SlotScript is Running.
            if tick_edge && slot.state() == script::RunState::Running {
                // One shared arm for both hooks: `walk_with` carries the
                // script's options through to `find_with`; `walk` is the
                // default-options adapter (rs2b0t `walk` semantics stay
                // default-off for teleports and wilderness). Each closure
                // owns its own clone of the arm.
                let arm = WalkArm {
                    here,
                    world: world.clone(),
                    navs: Arc::clone(navs),
                    name: name.to_string(),
                };
                let mut walk_with = {
                    let arm = arm.clone();
                    move |x: i32, z: i32, level: i32, o: script::FindOptions| -> bool {
                        arm.route(
                            x,
                            z,
                            level,
                            FindOptions {
                                allow_teleports: o.allow_teleports,
                                allow_wilderness: o.allow_wilderness,
                            },
                        )
                    }
                };
                let mut walk = {
                    let arm = arm.clone();
                    move |x: i32, z: i32, level: i32| -> bool {
                        arm.route(x, z, level, FindOptions::default())
                    }
                };
                slot.on_game_tick(&mut ScriptCtx {
                    driver,
                    tick,
                    here,
                    walk: Some(&mut walk),
                    walk_with: Some(&mut walk_with),
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

/// Per-slot nav latch key: the `(player gen, here)` pair the pump last
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

/// Per-uid nav state: the whole-world traveller plus the route it is
/// following. `ctx.walk` stores the route (found off-pump over the shared
/// [`NavWorld`]); the slot pump polls [`Traveller::follow`] with a clone of
/// it one step per player-info tick. `route` being set is the "armed"
/// gate the walk hook and the busy flag read.
#[derive(Default)]
struct NavBot {
    traveller: Traveller,
    route: Option<Route>,
}

/// The shared script walk arm: both `ctx.walk` (default options) and
/// `ctx.walk_with` (explicit options) route through [`WalkArm::route`].
/// Each observe clones the arm once per hook (all fields are `Clone`), so
/// the two `&mut` hooks never share a mutable borrow.
#[derive(Clone)]
struct WalkArm {
    here: Option<(i32, i32, i32)>,
    world: Option<Arc<NavWorld>>,
    navs: Arc<Mutex<HashMap<String, NavBot>>>,
    name: String,
}

impl WalkArm {
    /// Queue one walk toward `(x, z, level)` with `opts`, routing off-pump
    /// on a short-lived worker (`find_with` over the shared [`NavWorld`]).
    /// Refuses synchronously only when there is no player tile, no nav
    /// world, or a route already queued for the uid; the worker stores the
    /// route in the uid's nav bot when one exists. Returns whether the
    /// worker was spawned — not whether a path exists.
    fn route(&self, x: i32, z: i32, level: i32, opts: FindOptions) -> bool {
        let Some((hx, hz, hl)) = self.here else {
            return false;
        };
        let Some(world) = self.world.as_ref() else {
            return false;
        };
        // One route in flight per uid: a script spamming walk every tick
        // must not spawn a worker each tick.
        if self
            .navs
            .lock()
            .unwrap()
            .get(&self.name)
            .is_some_and(|b| b.route.is_some())
        {
            return false;
        }
        let from = WorldTile {
            x: hx,
            z: hz,
            level: hl,
        };
        let to = WorldTile { x, z, level };
        let world = Arc::clone(world);
        let navs = Arc::clone(&self.navs);
        let name = self.name.clone();
        // Routing is the expensive part: run `find_with` off-pump on a
        // short-lived worker. The worker is detached and exits right after
        // storing the route; it never touches the scripts map (lock order
        // stays scripts → navs).
        thread::Builder::new()
            .name(format!("nav-find-{name}"))
            .spawn(move || {
                if let Ok(route) = find_with(&world.collision, &world.graph, from, to, opts) {
                    navs.lock().unwrap().entry(name).or_default().route = Some(route);
                }
            })
            .is_ok()
    }
}

/// One pump step of a uid's nav bot: poll the armed route through
/// [`Traveller::follow`] one step against `snapshot` (the slot's per-tick
/// view, rebuilt from the same client that supplied `here`). `here` is the
/// player's world tile when the body decoded one (else the bot stands
/// still). Mirrors the armed route's dest into the status row's `walk_*`
/// fields (`-1` when idle); any terminal outcome clears the route —
/// arrival and stall alike — so the status flips back to idle and a script
/// may arm a fresh walk.
fn step_nav_bot<D: Driver>(
    driver: &mut D,
    name: &str,
    here: Option<(i32, i32, i32)>,
    snapshot: &GameSnapshot,
    navs: &Arc<Mutex<HashMap<String, NavBot>>>,
    statuses: &Arc<Mutex<Vec<SlotStatus>>>,
) {
    if here.is_none() {
        return;
    }
    let mut options = TravelOptions {
        // Exact arrival: the armed dest must be stood on before the route
        // clears (the v1 traveller arrived the same way).
        close_enough: 0,
        ..TravelOptions::default()
    };
    let queued = {
        let mut all = navs.lock().unwrap();
        let Some(bot) = all.get_mut(name) else {
            return;
        };
        let Some(route) = bot.route.clone() else {
            return;
        };
        if bot
            .traveller
            .follow(driver, snapshot, route, &mut options)
            .is_some()
        {
            bot.route = None;
        }
        bot.route.as_ref().map(|r| r.dest)
    };
    let mut rows = statuses.lock().unwrap();
    if let Some(s) = rows.iter_mut().find(|s| s.username == name) {
        match queued {
            Some(d) => {
                s.walk_x = d.x;
                s.walk_z = d.z;
                s.walk_level = d.level;
            }
            None => {
                s.walk_x = -1;
                s.walk_z = -1;
                s.walk_level = -1;
            }
        }
    }
}

/// The fat Client's inventory `(obj_id, count)` slots, zipped from the
/// TYPE_INV iface's linked obj ids/numbers (the server's `UPDATE_INV_FULL`
/// fills them each frame). The iface stores `obj_id + 1` (0 = empty), so
/// the view carries the real 0-based ids scripts resolve `has_item`
/// against — the same convention as `api::snapshot`'s inv view.
/// Short-lived: rebuilt per observe while the slot script is Running;
/// `None` when no TYPE_INV iface is loaded yet. Reads the client's
/// combined iface view (per-client overlay first) so server-written slots
/// show, not the shared decode.
fn inventory_from_ifaces(client: &Client) -> Option<Vec<(i32, i32)>> {
    let inv = client
        .ifaces_merged()
        .find(|f| f.r#type == ComponentType::TYPE_INV)?;
    let (Some(ids), Some(counts)) = (&inv.link_obj_type, &inv.link_obj_number) else {
        return None;
    };
    Some(
        ids.iter()
            .zip(counts)
            .filter(|(id, _)| **id > 0)
            .map(|(id, n)| (*id - 1, *n))
            .collect(),
    )
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
    /// is 16; after a grant this is true.
    pub reconnect: Arc<AtomicBool>,
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
        })
    }
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
fn tick_flags(client: &mut Client, ifaces: &[Option<Box<IfType>>], arm: &SlotArm) -> bool {
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
/// Flat slot model (M2): every profile owns **one** full `Client` on its
/// own slot thread — there is no channel head and no lean baton.
/// [`Play::focus`] only records which slot the panel samples (the old
/// head); switching focus never touches a socket. `profiles` keeps the
/// vault credentials for later spawns and reconnects.
pub struct Play {
    /// Shared status rows; panel tests push fakes here for `pump_status`.
    pub statuses: Arc<Mutex<Vec<SlotStatus>>>,
    handles: HashMap<String, thread::JoinHandle<()>>,
    options: PlayOptions,
    cache: Arc<Cache>,
    /// The shared obj-id → name table every script ctx resolves `has_item`
    /// against (built once from `cache.objs`).
    obj_names: Arc<api::obj_names::ObjNames>,
    ifaces: Arc<Vec<Option<Box<IfType>>>>,
    ifaces_mut_template: Vec<Option<Box<IfTypeMut>>>,
    queue: Arc<Mutex<LoginQueue>>,
    per_frame: SlotFrame,
    spawned: HashSet<String>,
    arms: HashMap<String, Arc<SlotArm>>,
    /// Vault profiles keyed by username (a later spawn/reconnect looks up
    /// the password/uid here).
    profiles: HashMap<String, Profile>,
    /// The slot the panel currently samples (the old channel head). Pure
    /// bookkeeping: every slot is a full `Client` on its own thread.
    focused: Option<String>,
    /// Per-slot compiled scripts: the slot threads drive `on_is_up` /
    /// `on_game_tick` on each drain, the panel arms them via the
    /// [`Play::script_start`] family. Keyed by username (the identity the
    /// status rows and arms use).
    scripts: Arc<Mutex<HashMap<String, SlotScript>>>,
    /// Per-slot cheat commands the panel queued; each slot thread runs
    /// `api::interact::cheat` on its own Driver and flushes the socket.
    cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>>,
    /// Per-uid nav bots: `ctx.walk` stores a route in the uid's bot and
    /// the slot pump polls `Traveller::follow` with it one step per
    /// player-info tick. One struct per bot on the pump — no per-bot nav
    /// thread.
    navs: Arc<Mutex<HashMap<String, NavBot>>>,
    /// Host-scope nav world (collision + transport graph) baked from the
    /// pack at construction (see [`default_pack_path`]); `None` when no
    /// pack loads, and `ctx.walk` then refuses to arm.
    world: Option<Arc<NavWorld>>,
    /// Per-slot control wake ends: [`SlotWake::wake`] kicks a parked slot
    /// thread (focus/draw/stop/spawn), which re-reads the shared state on
    /// its next tick. Inserted at spawn, removed after `stop_slot` joins.
    wakes: HashMap<String, SlotWake>,
}

impl Play {
    /// Shared construction for the public `run*` entry points: one login
    /// FIFO, one shared cache/iface template, and the per-slot script/cheat
    /// maps. `per_frame` starts as a no-op — slots stay draw-off (headless)
    /// until a caller's own per-frame hook turns a slot's renderer on.
    fn new(options: &PlayOptions) -> Play {
        let (cache, ifaces, ifaces_mut_template) = load_template(&options.cache_dir);
        let cache = Arc::new(cache);
        let obj_names = Arc::new(api::obj_names::ObjNames::from_objs(&cache.objs));
        Play {
            statuses: Arc::new(Mutex::new(Vec::new())),
            handles: HashMap::new(),
            options: options.clone(),
            cache,
            obj_names,
            ifaces: Arc::new(ifaces),
            ifaces_mut_template,
            queue: Arc::new(Mutex::new(LoginQueue::default())),
            per_frame: Arc::new(|_: &mut Client, _: &str| {}),
            spawned: HashSet::new(),
            arms: HashMap::new(),
            profiles: HashMap::new(),
            focused: None,
            scripts: Arc::new(Mutex::new(HashMap::new())),
            cheats: Arc::new(Mutex::new(HashMap::new())),
            navs: Arc::new(Mutex::new(HashMap::new())),
            world: NavWorld::load_pack(&default_pack_path()).ok().map(Arc::new),
            wakes: HashMap::new(),
        }
    }

    /// Make `name` the focused slot — the one the panel samples (the old
    /// channel head). Focus is pure bookkeeping in the flat model: every
    /// slot is a full `Client` on its own thread, so switching focus never
    /// parks/adopts a socket. Unknown names are allowed (the wall may spawn
    /// them later). The newly focused slot is kicked so a parked thread
    /// re-reads its draw state (the panel's per-frame hook applies
    /// `set_draw` on the next tick).
    pub fn focus(&mut self, name: &str) {
        self.focused = Some(name.to_string());
        self.wake(name);
    }

    /// The focused slot's name, `None` when nothing is focused yet.
    pub fn focused(&self) -> Option<String> {
        self.focused.clone()
    }

    /// Kick one slot's parked thread (a no-op when the name is not a
    /// running slot or the thread is already awake). The panel/host-play
    /// call this whenever a shared-state change must take effect within a
    /// frame instead of at the next game-tick park timeout.
    pub fn wake(&self, name: &str) {
        if let Some(w) = self.wakes.get(name) {
            w.wake();
        }
    }

    /// Kick every running slot (wall-policy changes like
    /// `only_render_selected` affect every member's draw state).
    pub fn wake_all(&self) {
        for w in self.wakes.values() {
            w.wake();
        }
    }

    /// Snapshot of every slot's status.
    pub fn statuses(&self) -> Vec<SlotStatus> {
        self.statuses.lock().unwrap().clone()
    }

    /// The shared obj-id → name table (built from the cache once per
    /// `Play`). Harness evidence and scripts resolve item names through
    /// it.
    pub fn obj_names(&self) -> Arc<api::obj_names::ObjNames> {
        Arc::clone(&self.obj_names)
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

    /// Keep vault credentials for a later [`Play::spawn_slot`] /
    /// reconnect.
    pub fn remember_profile(&mut self, profile: Profile) {
        self.profiles.insert(profile.username.clone(), profile);
    }

    /// Move `uid` to the front of the login FIFO so the TV head handshakes
    /// before slots that already queued. Mirrors the place onto the
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
            .start_compiled(make())?;
        self.wake(name);
        Ok(())
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
            .start_load(source, shape)?;
        self.wake(name);
        Ok(())
    }

    /// Pause `name`'s script (operator Pause; survives login until
    /// Resume re-arms it). No-op when the slot has no script.
    pub fn script_pause(&self, name: &str) {
        if let Some(slot) = self.scripts.lock().unwrap().get_mut(name) {
            slot.pause();
        }
        self.wake(name);
    }

    /// Resume `name`'s script; the next `on_is_up` re-gates it.
    pub fn script_resume(&self, name: &str) {
        if let Some(slot) = self.scripts.lock().unwrap().get_mut(name) {
            slot.resume();
        }
        self.wake(name);
    }

    /// Stop `name`'s script: teardown hook, instance dropped, Idle.
    pub fn script_stop(&self, name: &str) {
        if let Some(slot) = self.scripts.lock().unwrap().get_mut(name) {
            slot.stop();
        }
        self.wake(name);
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
        self.wake(user);
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
        if self.focused.as_deref() == Some(name) {
            self.focused = None;
        }
        // Wake a parked thread so its next probe sees `stop`; the wake end
        // stays alive (removed after the join) so the poll cannot miss it.
        if let Some(handle) = self.handles.remove(name) {
            self.wake(name);
            let _ = handle.join();
        }
        self.wakes.remove(name);
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
        mailbox: Option<Arc<FrameBuf>>,
        arm: Option<Arc<SlotArm>>,
    ) {
        // Keep the vault credentials on the wall for later spawns and
        // DC-reconnect re-handshakes.
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
        // The control wake: `Play::wake` kicks the parked slot thread on
        // focus/draw/stop/spawn changes; the slot thread polls the park end.
        let (wake, park) = wake_channel();
        self.wakes.insert(profile.username.clone(), wake);
        spawn_slot_thread(
            &self.options,
            profile,
            input,
            mailbox,
            Some(park),
            arm,
            Arc::clone(&self.cache),
            self.ifaces.clone(),
            self.ifaces_mut_template.clone(),
            Arc::clone(&self.queue),
            Arc::clone(&self.statuses),
            Arc::clone(&self.scripts),
            Arc::clone(&self.cheats),
            Arc::clone(&self.navs),
            self.world.clone(),
            Arc::clone(&self.obj_names),
            Arc::clone(&self.per_frame),
            &mut self.handles,
        );
    }
}

/// Spawn one slot thread per profile. Each slot waits for a login-queue
/// permit, sends the handshake, then drives `mainloop` at the host cadence
/// while mirroring its state into the shared status list. Slots run with no
/// input and no frame mailbox; [`run_with_io`] adds per-slot channels.
pub fn run(options: &PlayOptions, profiles: Vec<Profile>) -> Play {
    run_with_io(options, profiles, |_| (None, None), |_, _| {})
}

/// Like [`run`], but each slot gets the `SlotInput`/`FrameBuf` mailbox
/// returned by `per_slot` (called synchronously, keyed by username), and
/// `per_frame` runs inside the observe hook on every 20 ms frame so callers
/// can mirror panel state (e.g. `client.set_draw`) into the slot thread.
/// The FIFO login queue and mainland hop are shared by every slot.
pub fn run_with_io<F, G>(
    options: &PlayOptions,
    profiles: Vec<Profile>,
    per_slot: F,
    per_frame: G,
) -> Play
where
    F: Fn(&str) -> (Option<Arc<SlotInput>>, Option<Arc<FrameBuf>>),
    G: Fn(&mut Client, &str) + Send + Sync + 'static,
{
    let mut play = Play::new(options);
    play.per_frame = Arc::new(per_frame);
    for profile in profiles {
        let (slot_input, slot_mailbox) = per_slot(&profile.username);
        play.spawn_slot(profile, slot_input, slot_mailbox, None);
    }
    play
}

/// Wall spawn for the e2e ladder: every profile spawns one full `Client`
/// slot (the old `1 fat + N lean` split is gone). `heads` only selects
/// whether the first profile gets the login-FIFO front.
pub fn run_channels(options: &PlayOptions, profiles: Vec<Profile>, heads: usize) -> Play {
    let mut play = Play::new(options);
    let tv_uid = profiles.first().map(|p| p.uid);
    for profile in profiles {
        play.spawn_slot(profile, None, None, None);
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
    slot_mailbox: Option<Arc<FrameBuf>>,
    park: Option<SlotPark>,
    arm: Arc<SlotArm>,
    slot_cache: Arc<Cache>,
    ifaces_template: Arc<Vec<Option<Box<IfType>>>>,
    ifaces_mut_template: Vec<Option<Box<IfTypeMut>>>,
    slot_queue: Arc<Mutex<LoginQueue>>,
    slot_statuses: Arc<Mutex<Vec<SlotStatus>>>,
    slot_scripts: Arc<Mutex<HashMap<String, SlotScript>>>,
    slot_cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>>,
    slot_navs: Arc<Mutex<HashMap<String, NavBot>>>,
    slot_world: Option<Arc<NavWorld>>,
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
                // Publish the row before `prepare_client`/`maininit`
                // (a slow cache fetch can stall for seconds), so the
                // queue card shows the slot while it loads.
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
            // The park end survives re-login rounds (run_client is entered
            // once per ingame stretch), so wrap it once here.
            let park = park.map(Arc::new);
            let mut client = prepare_client(
                config,
                uid,
                slot_cache,
                ifaces_template.clone(),
                ifaces_mut_template.clone(),
            );
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
            // `maininit` is renderer-free now: progress recording lives on
            // the Client, and no `Renderer` is constructed for a headless
            // slot.
            client.maininit();
            if client.error_loading && debug_enabled() {
                eprintln!("[host-play] slot {username}: maininit failed");
            }

            let mut backoff = LoginBackoff::new();
            loop {
                if arm.stop.load(Ordering::Relaxed) {
                    slot_queue.lock().unwrap().leave(uid);
                    return;
                }
                if !client.ingame {
                    if !should_handshake(&arm, client.ingame) {
                        thread::sleep(Duration::from_millis(20));
                        continue;
                    }
                    wait_for_permit(&slot_queue, &slot_statuses, &username, uid, &arm.stop);
                    if arm.stop.load(Ordering::Relaxed) {
                        slot_queue.lock().unwrap().leave(uid);
                        return;
                    }
                    mark_login_started(&slot_statuses, &username);
                    let reconnect = arm.reconnect.load(Ordering::Relaxed);
                    if debug_enabled() {
                        eprintln!(
                            "[host-play] slot {username}: handshake begin reconnect={reconnect}"
                        );
                    }
                    match client.login(&username, &password, reconnect) {
                        Ok(()) => {
                            backoff.reset();
                            on_login_success(&arm);
                            if debug_enabled() {
                                eprintln!("[host-play] slot {username}: handshake ok");
                            }
                        }
                        Err(e) => {
                            record_login_error(&slot_statuses, &username, &e);
                            thread::sleep(login_retry_wait(&mut backoff, e.code));
                            continue;
                        }
                    }
                }
                let mut mainland_sent = false;
                let arm_obs = Arc::clone(&arm);
                let obs_name = username.clone();
                Host::run_client(
                    &mut client,
                    &username,
                    slot_input.clone(),
                    slot_mailbox.clone(),
                    park.clone(),
                    {
                        let slot_frame = Arc::clone(&slot_frame);
                        let slot_statuses = Arc::clone(&slot_statuses);
                        let slot_scripts = Arc::clone(&slot_scripts);
                        let slot_cheats = Arc::clone(&slot_cheats);
                        let slot_obj_names = Arc::clone(&slot_obj_names);
                        let slot_navs = Arc::clone(&slot_navs);
                        let slot_world = slot_world.clone();
                        let mut pump = Pump::new();
                        let mut script_tick: u64 = 0;
                        // Last `(player gen, here)` the nav bot stepped:
                        // skip until either changes so the hop budget counts
                        // server ticks, not 20 ms frames (panel `tick_latch`).
                        let mut last_nav_step: Option<NavStepKey> = None;
                        // The slot's per-tick nav snapshot, rebuilt only
                        // when a route is armed (the follow surface reads
                        // the canonical base + route-head tile).
                        let mut nav_snapshot = GameSnapshot::new();
                        move |c, _ignored, run_sends| {
                            let name = &obs_name;
                            slot_frame(c, name);
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
                                    if s.username == *name {
                                        s.ingame = c.ingame;
                                        s.scene_state = c.scene_state;
                                        s.runenergy = c.runenergy;
                                        s.run_sends = run_sends;
                                        s.main_modal_id = c.main_modal_id;
                                        copy_stream_bytes(c, s);
                                        s.chat_head = c.chat_text[0].clone();
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
                                            // the body yet (see gaps.md).
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
                            let inv = if script_running(&slot_scripts, name) {
                                inventory_from_ifaces(c)
                            } else {
                                None
                            };
                            script_observe(
                                c,
                                name,
                                up,
                                tick_edge,
                                script_tick,
                                here,
                                inv.as_deref(),
                                Some(slot_obj_names.as_ref()),
                                &slot_scripts,
                                &slot_cheats,
                                &slot_navs,
                                &slot_world,
                            );
                            // Per-uid nav step on the pump, gated on the
                            // player-gen/tile latch like the panel's WalkTo
                            // hook so a hop is sent once per server tick,
                            // not re-sent every 20 ms frame. The snapshot is
                            // rebuilt only when the bot has an armed route
                            // (the rebuild is cheap when no gen moved).
                            let nav_key = (c.gens.player, here);
                            if last_nav_step != Some(nav_key) {
                                last_nav_step = Some(nav_key);
                                if here.is_some()
                                    && slot_navs
                                        .lock()
                                        .unwrap()
                                        .get(name)
                                        .is_some_and(|b| b.route.is_some())
                                {
                                    nav_snapshot.rebuild(c);
                                    step_nav_bot(
                                        c,
                                        name,
                                        here,
                                        &nav_snapshot,
                                        &slot_navs,
                                        &slot_statuses,
                                    );
                                }
                            }
                            // Busy flag for the idle scheduler: a slot with
                            // a running script, queued cheats, or an armed
                            // nav bot must keep ticking (never parked), so
                            // the observe hook reports it every frame.
                            script_running(&slot_scripts, name)
                                || slot_cheats
                                    .lock()
                                    .unwrap()
                                    .get(name)
                                    .is_some_and(|q| !q.is_empty())
                                || slot_navs
                                    .lock()
                                    .unwrap()
                                    .get(name)
                                    .is_some_and(|b| b.route.is_some())
                        }
                    },
                    {
                        let ifaces_template = ifaces_template.clone();
                        move |c| tick_flags(c, &ifaces_template, &arm_obs) || !c.ingame
                    },
                );
                if arm.stop.load(Ordering::Relaxed) {
                    slot_queue.lock().unwrap().leave(uid);
                    return;
                }
                let mut all = slot_statuses.lock().unwrap();
                if let Some(s) = all.iter_mut().find(|s| s.username == username) {
                    s.ingame = client.ingame;
                    s.scene_state = client.scene_state;
                    s.chat_head = client.chat_text[0].clone();
                }
            }
            })
            .expect("failed to spawn slot thread"),
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
fn load_template(
    cache_dir: &str,
) -> (Cache, Vec<Option<Box<IfType>>>, Vec<Option<Box<IfTypeMut>>>) {
    let cache = match std::fs::read(format!("{cache_dir}/config")) {
        Ok(bytes) => {
            std::panic::catch_unwind(AssertUnwindSafe(|| Cache::unpack(&JagFile::new(bytes))))
                .unwrap_or_default()
        }
        Err(_) => Cache::default(),
    };
    let (ifaces, ifaces_mut) = match std::fs::read(format!("{cache_dir}/interface")) {
        Ok(bytes) => {
            std::panic::catch_unwind(AssertUnwindSafe(|| IfType::unpack(&JagFile::new(bytes))))
                .unwrap_or_default()
        }
        Err(_) => (Vec::new(), Vec::new()),
    };
    (cache, ifaces, ifaces_mut)
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

    use client::client::ClientConfig;
    use client::config::Cache;
    use vault::ProfileSettings;

    #[test]
    fn world_host_live_is_rs2b2t_everything_else_is_loopback() {
        assert_eq!(world_host_for_bot_target(Some("live")), "w1.rs2b2t.com");
        assert_eq!(world_host_for_bot_target(Some("prod")), "w1.rs2b2t.com");
        assert_eq!(world_host_for_bot_target(Some("local")), "127.0.0.1");
        assert_eq!(world_host_for_bot_target(None), "127.0.0.1");
    }

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
        let quiet = Profile {
            username: "a".into(),
            password: "a".into(),
            uid: 1,
            settings: ProfileSettings {
                lowmem: true,
                auto_login: false,
                tutorial_skipped: None,
                raster: vault::RasterMode::Gpu,
            },
        };
        let loud = Profile {
            username: "b".into(),
            password: "b".into(),
            uid: 2,
            settings: ProfileSettings {
                lowmem: false,
                auto_login: false,
                tutorial_skipped: None,
                raster: vault::RasterMode::Gpu,
            },
        };
        assert!(bot_client_config(&opt, &quiet).lowmem);
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
    fn obj_names_getter_shares_the_play_table() {
        // A cache-less temp dir falls back to `Cache::default()` (empty
        // objs), so the table is empty but still shared and queryable.
        let dir = std::env::temp_dir().join(format!("274bot-empty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let play = run_with_io(
            &PlayOptions {
                host: "127.0.0.1".into(),
                port: 43594,
                cache_dir: dir.display().to_string(),
                lowmem: true,
                mainland: false,
            },
            vec![],
            |_| (None, None),
            |_, _| {},
        );
        assert!(play.obj_names().name(526).is_none());
        assert!(play.obj_names().by_name("Bones").is_none());
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
        // uid 7 sits on the FIFO behind a full 30/60s address window;
        // stop_slot must drop it even though the thread is still running.
        {
            let mut q = play.queue.lock().unwrap();
            let now = Instant::now();
            for i in 0..30 {
                assert!(matches!(q.request_permit(1000 + i, now), Permit::Grant));
            }
            assert!(matches!(q.request_permit(7, now), Permit::Wait(_)));
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
    fn stop_slot_wakes_a_parked_thread_before_joining() {
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
        // A fake parked slot: blocks on the control park fd with a long
        // poll (like the real idle scheduler) and only exits once `stop`
        // is set *and* the wake fires it out of the poll.
        let (wake, park) = wake_channel();
        play.wakes.insert("bob".into(), wake);
        let arm = SlotArm::new(9, false);
        play.arms.insert("bob".into(), Arc::clone(&arm));
        play.spawned.insert("bob".into());
        let stop = Arc::clone(&arm.stop);
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let watchdog = thread::spawn(move || {
            let mut fds = [libc::pollfd {
                fd: park.fd(),
                events: libc::POLLIN,
                revents: 0,
            }];
            let rc = unsafe { libc::poll(fds.as_mut_ptr(), 1, 5000) };
            assert!(rc > 0, "stop_slot's wake must fire the parked poll");
            park.drain();
            assert!(stop.load(Ordering::Relaxed), "woken because stop was set");
            done_tx.send(()).unwrap();
        });
        play.handles.insert("bob".into(), watchdog);

        thread::sleep(Duration::from_millis(50));
        let start = Instant::now();
        play.stop_slot("bob");
        // Without the wake the join would wait out the 5 s poll; the wake
        // must return the rail ✕ within a frame.
        assert!(
            start.elapsed() < Duration::from_millis(500),
            "stop_slot must wake a parked thread, not wait for its poll"
        );
        done_rx
            .recv_timeout(Duration::from_millis(200))
            .expect("parked thread never acknowledged the wake");
        assert!(play.wakes.is_empty(), "stop_slot drops the wake end");
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
        // The profile uid 42 sits queued behind a full address window;
        // stopping must drop 42 from the FIFO, not the arm's stale uid 0.
        {
            let mut q = play.queue.lock().unwrap();
            let now = Instant::now();
            for i in 0..30 {
                assert!(matches!(q.request_permit(1000 + i, now), Permit::Grant));
            }
            assert!(matches!(q.request_permit(42, now), Permit::Wait(_)));
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
        let a = prepare_client(cfg, 1, Arc::clone(&cache), Arc::new(vec![]), Vec::new());
        assert!(Arc::ptr_eq(&a.cache, &cache));
        assert!(!a.error_loading);
    }

    #[test]
    fn slot_status_walk_defaults_cleared() {
        let s = SlotStatus::default();
        assert_eq!((s.walk_x, s.walk_z, s.walk_level), (-1, -1, -1));
    }

    #[test]
    fn slot_status_is_up_requires_scene_2() {
        let loading = SlotStatus {
            username: "s01".into(),
            ingame: true,
            scene_state: 1,
            ..SlotStatus::default()
        };
        assert!(!loading.is_up(), "still loading is not up");
        let mut ready = SlotStatus {
            username: "s01".into(),
            ingame: true,
            scene_state: 2,
            ..SlotStatus::default()
        };
        assert!(ready.is_up());
        ready.ingame = false;
        assert!(!ready.is_up(), "logged out is not up");
    }

    #[test]
    fn copy_stream_bytes_zeros_without_stream() {
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
            Arc::new(vec![]),
            Vec::new(),
        );
        let mut s = SlotStatus {
            username: "t".into(),
            ..SlotStatus::default()
        };
        copy_stream_bytes(&c, &mut s);
        assert_eq!(s.bytes_in, 0);
        assert_eq!(s.bytes_out, 0);
    }

    /// The flat slot row mirrors `Client.stream`'s payload byte counters;
    /// a completed handshake proves both directions count.
    #[test]
    fn copy_stream_bytes_mirrors_stream_counters() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let log = Arc::new(Mutex::new(Vec::new()));
        let server = thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            grant_login(&mut s, &log, 2);
            // Keep the socket open briefly so the writer thread flushes the
            // login block before the client drops it.
            thread::sleep(Duration::from_millis(50));
        });
        let mut c = prepare_client(
            ClientConfig {
                host: "127.0.0.1".into(),
                port: addr.port(),
                cache_dir: String::new(),
                members: true,
                lowmem: true,
            },
            1,
            Arc::new(Cache::default()),
            Arc::new(vec![]),
            Vec::new(),
        );
        c.login("a", "pw", false).unwrap();
        let mut s = SlotStatus {
            username: "a".into(),
            ..SlotStatus::default()
        };
        copy_stream_bytes(&c, &mut s);
        assert!(s.bytes_in > 0, "handshake reads count as bytes_in");
        assert!(s.bytes_out > 0, "handshake writes count as bytes_out");
        server.join().unwrap();
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
        ifaces[7] = Some(Box::new(com));
        let mut client = prepare_client(cfg, 1, Arc::new(Cache::default()), Arc::new(ifaces.clone()), Vec::new());
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
        // Fill the 30/60s address window so alice waits on the FIFO.
        {
            let mut q = queue.lock().unwrap();
            let now = Instant::now();
            for i in 0..30 {
                assert!(matches!(q.request_permit(1000 + i, now), Permit::Grant));
            }
            assert!(matches!(q.request_permit(7, now), Permit::Wait(_)));
        }
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
    fn focus_selects_the_sampled_slot() {
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
        assert_eq!(play.focused(), None, "no slot is focused before focus()");
        play.focus("b");
        assert_eq!(play.focused().as_deref(), Some("b"));
        play.focus("c");
        assert_eq!(
            play.focused().as_deref(),
            Some("c"),
            "focus only records the sampled slot — no socket is touched"
        );
    }

    #[test]
    fn stop_slot_clears_focus_on_the_stopped_name() {
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
        // No real client: fake arm + a handle that exits only once
        // `stop_slot` flags it.
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
        play.focus("alice");
        assert_eq!(play.focused().as_deref(), Some("alice"));

        play.stop_slot("alice");

        assert_eq!(
            play.focused(),
            None,
            "focus must not dangle on a stopped slot"
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
                tutorial_skipped: None,
                raster: vault::RasterMode::Gpu,
            },
        }
    }

    /// Flat model: spawning two profiles gives two full `Client` slots —
    /// one status row and one control arm each, no lean channel bookkeeping.
    #[test]
    fn two_profiles_spawn_two_client_slots() {
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
        // `auto_login = false` arms sit on the title (no TCP), so both
        // threads idle on a 20 ms sleep until `stop_slot` joins them.
        play.spawn_slot(profile("a", 1), None, None, Some(SlotArm::new(1, false)));
        play.spawn_slot(profile("b", 2), None, None, Some(SlotArm::new(2, false)));

        assert_eq!(play.arms.len(), 2, "one control arm per profile");
        // The slot threads publish their status rows asynchronously.
        assert!(
            wait_until(500, || {
                let names: Vec<String> = play.statuses().into_iter().map(|s| s.username).collect();
                names.contains(&"a".into()) && names.contains(&"b".into())
            }),
            "each profile's slot thread publishes one status row"
        );
        assert_eq!(play.statuses().len(), 2, "one status row per profile");

        play.stop_slot("a");
        play.stop_slot("b");
        assert_eq!(play.statuses().len(), 0, "both slots stopped");
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

    /// One `script_observe` wiring rig: a started slot script for
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
    /// nav bots and no nav world (the walk hook would refuse anyway).
    fn empty_nav() -> (Arc<Mutex<HashMap<String, NavBot>>>, Option<Arc<NavWorld>>) {
        (Arc::new(Mutex::new(HashMap::new())), None)
    }

    /// Poll `cond` for up to `ms` milliseconds. The route-arming worker is
    /// detached, so tests wait on the effect instead of joining it.
    fn wait_until(ms: u64, mut cond: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_millis(ms);
        while Instant::now() < deadline {
            if cond() {
                return true;
            }
            thread::sleep(Duration::from_millis(1));
        }
        cond()
    }

    #[test]
    fn script_observe_ticks_only_on_player_edge_while_up() {
        let ScriptWiring {
            scripts,
            cheats,
            count,
        } = script_wiring();
        let (navs, world) = empty_nav();
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
            Arc::new(vec![]),
            Vec::new(),
        );
        // Not up: the edge must not dispatch (the is_up pause gate).
        script_observe(
            &mut c, "alice", false, true, 1, None, None, None, &scripts, &cheats, &navs, &world,
        );
        assert_eq!(*count.lock().unwrap(), 0);
        // Up + edge: exactly one tick.
        script_observe(
            &mut c, "alice", true, true, 2, None, None, None, &scripts, &cheats, &navs, &world,
        );
        assert_eq!(*count.lock().unwrap(), 1);
        // Up but no edge: nothing.
        script_observe(
            &mut c, "alice", true, false, 2, None, None, None, &scripts, &cheats, &navs, &world,
        );
        assert_eq!(*count.lock().unwrap(), 1);
        // A dispatched tick wrote the driver's out buffer (the slot's own
        // `Client` sends it on the next mainloop pass).
        assert!(script_observe(
            &mut c, "alice", true, true, 3, None, None, None, &scripts, &cheats, &navs, &world
        ));
        assert_eq!(*count.lock().unwrap(), 2);
    }

    #[test]
    fn script_observe_idle_slot_publishes_nothing_on_tick_edge() {
        // Task 12: an Idle SlotScript must not publish a script snapshot —
        // no dispatch and no driver write, so the slot has nothing to send
        // on the next mainloop pass.
        let scripts = Arc::new(Mutex::new(HashMap::new()));
        let cheats = Arc::new(Mutex::new(HashMap::new()));
        let count = Arc::new(Mutex::new(0));
        let (navs, world) = empty_nav();
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
            Arc::new(vec![]),
            Vec::new(),
        );
        // Never started: no SlotScript entry (Idle). Edge + up publishes
        // nothing — the driver's out buffer stays empty.
        assert!(!script_observe(
            &mut c, "alice", true, true, 1, None, None, None, &scripts, &cheats, &navs, &world
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
            &mut c, "alice", true, true, 2, None, None, None, &scripts, &cheats, &navs, &world
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
        let (navs, world) = empty_nav();
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
            Arc::new(vec![]),
            Vec::new(),
        );
        cheats
            .lock()
            .unwrap()
            .get_mut("alice")
            .unwrap()
            .push_back("setvar tutorial 1000".into());
        let wrote = script_observe(
            &mut c, "alice", true, false, 0, None, None, None, &scripts, &cheats, &navs, &world,
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
        let scripts: Arc<Mutex<HashMap<String, SlotScript>>> = Arc::new(Mutex::new(HashMap::new()));
        let cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let (navs, world) = empty_nav();
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
            Arc::new(vec![]),
            Vec::new(),
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
            &navs,
            &world,
        );
        assert_eq!(
            *seen.lock().unwrap(),
            Some((true, true, true)),
            "a Running script sees the inventory view and resolves names"
        );
    }

    #[test]
    fn inventory_from_ifaces_maps_1_based_ids_to_0_based() {
        // The TYPE_INV iface stores `obj_id + 1` (0 = empty slot); scripts
        // resolve `has_item` against the 0-based ObjNames table, so the
        // view must carry `id - 1` and drop the empties.
        let mut ifaces = vec![None; 3];
        ifaces[1] = Some(Box::new(IfType {
            r#type: ComponentType::TYPE_INV,
            ..Default::default()
        }));
        let mut ifaces_mut = vec![None; 3];
        ifaces_mut[1] = Some(Box::new(IfTypeMut {
            link_obj_type: Some(vec![2, 0, 1]),
            link_obj_number: Some(vec![3, 0, 1]),
            ..Default::default()
        }));
        let mut client = prepare_client(
            ClientConfig {
                host: "127.0.0.1".into(),
                port: 1,
                cache_dir: String::new(),
                members: true,
                lowmem: true,
            },
            1,
            Arc::new(Cache::default()),
            Arc::new(ifaces),
            ifaces_mut,
        );
        let inv = inventory_from_ifaces(&client).expect("TYPE_INV iface present");
        assert_eq!(
            inv,
            vec![(1, 3), (0, 1)],
            "1-based ids map down by one and empty slots drop"
        );

        // End-to-end: the mapped id-0 slot must resolve via has_item.
        let mut objs = vec![client::config::ObjType::default(); 1];
        objs[0].id = 0;
        objs[0].name = "Bones".into();
        let names = api::obj_names::ObjNames::from_objs(&objs);
        let mut rec = NavRec {
            walked: None,
            sink: Sink,
        };
        let ctx = ScriptCtx {
            driver: &mut rec,
            tick: 0,
            here: None,
            walk: None,
            walk_with: None,
            inv: Some(&inv),
            obj_names: Some(&names),
        };
        assert!(ctx.has_item("Bones"));
        assert!(!ctx.has_item("Vial"));
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

    /// Walk-hook rig: a started `WalkProbe` for "alice" (target `(4,0,0)`),
    /// an empty nav-bot map, the open-1×40 fixture world (x in 0..40 at
    /// z=0), and a status row.
    struct NavRig {
        scripts: Arc<Mutex<HashMap<String, SlotScript>>>,
        cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>>,
        navs: Arc<Mutex<HashMap<String, NavBot>>>,
        world: Option<Arc<NavWorld>>,
        statuses: Arc<Mutex<Vec<SlotStatus>>>,
        walk_ret: Arc<Mutex<Option<bool>>>,
        walk_target: Arc<Mutex<(i32, i32, i32)>>,
    }

    fn nav_rig() -> NavRig {
        let scripts: Arc<Mutex<HashMap<String, SlotScript>>> = Arc::new(Mutex::new(HashMap::new()));
        let cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let navs: Arc<Mutex<HashMap<String, NavBot>>> = Arc::new(Mutex::new(HashMap::new()));
        // An all-walkable 40×1 world at (0,0): x in 0..40 at z=0, no
        // transport edges — the v2-world shape `find` consumes, built
        // directly (no pack file on disk in unit tests).
        let world = Some(Arc::new(NavWorld {
            collision: nav::collision::WorldCollision {
                origin: WorldTile {
                    x: 0,
                    z: 0,
                    level: 0,
                },
                width: 40,
                height: 1,
                flags: vec![0; 40],
                walkable: vec![0u32; 40],
            },
            graph: nav::transport::TransportGraph::default(),
        }));
        let statuses: Arc<Mutex<Vec<SlotStatus>>> = Arc::new(Mutex::new(Vec::new()));
        let walk_ret = Arc::new(Mutex::new(None));
        let walk_target = Arc::new(Mutex::new((4, 0, 0)));
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
            navs,
            world,
            statuses,
            walk_ret,
            walk_target,
        }
    }

    /// The armed route's dest, `None` when the uid has no route. The pump
    /// and the walk-refusal gate read the same field.
    fn queued(navs: &Arc<Mutex<HashMap<String, NavBot>>>) -> Option<WorldTile> {
        navs.lock()
            .unwrap()
            .get("alice")
            .and_then(|b| b.route.as_ref())
            .map(|r| r.dest)
    }

    /// A connected, ingame, scene-ready client on build base (0,0): world
    /// tiles equal scene tiles, so `find`/`follow` and the pump's `here`
    /// agree, and the snapshot passes `Interactions`' attached/ingame/
    /// scene preconditions (the scenario runner's follow client does the
    /// same).
    fn nav_client() -> Client {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let stream =
            client::io::ClientStream::connect(&addr.ip().to_string(), addr.port()).unwrap();
        // Keep the listener alive so the connect stays established.
        std::mem::forget(listener);
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
            Arc::new(vec![]),
            Vec::new(),
        );
        c.stream = Some(stream);
        c.ingame = true;
        c.scene_state = 2;
        c
    }

    /// Rebuild the slot's nav snapshot with the player at scene/world
    /// `(x, z)` (the body has no level decode, so level stays 0).
    fn nav_snapshot_at(c: &mut Client, snap: &mut GameSnapshot, x: i32, z: i32) {
        c.local_player = Some(client::dash3d::ClientPlayer::at(x, z));
        c.bump_gens(client::io::ServerProt::PLAYER_INFO);
        c.bump_gens(client::io::ServerProt::REBUILD_NORMAL);
        snap.rebuild(c);
    }

    #[test]
    fn script_observe_walk_arms_route_and_pump_steps_follow() {
        let NavRig {
            scripts,
            cheats,
            navs,
            world,
            statuses,
            walk_ret,
            ..
        } = nav_rig();
        let mut d = NavRec::default();
        let mut c = nav_client();

        // The observe dispatches the script tick with the walk hook; the
        // hook queues the request from the observed `here` and the worker
        // arms the uid's nav bot off-pump.
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
            &navs,
            &world,
        ));
        assert_eq!(
            *walk_ret.lock().unwrap(),
            Some(true),
            "ctx.walk queued the route request"
        );
        assert!(
            wait_until(100, || queued(&navs)
                == Some(WorldTile {
                    x: 4,
                    z: 0,
                    level: 0
                })),
            "the worker armed the route"
        );

        // The pump's per-uid nav step polls follow once, sending one hop
        // toward the dest and mirroring the armed dest into the status row.
        let mut snap = GameSnapshot::new();
        nav_snapshot_at(&mut c, &mut snap, 0, 0);
        step_nav_bot(&mut d, "alice", Some((0, 0, 0)), &snap, &navs, &statuses);
        assert_eq!(d.walked, Some((4, 0)), "the hop targets the dest tile");
        {
            let rows = statuses.lock().unwrap();
            assert_eq!(rows[0].walk_x, 4, "status mirrors the armed dest");
            assert_eq!(rows[0].walk_z, 0);
            assert_eq!(rows[0].walk_level, 0);
        }

        // Standing on the dest, the next pump poll reports Arrived and
        // clears the route; the status flips back to idle.
        nav_snapshot_at(&mut c, &mut snap, 4, 0);
        step_nav_bot(&mut d, "alice", Some((4, 0, 0)), &snap, &navs, &statuses);
        assert_eq!(queued(&navs), None, "arrival clears the armed route");
        {
            let rows = statuses.lock().unwrap();
            assert_eq!(rows[0].walk_x, -1, "idle bot reports no target");
            assert_eq!(rows[0].walk_z, -1);
            assert_eq!(rows[0].walk_level, -1);
        }
    }

    #[test]
    fn script_observe_walk_queues_off_pump_and_refuses_when_unarmable() {
        let NavRig {
            scripts,
            cheats,
            navs,
            world,
            walk_ret,
            walk_target,
            ..
        } = nav_rig();
        let no_world: Option<Arc<NavWorld>> = None;
        let mut d = NavRec::default();

        // No observed tile: synchronous refusal before any world lookup.
        script_observe(
            &mut d, "alice", true, true, 1, None, None, None, &scripts, &cheats, &navs, &world,
        );
        assert_eq!(*walk_ret.lock().unwrap(), Some(false), "no here → refuse");

        // No nav world: synchronous refusal, no worker.
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
            &navs,
            &no_world,
        );
        assert_eq!(*walk_ret.lock().unwrap(), Some(false), "no world → refuse");

        // A request the world cannot satisfy is still queued (true) but
        // never arms: the worker's find fails and it exits without
        // touching the map.
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
            &navs,
            &world,
        );
        assert_eq!(
            *walk_ret.lock().unwrap(),
            Some(true),
            "a no-path request is queued, not found synchronously"
        );
        thread::sleep(Duration::from_millis(20));
        assert_eq!(queued(&navs), None, "NoPath never arms a route");

        // A reachable request arms asynchronously on the worker.
        *walk_target.lock().unwrap() = (2, 0, 0);
        script_observe(
            &mut d,
            "alice",
            true,
            true,
            4,
            Some((0, 0, 0)),
            None,
            None,
            &scripts,
            &cheats,
            &navs,
            &world,
        );
        assert_eq!(*walk_ret.lock().unwrap(), Some(true));
        assert!(
            wait_until(100, || queued(&navs)
                == Some(WorldTile {
                    x: 2,
                    z: 0,
                    level: 0
                })),
            "the worker armed the queued route"
        );

        // A second walk while a route is queued refuses synchronously, so
        // a script spamming walk every tick spawns no worker per tick.
        *walk_target.lock().unwrap() = (1, 0, 0);
        script_observe(
            &mut d,
            "alice",
            true,
            true,
            5,
            Some((0, 0, 0)),
            None,
            None,
            &scripts,
            &cheats,
            &navs,
            &world,
        );
        assert_eq!(
            *walk_ret.lock().unwrap(),
            Some(false),
            "already-queued → refuse, no worker spawn"
        );
        assert_eq!(
            queued(&navs),
            Some(WorldTile {
                x: 2,
                z: 0,
                level: 0
            }),
            "the armed route is untouched"
        );
    }
}
