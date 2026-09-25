//! `host-play`: run vaulted profiles through the host kernel. The binary
//! unlocks a vault and runs the named profiles; the `e2e` harness links
//! this library so it can poll per-slot state instead of scraping logs.

#![recursion_limit = "256"]

pub mod audio;
pub mod cache;
pub mod catalog_core;
pub mod external_loader;
pub mod login_readiness;
pub mod nav_identity;
pub mod paired_core;
pub mod profile;
pub mod progress;
pub mod public_worlds;
pub use nav_identity::{
    bundled_nav_identities, install_resource_root, BundledNavIdentity, NavFlagsOrigin,
    NavLoadCounters, NavOrigin,
};
pub use profile::{
    parse_profile_args, parse_revision, ProfileOptions, ProfileSelection, ServerProfile,
    WorldMembersFact, WorldMembersSource,
};

use std::collections::{HashMap, HashSet, VecDeque};
use std::panic::AssertUnwindSafe;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use api::interact::Driver;
use client::client::client::SessionExitObservation;
use client::client::Client;
use client::client::ClientConfig;
use client::client::LoginError;
use client::config::{Cache, IfType, IfTypeMut};
use client::io::JagFile;
use client::BotTarget;
pub use host::debug_enabled;
use host::login_queue::{LoginBackoff, LoginQueue, Permit, QueuePos};
use parking_lot::Mutex as QueueMutex;
use host::prepare_client;
pub use host::set_debug;
pub use host::Host;
/// The random-event guardian's published status (see [`SlotStatus::random`]
/// — the chrome contract both the panel and the TUI bind).
pub use host::{RandomClaim, RandomStatus};
use rand_core::{OsRng, RngCore};
mod rss;
mod scatter;
mod script_runtime;
mod play_bootstrap;
mod play_login;
mod play_status;
mod play_wires;
mod walk_plan;
pub use walk_plan::PendingBankFetch;
use walk_plan::{route_or_bank_fetch, RouteOutcome};
pub use play_bootstrap::{
    default_pack_path, default_vault_path, default_vault_path_for, default_vault_rel,
    default_world_host, is_loopback_host, live_vault_passphrase, live_vault_passphrase_for,
    mint_live_entries, mint_live_entries_for_target, mint_live_names, open_vault,
    play_endpoint_for, profile_password, profile_password_for, run, run_channels,
    run_prepared_template, run_with_io, run_with_profile, run_with_template, validate_play_host,
    world_host_for_bot_target, PlayOptions, ProfilePlayOptions, SharedClientTemplate,
    ValidatedTemplate,
};
use play_bootstrap::{bot_client_config, PlayConnection};
use play_login::{
    apply_queue_wait, configure_slot_world, drop_queue_place, enqueue_queue_place,
    granted_permit_may_start_login, granted_permit_world_is_current, login_and_acknowledge_permit,
    login_retry_wait, on_login_success, permit_wait_cancelled, publish_login_latched,
    publish_login_latched_from_arm, refresh_slot_world_preference, retire_queue_place,
    should_handshake, sync_profile_arm, tick_flags, wait_for_permit, wait_for_transfer_response,
    GrantedReservation, PermitWait, QueuePlaceRetirement, SharedLoginQueue,
};
pub use play_login::SlotArm;
use play_status::{
    apply_startup_phase, clear_startup_progress, mark_login_started,
    publish_session_boundary_status, publish_slot_disconnected, publish_slot_observation_reset,
    publish_startup_phase, publish_startup_progress, record_login_error, reset_slot_observation,
    set_startup_phase, startup_phase_after_observation,
};
pub use play_status::{
    copy_stream_bytes, owned_terminal_startup_error, player_here_tile, player_world_tile,
    SlotStatus, StartupPhase,
};
use play_wires::dispatch_wires;
pub use play_wires::WireCmd;
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
pub use rss::{count_tcp_to, current_resident_bytes, parse_lsof_established, sample_process};
pub use scatter::{scatter_tile_for, tele_args};
use script_runtime::*;


/// Per-slot hook invoked by the slot thread after every mainloop pass.
/// Per-frame hook: `(client, username, hold)`. `hold` is the guardian's
/// published hold from the previous frame (same lag as `step_nav_bot`) —
/// panel/TUI skip scenario follow and WalkArm follow while it is set.
type SlotFrame = Arc<dyn Fn(&mut Client, &str, bool) + Send + Sync>;
use script::{ScriptCtx, SlotScript};
use vault::{Profile, Vault, VaultError};

/// Slot thread stack: 1 MiB (the Java client thread default).
const THREAD_STACK: usize = 1024 * 1024;




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
#[allow(clippy::too_many_arguments)]
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




/// Per-slot nav latch key: the `(player gen, here)` pair the pump last
/// pump last stepped. The step is skipped until either half changes, so a
/// hop is sent once per server tick, not every 20 ms frame (panel
/// `tick_latch`).
type NavStepKey = (u64, Option<(i32, i32, i32)>);


/// Public WalkArm BankBudget pump (panel / TUI follow path). Same step
/// semantics as the script [`NavBot`] pump.
pub fn step_walk_arm_bank_fetch<D: Driver>(
    driver: &mut D,
    snapshot: &GameSnapshot,
    arm: &mut WalkArm,
    world: Option<&NavWorld>,
    here: Option<(i32, i32, i32)>,
    map_members: bool,
) -> bool {
    // Reuse NavBot stepping by temporarily viewing the arm as the same
    // shape of pending session + route.
    let mut bot = NavBot {
        traveller: Traveller::default(),
        route: arm.route.clone(),
        bank_fetch: arm.bank_fetch.take(),
        allow_teleports: false,
        ..Default::default()
    };
    let wrote = step_bank_fetch_on_bot(driver, snapshot, &mut bot, world, here, map_members);
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
        allow_teleports: false,
        ..Default::default()
    })
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
#[cfg(test)]
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


/// Cloneable overlay source for a catalog Traveller (`InteractReq::Walk`).
/// The panel paints this when WalkTo's [`WalkArm`] is idle so a script
/// walk shows the same path / click as a picker walk.
#[derive(Clone)]
pub struct ScriptNavPaint {
    navs: Arc<Mutex<HashMap<String, NavBot>>>,
}

impl ScriptNavPaint {
    /// Armed route and current hop aim for `name`, if any.
    pub fn of(&self, name: &str) -> (Option<Route>, Option<WorldTile>) {
        match self.navs.lock().unwrap().get(name) {
            Some(b) => (b.route.clone(), b.traveller.current_aim()),
            None => (None, None),
        }
    }
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
    connection: PlayConnection,
    auto_world: Option<u16>,
    /// Generated facts only when the profile cache matches a checked-in asset.
    game_data: Option<Arc<api::game_data::SelectedGameData>>,
    /// Bound-world named bank aliases, resolved once with the nav world and
    /// the catalog alias configuration (`script::content::BANK_ALIASES`).
    named_banks: Arc<api::named_banks::NamedBankFacts>,
    cache: Arc<Cache>,
    /// The shared obj-id → name table every script ctx resolves `has_item`
    /// against (built once from `cache.objs`).
    obj_names: Arc<api::obj_names::ObjNames>,
    /// Dormant unless a visible catalog proof explicitly configures it.
    /// Slot threads feed it from the same snapshot publication used by scripts.
    catalog_core: catalog_core::CoreWatch,
    /// Dormant unless a visible pair proof explicitly configures it.
    paired_core: paired_core::PairWatch,
    ifaces: Arc<Vec<Option<Box<IfType>>>>,
    ifaces_mut_template: Arc<Vec<Option<Arc<IfTypeMut>>>>,
    queue: SharedLoginQueue,
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
    scripts: ScriptWall,
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

/// Cloneable isolate-start handle for slot-thread live pumps that cannot
/// hold `&Play` (the per-frame hook is built before `Play` is stored).
#[derive(Clone)]
pub struct ScriptStartHandle {
    scripts: ScriptWall,
    game_data: Option<Arc<api::game_data::SelectedGameData>>,
    named_banks: Arc<api::named_banks::NamedBankFacts>,
}

impl ScriptStartHandle {
    /// Start a loaded JS bot on `name`'s slot. Same isolate spawn as
    /// [`Play::script_start_load`], without the control-thread wake
    /// (the slot thread is already pumping). Operator loadouts come from
    /// the default store; harness scenario Start uses
    /// [`Self::start_load_with_loadouts`] when the scenario owns fixture
    /// loadouts.
    pub fn start_load(
        &self,
        name: &str,
        source: String,
        shape: script::LoadShape,
        settings_bag: Option<serde_json::Map<String, serde_json::Value>>,
        siblings: Vec<(String, String)>,
    ) -> Result<(), String> {
        if debug_enabled() {
            eprintln!("[script {name}] start load");
        }
        let result = script_slot_or_insert(&self.scripts, name)
            .lock()
            .unwrap()
            .start_load_with_settings_and_game_data(
                source,
                shape,
                settings_bag.as_ref(),
                siblings,
                self.game_data.clone(),
                Arc::clone(&self.named_banks),
            );
        if let Err(e) = &result {
            eprintln!("[script {name}] start failed: {e}");
        }
        result
    }

    /// Harness catalog Start with caller-owned loadouts. Uses the existing
    /// explicit-loadout isolate start, then posts `settings_bag` on success.
    /// Does not inspect loadout names and does not affect
    /// [`Play::script_start_load_typed`].
    pub fn start_load_with_loadouts(
        &self,
        name: &str,
        source: String,
        shape: script::LoadShape,
        settings_bag: Option<serde_json::Map<String, serde_json::Value>>,
        siblings: Vec<(String, String)>,
        loadouts: &[script::Loadout],
    ) -> Result<(), String> {
        if debug_enabled() {
            eprintln!("[script {name}] start load");
        }
        let slot = script_slot_or_insert(&self.scripts, name);
        let mut slot = slot.lock().unwrap();
        let result = slot.start_load_with_loadouts_and_game_data(
            source,
            shape,
            siblings,
            loadouts,
            self.game_data.clone(),
            Arc::clone(&self.named_banks),
        );
        if result.is_ok() {
            if let Some(bag) = settings_bag.as_ref() {
                slot.post_settings_bag(bag);
            }
        }
        if let Err(e) = &result {
            eprintln!("[script {name}] start failed: {e}");
        }
        result
    }

    /// Start a compiled registry card on `name`'s slot. Same constructor as
    /// [`Play::script_start`], without the control-thread wake (the slot
    /// thread is already pumping).
    pub fn start_compiled(&self, name: &str, id: script::CompiledId) -> Result<(), String> {
        if debug_enabled() {
            eprintln!("[script {name}] start compiled {}", id.0);
        }
        let make = script::factory(id).ok_or_else(|| format!("not ported: {}", id.0))?;
        let result = script_slot_or_insert(&self.scripts, name)
            .lock()
            .unwrap()
            .start_compiled(make(), self.game_data.clone());
        if let Err(e) = &result {
            eprintln!("[script {name}] start failed: {e}");
        }
        result
    }

    /// `name`'s latest Start, read atomically (see [`Play::script_poll_start`]).
    pub fn poll_start(&self, name: &str) -> script::StartPoll {
        poll_start(&self.scripts, name)
    }
}

impl Play {
    /// The immutable process profile, absent only for the legacy 274 entry.
    pub fn server_profile(&self) -> Option<&Arc<ServerProfile>> {
        self.connection.profile()
    }

    /// WORLD membership bound to this process profile. Unknown is false.
    pub fn map_members(&self) -> bool {
        self.connection
            .profile()
            .map(|p| p.map_members())
            .unwrap_or(false)
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
        let owner = self.arms.get(name).map(|arm| arm.queue_owner);
        self.queue.lock().set_preferred_owner(owner);
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

    pub fn game_data(&self) -> Option<Arc<api::game_data::SelectedGameData>> {
        self.game_data.clone()
    }

    pub fn named_banks(&self) -> Arc<api::named_banks::NamedBankFacts> {
        Arc::clone(&self.named_banks)
    }

    /// Overlay handle for catalog `walk` / `ctx.walk` Traveller (Play's
    /// per-uid NavBot). WalkTo's [`WalkArm`] map is a different latch.
    pub fn script_nav_paint(&self) -> ScriptNavPaint {
        ScriptNavPaint {
            navs: Arc::clone(&self.navs),
        }
    }

    /// Kick one slot's parked frame loop. Generic UI/script wakes do not
    /// shorten protocol retry deadlines.
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

    /// Shared headed catalog proof handle. It is disabled by default.
    pub fn catalog_core_watch(&self) -> catalog_core::CoreWatch {
        self.catalog_core.clone()
    }

    /// Shared headed pair proof handle. It is disabled by default.
    pub fn paired_core_watch(&self) -> paired_core::PairWatch {
        self.paired_core.clone()
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

    /// Keep vault credentials for a later [`Play::spawn_slot`] / reconnect,
    /// and publish handshake-time settings to an already-running slot.
    pub fn remember_profile(&mut self, profile: Profile) {
        if let Some(arm) = self.arms.get(&profile.username) {
            sync_profile_arm(arm, &profile);
        }
        self.profiles.insert(profile.username.clone(), profile);
    }

    /// Give `name` focused priority. Existing membership moves to the front;
    /// otherwise the preference is remembered until this worker reaches
    /// Queueing and creates its own place.
    pub fn prefer_login(&self, name: &str) {
        let Some(arm) = self.arms.get(name) else {
            return;
        };
        let mut q = self.queue.lock();
        q.prefer_owner(arm.queue_owner);
        let pos = q.status_owner(arm.queue_owner);
        apply_queue_wait(&mut self.statuses.lock().unwrap(), name, pos);
    }

    /// Snapshot of the worker-owned login FIFO (front first).
    pub fn login_queue_uids(&self) -> Vec<i32> {
        self.queue.lock().queued_uids()
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
    ///
    /// The compiled card is started with this Play's own selected-revision
    /// pin — the same facts a Load isolate is spawned with.
    pub fn script_start(&self, name: &str, id: script::CompiledId) -> Result<(), String> {
        if !self.slot_active(name) {
            return Err(format!("no slot: {name}"));
        }
        let make = script::factory(id).ok_or_else(|| format!("not ported: {}", id.0))?;
        script_slot_or_insert(&self.scripts, name)
            .lock()
            .unwrap()
            .start_compiled(make(), self.game_data.clone())?;
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
        settings_bag: Option<serde_json::Map<String, serde_json::Value>>,
        siblings: Vec<(String, String)>,
    ) -> Result<(), String> {
        self.script_start_load_typed(name, source, shape, settings_bag, siblings)
            .map_err(|e| e.to_string())
    }

    /// Initial load result with operational refusals separate from loader errors.
    pub fn script_start_load_typed(
        &self,
        name: &str,
        source: String,
        shape: script::LoadShape,
        settings_bag: Option<serde_json::Map<String, serde_json::Value>>,
        siblings: Vec<(String, String)>,
    ) -> Result<(), script::StartLoadError> {
        if !self.slot_active(name) {
            return Err(script::StartLoadError::Refused(format!("no slot: {name}")));
        }
        if debug_enabled() {
            eprintln!("[script {name}] start load");
        }
        let result = script_slot_or_insert(&self.scripts, name)
            .lock()
            .unwrap()
            .start_load_with_settings_and_game_data_typed(
                source,
                shape,
                settings_bag.as_ref(),
                siblings,
                self.game_data.clone(),
                Arc::clone(&self.named_banks),
            );
        if let Err(e) = &result {
            eprintln!("[script {name}] start failed: {e}");
        }
        result?;
        self.wake(name);
        Ok(())
    }

    /// Cloneable isolate-start handle for slot-thread live pumps that
    /// cannot hold `&Play`.
    pub fn script_start_handle(&self) -> ScriptStartHandle {
        ScriptStartHandle {
            scripts: Arc::clone(&self.scripts),
            game_data: self.game_data.clone(),
            named_banks: Arc::clone(&self.named_banks),
        }
    }

    /// Pause `name`'s script (operator Pause; survives login until
    /// Resume re-arms it). No-op when the slot has no script.
    pub fn script_pause(&self, name: &str) {
        if let Some(slot) = script_slot(&self.scripts, name) {
            let abort = slot.lock().unwrap().pause();
            if abort {
                abort_script_walk(&self.navs, name);
            }
        }
        self.wake(name);
    }

    /// Resume `name`'s script; the next `on_is_up` re-gates it.
    pub fn script_resume(&self, name: &str) {
        if let Some(slot) = script_slot(&self.scripts, name) {
            slot.lock().unwrap().resume();
        }
        self.wake(name);
    }

    /// Stop `name`'s script: teardown hook, instance dropped, Idle.
    pub fn script_stop(&self, name: &str) {
        if let Some(slot) = script_slot(&self.scripts, name) {
            slot.lock().unwrap().stop();
        }
        self.wake(name);
    }

    pub fn script_attach_identity(&self, name: &str, identity: impl Into<String>) {
        if let Some(slot) = script_slot(&self.scripts, name) {
            slot.lock().unwrap().attach_source_identity(identity);
        }
    }

    pub fn script_runtime_generation(&self, name: &str) -> Option<u64> {
        script_slot(&self.scripts, name).map(|slot| slot.lock().unwrap().runtime_generation())
    }

    pub fn script_source_identity(&self, name: &str) -> Option<String> {
        script_slot(&self.scripts, name)
            .and_then(|slot| slot.lock().unwrap().source_identity().map(str::to_string))
    }

    pub fn script_post_settings_fenced(
        &self,
        name: &str,
        bag: &serde_json::Map<String, serde_json::Value>,
        identity: &str,
        generation: u64,
    ) -> bool {
        let Some(slot) = script_slot(&self.scripts, name) else {
            return false;
        };
        let accepted = slot
            .lock()
            .unwrap()
            .post_settings_bag_fenced(bag, identity, generation);
        accepted
    }

    /// One-shot script-local paint button for `name`. No-op when there is
    /// no slot, the slot is not Running, `id` is empty, `generation` does
    /// not match the last forwarded frame, or that frame does not advertise
    /// `id`. Never walks, pauses, or stops.
    pub fn script_paint_click(&self, name: &str, id: &str, generation: u64) {
        if id.is_empty() {
            return;
        }
        let Some(slot) = script_slot(&self.scripts, name) else {
            return;
        };
        let slot = slot.lock().unwrap();
        if slot.state() != script::RunState::Running {
            return;
        }
        let Some(paint) = slot.paint() else {
            return;
        };
        if paint.generation != generation {
            return;
        }
        if !paint.buttons.iter().any(|b| b.id == id) {
            return;
        }
        slot.paint_click(id);
    }

    /// Persistent strip/rail/tabs selection for `name`. No-op when there is
    /// no slot, the slot is not Running, `key` or `name` is empty,
    /// `generation` does not match the last forwarded frame, or that frame
    /// does not advertise `name` under `key`. Never walks, pauses, or stops.
    pub fn script_paint_select(&self, name: &str, key: &str, select_name: &str, generation: u64) {
        if key.is_empty() || select_name.is_empty() {
            return;
        }
        let Some(slot) = script_slot(&self.scripts, name) else {
            return;
        };
        let slot = slot.lock().unwrap();
        if slot.state() != script::RunState::Running {
            return;
        }
        let Some(paint) = slot.paint() else {
            return;
        };
        if paint.generation != generation {
            return;
        }
        if !script_runtime::script_paint_select_advertised(&paint, key, select_name) {
            return;
        }
        slot.paint_select(key, select_name);
    }

    /// `name`'s script lifecycle state; `Idle` when the slot has none.
    pub fn script_state(&self, name: &str) -> script::RunState {
        script_slot(&self.scripts, name)
            .map(|slot| slot.lock().unwrap().state())
            .unwrap_or(script::RunState::Idle)
    }

    /// Resolve `name`'s non-blocking isolate Start/Stop. The slot thread
    /// does this every observe; a slot that is offline or queued for login
    /// has no observe, so the UI pumps it (see [`Self::pump_script_lifecycles`]).
    pub fn pump_script_lifecycle(&self, name: &str) {
        if let Some(slot) = script_slot(&self.scripts, name) {
            if let Ok(mut slot) = slot.lock() {
                slot.observe_lifecycle();
            }
        }
    }

    /// Resolve every slot's pending Start/Stop once. Called per UI frame by
    /// the panel and the TUI. A slot its own thread holds is skipped: that
    /// thread is observing it right now.
    pub fn pump_script_lifecycles(&self) {
        let slots: Vec<_> = self.scripts.lock().unwrap().values().cloned().collect();
        for slot in slots {
            if let Ok(mut slot) = slot.try_lock() {
                slot.observe_lifecycle();
            }
        }
    }

    /// `name`'s latest operator Load Start: Start returns before V8 setup,
    /// so the assignment and the load diagnostic are committed from the
    /// settled outcome, not from Start's `Ok`. The lifecycle observe, the
    /// outcome take and the in-flight check happen under one slot lock, so
    /// the slot thread's own observe can never settle it unseen between
    /// them. A removed slot owes nothing.
    pub fn script_poll_start(&self, name: &str) -> script::StartPoll {
        poll_start(&self.scripts, name)
    }

    #[cfg(feature = "memory-profile")]
    pub fn memory_script_metrics(&self, name: &str) -> Option<serde_json::Value> {
        script_slot(&self.scripts, name).and_then(|slot| slot.lock().unwrap().memory_metrics())
    }

    #[cfg(feature = "memory-profile")]
    pub fn memory_script_progress(&self, name: &str) -> serde_json::Value {
        script_slot(&self.scripts, name)
            .map(|slot| slot.lock().unwrap().memory_progress())
            .unwrap_or(serde_json::Value::Null)
    }

    /// `name`'s script `last_error`; `None` when the slot has none.
    pub fn script_last_error(&self, name: &str) -> Option<String> {
        script_slot(&self.scripts, name)
            .and_then(|slot| slot.lock().unwrap().last_error().map(str::to_string))
    }

    /// Latest bounded ScriptRunner.stop receipt. This is non-consuming and
    /// independent of [`Self::script_take_pending_logs`].
    pub fn script_lifecycle_receipt(&self, name: &str) -> Option<script::ScriptLifecycleReceipt> {
        script_slot(&self.scripts, name).and_then(|slot| slot.lock().unwrap().lifecycle_receipt())
    }

    /// Isolate log lines staged since the last take (panel log pane).
    pub fn script_take_pending_logs(&self, name: &str) -> Vec<String> {
        let Some(slot) = script_slot(&self.scripts, name) else {
            return Vec::new();
        };
        // A slot its own thread holds keeps its lines for the next frame;
        // a poisoned slot is a bug and still panics, as `lock` did.
        let mut slot = match slot.try_lock() {
            Ok(slot) => slot,
            Err(std::sync::TryLockError::WouldBlock) => return Vec::new(),
            Err(std::sync::TryLockError::Poisoned(e)) => panic!("script slot poisoned: {e}"),
        };
        slot.take_pending_logs()
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
            arm.notify_retry_wait();
            let mut queue = self.queue.lock();
            queue.leave_owner(arm.queue_owner);
        }
        self.spawned.remove(name);
        self.statuses.lock().unwrap().retain(|s| s.username != name);
        self.arms.remove(name);
        // Release the wall lock before the slot lock (wall-then-slot order).
        let removed = self.scripts.lock().unwrap().remove(name);
        if let Some(slot) = removed {
            slot.lock().unwrap().stop();
        }
        self.cheats.lock().unwrap().remove(name);
        self.wires.lock().unwrap().remove(name);
        if self.focused.as_deref() == Some(name) {
            self.focused = None;
            self.queue.lock().set_preferred_owner(None);
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
        if let Err(error) = self.try_spawn_slot(profile, input, mailbox, arm) {
            eprintln!("[host-play] {error}");
        }
    }

    pub fn try_spawn_slot(
        &mut self,
        profile: Profile,
        input: Option<Arc<SlotInput>>,
        mailbox: Option<Arc<FrameBuf>>,
        arm: Option<Arc<SlotArm>>,
    ) -> Result<(), String> {
        self.connection.require_bot_operation()?;
        let world_round = self
            .connection
            .profile()
            .and_then(|p| p.public_worlds().map(|worlds| (p, worlds)))
            .map(|(bound, worlds)| {
                let default = self.auto_world.or_else(|| {
                    worlds
                        .by_endpoint(bound.client().game_host(), bound.client().game_port())
                        .map(|world| world.number)
                });
                public_worlds::WorldRound::new(worlds, profile.settings.world, default)
            })
            .transpose()?;
        // Keep the vault credentials on the wall for later spawns and
        // publish a changed world preference to an existing slot before
        // the already-spawned early return.
        self.remember_profile(profile.clone());
        if !self.spawned.insert(profile.username.clone()) {
            return Ok(());
        }
        let arm = arm.unwrap_or_else(|| SlotArm::new(profile.uid, true));
        // Store through the shared inner fields so a caller's own clone
        // cannot retain stale profile settings.
        sync_profile_arm(&arm, &profile);
        self.arms.insert(profile.username.clone(), Arc::clone(&arm));
        // The control wake: `Play::wake` kicks the parked slot thread on
        // focus/draw/stop/spawn changes; the slot thread polls the park end.
        let (wake, park) = wake_channel();
        self.wakes.insert(profile.username.clone(), wake);
        spawn_slot_thread(
            &self.connection,
            profile,
            world_round,
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
            self.catalog_core.clone(),
            self.paired_core.clone(),
            Arc::clone(&self.per_frame),
            &mut self.handles,
        );
        Ok(())
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



/// End one connected session without carrying deferred game actions into the
/// next login. Operator intent survives (`on_is_up(false)` pauses a started
/// script); packets, isolate interactions, route workers and navigation state
/// belong to the disconnected session and are discarded.
fn reset_slot_session_work(
    name: &str,
    scripts: &ScriptWall,
    cheats: &Arc<Mutex<HashMap<String, VecDeque<String>>>>,
    wires: &Arc<Mutex<HashMap<String, VecDeque<WireCmd>>>>,
    navs: &Arc<Mutex<HashMap<String, NavBot>>>,
) {
    if let Some(slot) = script_slot(scripts, name) {
        let mut slot = slot.lock().unwrap();
        slot.reset_session_work();
    }
    if let Some(queue) = cheats.lock().unwrap().get_mut(name) {
        queue.clear();
    }
    if let Some(queue) = wires.lock().unwrap().get_mut(name) {
        queue.clear();
    }
    reset_script_nav(navs, name);
}


/// Compact prior-frame guardian fact for catalog proof. The observe hook
/// receives last frame's `client_frame` `RandomStatus`.
fn bounded_guardian_fact(status: &RandomStatus) -> catalog_core::BoundedGuardian {
    catalog_core::BoundedGuardian {
        kind: status.kind.map(|kind| {
            match kind {
                api::random::RandomKind::Dialog => "dialog",
                api::random::RandomKind::Pick => "pick",
                api::random::RandomKind::Evade => "evade",
                api::random::RandomKind::Maze => "maze",
                api::random::RandomKind::Mime => "mime",
                api::random::RandomKind::Box => "box",
                api::random::RandomKind::Lamp => "lamp",
                api::random::RandomKind::Hazard => "hazard",
                api::random::RandomKind::LostTool => "lost_tool",
                api::random::RandomKind::LostGear => "lost_gear",
            }
            .to_string()
        }),
        name: status.name.clone(),
        ours: status.ours,
        hold: status.hold,
    }
}

/// Catalog/paired observer hook shared by the slot pump. Lifecycle and guardian
/// facts are produced only while the catalog watch is configured; paired
/// snapshot conversion runs only while the pair watch is configured.
#[allow(clippy::too_many_arguments)]
fn observe_slot_catalog_and_paired(
    catalog: &catalog_core::CoreWatch,
    paired: &paired_core::PairWatch,
    account: &str,
    snapshot: &GameSnapshot,
    names: &api::obj_names::ObjNames,
    session_boundary: bool,
    lifecycle_receipt: impl FnOnce() -> Option<script::ScriptLifecycleReceipt>,
    guardian_fact: impl FnOnce() -> catalog_core::BoundedGuardian,
    inspect: Option<catalog_core::RouteInspectPublished>,
    paint: Option<&script::shim::ScriptPaint>,
    acts: Option<catalog_core::ScriptActsPublished>,
) {
    if catalog.configured() {
        let script_lifecycle = lifecycle_receipt();
        let guardian = if session_boundary {
            catalog_core::BoundedGuardian::default()
        } else {
            guardian_fact()
        };
        catalog.observe_snapshot_with_lifecycle(
            account,
            snapshot,
            names,
            script_lifecycle,
            guardian,
            session_boundary,
            inspect,
            paint,
            acts,
        );
    }
    if paired.configured() {
        paired.observe_snapshot(account, snapshot, session_boundary);
    }
}


/// Every profile spawns one slot thread; shared handles are threaded through
/// because the closure moves most of them (allowed: see `script_observe`).
#[allow(clippy::too_many_arguments)]
fn spawn_slot_thread(
    connection: &PlayConnection,
    profile: Profile,
    mut world_round: Option<public_worlds::WorldRound>,
    slot_input: Option<Arc<SlotInput>>,
    slot_mailbox: Option<Arc<FrameBuf>>,
    park: Option<SlotPark>,
    arm: Arc<SlotArm>,
    slot_cache: Arc<Cache>,
    ifaces_template: Arc<Vec<Option<Box<IfType>>>>,
    ifaces_mut_template: Arc<Vec<Option<Arc<IfTypeMut>>>>,
    slot_queue: SharedLoginQueue,
    slot_statuses: Arc<Mutex<Vec<SlotStatus>>>,
    slot_scripts: ScriptWall,
    slot_cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>>,
    slot_wires: Arc<Mutex<HashMap<String, VecDeque<WireCmd>>>>,
    slot_navs: Arc<Mutex<HashMap<String, NavBot>>>,
    slot_world: Option<Arc<NavWorld>>,
    slot_obj_names: Arc<api::obj_names::ObjNames>,
    slot_catalog_core: catalog_core::CoreWatch,
    slot_paired_core: paired_core::PairWatch,
    slot_frame: SlotFrame,
    handles: &mut HashMap<String, thread::JoinHandle<()>>,
) {
    let username = profile.username.clone();
    let uid = profile.uid;
    let password = profile.password.clone();
    let connection = connection.clone();
    let mainland = match &connection {
        PlayConnection::Legacy(options) => options.mainland,
        PlayConnection::Bound { mainland, .. } => *mainland,
    };

    handles.insert(
        username.clone(),
        thread::Builder::new()
            .name(username.clone())
            .stack_size(THREAD_STACK)
            .spawn(move || {
            let _queue_retirement = QueuePlaceRetirement {
                queue: &slot_queue,
                statuses: &slot_statuses,
                username: &username,
                arm: &arm,
            };
            {
                // Publish the row before `prepare_client`/`maininit`
                // (a slow cache fetch can stall for seconds), so the
                // queue card shows the slot while it loads.
                let mut all = slot_statuses.lock().unwrap();
                all.push(SlotStatus {
                    username: username.clone(),
                    world: world_round.as_ref().and_then(|round| connection.profile()
                        .and_then(|p| p.public_worlds())
                        .map(|worlds| worlds.worlds[round.index].number)),
                    ..SlotStatus::default()
                });
            }
            let slot_script = script_slot_or_insert(&slot_scripts, &username);
            let slot_input = slot_input.unwrap_or_else(SlotInput::new);
            {
                let mut slot_script = slot_script.lock().unwrap();
                slot_script.bind_native_input(slot_input.authority());
            }
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
            // Preparation has no determinate client percentage, but publish
            // a phase immediately so a slow cache fetch is visibly active.
            publish_startup_phase(&slot_statuses, &username, "Preparing client");
            // The park end survives re-login rounds (run_client is entered
            // once per ingame stretch), so wrap it once here.
            let park = park.map(Arc::new);
            let mut client = match &connection {
                PlayConnection::Legacy(options) => prepare_client(
                    bot_client_config(options, &profile), uid, Arc::clone(&slot_cache),
                    ifaces_template.clone(), ifaces_mut_template.clone(),
                ),
                PlayConnection::Bound { template, .. } => match template.prepare_client(uid, profile.settings.lowmem) {
                    Ok(client) => client,
                    Err(error) => {
                        if let Some(row) = slot_statuses.lock().unwrap().iter_mut().find(|s| s.username == username) {
                            row.startup_phase = StartupPhase::Error;
                            row.startup_phase_started = Instant::now();
                            row.error = Some(error);
                        }
                        clear_startup_progress(&slot_statuses, &username);
                        return;
                    }
                },
            };
            // The host owns every reconnect attempt so each fresh socket
            // returns through the shared FIFO and reservation accounting.
            client.set_external_reconnect_owner(true);
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
            client.maininit_with_progress(Some(&mut |_, message, percent| {
                publish_startup_progress(&slot_statuses, &username, message, percent);
            }));
            if client.error_loading && connection.profile().is_some() {
                if let Some(row) = slot_statuses.lock().unwrap().iter_mut().find(|s| s.username == username) {
                    row.startup_phase = StartupPhase::Error;
                    row.startup_phase_started = Instant::now();
                    row.error = Some(format!("profile asset initialization failed: {}", client.last_progress_message));
                }
                clear_startup_progress(&slot_statuses, &username);
                return;
            }
            clear_startup_progress(&slot_statuses, &username);
            set_startup_phase(&slot_statuses, &username, StartupPhase::Queueing);
            if client.error_loading && debug_enabled() {
                eprintln!("[host-play] slot {username}: maininit failed");
            }

            let mut backoff = LoginBackoff::new();
            let mut world_dirty = world_round.is_some();
            let mut refresh_key = false;
            let mut key_refreshed = false;
            let mut script_tick: u64 = 0;
            loop {
                if arm.stop.load(Ordering::Relaxed) {
                    return;
                }
                if !client.ingame {
                    if !should_handshake(&arm, client.ingame) {
                        // No pending intent (title hold, latched logout, or a
                        // withdrawn wait): a parked slot holds no FIFO place
                        // and publishes no `k of n`.
                        drop_queue_place(
                            &slot_queue,
                            &slot_statuses,
                            &username,
                            arm.queue_owner,
                        );
                        publish_login_latched_from_arm(&slot_statuses, &username, &arm);
                        thread::sleep(Duration::from_millis(20));
                        continue;
                    }
                    // Leaving title park for handshake: refresh latch from the
                    // arm so an explicit Log in cannot keep a stale TRUE from
                    // the last park publish through queue/connect.
                    publish_login_latched_from_arm(&slot_statuses, &username, &arm);
                    enqueue_queue_place(&slot_queue, &slot_statuses, &username, uid, &arm);
                    if let Some(round) = world_round.as_mut() {
                        let worlds = connection
                            .profile()
                            .and_then(|p| p.public_worlds())
                            .expect("public world round requires bound public worlds");
                        match refresh_slot_world_preference(round, worlds, &arm) {
                            Ok(true) => {
                                world_dirty = true;
                                refresh_key = false;
                                key_refreshed = false;
                            }
                            Ok(false) => {}
                            Err(error) => {
                                if let Some(row) = slot_statuses
                                    .lock()
                                    .unwrap()
                                    .iter_mut()
                                    .find(|s| s.username == username)
                                {
                                    row.startup_phase = StartupPhase::Error;
                                    row.startup_phase_started = Instant::now();
                                    row.error = Some(format!(
                                        "public world preference failed: {error}"
                                    ));
                                }
                                arm.withdraw_login();
                                continue;
                            }
                        }
                    }
                    if world_dirty {
                        let round = world_round.as_ref().expect("public world round");
                        let worlds = connection.profile().and_then(|p| p.public_worlds())
                            .expect("bound public worlds");
                        let world = &worlds.worlds[round.index];
                        if let Err(error) = configure_slot_world(&mut client, world, refresh_key, &arm.stop) {
                            if arm.stop.load(Ordering::Relaxed) {
                                return;
                            }
                            if let Some(row) = slot_statuses.lock().unwrap().iter_mut().find(|s| s.username == username) {
                                row.startup_phase = StartupPhase::Error;
                                row.startup_phase_started = Instant::now();
                                row.error = Some(format!("public world login configuration failed: {error}"));
                            }
                            clear_startup_progress(&slot_statuses, &username);
                            return;
                        }
                        if arm.stop.load(Ordering::Relaxed) {
                            return;
                        }
                        if let Some(row) = slot_statuses.lock().unwrap().iter_mut().find(|s| s.username == username) {
                            row.world = Some(world.number);
                        }
                        if debug_enabled() {
                            eprintln!(
                                "[host-play] slot {username}: world w{} {}:{} node {}",
                                world.number, world.host, world.port, world.node_id
                            );
                        }
                        world_dirty = false;
                        refresh_key = false;
                    }
                    let wait = wait_for_permit(&slot_queue, &slot_statuses, &username, uid, &arm);
                    if wait == PermitWait::Cancelled {
                        if arm.stop.load(Ordering::Relaxed) {
                            return;
                        }
                        continue;
                    }
                    if !granted_permit_world_is_current(
                        &slot_queue,
                        uid,
                        world_round.as_ref(),
                        &arm,
                    ) {
                        continue;
                    }
                    // A withdrawal or stop that lands after the granting poll
                    // releases the unused reservation before any login call.
                    if !granted_permit_may_start_login(
                        &slot_queue,
                        uid,
                        &arm,
                        client.ingame,
                    ) {
                        if arm.stop.load(Ordering::Relaxed) {
                            return;
                        }
                        continue;
                    }
                    let mut permit = GrantedReservation::new(&slot_queue, uid);
                    mark_login_started(&slot_statuses, &username);
                    let reconnect = arm.reconnect.load(Ordering::Relaxed);
                    if debug_enabled() {
                        eprintln!(
                            "[host-play] slot {username}: handshake begin reconnect={reconnect}"
                        );
                    }
                    let login = login_and_acknowledge_permit(&mut permit, || {
                        client.login(&username, &password, reconnect)
                    });
                    match login {
                        Ok(()) => {
                            backoff.reset();
                            if let Some(round) = world_round.as_mut() { round.reset(); }
                            key_refreshed = false;
                            on_login_success(&arm);
                            set_startup_phase(&slot_statuses, &username, StartupPhase::LoadingScene);
                            if debug_enabled() {
                                eprintln!("[host-play] slot {username}: handshake ok");
                            }
                        }
                        Err(e) => {
                            if wait_for_transfer_response(
                                &e,
                                &arm,
                                &slot_statuses,
                                &username,
                            )
                            .is_some()
                            {
                                continue;
                            }
                            record_login_error(&slot_statuses, &username, &e);
                            let decision = world_round.as_mut().map(|round| {
                                let count = connection.profile().and_then(|p| p.public_worlds())
                                    .expect("bound public worlds").worlds.len();
                                round.on_login_error(e.code, count)
                            });
                            match decision {
                                Some(public_worlds::WorldErrorStep::SwitchNow) => {
                                    world_dirty = true;
                                    key_refreshed = false;
                                    continue;
                                }
                                Some(public_worlds::WorldErrorStep::SwitchAfterWait) => {
                                    world_dirty = true;
                                    key_refreshed = false;
                                }
                                _ => {}
                            }
                            if world_round.is_some()
                                && public_worlds::refresh_after_login_error(e.code, &mut key_refreshed)
                            {
                                refresh_key = true;
                                world_dirty = true;
                            }
                            let retry = login_retry_wait(&mut backoff, e.code);
                            if e.code == 16 {
                                slot_queue.lock().hold_for(Instant::now(), retry);
                            }
                            arm.wait_for_retry(retry);
                            continue;
                        }
                    }
                }
                let mut mainland_sent = false;
                let arm_obs = Arc::clone(&arm);
                let arm_latch_obs = Arc::clone(&arm_obs);
                let obs_name = username.clone();
                let obs_catalog_core = slot_catalog_core.clone();
                let obs_paired_core = slot_paired_core.clone();
                let knock_name = username.clone();
                let knock_scripts = Arc::clone(&slot_scripts);
                let knock = move |ev: &DetectedRandom| -> RandomClaim {
                    let Some(slot) = script_slot(&knock_scripts, &knock_name) else {
                        return RandomClaim::Host;
                    };
                    let mut slot = slot.lock().unwrap();
                    slot.on_random(ev)
                };
                let run_policy_override = slot_script.lock().unwrap().run_policy_override_cell();
                Host::run_client(
                    &mut client,
                    &username,
                    profile.settings.clone(),
                    Arc::clone(&arm_obs.random_events),
                    Arc::clone(&arm_obs.lamp_auto),
                    Arc::clone(&arm_obs.lamp_skill),
                    Some(slot_input.clone()),
                    slot_mailbox.clone(),
                    park.clone(),
                    run_policy_override,
                    {
                        let slot_frame = Arc::clone(&slot_frame);
                        let slot_statuses = Arc::clone(&slot_statuses);
                        let slot_scripts = Arc::clone(&slot_scripts);
                        let slot_input = Arc::clone(&slot_input);
                        let slot_cheats = Arc::clone(&slot_cheats);
                        let slot_wires = Arc::clone(&slot_wires);
                        let slot_obj_names = Arc::clone(&slot_obj_names);
                        let slot_cache = Arc::clone(&slot_cache);
                        let slot_navs = Arc::clone(&slot_navs);
                        let slot_world = slot_world.clone();
                        let slot_canlight = connection.profile().and_then(|p| p.canlight());
                        let map_members = connection
                            .profile()
                            .map(|p| p.map_members())
                            .unwrap_or(false);
                        let mut pump = Pump::new();
                        let script_tick = &mut script_tick;
                        // Last `(player gen, here)` the nav bot stepped:
                        // skip until either changes so the hop budget counts
                        // server ticks, not 20 ms frames (panel `tick_latch`).
                        let mut last_nav_step: Option<NavStepKey> = None;
                        // The slot's per-tick nav snapshot: rebuilt each
                        // observe when a family's gen moved (the walk arm
                        // gates on its facts; the follow surface reads the
                        // canonical base + route-head tile from it).
                        let mut nav_snapshot = GameSnapshot::new();
                        let mut session_epoch = 0u64;
                        let mut welcome = login_readiness::LoginReadiness::default();
                        // The random status `client_frame` published last
                        // frame: copied onto the slot status row, and its
                        // hold freezes script tick and the nav follow.
                        move |c, _ignored, run_sends, status: &RandomStatus| {
                            let name = &obs_name;
                            let drain = pump.drain_client(c);
                            let session_boundary = publish_session_boundary_status(
                                &slot_statuses,
                                name,
                                drain.session_changed,
                                c.ingame,
                                nav_snapshot.ingame(),
                            );
                            if session_boundary {
                                session_epoch = session_epoch.wrapping_add(1);
                                reset_slot_session_work(name, &slot_scripts, &slot_cheats, &slot_wires, &slot_navs);
                                last_nav_step = None;
                            }
                            host::publish_snapshot(&mut nav_snapshot, c, drain);
                            // `script_observe_with_npc_boxes` below reaps the
                            // isolate and can publish a Stop receipt. The next
                            // frame attaches that bounded value here before
                            // status publication; pending panel logs are never
                            // consumed by this read.
                            // `status` is last frame's client_frame publication.
                            // Snapshot observe runs before this frame copies it
                            // onto the slot row.
                            // The hunt cards also read the slot's own record
                            // of the requests host-play dispatched for it.
                            let copies_inspect = obs_catalog_core.copies_route_inspect();
                            let copies_hunt = obs_catalog_core.copies_hunt();
                            let (inspect, acts) = if copies_inspect || copies_hunt {
                                let navs = slot_navs.lock().unwrap();
                                let bot = navs.get(name);
                                (
                                    bot.filter(|_| copies_inspect)
                                        .map(|bot| bot.inspect.published_core_facts()),
                                    copies_hunt.then(|| {
                                        bot.map(|bot| bot.acts.published()).unwrap_or_default()
                                    }),
                                )
                            } else {
                                (None, None)
                            };
                            let catalog_paint = if obs_catalog_core.copies_line_of_sight()
                                || obs_catalog_core.copies_actor_observation()
                                || obs_catalog_core.copies_fight_field()
                                || copies_hunt
                            {
                                script_paint_of(&slot_scripts, name)
                            } else {
                                None
                            };
                            observe_slot_catalog_and_paired(
                                &obs_catalog_core,
                                &obs_paired_core,
                                name,
                                &nav_snapshot,
                                slot_obj_names.as_ref(),
                                session_boundary,
                                || {
                                    script_slot(&slot_scripts, name).and_then(|slot| {
                                        slot.lock().unwrap().lifecycle_receipt()
                                    })
                                },
                                || bounded_guardian_fact(status),
                                inspect,
                                catalog_paint.as_deref(),
                                acts,
                            );
                            let ready = c.ingame && c.scene_state == 2
                                && nav_snapshot.local_player().is_some();
                            let welcome_obs = login_readiness::WelcomeObservation {
                                session_epoch,
                                tick: *script_tick,
                                ingame: c.ingame,
                                scene_state: c.scene_state,
                                welcome_interface_id: c.welcome_interface_id,
                                main_modal_id: c.main_modal_id,
                                allow_close: c.ingame && c.scene_state == 2 && !session_boundary,
                            };
                            let welcome_step = welcome.step(&welcome_obs, || {
                                login_readiness::try_close_welcome(&nav_snapshot, c)
                            });
                            if let Some(line) = welcome_step.notice.as_deref() {
                                eprintln!("[host-play] slot {name}: {line}");
                            }
                            let hold = status.hold || !ready || session_boundary || welcome_step.hold;
                            #[cfg(feature = "memory-profile")]
                            memory::client_frame(c, name, hold);
                            slot_frame(c, name, hold);
                            if !mainland_sent && mainland && ready {
                                api::interact::mainland_hop(c);
                                mainland_sent = true;
                                if debug_enabled() {
                                    eprintln!("[host-play] slot {name}: queued mainland tele+setvar (scene 2)");
                                }
                            }
                            let tick_edge = should_emit_tick(drain.player_info);
                            if tick_edge {
                                *script_tick = script_tick.wrapping_add(1);
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
                                        // Keep the producer gate closed until a current
                                        // player observation can authorize game actions.
                                        s.ingame = ready;
                                        s.scene_state = nav_snapshot.scene_state();
                                        s.login_latched =
                                            arm_latch_obs.latch.load(Ordering::Relaxed);
                                        apply_startup_phase(s, name, ready, c.ingame);
                                        s.runenergy = if ready { c.runenergy } else { 0 };
                                        s.run_sends = run_sends;
                                        s.main_modal_id = nav_snapshot.modals().main;
                                        s.welcome_hold = welcome_step.hold;
                                        s.welcome_failure = welcome_step.failure.clone();
                                        if welcome_step.notice.is_some() {
                                            s.welcome_notice = welcome_step.notice.clone();
                                        }
                                        copy_stream_bytes(c, s);
                                        s.chat_head = if ready { c.chat_text[0].clone() } else { String::new() };
                                        s.random = if session_boundary { RandomStatus::default() } else { status.clone() };
                                        publish_script_paint(s, paint.clone());
                                        here = nav_snapshot.tile();
                                        let (tx, tz, level) = here.unwrap_or((-1, -1, -1));
                                        s.tile_x = tx;
                                        s.tile_z = tz;
                                        s.tile_level = level;
                                        s.player = nav_snapshot.local_player()
                                            .and_then(|p| p.player.actor.name.clone())
                                            .unwrap_or_default();
                                        up = s.is_up();
                                    }
                                }
                                (up, here)
                            };
                            let running = script_running(&slot_scripts, name);
                            let inv = observe_script_inv(running, tick_edge, &nav_snapshot);
                            let nav_armed = slot_navs.lock().unwrap().get(name).is_some_and(|b| {
                                b.route.is_some() || b.bank_fetch.is_some()
                            });
                            let nav_state = nav_world_state_for_observe(
                                here,
                                &nav_snapshot,
                                running,
                                nav_armed,
                                map_members,
                            );
                            let npc_boxes = project_npc_boxes_for_isolate_snapshot(
                                &slot_scripts,
                                name,
                                tick_edge,
                                || projected_npc_boxes(c),
                            );
                            script_observe_cached(
                                c,
                                name,
                                up,
                                tick_edge,
                                *script_tick,
                                here,
                                inv,
                                nav_state,
                                Some(&nav_snapshot),
                                npc_boxes.as_deref(),
                                Some(slot_obj_names.as_ref()),
                                &slot_scripts,
                                &slot_cheats,
                                &slot_navs,
                                &slot_world,
                                hold,
                                status.ours,
                                slot_canlight.as_deref(),
                                Some(slot_input.as_ref()),
                                Some(Arc::clone(&slot_cache)),
                                Some(Arc::clone(&slot_obj_names)),
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
                                dispatch_wires(c, &nav_snapshot, wires.into(), hold);
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
                                    && !hold
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
                                        hold,
                                        map_members,
                                        || {
                                            slot_arrival_reach(
                                                &slot_scripts,
                                                name,
                                                &nav_snapshot,
                                                here,
                                                slot_world.as_deref(),
                                                slot_canlight.as_deref(),
                                            )
                                        },
                                    );
                                }
                            }
                            // Busy flag for the idle scheduler: a slot with
                            // a running script, queued cheats, or an armed
                            // nav bot must keep ticking (never parked), so
                            // the observe hook reports it every frame.
                            running
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
                publish_slot_disconnected(&slot_statuses, &username);
                reset_slot_session_work(
                    &username,
                    &slot_scripts,
                    &slot_cheats,
                    &slot_wires,
                    &slot_navs,
                );
                if arm.stop.load(Ordering::Relaxed) {
                    return;
                }
            }
            })
            .expect("failed to spawn slot thread"),
    );
}

#[cfg(feature = "memory-profile")]
pub mod memory;

#[cfg(feature = "memory-profile")]
mod memory_diagnostics;

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
