//! `host-play`: run vaulted profiles through the host kernel. The binary
//! unlocks a vault and runs the named profiles; the `e2e` harness links
//! this library so it can poll per-slot state instead of scraping logs.

pub mod audio;

use std::collections::{HashMap, HashSet, VecDeque};
use std::panic::AssertUnwindSafe;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use api::interact::Driver;
use client::client::Client;
use client::client::ClientConfig;
use client::client::LoginError;
use client::config::{Cache, IfType, IfTypeMut};
use client::io::JagFile;
pub use host::debug_enabled;
use host::login_queue::{LoginBackoff, LoginQueue, Permit, QueuePos};
use host::prepare_client;
pub use host::set_debug;
pub use host::Host;
/// The random-event guardian's published status (see [`SlotStatus::random`]
/// — the chrome contract both the panel and the TUI bind).
pub use host::{RandomClaim, RandomStatus};
mod rss;
mod scatter;
use api::snapshot::{GameSnapshot, WorldTile};
use host::{
    should_emit_tick, wake_channel, DetectedRandom, FrameBuf, Pump, SlotInput, SlotPark, SlotWake,
};
use nav::bank_fetch::{plan_bank_fetch, BankStep};
use nav::router::{find_missing_item_reqs, find_with, FindOptions, Route};
use nav::tile::Tile;
use nav::traveller::{TravelOptions, Traveller};
use nav::world::NavWorld;
use nav::WorldState;
pub use rss::{count_tcp_to, parse_lsof_established, sample_process};
pub use scatter::{scatter_tile_for, tele_args};

/// [`client::bot_target::world_host_for`] from a `BOT_TARGET` string.
pub fn world_host_for_bot_target(target: Option<&str>) -> String {
    client::bot_target::world_host_for(client::bot_target::bot_target_from_env(target)).into()
}

/// Active world host (`BOT_TARGET` / `--prod`).
pub fn default_world_host() -> String {
    client::world_host()
}

/// Mint `n` per-run usernames for a live boot (`live<token>_<i>`). The
/// engine auto-registers unknown names, so a minted name logs into a
/// fresh save instead of the shared `test` account. The engine enforces
/// the classic 12-character username limit, so the token is the pid plus
/// a per-process serial, truncated to fit `live<token>_<i>` in 12 chars.
/// Player saves accumulate under the engine's `player/` dir — wipe it to
/// reset.
pub fn mint_live_names(n: usize) -> Vec<String> {
    static SERIAL: AtomicUsize = AtomicUsize::new(0);
    let serial = SERIAL.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let slot_digits = n.saturating_sub(1).max(1).to_string().len();
    let max_token = 12usize.saturating_sub(4 + 1 + slot_digits).max(1);
    let token: String = format!("{pid:x}{serial:x}")
        .chars()
        .take(max_token)
        .collect();
    (0..n).map(|i| format!("live{token}_{i}")).collect()
}

/// Per-slot hook invoked by the slot thread after every mainloop pass.
/// Per-frame hook: `(client, username, hold)`. `hold` is the guardian's
/// published hold from the previous frame (same lag as `step_nav_bot`) —
/// panel/TUI skip scenario follow and WalkArm follow while it is set.
type SlotFrame = Arc<dyn Fn(&mut Client, &str, bool) + Send + Sync>;
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
    /// The random-event guardian's published status (kind/name/ours/
    /// handling/hold/toggle/claim/cooldown), copied from `Host`'s
    /// `client_frame` return each observe. The chrome contract both the
    /// panel and the TUI bind.
    pub random: RandomStatus,
    /// The slot script's latest recorded paint frame (the Load isolate
    /// forwards it after every tick that painted); `None` when the slot
    /// has no script or the script has not painted. Copied from the
    /// isolate each observe — the TUI shows it in the chat pane in place
    /// of the game chat.
    pub script_paint: Option<script::shim::ScriptPaint>,
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

/// A latched BankBudget session: remaining [`BankStep`]s plus the dest
/// the arm re-finds after the session lands. Follow freezes only for
/// Open / Deposit / Withdraw / Wear / Close; [`BankStep::Walk`] follows
/// the stand sub-route. Wear-from-inv and bank-trip deposit/withdraw
/// both pump through the same path. `final_route` is the post-session
/// route (status row + follow once steps clear); a Walk-to-stand may
/// temporarily replace `WalkArm::route` / `NavBot::route`.
#[derive(Debug, Clone)]
pub struct PendingBankFetch {
    pub steps: VecDeque<BankStep>,
    pub dest: WorldTile,
    pub opts: FindOptions,
    pub final_route: Route,
}

/// Per-username WalkTo arm: the whole-world [`Traveller`] plus the
/// [`Route`] it is following. [`arm_walk_on`] stores the route (found
/// over the shared [`NavWorld`]); the slot hook polls
/// [`Traveller::follow`] with a clone of it one step per player-info
/// tick. `route` being set is the "armed" gate the status row and the
/// overlay read; any terminal outcome clears it (arrival and stall
/// alike). A pending [`PendingBankFetch`] freezes follow for non-Walk
/// steps (Open / deposit / withdraw / Wear / Close); Walk follows the
/// stand sub-route only (never `final_route` until the session clears).
/// Shared by the panel and the TUI so a walk armed from either view
/// drives the same follow path.
#[derive(Default)]
pub struct WalkArm {
    pub traveller: Traveller,
    pub route: Option<Route>,
    pub bank_fetch: Option<PendingBankFetch>,
}

impl WalkArm {
    /// The armed route's dest as a tile, `None` when idle.
    pub fn queued_tile(&self) -> Option<Tile> {
        self.route.as_ref().map(|r| Tile {
            x: r.dest.x,
            z: r.dest.z,
            level: r.dest.level,
        })
    }

    /// Whether a WalkArm / scenario follow may poll this frame. Guardian
    /// hold freezes follow; the armed route stays latched.
    pub fn may_follow(hold: bool) -> bool {
        !hold
    }
}

/// One operator interaction queued from a view (the TUI) onto a slot.
/// The slot thread drains the queue in its observe hook through
/// [`api::interact::Interactions`] on its own `Client` — the same wire
/// path the scenario runner and the guardian use, so a queued send
/// respects the same preconditions and lands on the slot's live socket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireCmd {
    /// `Interactions::continue_dialog`: press the chat modal's Continue
    /// button. Unsticks NPC dialogue the guardian is not handling.
    Continue,
    /// `Interactions::answer_choice(option)`: press the chat modal's
    /// `option`-th BUTTON_OK choice (1-based).
    Answer(i32),
    /// `Interactions::walk` to an adjacent world tile (WASD one-step, a
    /// direct `try_move` — not a routed walk arm).
    Walk { x: i32, z: i32, level: i32 },
}

/// The per-slot walk arms keyed by username (shared with the panel and
/// TUI; [`arm_walk_on`] latches the picked route on the focused slot).
pub type WalkArms = Arc<Mutex<HashMap<String, Arc<Mutex<WalkArm>>>>>;

/// `arm_walk_on` no-path result: the picked dest has no route under the
/// caller's nav options (the caller keeps its picked dest and shows a
/// short error).
#[derive(Debug)]
pub struct NoPath;

/// Route `from` → `dest` over `world` and latch the found route on the
/// focused slot's [`WalkArm`] (keyed by username) when one is named.
/// `options` carries the caller's nav settings (`allow_teleports` /
/// `allow_wilderness` / `allow_bank_fetch`); the focused arm's latched
/// essence-mine session is fed in after — a player inside the mine can
/// walk out through the exit portal's return hop, a slot with no latch
/// keeps the mine sealed. `state` gates payable edges: the focused slot's
/// last published snapshot facts, fail-closed [`WorldState::empty`] when
/// none. `bank` is the open bank's rows (obj id, count) for the BankBudget
/// session — empty when the bank is closed (no closed-bank inventory).
/// On success the caller's picked dest is stored by the arm's route;
/// when `allow_bank_fetch` is on and `find` fails only on missing
/// item/worn reqs, a [`PendingBankFetch`] is latched and the post-session
/// route is armed. Returns `Err(NoPath)` when no path (and no session)
/// exists.
pub fn arm_walk_on(
    world: &NavWorld,
    from: Tile,
    dest: Tile,
    options: FindOptions,
    state: &WorldState,
    bank: &[(i32, i32)],
    travellers: &WalkArms,
    focused: Option<&str>,
) -> Result<Route, NoPath> {
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
    // The focused slot's latched essence-mine session: a player standing
    // inside the mine can WalkTo out through the exit portal (the router
    // synthesizes the return hop from the latch). A slot with no latch
    // stays fail-closed — the mine is sealed, exactly like the packed
    // graph.
    let mut options = options;
    options.essence = focused.and_then(|name| {
        travellers
            .lock()
            .unwrap()
            .get(name)
            .and_then(|arm| arm.lock().unwrap().traveller.essence())
    });
    let outcome = route_or_bank_fetch(world, from_w, dest_w, options, state, bank);
    match outcome {
        RouteOutcome::Routed(route) => {
            if let Some(name) = focused {
                let arm = travellers
                    .lock()
                    .unwrap()
                    .entry(name.to_string())
                    .or_insert_with(|| Arc::new(Mutex::new(WalkArm::default())))
                    .clone();
                let mut arm = arm.lock().unwrap();
                // A fresh arm replaces any in-flight follow run.
                arm.traveller.clear();
                arm.bank_fetch = None;
                arm.route = Some(route.clone());
            }
            Ok(route)
        }
        RouteOutcome::BankSession { pending, route } => {
            if let Some(name) = focused {
                let arm = travellers
                    .lock()
                    .unwrap()
                    .entry(name.to_string())
                    .or_insert_with(|| Arc::new(Mutex::new(WalkArm::default())))
                    .clone();
                let mut arm = arm.lock().unwrap();
                arm.traveller.clear();
                arm.bank_fetch = Some(pending);
                arm.route = Some(route.clone());
            }
            Ok(route)
        }
        RouteOutcome::NoPath => Err(NoPath),
    }
}

/// Outcome of a walk-arm route attempt: a direct route, a BankBudget
/// session plus the post-session route, or no path.
enum RouteOutcome {
    Routed(Route),
    BankSession {
        pending: PendingBankFetch,
        route: Route,
    },
    NoPath,
}

/// Strict `find_with`, then — only when `allow_bank_fetch` is on and the
/// failure is solely missing item/worn reqs — plan a BankBudget session
/// and re-find against the session's post state. Never inserts a virtual
/// bank edge into Dijkstra.
fn route_or_bank_fetch(
    world: &NavWorld,
    from: WorldTile,
    to: WorldTile,
    opts: FindOptions,
    state: &WorldState,
    bank: &[(i32, i32)],
) -> RouteOutcome {
    match find_with(&world.collision, &world.graph, from, to, opts, state) {
        Ok(route) => RouteOutcome::Routed(route),
        Err(_) if opts.allow_bank_fetch => {
            let Some(missing) =
                find_missing_item_reqs(&world.collision, &world.graph, from, to, opts, state)
            else {
                return RouteOutcome::NoPath;
            };
            let Some(fetch) = plan_bank_fetch(&missing, state, bank, world.banks(), from) else {
                return RouteOutcome::NoPath;
            };
            // Re-find against the post-session state (ADR 0005: find itself
            // stayed fail-closed; the session is what unblocks).
            let Ok(route) = find_with(
                &world.collision,
                &world.graph,
                from,
                to,
                FindOptions {
                    allow_bank_fetch: false,
                    ..opts
                },
                &fetch.state,
            ) else {
                return RouteOutcome::NoPath;
            };
            RouteOutcome::BankSession {
                pending: PendingBankFetch {
                    steps: fetch.steps.into(),
                    dest: to,
                    opts: FindOptions {
                        allow_bank_fetch: false,
                        ..opts
                    },
                    final_route: route.clone(),
                },
                route,
            }
        }
        Err(_) => RouteOutcome::NoPath,
    }
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
            random: RandomStatus::default(),
            script_paint: None,
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
/// hooks refuse to arm). `state` is the slot's gating facts at observe
/// time (built from its live snapshot); `None` when no player is decoded
/// — the walk arm then routes with the fail-closed empty [`WorldState`],
/// so an edge whose requirements the state cannot prove is never relaxed.
/// `snapshot` is the same per-tick [`GameSnapshot`] the ctx getters read
/// (`varp`, `stat_level`, `chat`, `bank`, …); it stays `None` only when
/// no snapshot was built, and the getters fail closed on it.
/// `navs`/`world` back the `ctx.walk` and `ctx.walk_with` closures — one
/// shared arm ([`ScriptWalkArm`]) takes the [`FindOptions`] (`walk` passes the
/// defaults): the arm refuses synchronously only when there is no tile,
/// no nav world, or a route already queued; `find_with` runs off-pump on
/// a short-lived worker per request, storing the route in the uid's nav
/// bot when one exists (a walk that would panic on the first follow step
/// must not succeed when no route can arm). `hold` is the guardian's
/// random-event freeze: while true the tick is not dispatched (follow is
/// frozen by the pump too), so a script cannot walk through an in-flight
/// dialog or a trapped maze/mime/box — the snapshot blob still posts on
/// the held tick edge, so EventSignal reads the freeze the script is
/// frozen by. `ours` is the guardian's published
/// detected-ours flag (posted into the isolate for `EventSignal.pending()`).
/// Returns whether the driver's
/// out buffer was written (the slot's own `Client` sends on its next
/// mainloop pass). A slot whose script is Idle/Paused publishes nothing
/// — no dispatch, no flush.
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
    state: Option<WorldState>,
    snapshot: Option<&GameSnapshot>,
    obj_names: Option<&api::obj_names::ObjNames>,
    scripts: &Arc<Mutex<HashMap<String, SlotScript>>>,
    cheats: &Arc<Mutex<HashMap<String, VecDeque<String>>>>,
    navs: &Arc<Mutex<HashMap<String, NavBot>>>,
    world: &Option<Arc<NavWorld>>,
    hold: bool,
    ours: bool,
) -> bool {
    let mut wrote = false;
    let mut interact = Vec::new();
    {
        let mut all = scripts.lock().unwrap();
        if let Some(slot) = all.get_mut(name) {
            slot.on_is_up(up);
            // Post the snapshot only while the slot script is Running.
            // While the guardian holds: still dispatch the isolate tick so
            // `onPaint` runs (loop/pump stay frozen inside the isolate);
            // compiled scripts stay fully frozen (0.1.2). The blob still
            // posts while held so EventSignal reads the freeze.
            if tick_edge && slot.state() == script::RunState::Running {
                // Task 9b: post the FlatBuffer snapshot blob on every tick
                // edge — held or not — so the isolate's
                // Game/Inventory/Skills/Bank/Banking/EventSignal read what
                // this observe saw (only these fields — no World clone).
                // The blob's `hold` mirrors onto `__rs2b0t_host.hold` so a
                // posted hold freezes the isolate without a probe poke.
                // Task 9c: the post is a DELTA — `tick` always, other
                // fields only when changed vs the slot's last post (a
                // 50+ isolate wall never resends unchanged tables). The
                // packed banks are re-posted when the NavWorld identity
                // changed (identity = the shared Arc's pointer), not when
                // the stand list merely rebuilds identical.
                let world_id = world.as_ref().map(|w| Arc::as_ptr(w) as usize);
                let force_banks = world_id.is_some_and(|id| slot.last_world_id() != Some(id));
                let bytes = with_script_snapshot_input(
                    tick,
                    here,
                    up,
                    inv,
                    snapshot,
                    obj_names,
                    world.as_deref(),
                    hold,
                    ours,
                    |input| slot.encode_snapshot_delta(input, force_banks),
                );
                slot.post_snapshot(bytes);
                slot.store_last_world_id(world_id);
                if hold {
                    // Isolate: tick for onPaint only (hold gate inside V8).
                    // Compiled: skip — keep tick/nav follow frozen.
                    if slot.load_active() {
                        slot.on_game_tick(&mut ScriptCtx {
                            driver,
                            tick,
                            here,
                            walk: None,
                            walk_with: None,
                            inv,
                            snapshot,
                            obj_names,
                        });
                        wrote = true;
                    }
                } else {
                    // One shared arm for both hooks: `walk_with` carries the
                    // script's options through to `find_with`; `walk` is the
                    // default-options adapter (rs2b0t `walk` semantics stay
                    // default-off for teleports and wilderness). Each closure
                    // owns its own clone of the arm.
                    let bank_rows: Vec<(i32, i32)> = snapshot
                        .map(|s| s.bank().iter().map(|it| (it.def.id, it.count)).collect())
                        .unwrap_or_default();
                    let arm = ScriptWalkArm {
                        here,
                        world: world.clone(),
                        navs: Arc::clone(navs),
                        name: name.to_string(),
                        state: state.clone(),
                        bank: bank_rows,
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
                                    allow_bank_fetch: o.allow_bank_fetch,
                                    ..FindOptions::default()
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
                        snapshot,
                        obj_names,
                    });
                    wrote = true;
                }
            }
            // Fold the isolate's forwarded shim interact requests (queued
            // by the tick's Bank/Banking calls) on every frame — they land
            // a frame after the tick that produced them, tick edge or not —
            // and dispatch them below.
            interact = slot.drain_interacts();
        }
    }
    // Dispatch the shim's interact requests through the slot's own Driver
    // (open/deposit/withdraw) and the shared walk arm (bank-stand walks
    // with default FindOptions, so wilderness/quest gates fail closed).
    // The guardian's hold drops them: the script's parked wait stays
    // frozen, and a later retry re-queues what still matters.
    if !hold && !interact.is_empty() {
        if let Some(snapshot) = snapshot {
            wrote |= dispatch_script_interact(
                driver,
                snapshot,
                obj_names,
                here,
                navs,
                world,
                state.clone(),
                name,
                interact,
            );
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

/// Dispatch one isolate's shim interact requests. Open/close/deposit/
/// withdraw run through [`api::interact::Interactions`] on the slot's
/// snapshot + Driver — a request whose target is missing (no loc at the
/// tile, no bank-side row with the resolved name, no bank open) fails
/// closed with no send; `walk` routes through the shared
/// [`ScriptWalkArm`] with default [`FindOptions`], so the traveller's
/// wilderness/quest gates decide. Returns whether the driver's out buffer
/// was written.
#[allow(clippy::too_many_arguments)]
fn dispatch_script_interact(
    driver: &mut dyn Driver,
    snapshot: &GameSnapshot,
    obj_names: Option<&api::obj_names::ObjNames>,
    here: Option<(i32, i32, i32)>,
    navs: &Arc<Mutex<HashMap<String, NavBot>>>,
    world: &Option<Arc<NavWorld>>,
    state: Option<WorldState>,
    name: &str,
    reqs: Vec<script::shim::InteractReq>,
) -> bool {
    use api::interact::{ActionSpec, OpTarget, SendResult};
    use script::shim::InteractReq;
    let mut wrote = false;
    let mut ix = api::interact::Interactions::new(snapshot, driver);
    let open_booth = |ix: &mut api::interact::Interactions<'_>, x: i32, z: i32, level: i32| {
        let loc = snapshot
            .locs()
            .iter()
            .find(|l| l.tile.x == x && l.tile.z == z && l.tile.level == level);
        if let Some(loc) = loc {
            if let Some(op) = action_slot(&loc.actions, "Use-quickly") {
                return matches!(
                    ix.interact(OpTarget::Loc(loc), ActionSpec::Operation(op)),
                    SendResult::Sent { .. }
                );
            }
        }
        false
    };
    for req in reqs {
        match req {
            InteractReq::OpenBooth { x, z, level } => {
                wrote |= open_booth(&mut ix, x, z, level);
            }
            InteractReq::OpenStand {
                x,
                z,
                level,
                kind,
                name,
                stand_op,
                ..
            } => {
                if kind == "booth" {
                    wrote |= open_booth(&mut ix, x, z, level);
                } else if kind == "npc" {
                    let npc = snapshot.npcs().iter().find(|n| {
                        name.as_deref().is_some_and(|wanted| {
                            n.name
                                .as_deref()
                                .is_some_and(|n| n.eq_ignore_ascii_case(wanted))
                        })
                    });
                    if let Some(npc) = npc {
                        if let Some(op) = stand_op {
                            wrote |= matches!(
                                ix.interact(OpTarget::Npc(npc), ActionSpec::Operation(op)),
                                SendResult::Sent { .. }
                            );
                        }
                    }
                }
            }
            InteractReq::Walk { x, z, level } => {
                let bank_rows: Vec<(i32, i32)> =
                    snapshot.bank().iter().map(|it| (it.def.id, it.count)).collect();
                let arm = ScriptWalkArm {
                    here,
                    world: world.clone(),
                    navs: Arc::clone(navs),
                    name: name.to_string(),
                    state: state.clone(),
                    bank: bank_rows,
                };
                wrote |= arm.route(x, z, level, FindOptions::default());
            }
            InteractReq::Deposit { name } => {
                let wanted = name.to_lowercase();
                for item in snapshot.bank_side() {
                    let resolved = obj_names.and_then(|n| n.name(item.def.id));
                    if resolved.is_some_and(|n| n.eq_ignore_ascii_case(&wanted)) {
                        if let Some(op) = all_slot(&item.actions) {
                            wrote |= matches!(
                                ix.interact(OpTarget::Item(item), ActionSpec::Operation(op)),
                                SendResult::Sent { .. }
                            );
                        }
                    }
                }
            }
            InteractReq::Withdraw { name, action } => {
                let wanted = name.to_lowercase();
                if let Some(item) = snapshot.bank().iter().find(|it| {
                    obj_names
                        .and_then(|n| n.name(it.def.id))
                        .is_some_and(|n| n.eq_ignore_ascii_case(&wanted))
                }) {
                    if let Some(op) = action_slot(&item.actions, &action) {
                        wrote |= matches!(
                            ix.interact(OpTarget::Item(item), ActionSpec::Operation(op)),
                            SendResult::Sent { .. }
                        );
                    }
                }
            }
            InteractReq::Held { name, action } => {
                // rs2b0t `Item.interact` / `Inventory.first`: one name → one
                // held row (same as Withdraw's `.find`). A name the table
                // does not know or an item that is no longer held fails
                // closed — nothing is sent.
                let wanted = name.to_lowercase();
                if let Some(item) = snapshot.inventory().iter().find(|it| {
                    obj_names
                        .and_then(|n| n.name(it.def.id))
                        .is_some_and(|n| n.eq_ignore_ascii_case(&wanted))
                }) {
                    let res =
                        ix.interact(OpTarget::Item(item), ActionSpec::Label(action.clone()));
                    if host::debug_enabled() {
                        let outcome = match &res {
                            SendResult::Sent { .. } => "sent".to_string(),
                            SendResult::Refused { reason, .. } => format!("refused {reason:?}"),
                        };
                        eprintln!(
                            "[shim-held] {name} {action} slot={} -> {outcome}",
                            item.slot
                        );
                    }
                    wrote |= matches!(res, SendResult::Sent { .. });
                }
            }
            InteractReq::Close => {
                wrote |= matches!(ix.close_modal(), SendResult::Sent { .. });
            }
        }
    }
    wrote
}

/// Action-label lookup matching rs2b0t's `norm` (lowercase, whitespace and
/// `-`/`_` separators gone): `Withdraw All`, `Withdraw-All` and
/// `Withdraw  All` all resolve to the same slot.
fn action_slot(actions: &[Option<String>], wanted: &str) -> Option<i32> {
    let wanted = norm_action(wanted);
    actions
        .iter()
        .position(|a| a.as_deref().map(norm_action).as_deref() == Some(wanted.as_str()))
        .map(|i| i as i32 + 1)
}

/// The bank-side op slot whose label contains "all" (Deposit All / the
/// deposit window's bulk op), 1-based.
fn all_slot(actions: &[Option<String>]) -> Option<i32> {
    actions
        .iter()
        .position(|a| a.as_deref().is_some_and(|s| norm_action(s).contains("all")))
        .map(|i| i as i32 + 1)
}

fn norm_action(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-' && *c != '_')
        .collect()
}

/// Per-slot nav latch key: the `(player gen, here)` pair the pump last
/// pump last stepped. The step is skipped until either half changes, so a
/// hop is sent once per server tick, not every 20 ms frame (panel
/// `tick_latch`).
type NavStepKey = (u64, Option<(i32, i32, i32)>);

/// Run the queued [`WireCmd`]s through `Interactions` on the slot's own
/// Driver. `hold` freezes WASD walks (the guardian's hold freezes the
/// follow too); chat sends still go out so the operator can unstick a
/// dialog the guardian is not talking through.
fn dispatch_wires(
    driver: &mut dyn Driver,
    snapshot: &GameSnapshot,
    cmds: Vec<WireCmd>,
    hold: bool,
) {
    let mut ix = api::interact::Interactions::new(snapshot, driver);
    for cmd in cmds {
        match cmd {
            WireCmd::Continue => {
                ix.continue_dialog();
            }
            WireCmd::Answer(option) => {
                ix.answer_choice(option);
            }
            WireCmd::Walk { x, z, level } => {
                if !hold {
                    ix.walk(WorldTile { x, z, level });
                }
            }
        }
    }
}

/// The FlatBuffer snapshot blob posted into a Load isolate each
/// PLAYER_INFO (schema: `crates/script/schema/isolate.fbs`): `tick, here,
/// ingame, inv, stats, booths, banks, bank, bank_side, bank_open,
/// bank_loaded, hold, ours` — the exact fields the shim
/// Game/Inventory/Skills/Bank/Banking/EventSignal read, and nothing else
/// (no World clone). `here` is the local player's tile `{x, z, level}`
/// (absent when the body decoded none); `inv` rows carry the obj's
/// resolved name (`None` when the shared table has none — a name a script
/// queries never matches); `stats` rows carry the snapshot's stat
/// index/name/xp; `booths` are the scene locs whose actions include
/// `Use-quickly` (a name/action a script interacts with never appears
/// otherwise); `banks` are the packed bank stands (`{name, x, z, level,
/// kind: booth|npc, op, choose}`) the shim walks to; `bank`/`bank_side`
/// are the open bank's withdraw/deposit rows with the obj's resolved
/// name (`None` when the table has none — a deposit/withdraw by that name
/// never matches); `hold`/`ours` are the guardian's published status that
/// `EventSignal.pending()` reads.
///
/// Posts are DELTAS: `tick` is always carried; every other field only
/// when it changed vs `last` (the per-slot last-post fingerprint, `None`
/// right after Start — the first post is then the full keyframe). A 50+
/// isolate wall never resends unchanged inv/bank/stats/booths/packed
/// banks. Packed `banks` are additionally re-posted when `force_banks`
/// (the `NavWorld` identity changed) even though the stand list is
/// byte-identical. Returns the blob and the fingerprint to store as the
/// new last-post baseline.
/// Tests encode through a one-shot builder; the live observe path uses
/// [`with_script_snapshot_input`] + the slot's [`script::isolate_fb::IsolateBuf`].
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
fn script_snapshot_fb(
    last: Option<&script::isolate_fb::SnapshotFingerprint>,
    force_banks: bool,
    tick: u64,
    here: Option<(i32, i32, i32)>,
    ingame: bool,
    inv: Option<&[(i32, i32)]>,
    snapshot: Option<&GameSnapshot>,
    obj_names: Option<&api::obj_names::ObjNames>,
    world: Option<&NavWorld>,
    hold: bool,
    ours: bool,
) -> (Vec<u8>, script::isolate_fb::SnapshotFingerprint) {
    with_script_snapshot_input(
        tick,
        here,
        ingame,
        inv,
        snapshot,
        obj_names,
        world,
        hold,
        ours,
        |input| script::isolate_fb::encode_snapshot_delta(last, input, force_banks),
    )
}

/// Build the observed snapshot input and hand it to `f`. The live observe
/// path encodes through the slot's reusable [`script::isolate_fb::IsolateBuf`];
/// tests encode through a one-shot builder via [`script_snapshot_fb`].
#[allow(clippy::too_many_arguments)]
fn with_script_snapshot_input<R>(
    tick: u64,
    here: Option<(i32, i32, i32)>,
    ingame: bool,
    inv: Option<&[(i32, i32)]>,
    snapshot: Option<&GameSnapshot>,
    obj_names: Option<&api::obj_names::ObjNames>,
    world: Option<&NavWorld>,
    hold: bool,
    ours: bool,
    f: impl FnOnce(&script::isolate_fb::SnapshotInput<'_>) -> R,
) -> R {
    use script::isolate_fb::{BankStandInput, SnapshotInput, StatInput, TileInput};

    let here = here.map(|(x, z, level)| TileInput { x, z, level });
    let inv = inv.map(|rows| {
        rows.iter()
            .map(|(id, count)| (obj_names.and_then(|names| names.name(*id)), *count))
            .collect::<Vec<_>>()
    });
    let stats = snapshot.map(|s| {
        s.stats()
            .iter()
            .map(|st| StatInput {
                index: st.index,
                name: &st.name,
                xp: st.xp,
            })
            .collect::<Vec<_>>()
    });
    // The scene bank booths: the openable locs (`Use-quickly` is the
    // bankbooth op the pack bakes from `scripts/interface_bank/configs/
    // bank_booth.loc`). Only the tile is posted — the shim never reads a
    // loc definition.
    let booths = snapshot.map(|s| {
        s.locs()
            .iter()
            .filter(|l| {
                l.actions.iter().any(|a| {
                    a.as_deref()
                        .is_some_and(|a| a.eq_ignore_ascii_case("Use-quickly"))
                })
            })
            .map(|l| TileInput {
                x: l.tile.x,
                z: l.tile.z,
                level: l.tile.level,
            })
            .collect::<Vec<_>>()
    });
    let banks = world.map(|w| {
        use nav::pack::BankAccess;
        w.banks()
            .iter()
            .map(|b| {
                let (kind, op, choose) = match &b.access {
                    BankAccess::Booth { op } => ("booth", *op, None),
                    BankAccess::Npc { op, choose, .. } => ("npc", *op, choose.as_deref()),
                };
                BankStandInput {
                    name: &b.name,
                    x: b.tile.x,
                    z: b.tile.z,
                    level: b.tile.level,
                    kind,
                    op,
                    choose,
                }
            })
            .collect::<Vec<_>>()
    });
    let bank_rows = |items: &[api::snapshot::ItemView]| {
        items
            .iter()
            .map(|it| (obj_names.and_then(|names| names.name(it.def.id)), it.count))
            .collect::<Vec<_>>()
    };
    let bank = snapshot.map(|s| bank_rows(s.bank()));
    let bank_side = snapshot.map(|s| bank_rows(s.bank_side()));
    let input = SnapshotInput {
        tick,
        here,
        ingame,
        inv: inv.as_deref().unwrap_or(&[]),
        // The inv tab's slot count (28 when bound, 0 while the side icons
        // stay tutorial-locked): the `reader.inventorySize()` gate a
        // script's onStart parks on.
        inv_size: snapshot.map_or(0, |s| s.inventory_size()),
        stats: stats.as_deref().unwrap_or(&[]),
        booths: booths.as_deref().unwrap_or(&[]),
        banks: banks.as_deref().unwrap_or(&[]),
        bank: bank.as_deref().unwrap_or(&[]),
        bank_side: bank_side.as_deref().unwrap_or(&[]),
        bank_open: snapshot.is_some_and(|s| s.bank_component_id() != -1),
        bank_loaded: snapshot.is_some_and(|s| s.bank_component_id() != -1 && !s.bank().is_empty()),
        hold,
        ours,
    };
    f(&input)
}
/// the per-observe inventory view (the observe re-checks the gate inside).
fn script_running(scripts: &Arc<Mutex<HashMap<String, SlotScript>>>, name: &str) -> bool {
    scripts
        .lock()
        .unwrap()
        .get(name)
        .is_some_and(|s| s.state() == script::RunState::Running)
}

/// `name`'s slot script's latest paint frame, `None` for a slot with no
/// script or a script that has not painted. Copied onto the status row
/// each observe so the TUI can show paint-as-chat without a probe
/// round-trip (the isolate forwards the frame after each tick).
fn script_paint_of(
    scripts: &Arc<Mutex<HashMap<String, SlotScript>>>,
    name: &str,
) -> Option<script::shim::ScriptPaint> {
    scripts.lock().unwrap().get(name).and_then(|s| s.paint())
}

/// Per-uid nav state: the whole-world traveller plus the route it is
/// following. `ctx.walk` stores the route (found off-pump over the shared
/// [`NavWorld`]); the slot pump polls [`Traveller::follow`] with a clone of
/// it one step per player-info tick. `route` being set is the "armed"
/// gate the walk hook and the busy flag read. A pending BankBudget
/// session freezes follow until its steps finish.
#[derive(Default)]
struct NavBot {
    traveller: Traveller,
    route: Option<Route>,
    bank_fetch: Option<PendingBankFetch>,
}

/// The shared script walk arm: both `ctx.walk` (default options) and
/// `ctx.walk_with` (explicit options) route through
/// [`ScriptWalkArm::route`]. Each observe clones the arm once per hook
/// (all fields are `Clone`), so the two `&mut` hooks never share a
/// mutable borrow.
#[derive(Clone)]
struct ScriptWalkArm {
    here: Option<(i32, i32, i32)>,
    world: Option<Arc<NavWorld>>,
    navs: Arc<Mutex<HashMap<String, NavBot>>>,
    name: String,
    /// The slot's gating facts at arm time (from its live snapshot);
    /// `None` when no player is decoded — the worker then routes with
    /// the fail-closed empty [`WorldState`].
    state: Option<WorldState>,
    /// Open bank rows (obj id, count) at arm time — empty when closed.
    bank: Vec<(i32, i32)>,
}

impl ScriptWalkArm {
    /// Queue one walk toward `(x, z, level)` with `opts`, routing off-pump
    /// on a short-lived worker (`find_with` over the shared [`NavWorld`]).
    /// When `allow_bank_fetch` is on and the strict find fails only on
    /// missing item/worn reqs, latches a [`PendingBankFetch`] session and
    /// the post-session route. Refuses synchronously only when there is
    /// no player tile, no nav world, or a route/session already queued;
    /// the worker stores the outcome on the uid's nav bot. Returns whether
    /// the worker was spawned — not whether a path exists.
    fn route(&self, x: i32, z: i32, level: i32, opts: FindOptions) -> bool {
        let Some((hx, hz, hl)) = self.here else {
            return false;
        };
        let Some(world) = self.world.as_ref() else {
            return false;
        };
        // One route/session in flight per uid: a script spamming walk
        // every tick must not spawn a worker each tick.
        if self.navs.lock().unwrap().get(&self.name).is_some_and(|b| {
            b.route.is_some() || b.bank_fetch.is_some()
        }) {
            return false;
        }
        let from = WorldTile {
            x: hx,
            z: hz,
            level: hl,
        };
        let to = WorldTile { x, z, level };
        // The uid's latched essence-mine session lets a script walk out
        // of the mine through the exit portal's return hop; a uid with no
        // latch keeps the mine sealed (fail-closed, exactly like the
        // packed graph). The bot's own latch wins over a caller-supplied
        // `opts.essence` (the script `FindOptions` carries none).
        let mut opts = opts;
        if let Some(ess) = self
            .navs
            .lock()
            .unwrap()
            .get(&self.name)
            .and_then(|b| b.traveller.essence())
        {
            opts.essence = Some(ess);
        }
        let world = Arc::clone(world);
        let navs = Arc::clone(&self.navs);
        let name = self.name.clone();
        let state = self.state.clone();
        let bank = self.bank.clone();
        // Routing is the expensive part: run `find_with` / BankBudget
        // planning off-pump on a short-lived worker. The worker is
        // detached and exits right after storing the outcome; it never
        // touches the scripts map (lock order stays scripts → navs).
        thread::Builder::new()
            .name(format!("nav-find-{name}"))
            .spawn(move || {
                let empty = WorldState::empty();
                let state = state.as_ref().unwrap_or(&empty);
                match route_or_bank_fetch(&world, from, to, opts, state, &bank) {
                    RouteOutcome::Routed(route) => {
                        let mut guard = navs.lock().unwrap();
                        let bot = guard.entry(name).or_default();
                        bot.bank_fetch = None;
                        bot.route = Some(route);
                    }
                    RouteOutcome::BankSession { pending, route } => {
                        let mut guard = navs.lock().unwrap();
                        let bot = guard.entry(name).or_default();
                        bot.bank_fetch = Some(pending);
                        bot.route = Some(route);
                    }
                    RouteOutcome::NoPath => {}
                }
            })
            .is_ok()
    }
}

/// One pump step of a uid's nav bot: advance a pending BankBudget session
/// first (Wear / deposit / withdraw), then poll the armed route through
/// [`Traveller::follow`] one step against `snapshot`. `here` is the
/// player's world tile when the body decoded one (else the bot stands
/// still). `world` is the shared nav world, `None` when no pack loaded —
/// its packed any-tile teleport list rides into the follow so a jewellery
/// rub hop answers the destination dialog's choice for its landing (the
/// same pass-through the panel and scenario follow make). Mirrors the
/// armed route's dest into the status row's `walk_*` fields (`-1` when
/// idle); any terminal outcome clears the route — arrival and stall alike
/// — so the status flips back to idle and a script may arm a fresh walk.
// Shared handles threaded like `script_observe`; the arg count is allowed.
#[allow(clippy::too_many_arguments)]
fn step_nav_bot<D: Driver>(
    driver: &mut D,
    name: &str,
    here: Option<(i32, i32, i32)>,
    snapshot: &GameSnapshot,
    navs: &Arc<Mutex<HashMap<String, NavBot>>>,
    statuses: &Arc<Mutex<Vec<SlotStatus>>>,
    world: Option<&NavWorld>,
    hold: bool,
) {
    // The random-event freeze: the follow is not stepped while the
    // guardian holds the slot, and the armed route stays latched so it
    // resumes when the hold lifts. BankBudget steps freeze the same way.
    if here.is_none() || hold {
        return;
    }
    {
        let mut all = navs.lock().unwrap();
        if let Some(bot) = all.get_mut(name) {
            if bot.bank_fetch.is_some() {
                step_bank_fetch_on_bot(driver, snapshot, bot, world, here);
                // Freeze follow for Open / Deposit / Withdraw / Wear /
                // Close. Walk with a stand sub-route armed falls through
                // to Traveller::follow — never final_route mid-session.
                if bank_fetch_freezes_follow(bot) {
                    let queued = bot.route.as_ref().map(|r| r.dest);
                    drop(all);
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
                    return;
                }
            }
        }
    }
    let mut options = TravelOptions {
        // Exact arrival: the armed dest must be stood on before the route
        // clears (the v1 traveller arrived the same way).
        close_enough: 0,
        teleports: world.map(|w| w.graph.teleports.as_slice()),
        edges: world.map(|w| w.graph.edges.as_slice()),
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
        let walking_stand = bot.bank_fetch.as_ref().is_some_and(|p| {
            matches!(
                p.steps.front(),
                Some(BankStep::Walk { x, z, level })
                    if route.dest.x == *x && route.dest.z == *z && route.dest.level == *level
            )
        });
        match bot
            .traveller
            .follow(driver, snapshot, route, &mut options)
        {
            Some(nav::traveller::TravelOutcome::Arrived { .. }) => {
                bot.route = None;
            }
            Some(_) => {
                // Stall / refuse / block / give-up during stand Walk → NoPath.
                bot.route = None;
                if walking_stand {
                    bot.bank_fetch = None;
                }
            }
            None => {}
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

/// Advance one BankBudget session step on a [`NavBot`]. Walk completes
/// when the player is already on the stand tile (or a sub-route is
/// armed for follow); Open is a no-op while the bank is already
/// open+loaded; DepositAll / Withdraw / Wear / Close dispatch through
/// [`api::interact::Interactions`]. Clears the pending session when
/// steps are exhausted, or on walk/open/withdraw failure (NoPath).
/// Returns whether the driver was written.
fn step_bank_fetch_on_bot<D: Driver>(
    driver: &mut D,
    snapshot: &GameSnapshot,
    bot: &mut NavBot,
    world: Option<&NavWorld>,
    here: Option<(i32, i32, i32)>,
) -> bool {
    let Some(pending) = bot.bank_fetch.as_mut() else {
        return false;
    };
    let Some(step) = pending.steps.front().cloned() else {
        bot.bank_fetch = None;
        return false;
    };
    let mut abort = false;
    let wrote = match step {
        BankStep::Walk { x, z, level } => {
            if here == Some((x, z, level)) {
                pending.steps.pop_front();
                // Restore the post-session route for follow / status.
                bot.route = Some(pending.final_route.clone());
                false
            } else if bot.route.as_ref().is_some_and(|r| {
                r.dest.x == x && r.dest.z == z && r.dest.level == level
            }) {
                // Stand sub-route armed; follow polls it outside this step.
                false
            } else if let Some(w) = world {
                let from = match here {
                    Some((hx, hz, hl)) => WorldTile {
                        x: hx,
                        z: hz,
                        level: hl,
                    },
                    None => return false,
                };
                let to = WorldTile { x, z, level };
                // Live snapshot facts (same fail-closed gates as execute),
                // not an empty WorldState that would refuse gated walks.
                let state = WorldState::from_snapshot(snapshot);
                let opts = FindOptions {
                    allow_bank_fetch: false,
                    ..pending.opts
                };
                match find_with(&w.collision, &w.graph, from, to, opts, &state) {
                    Ok(route) => {
                        bot.route = Some(route);
                        false
                    }
                    Err(_) => {
                        abort = true;
                        false
                    }
                }
            } else {
                abort = true;
                false
            }
        }
        BankStep::Open => {
            if snapshot.bank_component_id() != -1 && !snapshot.bank().is_empty() {
                pending.steps.pop_front();
                false
            } else {
                let wrote = open_bank_at_here(driver, snapshot, here, world);
                if wrote || (snapshot.bank_component_id() != -1) {
                    pending.steps.pop_front();
                }
                wrote
            }
        }
        BankStep::DepositAll => {
            let wrote = deposit_all_backpack(driver, snapshot);
            pending.steps.pop_front();
            wrote
        }
        BankStep::Withdraw { id, count } => {
            let wrote = withdraw_id(driver, snapshot, id, count);
            if wrote {
                pending.steps.pop_front();
            } else {
                abort = true;
            }
            wrote
        }
        BankStep::Wear { id } => {
            let mut ix = api::interact::Interactions::new(snapshot, driver);
            let wrote = matches!(ix.wear(id), api::interact::SendResult::Sent { .. });
            pending.steps.pop_front();
            wrote
        }
        BankStep::Close => {
            let mut ix = api::interact::Interactions::new(snapshot, driver);
            let wrote = matches!(ix.close_modal(), api::interact::SendResult::Sent { .. });
            pending.steps.pop_front();
            wrote
        }
    };
    if abort {
        bot.bank_fetch = None;
        bot.route = None;
        return wrote;
    }
    if bot
        .bank_fetch
        .as_ref()
        .is_some_and(|p| p.steps.is_empty())
    {
        bot.bank_fetch = None;
    }
    wrote
}

/// Whether a latched BankBudget session must freeze [`Traveller::follow`].
/// Walk with the stand sub-route armed does **not** freeze; Open /
/// Deposit / Withdraw / Wear / Close do. Mid-session `final_route` is
/// never followed.
fn bank_fetch_freezes_follow(bot: &NavBot) -> bool {
    let Some(pending) = bot.bank_fetch.as_ref() else {
        return false;
    };
    match pending.steps.front() {
        Some(BankStep::Walk { x, z, level }) => {
            // Freeze only until the stand sub-route is armed; once armed,
            // follow that route. If route still points at final_route,
            // stay frozen this tick (Walk arms next pump / this pump).
            !bot.route.as_ref().is_some_and(|r| {
                r.dest.x == *x && r.dest.z == *z && r.dest.level == *level
            })
        }
        Some(_) => true,
        None => false,
    }
}

/// Public WalkArm BankBudget pump (panel / TUI follow path). Same step
/// semantics as the script [`NavBot`] pump.
pub fn step_walk_arm_bank_fetch<D: Driver>(
    driver: &mut D,
    snapshot: &GameSnapshot,
    arm: &mut WalkArm,
    world: Option<&NavWorld>,
    here: Option<(i32, i32, i32)>,
) -> bool {
    // Reuse NavBot stepping by temporarily viewing the arm as the same
    // shape of pending session + route.
    let mut bot = NavBot {
        traveller: Traveller::default(),
        route: arm.route.clone(),
        bank_fetch: arm.bank_fetch.take(),
    };
    let wrote = step_bank_fetch_on_bot(driver, snapshot, &mut bot, world, here);
    arm.bank_fetch = bot.bank_fetch;
    // Walk-to-stand may have armed a temporary route on the bot; abort
    // clears both session and route.
    if arm.bank_fetch.is_none() && bot.route.is_none() {
        arm.route = None;
    } else if let Some(r) = bot.route {
        arm.route = Some(r);
    }
    wrote
}

/// Whether a WalkArm BankBudget session freezes follow this frame
/// (panel / TUI). Same rule as the script [`NavBot`] pump.
pub fn walk_arm_bank_fetch_freezes_follow(arm: &WalkArm) -> bool {
    bank_fetch_freezes_follow(&NavBot {
        traveller: Traveller::default(),
        route: arm.route.clone(),
        bank_fetch: arm.bank_fetch.clone(),
    })
}

fn open_bank_at_here<D: Driver>(
    driver: &mut D,
    snapshot: &GameSnapshot,
    here: Option<(i32, i32, i32)>,
    world: Option<&NavWorld>,
) -> bool {
    use api::interact::{ActionSpec, OpTarget, SendResult};
    let mut ix = api::interact::Interactions::new(snapshot, driver);
    // Prefer a packed booth stand's tile; else any Use-quickly loc.
    let target_tile = world.and_then(|w| {
        here.and_then(|(hx, hz, hl)| {
            w.banks()
                .iter()
                .min_by_key(|s| {
                    (
                        s.tile.level != hl,
                        (s.tile.x - hx).abs().max((s.tile.z - hz).abs()),
                    )
                })
                .map(|s| (s.tile.x, s.tile.z, s.tile.level))
        })
    });
    if let Some((x, z, level)) = target_tile {
        if let Some(loc) = snapshot
            .locs()
            .iter()
            .find(|l| l.tile.x == x && l.tile.z == z && l.tile.level == level)
        {
            if let Some(op) = action_slot(&loc.actions, "Use-quickly") {
                return matches!(
                    ix.interact(OpTarget::Loc(loc), ActionSpec::Operation(op)),
                    SendResult::Sent { .. }
                );
            }
        }
    }
    for loc in snapshot.locs() {
        if let Some(op) = action_slot(&loc.actions, "Use-quickly") {
            return matches!(
                ix.interact(OpTarget::Loc(loc), ActionSpec::Operation(op)),
                SendResult::Sent { .. }
            );
        }
    }
    false
}

fn deposit_all_backpack<D: Driver>(driver: &mut D, snapshot: &GameSnapshot) -> bool {
    use api::interact::{ActionSpec, OpTarget, SendResult};
    let mut ix = api::interact::Interactions::new(snapshot, driver);
    let mut wrote = false;
    for item in snapshot.bank_side() {
        if let Some(op) = all_slot(&item.actions) {
            wrote |= matches!(
                ix.interact(OpTarget::Item(item), ActionSpec::Operation(op)),
                SendResult::Sent { .. }
            );
        }
    }
    wrote
}

fn withdraw_id<D: Driver>(driver: &mut D, snapshot: &GameSnapshot, id: i32, count: i32) -> bool {
    use api::interact::{ActionSpec, OpTarget, SendResult};
    let mut ix = api::interact::Interactions::new(snapshot, driver);
    let Some(item) = snapshot.bank().iter().find(|it| it.def.id == id) else {
        return false;
    };
    let label = match count {
        1 => "Withdraw 1",
        5 => "Withdraw 5",
        10 => "Withdraw 10",
        _ => "Withdraw All",
    };
    if let Some(op) = action_slot(&item.actions, label)
        .or_else(|| action_slot(&item.actions, "Withdraw 1"))
        .or_else(|| action_slot(&item.actions, "Withdraw All"))
    {
        return matches!(
            ix.interact(OpTarget::Item(item), ActionSpec::Operation(op)),
            SendResult::Sent { .. }
        );
    }
    false
}

/// The fat Client's inventory `(obj_id, count)` slots, zipped from the
/// TYPE_INV iface's linked obj ids/numbers (the server's `UPDATE_INV_FULL`
/// fills them each frame). The iface stores `obj_id + 1` (0 = empty), so
/// the view carries the real 0-based ids scripts resolve `has_item`
/// against — the same convention as `api::snapshot`'s inv view.
/// Short-lived: rebuilt per observe while the slot script is Running;
/// `None` when the inv tab is not bound yet. Reads the **side-tab-3**
/// inventory (the same lookup the snapshot's inv view uses) — a bare
/// first-TYPE_INV scan grabs whatever inventory component sorts first
/// (a bank/trade widget), which stays empty while the backpack is full.
fn inventory_from_ifaces(client: &Client) -> Option<Vec<(i32, i32)>> {
    let inv = api::snapshot::tab_inv_component(client, 3).and_then(|id| client.if_(id as usize))?;
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
    /// Live guardian toggle (`ProfileSettings.random_events`). Mirrored
    /// from the vault on spawn and by panel/TUI settings writes so a
    /// toggle-off never acts/holds without a respawn.
    pub random_events: Arc<AtomicBool>,
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
            random_events: Arc::new(AtomicBool::new(true)),
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
    ifaces_mut_template: Arc<Vec<Option<Arc<IfTypeMut>>>>,
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
    /// Per-slot wire commands the TUI queued (chat Continue/Answer, WASD
    /// walk); each slot thread runs them through `Interactions` on its own
    /// Driver and flushes the socket.
    wires: Arc<Mutex<HashMap<String, VecDeque<WireCmd>>>>,
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
            ifaces_mut_template: Arc::new(ifaces_mut_template),
            queue: Arc::new(Mutex::new(LoginQueue::default())),
            per_frame: Arc::new(|_: &mut Client, _: &str, _hold: bool| {}),
            spawned: HashSet::new(),
            arms: HashMap::new(),
            profiles: HashMap::new(),
            focused: None,
            scripts: Arc::new(Mutex::new(HashMap::new())),
            cheats: Arc::new(Mutex::new(HashMap::new())),
            wires: Arc::new(Mutex::new(HashMap::new())),
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

    /// The shared nav world (collision + transport graph) the slots route
    /// with, cloned from the same `Arc` the picker maps. `None` when no
    /// pack loaded.
    pub fn world(&self) -> Option<Arc<NavWorld>> {
        self.world.clone()
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
    pub fn join(mut self) {
        for (_, handle) in std::mem::take(&mut self.handles) {
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

    /// Queue a [`WireCmd`] (chat Continue/Answer or a WASD one-tile walk)
    /// for `user`'s slot: its own thread runs it through `Interactions`
    /// on the slot's Driver and flushes. No-op when the user is not a
    /// running slot.
    pub fn queue_wire(&self, user: &str, cmd: WireCmd) {
        if let Some(q) = self.wires.lock().unwrap().get_mut(user) {
            q.push_back(cmd);
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
        self.wires.lock().unwrap().remove(name);
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
        arm.random_events
            .store(profile.settings.random_events, Ordering::Relaxed);
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
            Arc::clone(&self.wires),
            Arc::clone(&self.navs),
            self.world.clone(),
            Arc::clone(&self.obj_names),
            Arc::clone(&self.per_frame),
            &mut self.handles,
        );
    }
}

/// Stop every slot thread and join it before the play goes away, so no
/// observe hook (the panel's per-frame paint reads `picker::pack`) can run
/// after the shared nav world is detached.
impl Drop for Play {
    fn drop(&mut self) {
        let names: Vec<String> = self.handles.keys().cloned().collect();
        for name in names {
            self.stop_slot(&name);
        }
    }
}

/// Spawn one slot thread per profile. Each slot waits for a login-queue
/// permit, sends the handshake, then drives `mainloop` at the host cadence
/// while mirroring its state into the shared status list. Slots run with no
/// input and no frame mailbox; [`run_with_io`] adds per-slot channels.
pub fn run(options: &PlayOptions, profiles: Vec<Profile>) -> Play {
    run_with_io(options, profiles, |_| (None, None), |_, _, _| {})
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
    G: Fn(&mut Client, &str, bool) + Send + Sync + 'static,
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
    ifaces_mut_template: Arc<Vec<Option<Arc<IfTypeMut>>>>,
    slot_queue: Arc<Mutex<LoginQueue>>,
    slot_statuses: Arc<Mutex<Vec<SlotStatus>>>,
    slot_scripts: Arc<Mutex<HashMap<String, SlotScript>>>,
    slot_cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>>,
    slot_wires: Arc<Mutex<HashMap<String, VecDeque<WireCmd>>>>,
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
            slot_wires
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
                let knock_name = username.clone();
                let knock_scripts = Arc::clone(&slot_scripts);
                let knock = move |ev: &DetectedRandom| -> RandomClaim {
                    let mut all = knock_scripts.lock().unwrap();
                    let Some(slot) = all.get_mut(&knock_name) else {
                        return RandomClaim::Host;
                    };
                    // Task 12: the bot instance's ignore list. An ignored
                    // name is a host-declined claim — no flee / Talk-to /
                    // etc. — while detect still publishes the event. The
                    // eval rides the rising edge (once per event), never
                    // the frame path. The claim knock itself stays Host
                    // for JS isolates: no Handle comes from V8.
                    if slot
                        .ignored_randoms()
                        .iter()
                        .any(|n| n.eq_ignore_ascii_case(&ev.name))
                    {
                        return RandomClaim::Handle;
                    }
                    slot.on_random(ev)
                };
                Host::run_client(
                    &mut client,
                    &username,
                    profile.settings.clone(),
                    Arc::clone(&arm_obs.random_events),
                    slot_input.clone(),
                    slot_mailbox.clone(),
                    park.clone(),
                    {
                        let slot_frame = Arc::clone(&slot_frame);
                        let slot_statuses = Arc::clone(&slot_statuses);
                        let slot_scripts = Arc::clone(&slot_scripts);
                        let slot_cheats = Arc::clone(&slot_cheats);
                        let slot_wires = Arc::clone(&slot_wires);
                        let slot_obj_names = Arc::clone(&slot_obj_names);
                        let slot_navs = Arc::clone(&slot_navs);
                        let slot_world = slot_world.clone();
                        let mut pump = Pump::new();
                        let mut script_tick: u64 = 0;
                        // Last `(player gen, here)` the nav bot stepped:
                        // skip until either changes so the hop budget counts
                        // server ticks, not 20 ms frames (panel `tick_latch`).
                        let mut last_nav_step: Option<NavStepKey> = None;
                        // The slot's per-tick nav snapshot: rebuilt each
                        // observe when a family's gen moved (the walk arm
                        // gates on its facts; the follow surface reads the
                        // canonical base + route-head tile from it).
                        let mut nav_snapshot = GameSnapshot::new();
                        // The random status `client_frame` published last
                        // frame: copied onto the slot status row, and its
                        // hold freezes script tick and the nav follow.
                        move |c, _ignored, run_sends, status: &RandomStatus| {
                            let name = &obs_name;
                            // Panel/TUI WalkArm + scenario follow gate on the
                            // same hold as step_nav_bot (prev-frame status).
                            slot_frame(c, name, status.hold);
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
                            // The slot's paint frame is read before the
                            // status lock (scripts -> statuses is the only
                            // order the two mutexes may nest).
                            let paint = script_paint_of(&slot_scripts, name);
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
                                        s.random = status.clone();
                                        s.script_paint = paint.clone();
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
                            // The slot's per-tick nav snapshot, rebuilt when
                            // a family's gen moved (the incremental rebuild
                            // is cheap when nothing changed). The walk arm
                            // gates its routes on the same facts the
                            // snapshot proves — a slot with coins on the
                            // player can cross a toll, a player without
                            // them cannot.
                            nav_snapshot.rebuild(c);
                            let nav_state = here
                                .is_some()
                                .then(|| WorldState::from_snapshot(&nav_snapshot));
                            script_observe(
                                c,
                                name,
                                up,
                                tick_edge,
                                script_tick,
                                here,
                                inv.as_deref(),
                                nav_state,
                                Some(&nav_snapshot),
                                Some(slot_obj_names.as_ref()),
                                &slot_scripts,
                                &slot_cheats,
                                &slot_navs,
                                &slot_world,
                                status.hold,
                                status.ours,
                            );
                            // TUI chat / WASD sends: run the queued wire
                            // commands through `Interactions` on this
                            // slot's own Client, so Continue/Answer/Walk
                            // respect the same preconditions as the
                            // guardian and the scenario runner. The slot
                            // must not be frozen by the guardian's hold
                            // when it presses a dialog the guardian is
                            // talking through, but a walk while held is
                            // dropped (the hold freezes the follow too).
                            let wires = {
                                let mut all = slot_wires.lock().unwrap();
                                all.get_mut(name)
                                    .map(std::mem::take)
                                    .unwrap_or_default()
                            };
                            if !wires.is_empty() {
                                dispatch_wires(c, &nav_snapshot, wires.into(), status.hold);
                            }
                            // Per-uid nav step on the pump, gated on the
                            // player-gen/tile latch like the panel's WalkTo
                            // hook so a hop is sent once per server tick,
                            // not re-sent every 20 ms frame. The snapshot
                            // was already rebuilt above. The guardian's
                            // hold freezes the follow — the armed route
                            // stays latched and resumes when it lifts.
                            let nav_key = (c.gens.player, here);
                            if last_nav_step != Some(nav_key) {
                                last_nav_step = Some(nav_key);
                                if here.is_some()
                                    && !status.hold
                                    && slot_navs.lock().unwrap().get(name).is_some_and(|b| {
                                        b.route.is_some() || b.bank_fetch.is_some()
                                    })
                                {
                                    step_nav_bot(
                                        c,
                                        name,
                                        here,
                                        &nav_snapshot,
                                        &slot_navs,
                                        &slot_statuses,
                                        slot_world.as_deref(),
                                        status.hold,
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
                                || slot_wires
                                    .lock()
                                    .unwrap()
                                    .get(name)
                                    .is_some_and(|q| !q.is_empty())
                                || slot_navs.lock().unwrap().get(name).is_some_and(|b| {
                                    b.route.is_some() || b.bank_fetch.is_some()
                                })
                        }
                    },
                    {
                        let ifaces_template = ifaces_template.clone();
                        move |c| tick_flags(c, &ifaces_template, &arm_obs) || !c.ingame
                    },
                    knock,
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
type IfaceTables = (Cache, Vec<Option<Box<IfType>>>, Vec<Option<Arc<IfTypeMut>>>);
fn load_template(cache_dir: &str) -> IfaceTables {
    let cache = match std::fs::read(format!("{cache_dir}/config")) {
        Ok(bytes) => {
            std::panic::catch_unwind(AssertUnwindSafe(|| Cache::unpack(&JagFile::new(bytes))))
                .unwrap_or_default()
        }
        Err(_) => Cache::default(),
    };
    let (ifaces, ifaces_mut) = match std::fs::read(format!("{cache_dir}/interface")) {
        Ok(bytes) => std::panic::catch_unwind(AssertUnwindSafe(|| {
            let (ifaces, ifaces_mut) = IfType::unpack(&JagFile::new(bytes));
            (
                ifaces,
                ifaces_mut
                    .into_iter()
                    .map(|o| o.map(|b| Arc::new(*b)))
                    .collect(),
            )
        }))
        .unwrap_or_default(),
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
    use client::config::if_type::ComponentType;
    use host::Guardian;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::Arc;
    use std::thread;

    use client::client::ClientConfig;
    use client::client::MiniMenuAction;
    use client::config::if_type::ButtonType;
    use client::config::Cache;
    use client::io::ServerProt;
    use nav::collision::WorldCollision;
    use nav::router::Leg;
    use nav::tile::Tile;
    use nav::transport::{TransportEdge, TransportGraph, TransportKind};
    use vault::ProfileSettings;

    #[test]
    fn world_host_live_is_rs2b2t_everything_else_is_loopback() {
        assert_eq!(world_host_for_bot_target(Some("live")), "w1.rs2b2t.com");
        assert_eq!(world_host_for_bot_target(Some("prod")), "w1.rs2b2t.com");
        assert_eq!(world_host_for_bot_target(Some("local")), "127.0.0.1");
        assert_eq!(world_host_for_bot_target(None), "127.0.0.1");
    }

    #[test]
    fn mint_live_names_are_unique_and_never_test() {
        let a = mint_live_names(2);
        let b = mint_live_names(2);
        assert_eq!(a.len(), 2);
        assert_eq!(b.len(), 2);
        let mut all: Vec<&String> = a.iter().chain(b.iter()).collect();
        all.sort();
        all.dedup();
        assert_eq!(
            all.len(),
            4,
            "every minted name must be unique per invocation: {all:?}"
        );
        for name in all {
            assert_ne!(name, "test", "a live boot must never log in `test`");
            assert!(name.starts_with("live"), "minted name: {name}");
            assert!(
                name.len() <= 12,
                "the engine enforces the 12-char username limit: {name}"
            );
        }
    }

    #[test]
    fn mint_live_names_keep_the_12_char_limit_at_scale() {
        for n in [1, 2, 10, 50] {
            for name in mint_live_names(n) {
                assert!(
                    name.len() <= 12,
                    "{name} (n={n}) exceeds the 12-char username limit"
                );
            }
        }
    }

    #[test]
    fn mint_live_names_zero_is_empty() {
        assert!(mint_live_names(0).is_empty());
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
                random_events: true,
                lamp_skill: "strength".into(),
                lamp_auto: true,
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
                random_events: true,
                lamp_skill: "strength".into(),
                lamp_auto: true,
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
            |_, _, _| {},
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
            |_, _, _| {},
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
            |_, _, _| {},
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
            |_, _, _| {},
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
            |_, _, _| {},
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
        let mut client = prepare_client(
            cfg,
            1,
            Arc::new(Cache::default()),
            Arc::new(ifaces.clone()),
            Vec::new(),
        );
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
            |_, _, _| {},
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
            |_, _, _| {},
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
            |_, _, _| {},
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
            |_, _, _| {},
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

    fn open_world(w: usize, h: usize) -> NavWorld {
        NavWorld::from_parts(
            WorldCollision {
                origin: WorldTile {
                    x: 0,
                    z: 0,
                    level: 0,
                },
                width: w,
                height: h,
                walk: vec![0u8; w * h],
                blocked: vec![0u64; (w * h).div_ceil(64)],
                flags: None,
            },
            TransportGraph::default(),
            Vec::new(),
        )
    }

    #[test]
    fn walk_arm_may_follow_freezes_under_hold() {
        assert!(WalkArm::may_follow(false), "unheld follow may poll");
        assert!(!WalkArm::may_follow(true), "hold freezes WalkArm follow");
    }

    /// The shared walk arm (panel `Session::arm_walk_on` is a thin
    /// wrapper over this; the TUI calls it directly) routes over the
    /// world and latches the route on the focused uid's arm.
    #[test]
    fn arm_walk_on_routes_and_latches_the_focused_arm() {
        let world = open_world(3, 3);
        let travellers: Arc<Mutex<HashMap<String, Arc<Mutex<WalkArm>>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let dest = Tile {
            x: 2,
            z: 2,
            level: 0,
        };
        let route = arm_walk_on(
            &world,
            Tile {
                x: 0,
                z: 0,
                level: 0,
            },
            dest,
            FindOptions::default(),
            &WorldState::empty(),
            &[],
            &travellers,
            Some("alice"),
        )
        .expect("the open 3x3 world routes");
        assert_eq!((route.dest.x, route.dest.z, route.dest.level), (2, 2, 0));
        let all = travellers.lock().unwrap();
        let arm = all
            .get("alice")
            .expect("the focused slot's walk arm exists");
        assert_eq!(
            arm.lock().unwrap().queued_tile(),
            Some(dest),
            "the arm latches the routed dest"
        );
    }

    #[test]
    fn arm_walk_on_without_focus_latches_no_arm() {
        let world = open_world(3, 3);
        let travellers: Arc<Mutex<HashMap<String, Arc<Mutex<WalkArm>>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        arm_walk_on(
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
            FindOptions::default(),
            &WorldState::empty(),
            &[],
            &travellers,
            None,
        )
        .expect("the open 3x3 world routes without a focused slot");
        assert!(
            travellers.lock().unwrap().is_empty(),
            "no focused name to key a walk arm"
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
                random_events: true,
                lamp_skill: "strength".into(),
                lamp_auto: true,
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
            |_, _, _| {},
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
            |_, _, _| {},
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
            |_, _, _| {},
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
            |_, _, _| {},
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
            |_, _, _| {},
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
            |_, _, _| {},
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
        play.queue_wire("ghost", WireCmd::Continue);
        play.queue_wire(
            "ghost",
            WireCmd::Walk {
                x: 3220,
                z: 3221,
                level: 0,
            },
        );
        assert!(
            play.wires.lock().unwrap().is_empty(),
            "unknown uid wire is a no-op"
        );
    }

    #[test]
    fn queue_wire_lands_on_the_named_slots_queue() {
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
            |_, _, _| {},
        );
        // `auto_login = false` arm sits on the title (no TCP).
        play.spawn_slot(profile("a", 1), None, None, Some(SlotArm::new(1, false)));
        assert!(
            wait_until(500, || play.wires.lock().unwrap().contains_key("a")),
            "the slot thread registers its wire queue at spawn"
        );
        play.queue_wire("a", WireCmd::Continue);
        play.queue_wire("a", WireCmd::Answer(2));
        let queued = play.wires.lock().unwrap().get("a").unwrap().clone();
        assert_eq!(
            queued,
            VecDeque::from([WireCmd::Continue, WireCmd::Answer(2)]),
            "queued wires keep their order"
        );
        play.stop_slot("a");
        assert!(
            !play.wires.lock().unwrap().contains_key("a"),
            "stop_slot drops the slot's wire queue"
        );
    }

    /// `dispatch_wires` is safe on a snapshot with nothing open: the
    /// Continue/Answer sends refuse (no chat modal) and the driver's out
    /// buffer stays untouched.
    #[test]
    fn dispatch_wires_with_no_modal_stays_quiet() {
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
        let snap = GameSnapshot::new();
        dispatch_wires(
            &mut c,
            &snap,
            vec![WireCmd::Continue, WireCmd::Answer(1)],
            false,
        );
        assert_eq!(c.out.pos, 0, "no chat modal → nothing dispatched");
    }

    /// A guardian hold drops WASD walks (the follow is frozen too) but
    /// still lets chat Continue/Answer through.
    #[test]
    fn dispatch_wires_drops_walk_while_hold_but_keeps_chat() {
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
        let snap = GameSnapshot::new();
        dispatch_wires(
            &mut c,
            &snap,
            vec![WireCmd::Walk {
                x: 3220,
                z: 3221,
                level: 0,
            }],
            true,
        );
        assert_eq!(c.out.pos, 0, "hold drops the walk send");
    }

    /// A client seeded for the shim interact dispatch: ingame with a ready
    /// scene at base (3200, 3200), the player at scene (5, 5), a Bank
    /// booth loc at scene (5, 6) whose def's second op is `Use-quickly`,
    /// an open bank (main modal 600 wrapping withdraw component 601
    /// holding Bones × 20 with a `Withdraw 1` op) and its deposit side
    /// modal (700 wrapping 701 holding Bones × 3 with a `Deposit All` op).
    fn bank_client() -> Client {
        use client::client::ClientPlayer;
        use client::config::{LocType, ObjType};
        // A real (loopback) stream so the snapshot passes `Interactions`'
        // attached precondition, like the nav-client fixture.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let stream =
            client::io::ClientStream::connect(&addr.ip().to_string(), addr.port()).unwrap();
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
        c.map_build_base_x = 3200;
        c.map_build_base_z = 3200;
        c.minusedlevel = 0;
        c.local_player = Some(ClientPlayer::at(5, 5));
        {
            let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
            cache.objs.resize(3, ObjType::default());
            cache.objs[1].id = 1;
            cache.objs[1].name = "Bones".into();
            cache.objs[2].id = 2;
            cache.objs[2].name = "Lobster".into();
            cache.locs.extend(
                (0..(2214usize.saturating_sub(cache.locs.len()))).map(|_| LocType::default()),
            );
            cache.locs[2213].id = 2213;
            cache.locs[2213].name = "Bank booth".into();
            cache.locs[2213].op = vec![None, Some("Use-quickly".into()), None, None, None];
        }
        // The booth: a wall at scene (5, 6) whose typecode encodes loc 2213.
        let booth_typecode = 0x4000_0000 + (2213 << 14) + 1 + (2 << 7);
        c.world
            .set_wall(0, 5, 6, 0, 0, 0, booth_typecode, 0, 0, 0, 0, 0);
        // Bank: main modal 600 wrapping the withdraw component 601
        // (Bones × 20, ops `[Withdraw 1, ...]`).
        c.main_modal_id = 600;
        c.set_iface(
            600,
            IfType {
                id: 600,
                layer_id: 600,
                r#type: ComponentType::TYPE_LAYER,
                children: Some(vec![601]),
                ..Default::default()
            },
        );
        c.set_iface(
            601,
            IfType {
                id: 601,
                layer_id: 600,
                r#type: ComponentType::TYPE_INV,
                iop: [
                    Some("Withdraw 1".into()),
                    Some("Withdraw 5".into()),
                    Some("Withdraw 10".into()),
                    Some("Withdraw All".into()),
                    None,
                ],
                ..Default::default()
            },
        );
        c.set_iface_mut(
            601,
            IfTypeMut {
                link_obj_type: Some(vec![2, 0]),
                link_obj_number: Some(vec![20, 0]),
                ..Default::default()
            },
        );
        // Deposit side modal: 700 wrapping 701 (Bones × 3, `Deposit All`).
        c.side_modal_id = 700;
        c.set_iface(
            700,
            IfType {
                id: 700,
                layer_id: 700,
                r#type: ComponentType::TYPE_LAYER,
                children: Some(vec![701]),
                ..Default::default()
            },
        );
        c.set_iface(
            701,
            IfType {
                id: 701,
                layer_id: 700,
                r#type: ComponentType::TYPE_INV,
                iop: [Some("Deposit All".into()), None, None, None, None],
                ..Default::default()
            },
        );
        c.set_iface_mut(
            701,
            IfTypeMut {
                link_obj_type: Some(vec![2, 0]),
                link_obj_number: Some(vec![3, 0]),
                ..Default::default()
            },
        );
        for prot in [
            ServerProt::IF_OPENMAIN,
            ServerProt::IF_OPENCHAT,
            ServerProt::UPDATE_INV_FULL,
            ServerProt::REBUILD_NORMAL,
            ServerProt::PLAYER_INFO,
        ] {
            c.bump_gens(prot);
        }
        c
    }

    /// Task 7 — the shim's interact requests dispatch through the slot
    /// Driver: the booth Use-quickly op at the loc tile, the bank-side
    /// Deposit-All for a matching name, the bank withdraw op, and close.
    /// A request whose target the snapshot lacks fails closed (nothing is
    /// sent).
    #[test]
    fn dispatch_script_interact_sends_open_deposit_withdraw() {
        let mut c = bank_client();
        let mut snap = GameSnapshot::new();
        snap.rebuild(&c);
        let names = api::obj_names::ObjNames::from_objs(&{
            let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
            cache.objs.clone()
        });
        let (navs, world) = empty_nav();
        assert_eq!(
            snap.locs()
                .iter()
                .find(|l| l.tile.x == 3205 && l.tile.z == 3206)
                .map(|l| l.actions.clone()),
            Some(vec![None, Some("Use-quickly".into()), None, None, None]),
            "the seeded booth loc carries the Use-quickly op"
        );
        let out_before = c.out.pos;
        // Each op must send on its own, not be masked by the others:
        // open-booth (Use-quickly), bank-side Deposit-All, a withdraw by
        // name whose hyphenated label resolves to the seeded "Withdraw All"
        // op, and close. `dispatch_script_interact` returns whether the
        // driver's out buffer was written.
        for (label, req) in [
            (
                "open-booth",
                script::shim::InteractReq::OpenBooth {
                    x: 3205,
                    z: 3206,
                    level: 0,
                },
            ),
            (
                "deposit",
                script::shim::InteractReq::Deposit {
                    name: "Bones".into(),
                },
            ),
            (
                "withdraw",
                script::shim::InteractReq::Withdraw {
                    name: "Bones".into(),
                    action: "Withdraw-All".into(),
                },
            ),
            ("close", script::shim::InteractReq::Close),
        ] {
            let before = c.out.pos;
            assert!(
                dispatch_script_interact(
                    &mut c,
                    &snap,
                    Some(&names),
                    Some((3205, 3205, 0)),
                    &navs,
                    &world,
                    None,
                    "alice",
                    vec![req],
                ),
                "{label} must dispatch"
            );
            assert!(
                c.out.pos > before,
                "{label} must write the driver (masked by the other ops?)"
            );
        }
        assert!(
            c.out.pos > out_before,
            "open + deposit + withdraw + close wrote to the driver"
        );
        // A request with no matching target sends nothing.
        let out_before = c.out.pos;
        assert!(!dispatch_script_interact(
            &mut c,
            &snap,
            Some(&names),
            Some((3205, 3205, 0)),
            &navs,
            &world,
            None,
            "alice",
            vec![
                script::shim::InteractReq::OpenBooth {
                    x: 3299,
                    z: 3299,
                    level: 0
                },
                script::shim::InteractReq::Deposit {
                    name: "Lobster".into()
                },
                script::shim::InteractReq::Withdraw {
                    name: "Lobster".into(),
                    action: "Withdraw 1".into()
                },
            ],
        ));
        assert_eq!(
            c.out.pos, out_before,
            "missing targets fail closed: nothing is sent"
        );
    }

    /// The shim `Item.interact` arm: a `Held { name, action }` request
    /// resolves the held item through ObjNames and dispatches its menu op
    /// by label (Bones → Bury). The `bank_fetch_client` inv tab holds
    /// Bones (obj 1) × 3.
    #[test]
    fn dispatch_script_interact_sends_held_item_bury() {
        let mut c = bank_fetch_client();
        // Bones (obj 1) has the `Bury` held op (`[opheld1,_bones]`).
        {
            let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
            cache.objs[1].iop = [Some("Bury".into()), None, None, None, None];
        }
        let mut snap = GameSnapshot::new();
        snap.rebuild(&c);
        let names = api::obj_names::ObjNames::from_objs(&{
            let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
            cache.objs.clone()
        });
        let (navs, world) = empty_nav();
        assert_eq!(
            snap.inventory()
                .iter()
                .find(|it| it.def.id == 1)
                .map(|it| (it.def.id, it.count)),
            Some((1, 3)),
            "the inv tab holds Bones"
        );
        let before = c.out.pos;
        assert!(
            dispatch_script_interact(
                &mut c,
                &snap,
                Some(&names),
                Some((3205, 3205, 0)),
                &navs,
                &world,
                None,
                "alice",
                vec![script::shim::InteractReq::Held {
                    name: "Bones".into(),
                    action: "Bury".into()
                }],
            ),
            "held Bury must dispatch"
        );
        assert!(c.out.pos > before, "held Bury must write the driver");
        // An action the item has no op for sends nothing (Bones has no
        // "Wear"), and an unknown name matches no held item.
        let before = c.out.pos;
        assert!(!dispatch_script_interact(
            &mut c,
            &snap,
            Some(&names),
            Some((3205, 3205, 0)),
            &navs,
            &world,
            None,
            "alice",
            vec![
                script::shim::InteractReq::Held {
                    name: "Bones".into(),
                    action: "Wear".into()
                },
                script::shim::InteractReq::Held {
                    name: "Lobster".into(),
                    action: "Bury".into()
                },
            ],
        ));
        assert_eq!(
            c.out.pos, before,
            "a label no held op resolves and an unknown name send nothing"
        );
    }

    /// `Inventory.first` / one `{op:'held',...}` queue entry target a single
    /// inv row. Two Bones slots must still write one bury op, not one per
    /// name match (Withdraw already `.find`s; Held must match).
    #[test]
    fn dispatch_script_interact_held_first_match_only() {
        let mut one = bank_fetch_client();
        {
            let cache = Arc::get_mut(&mut one.cache).expect("sole cache owner");
            cache.objs[1].iop = [Some("Bury".into()), None, None, None, None];
        }
        let mut snap_one = GameSnapshot::new();
        snap_one.rebuild(&one);
        let names = api::obj_names::ObjNames::from_objs(&{
            let cache = Arc::get_mut(&mut one.cache).expect("sole cache owner");
            cache.objs.clone()
        });
        let (navs, world) = empty_nav();
        let before_one = one.out.pos;
        assert!(dispatch_script_interact(
            &mut one,
            &snap_one,
            Some(&names),
            Some((3205, 3205, 0)),
            &navs,
            &world,
            None,
            "alice",
            vec![script::shim::InteractReq::Held {
                name: "Bones".into(),
                action: "Bury".into()
            }],
        ));
        let one_op = one.out.pos - before_one;
        assert!(one_op > 0, "control bury must write");

        let mut two = bank_fetch_client();
        {
            let cache = Arc::get_mut(&mut two.cache).expect("sole cache owner");
            cache.objs[1].iop = [Some("Bury".into()), None, None, None, None];
        }
        // Two separate Bones rows (stored 2 = obj 1), matching live multi-slot
        // inv the shim's Inventory.first would pick from once.
        two.set_iface_mut(
            500,
            IfTypeMut {
                link_obj_type: Some(vec![2, 2]),
                link_obj_number: Some(vec![1, 1]),
                ..Default::default()
            },
        );
        let mut snap_two = GameSnapshot::new();
        snap_two.rebuild(&two);
        assert_eq!(
            snap_two
                .inventory()
                .iter()
                .filter(|it| it.def.id == 1)
                .count(),
            2,
            "fixture must expose two Bones slots"
        );
        let before_two = two.out.pos;
        assert!(dispatch_script_interact(
            &mut two,
            &snap_two,
            Some(&names),
            Some((3205, 3205, 0)),
            &navs,
            &world,
            None,
            "alice",
            vec![script::shim::InteractReq::Held {
                name: "Bones".into(),
                action: "Bury".into()
            }],
        ));
        assert_eq!(
            two.out.pos - before_two,
            one_op,
            "one Held request must bury Inventory.first only, not every Bones slot"
        );
    }

    /// The BankBudget session fixture: [`bank_client`] plus a junk
    /// inventory (Bones × 3 on side tab 3), the bank's obj 2 renamed
    /// "Knife", and the bank's withdraw component holding the knife
    /// (stored 3 = obj 2) — the worn-req item the session must fetch.
    fn bank_fetch_client() -> Client {
        let mut c = bank_client();
        {
            let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
            cache.objs[2].name = "Knife".into();
        }
        c.side_icon[3] = 500;
        c.set_iface(
            500,
            IfType {
                id: 500,
                r#type: ComponentType::TYPE_INV,
                obj_ops: true,
                ..Default::default()
            },
        );
        c.set_iface_mut(
            500,
            IfTypeMut {
                link_obj_type: Some(vec![2, 0]), // stored 2 = obj 1 (Bones)
                link_obj_number: Some(vec![3, 0]),
                ..Default::default()
            },
        );
        // The bank's withdraw component (601) holds the knife, not Bones.
        c.set_iface_mut(
            601,
            IfTypeMut {
                link_obj_type: Some(vec![3, 0]), // stored 3 = obj 2 (Knife)
                link_obj_number: Some(vec![20, 0]),
                ..Default::default()
            },
        );
        c.bump_gens(ServerProt::IF_OPENMAIN);
        c.bump_gens(ServerProt::UPDATE_INV_FULL);
        c
    }

    /// A 5×5 world walled between x=1 and x=2, crossed only by a door
    /// gated on wearing a knife (obj `knife_id`), with a bank booth
    /// stand at (0, 4).
    fn knife_nav_world(knife_id: i32) -> NavWorld {
        let mut flags = vec![0u32; 25];
        for z in 0..5 {
            flags[z * 5 + 1] |= client::dash3d::CollisionFlag::W_E as u32;
            flags[z * 5 + 2] |= client::dash3d::CollisionFlag::W_W as u32;
        }
        let edge = TransportEdge {
            kind: TransportKind::Door,
            at: WorldTile {
                x: 1,
                z: 2,
                level: 0,
            },
            to: WorldTile {
                x: 2,
                z: 2,
                level: 0,
            },
            loc_id: 2882,
            option: 1,
            ticks: 2,
            dir: None,
            open_loc_id: None,
            skill_req: vec![],
            item_req: vec![],
            quest_req: vec![],
            varp_req: vec![],
            worn_req: vec![knife_id],
        };
        let mut graph = TransportGraph::default();
        graph.at.entry(edge.at).or_default().push(0);
        graph.edges.push(edge);
        let (walk, blocked) = nav::collision::pack_walk(&flags);
        NavWorld::from_parts(
            nav::collision::WorldCollision {
                origin: WorldTile {
                    x: 0,
                    z: 0,
                    level: 0,
                },
                width: 5,
                height: 5,
                walk,
                blocked,
                flags: None,
            },
            graph,
            vec![nav::pack::BankStand {
                name: "Bank booth".into(),
                tile: WorldTile {
                    x: 0,
                    z: 4,
                    level: 0,
                },
                access: nav::pack::BankAccess::Booth { op: 2 },
            }],
        )
    }

    /// Task 8 — the BankBudget session unit: the inventory is full of
    /// junk and the knife is in the **bank snapshot**. The strict
    /// `find_with` stays fail-closed (no knife worn); the diagnosis
    /// names only the worn knife; the session plans walk → open →
    /// deposit the backpack → withdraw the knife → wear → close; and the
    /// post-session strict re-find crosses. `find` itself never fetches.
    #[test]
    fn bank_fetch_session_deposits_withdraws_wears_then_finds() {
        let c = bank_fetch_client();
        let mut snap = GameSnapshot::new();
        snap.rebuild(&c);
        // The junk backpack and the open bank's knife row come from the
        // snapshot, exactly as the pump would read them.
        assert_eq!(snap.inv(), &[(1, 3)], "the junk backpack");
        assert!(
            snap.bank().iter().any(|it| it.def.id == 2 && it.count >= 1),
            "the knife is in the open bank"
        );
        let world = knife_nav_world(2);
        let from = WorldTile {
            x: 0,
            z: 0,
            level: 0,
        };
        let to = WorldTile {
            x: 4,
            z: 4,
            level: 0,
        };
        let state = WorldState::from_snapshot(&snap);
        assert!(
            matches!(
                find_with(
                    &world.collision,
                    &world.graph,
                    from,
                    to,
                    FindOptions::default(),
                    &state,
                ),
                Err(nav::router::RouteError::NoPath)
            ),
            "junk but no knife stays fail-closed even for the session"
        );
        let missing = nav::router::find_missing_item_reqs(
            &world.collision,
            &world.graph,
            from,
            to,
            FindOptions::default(),
            &state,
        )
        .expect("only the worn knife is missing");
        assert_eq!(
            missing,
            vec![nav::router::MissingReq::WearAny { ids: vec![2] }]
        );
        let bank_rows: Vec<(i32, i32)> =
            snap.bank().iter().map(|it| (it.def.id, it.count)).collect();
        let fetch =
            nav::bank_fetch::plan_bank_fetch(&missing, &state, &bank_rows, world.banks(), from)
                .expect("the banked knife plans a trip");
        assert_eq!(
            fetch.steps,
            vec![
                nav::bank_fetch::BankStep::Walk {
                    x: 0,
                    z: 4,
                    level: 0
                },
                nav::bank_fetch::BankStep::Open,
                nav::bank_fetch::BankStep::DepositAll,
                nav::bank_fetch::BankStep::Withdraw { id: 2, count: 1 },
                nav::bank_fetch::BankStep::Wear { id: 2 },
                nav::bank_fetch::BankStep::Close,
            ],
            "deposit the junk, withdraw the knife, wear it, close"
        );
        let r = find_with(
            &world.collision,
            &world.graph,
            from,
            to,
            FindOptions::default(),
            &fetch.state,
        )
        .expect("the post-session strict re-find crosses");
        assert_eq!(r.dest, to);
    }

    /// Fix round — BankBudget execute on the live walk arm: `allow_bank_fetch`
    /// on + junk inv + knife in the open bank snapshot must latch a session
    /// on [`ScriptWalkArm::route`] and actually drive Deposit/Withdraw on
    /// the Driver (not only `plan_bank_fetch` in isolation). Start on the
    /// packed bank stand so Walk completes in place; the client's bank is
    /// already open so Open is a no-op; DepositAll + Withdraw must write.
    #[test]
    fn allow_bank_fetch_on_script_walk_arm_drives_deposit_withdraw() {
        let mut c = bank_fetch_client();
        let mut snap = GameSnapshot::new();
        snap.rebuild(&c);
        let world = Arc::new(knife_nav_world(2));
        let state = WorldState::from_snapshot(&snap);
        let bank_rows: Vec<(i32, i32)> =
            snap.bank().iter().map(|it| (it.def.id, it.count)).collect();
        assert!(
            bank_rows.iter().any(|&(id, n)| id == 2 && n >= 1),
            "knife is in the open bank"
        );
        let navs: Arc<Mutex<HashMap<String, NavBot>>> = Arc::new(Mutex::new(HashMap::new()));
        // Stand on the packed bank booth so the session's Walk completes
        // without a follow hop; Open sees the already-open bank.
        let arm = ScriptWalkArm {
            here: Some((0, 4, 0)),
            world: Some(Arc::clone(&world)),
            navs: Arc::clone(&navs),
            name: "alice".into(),
            state: Some(state),
            bank: bank_rows,
        };
        assert!(
            arm.route(
                4,
                4,
                0,
                FindOptions {
                    allow_bank_fetch: true,
                    ..FindOptions::default()
                },
            ),
            "the walk arm must accept the bank-fetch route"
        );
        // The worker latches the session; wait briefly for it.
        let mut latched = false;
        for _ in 0..200 {
            if navs
                .lock()
                .unwrap()
                .get("alice")
                .is_some_and(|b| b.bank_fetch.is_some())
            {
                latched = true;
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        assert!(latched, "allow_bank_fetch must latch a BankFetch session");
        let out_before = c.out.pos;
        // Pump until DepositAll + Withdraw have had a chance to write (Walk
        // and Open complete immediately on this fixture).
        for _ in 0..16 {
            let mut all = navs.lock().unwrap();
            let bot = all.get_mut("alice").expect("nav bot");
            if bot.bank_fetch.is_none() {
                break;
            }
            step_bank_fetch_on_bot(&mut c, &snap, bot, Some(world.as_ref()), Some((0, 4, 0)));
        }
        assert!(
            c.out.pos > out_before,
            "BankBudget execute must drive deposit/withdraw on the Driver (pos {} → {})",
            out_before,
            c.out.pos
        );
    }

    /// Fix round 2 — BankBudget Walk from off the stand must poll
    /// [`Traveller::follow`] on the stand sub-route. Freeze-all-follow
    /// while `bank_fetch` is Some stalls Walk forever; following
    /// `final_route` would skip the booth. Start at (0,0), stand at
    /// (0,4), final dest (4,4).
    #[test]
    fn allow_bank_fetch_off_stand_walk_follows_stand_sub_route() {
        let mut c = bank_fetch_client();
        c.local_player = Some(client::dash3d::ClientPlayer::at(0, 0));
        c.bump_gens(client::io::ServerProt::PLAYER_INFO);
        c.bump_gens(client::io::ServerProt::REBUILD_NORMAL);
        let mut snap = GameSnapshot::new();
        snap.rebuild(&c);
        let world = Arc::new(knife_nav_world(2));
        let state = WorldState::from_snapshot(&snap);
        let bank_rows: Vec<(i32, i32)> =
            snap.bank().iter().map(|it| (it.def.id, it.count)).collect();
        assert!(
            bank_rows.iter().any(|&(id, n)| id == 2 && n >= 1),
            "knife is in the open bank"
        );
        let navs: Arc<Mutex<HashMap<String, NavBot>>> = Arc::new(Mutex::new(HashMap::new()));
        let statuses: Arc<Mutex<Vec<SlotStatus>>> = Arc::new(Mutex::new(vec![SlotStatus {
            username: "alice".into(),
            ..SlotStatus::default()
        }]));
        // Off the packed booth: Walk must arm a stand sub-route and follow
        // it — not stall, not jump to the knife-gated final dest.
        let arm = ScriptWalkArm {
            here: Some((0, 0, 0)),
            world: Some(Arc::clone(&world)),
            navs: Arc::clone(&navs),
            name: "alice".into(),
            state: Some(state),
            bank: bank_rows,
        };
        assert!(
            arm.route(
                4,
                4,
                0,
                FindOptions {
                    allow_bank_fetch: true,
                    ..FindOptions::default()
                },
            ),
            "the walk arm must accept the bank-fetch route"
        );
        let mut latched = false;
        for _ in 0..200 {
            if navs
                .lock()
                .unwrap()
                .get("alice")
                .is_some_and(|b| b.bank_fetch.is_some())
            {
                latched = true;
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        assert!(latched, "allow_bank_fetch must latch a BankFetch session");
        // Live pump: session Walk + follow (not step_bank_fetch alone).
        step_nav_bot(
            &mut c,
            "alice",
            Some((0, 0, 0)),
            &snap,
            &navs,
            &statuses,
            Some(world.as_ref()),
            false,
        );
        let all = navs.lock().unwrap();
        let bot = all.get("alice").expect("nav bot");
        assert!(
            bot.bank_fetch
                .as_ref()
                .is_some_and(|p| matches!(p.steps.front(), Some(BankStep::Walk { .. }))),
            "Walk must still be the front step (not skipped to Open/final)"
        );
        let dest = bot.route.as_ref().map(|r| r.dest);
        assert_eq!(
            dest,
            Some(WorldTile {
                x: 0,
                z: 4,
                level: 0
            }),
            "armed route must be the stand sub-route, not final (4,4)"
        );
        assert!(
            bot.traveller.current_aim().is_some(),
            "Walk must poll Traveller::follow on the stand sub-route (freeze-all-follow stalls)"
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
    type EmptyNav = (Arc<Mutex<HashMap<String, NavBot>>>, Option<Arc<NavWorld>>);
    fn empty_nav() -> EmptyNav {
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
            &mut c, "alice", false, true, 1, None, None, None, None, None, &scripts, &cheats,
            &navs, &world, false, false,
        );
        assert_eq!(*count.lock().unwrap(), 0);
        // Up + edge: exactly one tick.
        script_observe(
            &mut c, "alice", true, true, 2, None, None, None, None, None, &scripts, &cheats, &navs,
            &world, false, false,
        );
        assert_eq!(*count.lock().unwrap(), 1);
        // Up but no edge: nothing.
        script_observe(
            &mut c, "alice", true, false, 2, None, None, None, None, None, &scripts, &cheats,
            &navs, &world, false, false,
        );
        assert_eq!(*count.lock().unwrap(), 1);
        // A dispatched tick wrote the driver's out buffer (the slot's own
        // `Client` sends it on the next mainloop pass).
        assert!(script_observe(
            &mut c, "alice", true, true, 3, None, None, None, None, None, &scripts, &cheats, &navs,
            &world, false, false
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
            &mut c, "alice", true, true, 1, None, None, None, None, None, &scripts, &cheats, &navs,
            &world, false, false
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
            &mut c, "alice", true, true, 2, None, None, None, None, None, &scripts, &cheats, &navs,
            &world, false, false
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
            &mut c, "alice", true, false, 0, None, None, None, None, None, &scripts, &cheats,
            &navs, &world, false, false,
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
            None,
            None,
            Some(&names),
            &scripts,
            &cheats,
            &navs,
            &world,
            false,
            false,
        );
        assert_eq!(
            *seen.lock().unwrap(),
            Some((true, true, true)),
            "a Running script sees the inventory view and resolves names"
        );
    }

    /// Records whether the per-tick snapshot reached the script ctx and
    /// what the `varp` getter read through it.
    type SnapProbeSeen = Option<(bool, Option<i32>)>;

    #[derive(Default)]
    struct SnapProbe(Arc<Mutex<SnapProbeSeen>>);

    impl script::Script for SnapProbe {
        fn name(&self) -> &str {
            "SnapProbe"
        }
        fn tick(&mut self, ctx: &mut ScriptCtx<'_>) {
            *self.0.lock().unwrap() = Some((ctx.snapshot.is_some(), ctx.varp(101)));
        }
    }

    #[test]
    fn script_observe_passes_the_tick_snapshot_to_the_ctx() {
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
            .start_compiled(Box::new(SnapProbe(Arc::clone(&seen))))
            .unwrap();
        // A transmitted varp table so the probe's `varp(101)` has a value
        // to read (the snapshot only lists transmitted definitions).
        let cache = Cache {
            varps: (0..102)
                .map(|_| client::config::VarpType::default())
                .collect(),
            ..Default::default()
        };
        let mut c = prepare_client(
            ClientConfig {
                host: "127.0.0.1".into(),
                port: 1,
                cache_dir: String::new(),
                members: true,
                lowmem: true,
            },
            1,
            Arc::new(cache),
            Arc::new(vec![]),
            Vec::new(),
        );
        c.var = vec![0; 102];
        c.var[101] = 5;
        c.bump_gens(ServerProt::VARP_SYNC);
        let mut snap = GameSnapshot::new();
        snap.rebuild(&c);
        script_observe(
            &mut c,
            "alice",
            true,
            true,
            1,
            None,
            None,
            None,
            Some(&snap),
            None,
            &scripts,
            &cheats,
            &navs,
            &world,
            false,
            false,
        );
        assert_eq!(
            *seen.lock().unwrap(),
            Some((true, Some(5))),
            "the observe snapshot reaches the ctx and the varp getter reads it"
        );
    }

    // Task 9b — the posted blob is FlatBuffers (schema:
    // crates/script/schema/isolate.fbs) and carries exactly the fields the
    // shim Game/Inventory/Skills/EventSignal read: inv rows carry resolved
    // obj names (None when the table has none), stats rows the stat
    // index/name/xp, bank flags from the snapshot, and hold/ours pass
    // through for EventSignal.pending(). No World clone — only these
    // fields. Round-trips through the script crate's decoder.
    #[test]
    fn script_snapshot_fb_carries_observed_fields_only() {
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
        c.runenergy = 42;
        c.stat_effective_level[7] = 40;
        c.stat_xp[7] = 1300;
        c.bump_gens(ServerProt::UPDATE_STAT);
        let mut snap = GameSnapshot::new();
        snap.rebuild(&c);
        let mut objs = vec![client::config::ObjType::default(); 2];
        objs[1].id = 1;
        objs[1].name = "Bones".into();
        let names = api::obj_names::ObjNames::from_objs(&objs);
        let inv = vec![(1, 2), (99, 5)];
        let (bytes, _fp) = script_snapshot_fb(
            None,
            false,
            7,
            Some((3200, 3200, 0)),
            true,
            Some(&inv),
            Some(&snap),
            Some(&names),
            None,
            true,
            false,
        );
        let view = script::isolate_fb::decode_snapshot(&bytes).expect("blob decodes");
        assert_eq!(view.tick(), 7);
        assert!(view.has_here(), "keyframe carries here");
        let here = view.here().expect("here posted");
        assert_eq!((here.x(), here.z(), here.level()), (3200, 3200, 0));
        assert!(view.ingame());
        assert!(view.has_inv(), "keyframe carries inv");
        let inv = view.inv();
        assert_eq!(inv.len(), 2);
        assert_eq!((inv[0].name(), inv[0].count()), (Some("Bones"), 2));
        assert_eq!(
            (inv[1].name(), inv[1].count()),
            (None, 5),
            "an obj the table does not know posts a null name, never invented"
        );
        assert!(view.has_stats(), "keyframe carries stats");
        let stats = view.stats();
        assert_eq!(
            (stats[7].index(), stats[7].name(), stats[7].xp()),
            (7, "cooking", 1300)
        );
        assert!(!view.bank_open(), "no bank component in the fixture");
        assert!(!view.bank_loaded());
        assert!(
            view.booths().is_empty(),
            "no Use-quickly scene locs in the fixture"
        );
        assert!(
            view.banks().is_empty(),
            "no nav world: no packed stands posted"
        );
        assert!(view.bank().is_empty());
        assert!(view.bank_side().is_empty());
        assert!(view.hold());
        assert!(!view.ours());

        // No tile / no snapshot: fail-closed nulls and flags.
        let (bare_bytes, _) = script_snapshot_fb(
            None, false, 1, None, false, None, None, None, None, false, true,
        );
        let bare = script::isolate_fb::decode_snapshot(&bare_bytes).expect("bare blob decodes");
        assert!(bare.here().is_none());
        assert!(bare.inv().is_empty());
        assert!(bare.stats().is_empty());
        assert!(!bare.bank_open());
        assert!(bare.ours(), "ours rides the blob for EventSignal");
    }

    // Task 9c — delta posts: the keyframe carries every field; a later
    // post carries only the fields that changed (plus tick), so a 50+
    // isolate wall never resends unchanged inv/bank/stats/booths/packed
    // banks. The returned fingerprint is the per-slot last-post state the
    // observe stores; banks are re-included on `force_banks` even when the
    // stand list did not change (NavWorld identity change).
    #[test]
    fn script_snapshot_fb_posts_only_changed_tables() {
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
        c.stat_effective_level[7] = 40;
        c.stat_xp[7] = 1300;
        c.bump_gens(ServerProt::UPDATE_STAT);
        let mut snap = GameSnapshot::new();
        snap.rebuild(&c);
        let mut objs = vec![client::config::ObjType::default(); 2];
        objs[1].id = 1;
        objs[1].name = "Bones".into();
        let names = api::obj_names::ObjNames::from_objs(&objs);
        let inv = vec![(1, 2)];
        let world = Some(Arc::new(NavWorld::from_parts(
            nav::collision::WorldCollision {
                origin: WorldTile {
                    x: 0,
                    z: 0,
                    level: 0,
                },
                width: 2,
                height: 1,
                walk: vec![0u8; 2],
                blocked: vec![0u64; 2usize.div_ceil(64)],
                flags: None,
            },
            nav::transport::TransportGraph::default(),
            vec![nav::pack::BankStand {
                name: "Bank booth".into(),
                tile: WorldTile {
                    x: 1,
                    z: 0,
                    level: 0,
                },
                access: nav::pack::BankAccess::Booth { op: 2 },
            }],
        )));

        // Keyframe (no last post): every observed field is present.
        let (keyframe, fp1) = script_snapshot_fb(
            None,
            false,
            7,
            Some((3200, 3200, 0)),
            true,
            Some(&inv),
            Some(&snap),
            Some(&names),
            world.as_deref(),
            true,
            false,
        );
        let kf = script::isolate_fb::decode_snapshot(&keyframe).expect("keyframe decodes");
        assert!(kf.has_here());
        assert!(kf.has_ingame());
        assert!(kf.has_inv());
        assert!(kf.has_stats());
        assert!(kf.has_banks(), "keyframe carries the packed banks");

        // Same observed data again: only tick is carried.
        let (delta, fp2) = script_snapshot_fb(
            Some(&fp1),
            false,
            8,
            Some((3200, 3200, 0)),
            true,
            Some(&inv),
            Some(&snap),
            Some(&names),
            world.as_deref(),
            true,
            false,
        );
        let view = script::isolate_fb::decode_snapshot(&delta).expect("delta decodes");
        assert_eq!(view.tick(), 8, "tick is always present");
        assert!(!view.has_here(), "unchanged here omitted");
        assert!(!view.has_ingame(), "unchanged ingame omitted");
        assert!(!view.has_inv(), "unchanged inv omitted");
        assert!(!view.has_stats(), "unchanged stats omitted");
        assert!(!view.has_banks(), "unchanged packed banks omitted");
        assert!(!view.has_bank());
        assert!(!view.has_bank_side());
        assert!(!view.hold(), "unchanged hold omitted");

        // inv changed -> the inv table comes back; banks stay omitted even
        // when force_banks is false.
        let inv2 = vec![(1, 2), (99, 5)];
        let (delta, _fp3) = script_snapshot_fb(
            Some(&fp2),
            false,
            9,
            Some((3200, 3200, 0)),
            true,
            Some(&inv2),
            Some(&snap),
            Some(&names),
            world.as_deref(),
            true,
            false,
        );
        let view = script::isolate_fb::decode_snapshot(&delta).expect("delta decodes");
        assert!(view.has_inv(), "changed inv is carried");
        assert_eq!(view.inv().len(), 2);
        assert!(!view.has_banks(), "unchanged banks still omitted");

        // NavWorld identity change (force_banks): the packed banks are
        // re-posted even though the stand list is unchanged.
        let (delta, _) = script_snapshot_fb(
            Some(&fp2),
            true,
            10,
            Some((3200, 3200, 0)),
            true,
            Some(&inv),
            Some(&snap),
            Some(&names),
            world.as_deref(),
            true,
            false,
        );
        let view = script::isolate_fb::decode_snapshot(&delta).expect("delta decodes");
        assert!(view.has_banks(), "force_banks re-posts the packed banks");
        assert!(!view.has_inv(), "unchanged inv stays omitted");
    }

    #[test]
    fn inventory_from_ifaces_maps_1_based_ids_to_0_based() {
        // The TYPE_INV iface stores `obj_id + 1` (0 = empty slot); scripts
        // resolve `has_item` against the 0-based ObjNames table, so the
        // view must carry `id - 1` and drop the empties.
        let mut ifaces = vec![None; 3];
        ifaces[1] = Some(Box::new(IfType {
            r#type: ComponentType::TYPE_INV,
            obj_ops: true,
            ..Default::default()
        }));
        let mut ifaces_mut = vec![None; 3];
        ifaces_mut[1] = Some(Arc::new(IfTypeMut {
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
        // The inv tab (side 3) binds this root; a TYPE_INV that is not the
        // backpack (no `obj_ops`, or under another side tab) must not
        // satisfy the read.
        client.side_icon[3] = 1;
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
            held_ops: 0,
            if_button_components: Vec::new(),
            sink: Sink,
        };
        let ctx = ScriptCtx {
            driver: &mut rec,
            tick: 0,
            here: None,
            walk: None,
            walk_with: None,
            inv: Some(&inv),
            snapshot: None,
            obj_names: Some(&names),
        };
        assert!(ctx.has_item("Bones"));
        assert!(!ctx.has_item("Vial"));
    }

    // --- Task 5: guardian hold + knock plumbing over `host::Guardian` ---

    /// Recording driver for the guardian tests: captures every
    /// menu/action/try_move send (the host crate's fake driver shape;
    /// `walks` is the flee/ground-walk trace).
    #[derive(Default)]
    struct GuardRec {
        menus: Vec<(i32, i32, i32, i32, i32)>,
        actions: Vec<i32>,
        walks: Vec<(i32, i32)>,
        sink: Sink,
    }

    impl Driver for GuardRec {
        fn set_menu(&mut self, slot: i32, action: i32, a: i32, b: i32, c: i32) {
            self.menus.push((slot, action, a, b, c));
        }
        fn do_action(&mut self, slot: i32) -> bool {
            self.actions.push(slot);
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
            _t: i32,
        ) -> bool {
            self.walks.push((dx, dz));
            true
        }
        fn local_route(&self) -> Option<(i32, i32)> {
            // (0,0) with build_base (0,0): absolute world tiles equal the
            // recorded `try_move` targets (the flee/ground walks land).
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
            false
        }
    }

    /// An attached ingame scene-2 client with a named local player at
    /// world (0,0) (base 0), ready for the guardian's detect to see.
    fn guardian_client() -> Client {
        let mut c = nav_client();
        c.map_build_base_x = 0;
        c.map_build_base_z = 0;
        c.self_slot = 0;
        let mut lp = client::dash3d::ClientPlayer::at(0, 0);
        lp.name = Some("Test".to_string());
        c.local_player = Some(lp);
        c
    }

    /// Plant NPC `name` in client table slot `slot` with an overhead line
    /// (the snapshot `NpcView` the guardian's detect reads).
    fn plant_npc(c: &mut Client, slot: usize, name: &str, overhead: Option<&str>) {
        plant_npc_with_face(c, slot, name, -1, overhead);
    }

    /// Plant NPC `name` in client table slot `slot` facing the local
    /// player (`face_entity` >= 32768 decodes as Player kind; + self_slot
    /// 0 targets us — the host-owned evade shape).
    fn plant_attacking_npc(c: &mut Client, slot: usize, name: &str) {
        plant_npc_with_face(c, slot, name, 32768, None);
    }

    fn plant_npc_with_face(
        c: &mut Client,
        slot: usize,
        name: &str,
        face_entity: i32,
        overhead: Option<&str>,
    ) {
        let type_id = 500 + slot;
        {
            let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
            while cache.npcs.len() <= type_id {
                cache.npcs.push(client::config::NpcType::default());
            }
            cache.npcs[type_id] = client::config::NpcType {
                id: type_id as i32,
                name: name.to_string(),
                op: vec![Some("Talk-to".to_string())],
                ..Default::default()
            };
        }
        let mut npc = client::dash3d::ClientNpc::at(0, 0);
        npc.r#type = Some(type_id);
        npc.entity.face_entity = face_entity;
        npc.entity.chat_message = overhead.map(str::to_string);
        while c.npc.len() <= slot {
            c.npc.push(None);
        }
        c.npc[slot] = Some(Box::new(npc));
        c.npc_ids[c.npc_count as usize] = slot as i32;
        c.npc_count += 1;
    }

    /// Advance every packet family and rebuild the **persistent**
    /// snapshot (one call per game tick, so `snap.tick()` climbs).
    fn tick_at(c: &mut Client, snap: &mut GameSnapshot) {
        c.gens.npc = c.gens.npc.wrapping_add(1);
        c.gens.player = c.gens.player.wrapping_add(1);
        c.gens.inv = c.gens.inv.wrapping_add(1);
        c.gens.scene = c.gens.scene.wrapping_add(1);
        c.gens.iface = c.gens.iface.wrapping_add(1);
        c.gens.chat = c.gens.chat.wrapping_add(1);
        snap.rebuild(c);
    }

    /// A guardian hold still posts the blob and dispatches the isolate
    /// tick (onPaint only); compiled scripts stay frozen. The unheld edge
    /// dispatches fully.
    #[test]
    fn script_observe_skips_the_tick_dispatch_while_hold() {
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
        // Up + edge but held by the guardian: compiled tick stays frozen.
        script_observe(
            &mut c, "alice", true, true, 1, None, None, None, None, None, &scripts, &cheats, &navs,
            &world, true, false,
        );
        assert_eq!(*count.lock().unwrap(), 0, "hold freezes compiled on_game_tick");
        // The same edge unheld dispatches.
        script_observe(
            &mut c, "alice", true, true, 2, None, None, None, None, None, &scripts, &cheats, &navs,
            &world, false, false,
        );
        assert_eq!(*count.lock().unwrap(), 1, "an unheld edge dispatches");
    }

    #[test]
    fn script_observe_posts_blob_while_held_and_skips_dispatch() {
        // Fix-round: the snapshot blob must post on the held tick edge, and
        // the isolate tick still dispatches for onPaint (loop frozen inside
        // V8). Compiled-path skip is covered by
        // `script_observe_skips_the_tick_dispatch_while_hold`.
        let scripts: Arc<Mutex<HashMap<String, SlotScript>>> = Arc::new(Mutex::new(HashMap::new()));
        let cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>> =
            Arc::new(Mutex::new(HashMap::new()));
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
        let src =
            "export default class T extends LoopingBot { loop() { globalThis.__rs_loops = (globalThis.__rs_loops || 0) + 1; } }";
        scripts
            .lock()
            .unwrap()
            .entry("alice".into())
            .or_default()
            .start_load(src.to_string(), script::LoadShape::CompatClass)
            .expect("load isolate starts");
        // Held edge: blob posts + isolate tick (paint-only); loop must not
        // advance.
        assert!(script_observe(
            &mut c,
            "alice",
            true,
            true,
            1,
            Some((3200, 3200, 0)),
            None,
            None,
            None,
            None,
            &scripts,
            &cheats,
            &navs,
            &world,
            true,
            false
        ));
        let loops = scripts
            .lock()
            .unwrap()
            .get("alice")
            .unwrap()
            .probe("globalThis.__rs_loops || 0")
            .unwrap();
        assert_eq!(loops, 0, "held isolate tick must not run loop()");
        // Unheld edge: the same slot dispatches a full tick (loop runs).
        assert!(script_observe(
            &mut c,
            "alice",
            true,
            true,
            2,
            Some((3200, 3200, 0)),
            None,
            None,
            None,
            None,
            &scripts,
            &cheats,
            &navs,
            &world,
            false,
            false
        ));
        let loops = scripts
            .lock()
            .unwrap()
            .get("alice")
            .unwrap()
            .probe("globalThis.__rs_loops || 0")
            .unwrap();
        assert_eq!(loops, 1, "unheld isolate tick runs loop()");
        scripts.lock().unwrap().get_mut("alice").unwrap().stop();
    }

    #[test]
    fn hold_freezes_follow_and_keeps_the_armed_route() {
        let NavRig {
            scripts,
            cheats,
            navs,
            world,
            statuses,
            ..
        } = nav_rig();
        let mut d = NavRec::default();
        let mut c = nav_client();
        assert!(script_observe(
            &mut d,
            "alice",
            true,
            true,
            1,
            Some((0, 0, 0)),
            None,
            None,
            None,
            None,
            &scripts,
            &cheats,
            &navs,
            &world,
            false,
            false,
        ));
        assert!(
            wait_until(100, || queued(&navs).is_some()),
            "the worker armed the route"
        );

        // Held: no follow step, and the armed route is not consumed.
        let mut snap = GameSnapshot::new();
        nav_snapshot_at(&mut c, &mut snap, 0, 0);
        step_nav_bot(
            &mut d,
            "alice",
            Some((0, 0, 0)),
            &snap,
            &navs,
            &statuses,
            world.as_deref(),
            true,
        );
        assert_eq!(d.walked, None, "hold freezes the follow");
        assert!(
            queued(&navs).is_some(),
            "the route stays latched under hold"
        );

        // Hold lifted: the next pump step resumes the latched route.
        step_nav_bot(
            &mut d,
            "alice",
            Some((0, 0, 0)),
            &snap,
            &navs,
            &statuses,
            world.as_deref(),
            false,
        );
        assert_eq!(d.walked, Some((4, 0)), "the hop resumes after the hold");
    }

    /// A script that counts ticks and claims every detected event for
    /// itself (the Task 5 `Handle` override).
    struct ClaimHandle(Arc<Mutex<u32>>);

    impl script::Script for ClaimHandle {
        fn name(&self) -> &str {
            "ClaimHandle"
        }
        fn tick(&mut self, _ctx: &mut ScriptCtx<'_>) {
            *self.0.lock().unwrap() += 1;
        }
        fn on_random(&mut self, _ev: &DetectedRandom) -> RandomClaim {
            RandomClaim::Handle
        }
    }

    #[test]
    fn handle_claim_keeps_ticks_and_blocks_host_talk() {
        let count = Arc::new(Mutex::new(0u32));
        let scripts: Arc<Mutex<HashMap<String, SlotScript>>> = Arc::new(Mutex::new(HashMap::new()));
        scripts
            .lock()
            .unwrap()
            .entry("alice".into())
            .or_default()
            .start_compiled(Box::new(ClaimHandle(Arc::clone(&count))))
            .unwrap();
        // The production knock arm: ask the running slot script.
        let knock_scripts = Arc::clone(&scripts);
        let mut knock = move |ev: &DetectedRandom| -> RandomClaim {
            knock_scripts
                .lock()
                .unwrap()
                .get_mut("alice")
                .map(|slot| slot.on_random(ev))
                .unwrap_or(RandomClaim::Host)
        };

        // The guardian's rising edge knocks; the script claims the event,
        // so no Talk-to goes out and the slot is not held.
        let mut c = guardian_client();
        plant_npc(&mut c, 0, "Genie", Some("Greetings Test!"));
        let mut g = Guardian::new();
        let mut drv = GuardRec::default();
        let settings = ProfileSettings::default();
        let mut snap = GameSnapshot::new();
        tick_at(&mut c, &mut snap);
        let status = g.tick(&mut drv, &snap, &settings, 0, Some(&mut knock));
        assert_eq!(status.claim, RandomClaim::Handle);
        assert!(!status.hold, "a Handle claim never holds");
        assert!(drv.menus.is_empty(), "no Talk-to under a Handle claim");
        assert!(drv.actions.is_empty());

        // And the unheld slot still gets its game tick (the script's
        // `Handle` claim means the host leaves the random to it).
        let (navs, world) = empty_nav();
        let cheats = Arc::new(Mutex::new(HashMap::new()));
        script_observe(
            &mut c, "alice", true, true, 1, None, None, None, None, None, &scripts, &cheats, &navs,
            &world, false, false,
        );
        assert_eq!(*count.lock().unwrap(), 1, "a Handle claim still ticks");
    }

    #[test]
    fn ignored_randoms_skips_flee_but_detect_still_publishes() {
        // Task 12: the bot instance's `ignoredRandoms()` list makes the
        // host's knock path decline the event — no flee / Talk-to / etc.
        // — while detect still publishes the kind (chrome can show it).
        // The claim knock itself stays Host for JS isolates (no Handle
        // from V8); the Handle here is the host-play path's own decision.
        let scripts: Arc<Mutex<HashMap<String, SlotScript>>> = Arc::new(Mutex::new(HashMap::new()));
        let src = "export default class T extends LoopingBot { ignoredRandoms() { return ['swarm']; } loop() {} }";
        scripts
            .lock()
            .unwrap()
            .entry("alice".into())
            .or_default()
            .start_load(src.to_string(), script::LoadShape::CompatClass)
            .expect("load isolate starts");
        // The production knock arm (see the slot thread): an ignored name
        // is a host-declined claim; everything else asks the running slot
        // script.
        let knock_scripts = Arc::clone(&scripts);
        let mut knock = move |ev: &DetectedRandom| -> RandomClaim {
            let mut all = knock_scripts.lock().unwrap();
            let Some(slot) = all.get_mut("alice") else {
                return RandomClaim::Host;
            };
            if slot
                .ignored_randoms()
                .iter()
                .any(|n| n.eq_ignore_ascii_case(&ev.name))
            {
                return RandomClaim::Handle;
            }
            slot.on_random(ev)
        };

        let mut c = guardian_client();
        plant_attacking_npc(&mut c, 0, "Swarm");
        let mut g = Guardian::new();
        let mut drv = GuardRec::default();
        let settings = ProfileSettings::default();
        let mut snap = GameSnapshot::new();
        tick_at(&mut c, &mut snap);
        let status = g.tick(&mut drv, &snap, &settings, 0, Some(&mut knock));
        assert_eq!(status.kind, Some(api::random::RandomKind::Evade));
        assert_eq!(status.name.as_deref(), Some("swarm"));
        assert!(status.ours, "detect still publishes the event");
        assert_eq!(status.claim, RandomClaim::Handle);
        assert!(!status.hold);
        assert!(drv.walks.is_empty(), "an ignored swarm is not fled");
        assert!(drv.actions.is_empty());

        // Control: a host-owned slot (no ignore list) flees the same
        // swarm — the ignore list is what suppressed the act.
        scripts.lock().unwrap().get_mut("alice").unwrap().stop();
        let mut c2 = guardian_client();
        plant_attacking_npc(&mut c2, 0, "Swarm");
        let mut g2 = Guardian::new();
        let mut drv2 = GuardRec::default();
        let mut snap2 = GameSnapshot::new();
        tick_at(&mut c2, &mut snap2);
        let status2 = g2.tick(&mut drv2, &snap2, &settings, 0, None);
        assert_eq!(status2.kind, Some(api::random::RandomKind::Evade));
        assert_eq!(status2.claim, RandomClaim::Host);
        assert!(
            !drv2.walks.is_empty(),
            "an unignored swarm is fled (the ignore list suppressed it)"
        );
    }

    #[test]
    fn event_signal_pending_reads_true_during_dialog_hold() {
        // Task 12: the guardian's dialog hold is posted into the isolate
        // (`hold: true` on the blob), so `EventSignal.pending()` reads
        // true while the slot is frozen by the in-flight dialog. The
        // shim's `pending()` mapping is pinned by the script crate's
        // load_isolate tests; here the held edge drives the full
        // guardian -> observe -> isolate chain.
        let scripts: Arc<Mutex<HashMap<String, SlotScript>>> = Arc::new(Mutex::new(HashMap::new()));
        let src = "export default class T extends LoopingBot { loop() {} }";
        scripts
            .lock()
            .unwrap()
            .entry("alice".into())
            .or_default()
            .start_load(src.to_string(), script::LoadShape::CompatClass)
            .expect("load isolate starts");

        // The guardian talks to the old man and holds the slot.
        let mut c = guardian_client();
        plant_npc(&mut c, 0, "Mysterious old man", Some("Greetings Test!"));
        let mut g = Guardian::new();
        let mut drv = GuardRec::default();
        let settings = ProfileSettings::default();
        let mut snap = GameSnapshot::new();
        tick_at(&mut c, &mut snap);
        let status = g.tick(&mut drv, &snap, &settings, 0, None);
        assert!(status.handling);
        assert!(status.hold, "the in-flight dialog holds the slot");

        // The held edge posts the blob and dispatches the isolate tick
        // (onPaint only); EventSignal reads the freeze from the blob.
        let (navs, world) = empty_nav();
        let cheats = Arc::new(Mutex::new(HashMap::new()));
        assert!(script_observe(
            &mut c,
            "alice",
            true,
            true,
            1,
            Some((3200, 3200, 0)),
            None,
            None,
            None,
            None,
            &scripts,
            &cheats,
            &navs,
            &world,
            status.hold,
            status.ours
        ));
        let hold = scripts
            .lock()
            .unwrap()
            .get("alice")
            .unwrap()
            .probe("globalThis.__rs2b0t_host.snapshot.hold")
            .expect("posted snapshot reads back");
        assert_eq!(hold, true, "the dialog hold is what pending() reads");
        scripts.lock().unwrap().get_mut("alice").unwrap().stop();
    }

    #[test]
    fn after_genie_gone_lamp_auto_off_in_inv_detects_without_hold() {
        let mut c = guardian_client();
        plant_npc(&mut c, 0, "Genie", Some("Greetings Test!"));
        let mut g = Guardian::new();
        let mut drv = GuardRec::default();
        let settings = ProfileSettings {
            lamp_auto: false,
            ..ProfileSettings::default()
        };
        let mut snap = GameSnapshot::new();

        // Tick 1: the host talks to the genie and holds the slot.
        tick_at(&mut c, &mut snap);
        let status = g.tick(&mut drv, &snap, &settings, 0, None);
        assert!(status.handling);
        assert!(status.hold);
        assert_eq!(drv.actions, vec![0]);

        // Tick 2: the genie is gone and the lamp sits in the inventory:
        // the handle lifts and the lamp is inert XP — no hold.
        drv.menus.clear();
        drv.actions.clear();
        c.npc[0] = None;
        c.npc_count = 0;
        plant_inv_item(&mut c, 2528); // Lamp (the genie lamp) obj id
        tick_at(&mut c, &mut snap);
        let status = g.tick(&mut drv, &snap, &settings, 0, None);
        assert_eq!(status.kind, Some(api::random::RandomKind::Lamp));
        assert_eq!(status.name.as_deref(), Some("lamp"));
        assert!(!status.hold, "leftover lamp must not keep the slot held");
        assert!(drv.menus.is_empty(), "a lamp is never talked to");
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
        /// Held-item ops (OP_HELD1..=5): the jewellery rub arm's press.
        held_ops: usize,
        /// The component ids pressed via IF_BUTTON, in order (the follow's
        /// dialog-ride arm asserts *which* choice was answered).
        if_button_components: Vec<i32>,
        sink: Sink,
    }

    impl Driver for NavRec {
        fn set_menu(&mut self, _slot: i32, action: i32, _a: i32, _b: i32, c: i32) {
            match action {
                MiniMenuAction::OP_HELD1
                | MiniMenuAction::OP_HELD2
                | MiniMenuAction::OP_HELD3
                | MiniMenuAction::OP_HELD4
                | MiniMenuAction::OP_HELD5 => self.held_ops += 1,
                MiniMenuAction::IF_BUTTON => self.if_button_components.push(c),
                _ => {}
            }
        }
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
        // An all-walkable 40×1 world at (0,0): x in 0..40 at z=0, no
        // transport edges — the collision+graph shape `find` consumes, built
        // directly (no pack file on disk in unit tests).
        nav_rig_with(Some(Arc::new(NavWorld::from_parts(
            nav::collision::WorldCollision {
                origin: WorldTile {
                    x: 0,
                    z: 0,
                    level: 0,
                },
                width: 40,
                height: 1,
                walk: vec![0u8; 40],
                blocked: vec![0u64; 40usize.div_ceil(64)],
                flags: None,
            },
            nav::transport::TransportGraph::default(),
            Vec::new(),
        ))))
    }

    /// [`nav_rig`] on the given world (the walk-probe target stays
    /// `(4,0,0)`; callers that walk elsewhere override `walk_target`).
    fn nav_rig_with(world: Option<Arc<NavWorld>>) -> NavRig {
        let scripts: Arc<Mutex<HashMap<String, SlotScript>>> = Arc::new(Mutex::new(HashMap::new()));
        let cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let navs: Arc<Mutex<HashMap<String, NavBot>>> = Arc::new(Mutex::new(HashMap::new()));
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
            None,
            None,
            &scripts,
            &cheats,
            &navs,
            &world,
            false,
            false,
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
        step_nav_bot(
            &mut d,
            "alice",
            Some((0, 0, 0)),
            &snap,
            &navs,
            &statuses,
            world.as_deref(),
            false,
        );
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
        step_nav_bot(
            &mut d,
            "alice",
            Some((4, 0, 0)),
            &snap,
            &navs,
            &statuses,
            world.as_deref(),
            false,
        );
        assert_eq!(queued(&navs), None, "arrival clears the armed route");
        {
            let rows = statuses.lock().unwrap();
            assert_eq!(rows[0].walk_x, -1, "idle bot reports no target");
            assert_eq!(rows[0].walk_z, -1);
            assert_eq!(rows[0].walk_level, -1);
        }
    }

    /// A packed glory-style jewellery edge (obj 1712, `opheld4` Rub): the
    /// shape every dest of the multi-location glory group shares. The
    /// `to` names the landing (default Edgeville, `switch_int($choice)`
    /// case 1); the group's sibling edges share `loc_id` + option and
    /// differ only in `to`, exactly as the bake emits them.
    fn glory_edge() -> TransportEdge {
        TransportEdge {
            kind: TransportKind::Teleport,
            at: WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            to: WorldTile {
                x: 3087,
                z: 3496,
                level: 0, // Edgeville (case 1)
            },
            loc_id: 1712,
            option: 4, // Rub (opheld4)
            ticks: 2,  // OP_BASE + the rub p_delay(1)
            dir: None,
            open_loc_id: None,
            skill_req: vec![],
            item_req: vec![(1712, 1)],
            quest_req: vec![],
            varp_req: vec![],
            worn_req: vec![],
        }
    }

    /// The charged glory in the inv tab (side 3) TYPE_INV container: the
    /// shape `teleport_send` reads the rub's held item from.
    fn plant_inv_item(c: &mut Client, obj_id: i32) {
        {
            let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
            while cache.objs.len() <= obj_id as usize {
                cache.objs.push(client::config::ObjType::default());
            }
            cache.objs[obj_id as usize] = client::config::ObjType {
                id: obj_id,
                iop: [None, None, None, Some("Rub".into()), None],
                ..Default::default()
            };
        }
        c.side_icon[3] = 300;
        c.set_iface(
            300,
            IfType {
                id: 300,
                layer_id: 300,
                children: Some(vec![301]),
                ..Default::default()
            },
        );
        c.set_iface(
            301,
            IfType {
                id: 301,
                layer_id: 300,
                r#type: ComponentType::TYPE_INV,
                obj_ops: true,
                ..Default::default()
            },
        );
        c.set_iface_mut(
            301,
            IfTypeMut {
                link_obj_type: Some(vec![obj_id + 1]),
                link_obj_number: Some(vec![1]),
                ..Default::default()
            },
        );
    }

    /// A chat destination dialog (root 100 with one BUTTON_OK choice
    /// button per option, at components 101..): the shape a jewellery
    /// rub's "Where would you like to teleport to?" opens.
    fn plant_choice_dialog(c: &mut Client, options: &[&str]) {
        let root = 100;
        let children: Vec<i32> = (0..options.len()).map(|i| (101 + i) as i32).collect();
        for (i, text) in options.iter().enumerate() {
            let id = 101 + i;
            c.set_iface(
                id,
                IfType {
                    id: id as i32,
                    layer_id: root,
                    ..Default::default()
                },
            );
            c.set_iface_mut(
                id,
                IfTypeMut {
                    button_type: ButtonType::BUTTON_OK,
                    text: (*text).to_string(),
                    ..Default::default()
                },
            );
        }
        c.set_iface(
            root as usize,
            IfType {
                id: root,
                layer_id: root,
                children: Some(children),
                ..Default::default()
            },
        );
        c.chat_modal_id = root;
        c.bump_gens(ServerProt::IF_OPENCHAT);
    }

    /// Bump every gen and rebuild into the existing snapshot (tick + 1).
    fn bump_rebuild(c: &mut Client, snap: &mut GameSnapshot) {
        c.bump_gens(ServerProt::REBUILD_NORMAL);
        snap.rebuild(c);
    }

    /// The pump's per-uid nav step must hand the follow the packed any-tile
    /// teleport list: a multi-destination jewellery rub (two glory
    /// siblings, same `loc_id`, differing `to`) executes the SECOND
    /// landing only when the traveller answers dialog choice 2 — the
    /// 1-based index of the followed edge's `to` among the packed
    /// same-`loc_id` rub edges. Without `world.graph.teleports` the
    /// follow falls back to the modal's FIRST choice and rubs to the
    /// wrong place (the same pass-through the panel and scenario follow
    /// make).
    #[test]
    fn step_nav_bot_passes_graph_teleports_for_a_multi_dest_jewellery_rub() {
        let karamja = WorldTile {
            x: 2918,
            z: 3176,
            level: 0, // the packed glory case-2 landing
        };
        let edgeville = WorldTile {
            x: 3087,
            z: 3496,
            level: 0, // case 1
        };
        let glory = [
            TransportEdge {
                to: edgeville,
                ..glory_edge()
            },
            TransportEdge {
                to: karamja,
                ..glory_edge()
            },
        ];
        let world = Some(Arc::new(NavWorld::from_parts(
            nav::collision::WorldCollision {
                origin: WorldTile {
                    x: 0,
                    z: 0,
                    level: 0,
                },
                width: 1,
                height: 1,
                walk: vec![0u8; 1],
                blocked: vec![0u64; 1],
                flags: None,
            },
            TransportGraph {
                teleports: glory.to_vec(),
                ..TransportGraph::default()
            },
            Vec::new(),
        )));
        let NavRig {
            navs,
            world,
            statuses,
            ..
        } = nav_rig_with(world);

        let mut c = nav_client();
        plant_inv_item(&mut c, 1712);
        let mut snap = GameSnapshot::new();
        nav_snapshot_at(&mut c, &mut snap, 0, 0);
        let mut d = NavRec::default();

        // Arm the route for the SECOND landing directly in the uid's bot
        // (the arm is the router's job; this test pins the follow's
        // dialog answer).
        navs.lock()
            .unwrap()
            .entry("alice".into())
            .or_default()
            .route = Some(Route {
            legs: vec![Leg::Transport {
                edge: glory[1].clone(),
            }],
            dest: karamja,
            ticks: 2.0,
        });

        // Poll 1: the hop rubs the charged item.
        step_nav_bot(
            &mut d,
            "alice",
            Some((0, 0, 0)),
            &snap,
            &navs,
            &statuses,
            world.as_deref(),
            false,
        );
        assert_eq!(d.held_ops, 1, "one OP_HELD4 rub sent");
        assert!(queued(&navs).is_some(), "the route stays armed");

        // The rub opens the destination choice: the next pump poll answers
        // the SECOND option (Karamja), never the constant first.
        plant_choice_dialog(&mut c, &["Edgeville.", "Karamja."]);
        bump_rebuild(&mut c, &mut snap);
        step_nav_bot(
            &mut d,
            "alice",
            Some((0, 0, 0)),
            &snap,
            &navs,
            &statuses,
            world.as_deref(),
            false,
        );
        assert_eq!(
            d.if_button_components,
            vec![102],
            "the second destination answers choice 2, not the modal's first"
        );
    }

    /// A 5×5 world walled between x=1 and x=2, crossed only by a 10-coin
    /// toll door (the `toll_edges` shape: loc 2882, `item_req` coins 10).
    fn toll_nav_world() -> NavWorld {
        let mut flags = vec![0u32; 25];
        for z in 0..5 {
            flags[z * 5 + 1] |= client::dash3d::CollisionFlag::W_E as u32;
            flags[z * 5 + 2] |= client::dash3d::CollisionFlag::W_W as u32;
        }
        let edge = TransportEdge {
            kind: TransportKind::Door,
            at: WorldTile {
                x: 1,
                z: 2,
                level: 0,
            },
            to: WorldTile {
                x: 2,
                z: 2,
                level: 0,
            },
            loc_id: 2882,
            option: 1,
            ticks: 2,
            dir: None,
            open_loc_id: None,
            skill_req: vec![],
            item_req: vec![(995, 10)],
            quest_req: vec![],
            varp_req: vec![],
            worn_req: vec![],
        };
        let mut graph = TransportGraph::default();
        graph.at.entry(edge.at).or_default().push(0);
        graph.edges.push(edge);
        let (walk, blocked) = nav::collision::pack_walk(&flags);
        NavWorld::from_parts(
            nav::collision::WorldCollision {
                origin: WorldTile {
                    x: 0,
                    z: 0,
                    level: 0,
                },
                width: 5,
                height: 5,
                walk,
                blocked,
                flags: None,
            },
            graph,
            Vec::new(),
        )
    }

    /// The script walk arm's facts gate the route: with 10 coins in the
    /// slot's state, `ctx.walk` crosses the toll; the armed route carries
    /// the toll Transport leg.
    #[test]
    fn script_observe_walk_uses_slot_state_across_a_toll() {
        let NavRig {
            scripts,
            cheats,
            navs,
            world,
            walk_ret,
            walk_target,
            ..
        } = nav_rig_with(Some(Arc::new(toll_nav_world())));
        *walk_target.lock().unwrap() = (4, 4, 0);
        let mut d = NavRec::default();
        let state = Some(WorldState {
            inv: std::collections::HashMap::from([(995, 10)]),
            ..WorldState::default()
        });
        assert!(script_observe(
            &mut d,
            "alice",
            true,
            true,
            1,
            Some((0, 0, 0)),
            None,
            state,
            None,
            None,
            &scripts,
            &cheats,
            &navs,
            &world,
            false,
            false,
        ));
        assert_eq!(*walk_ret.lock().unwrap(), Some(true));
        assert!(
            wait_until(100, || queued(&navs)
                == Some(WorldTile {
                    x: 4,
                    z: 4,
                    level: 0
                })),
            "the worker armed the toll route"
        );
        let route = navs
            .lock()
            .unwrap()
            .get("alice")
            .and_then(|b| b.route.clone())
            .expect("armed route");
        assert!(
            route.legs.iter().any(|l| matches!(
                l,
                nav::router::Leg::Transport { edge } if edge.item_req == vec![(995, 10)]
            )),
            "the route must cross the toll"
        );
    }

    /// The Rune Essence mine mapsquare (m45_75) as a sealed 64×64
    /// all-walkable bake at (2880,4800): the pad and the four exit portal
    /// placements inside, nothing packed — the session return hop is
    /// synthesized by the router, so a script walk out only arms with a
    /// latch.
    fn mine_nav_world() -> NavWorld {
        NavWorld::from_parts(
            nav::collision::WorldCollision {
                origin: WorldTile {
                    x: 2880,
                    z: 4800,
                    level: 0,
                },
                width: 64,
                height: 64,
                walk: vec![0u8; 64 * 64],
                blocked: vec![0u64; (64usize * 64).div_ceil(64)],
                flags: None,
            },
            TransportGraph::default(),
            Vec::new(),
        )
    }

    /// `ctx.walk` feeds the uid's latched essence session: a bot whose
    /// traveller already latched the mine (entered via Aubury) can walk
    /// out through the exit portal's return hop to the wizard's anchor.
    #[test]
    fn script_observe_walk_uses_the_latched_essence_session() {
        let NavRig {
            scripts,
            cheats,
            navs,
            world,
            walk_ret,
            walk_target,
            ..
        } = nav_rig_with(Some(Arc::new(mine_nav_world())));
        // Seed the bot's traveller with the latched session (the entry-hop
        // latch path itself is the traveller's own test).
        navs.lock().unwrap().insert(
            "alice".into(),
            NavBot {
                traveller: {
                    let mut t = Traveller::new();
                    t.set_essence(nav::essence::essence_session_for_wizard(553));
                    t
                },
                route: None,
                bank_fetch: None,
            },
        );
        // The walk target is Aubury's anchor; the origin is the mine pad.
        *walk_target.lock().unwrap() = (3253, 3401, 0);
        let mut d = NavRec::default();
        assert!(script_observe(
            &mut d,
            "alice",
            true,
            true,
            1,
            Some((2912, 4833, 0)),
            None,
            None,
            None,
            None,
            &scripts,
            &cheats,
            &navs,
            &world,
            false,
            false,
        ));
        assert_eq!(*walk_ret.lock().unwrap(), Some(true));
        assert!(
            wait_until(100, || queued(&navs)
                == Some(WorldTile {
                    x: 3253,
                    z: 3401,
                    level: 0
                })),
            "the worker armed the exit route with the latched session"
        );
    }

    /// No latch: the session return hop is never relaxed — the sealed
    /// mine stays NoPath for `ctx.walk` (fail-closed remains correct).
    #[test]
    fn script_observe_walk_without_a_latch_keeps_the_mine_sealed() {
        let NavRig {
            scripts,
            cheats,
            navs,
            world,
            walk_ret,
            walk_target,
            ..
        } = nav_rig_with(Some(Arc::new(mine_nav_world())));
        *walk_target.lock().unwrap() = (3253, 3401, 0);
        let mut d = NavRec::default();
        assert!(script_observe(
            &mut d,
            "alice",
            true,
            true,
            1,
            Some((2912, 4833, 0)),
            None,
            None,
            None,
            None,
            &scripts,
            &cheats,
            &navs,
            &world,
            false,
            false,
        ));
        assert_eq!(
            *walk_ret.lock().unwrap(),
            Some(true),
            "a no-path request is queued, not found synchronously"
        );
        thread::sleep(Duration::from_millis(50));
        assert_eq!(
            queued(&navs),
            None,
            "no latch -> the mine is sealed (NoPath never arms a route)"
        );
    }

    /// No slot state (no player decoded): the walk arm fails closed — the
    /// toll stays unusable and no route arms.
    #[test]
    fn script_observe_walk_falls_back_to_empty_when_slot_has_no_state() {
        let NavRig {
            scripts,
            cheats,
            navs,
            world,
            walk_ret,
            walk_target,
            ..
        } = nav_rig_with(Some(Arc::new(toll_nav_world())));
        *walk_target.lock().unwrap() = (4, 4, 0);
        let mut d = NavRec::default();
        assert!(script_observe(
            &mut d,
            "alice",
            true,
            true,
            1,
            Some((0, 0, 0)),
            None,
            None,
            None,
            None,
            &scripts,
            &cheats,
            &navs,
            &world,
            false,
            false,
        ));
        assert_eq!(*walk_ret.lock().unwrap(), Some(true), "request queued");
        thread::sleep(Duration::from_millis(20));
        assert_eq!(
            queued(&navs),
            None,
            "no facts -> the toll stays unusable, no route arms"
        );
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
            &mut d, "alice", true, true, 1, None, None, None, None, None, &scripts, &cheats, &navs,
            &world, false, false,
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
            None,
            None,
            &scripts,
            &cheats,
            &navs,
            &no_world,
            false,
            false,
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
            None,
            None,
            &scripts,
            &cheats,
            &navs,
            &world,
            false,
            false,
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
            None,
            None,
            &scripts,
            &cheats,
            &navs,
            &world,
            false,
            false,
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
            None,
            None,
            &scripts,
            &cheats,
            &navs,
            &world,
            false,
            false,
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
