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
#[cfg(test)]
use std::panic::AssertUnwindSafe;
#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
#[cfg(test)]
use std::time::{Duration, Instant};

#[cfg(test)]
use api::interact::Driver;
#[cfg(test)]
use client::client::Client;
#[cfg(test)]
use client::client::LoginError;
use client::config::{Cache, IfType, IfTypeMut};
pub use host::debug_enabled;
#[cfg(test)]
use host::login_queue::{LoginBackoff, LoginQueue, Permit};
#[cfg(test)]
use host::prepare_client;
pub use host::set_debug;
pub use host::Host;
/// The random-event guardian's published status (see [`SlotStatus::random`]
/// — the chrome contract both the panel and the TUI bind).
pub use host::{RandomClaim, RandomStatus};
#[cfg(test)]
use parking_lot::Mutex as QueueMutex;
mod play_bootstrap;
mod play_login;
mod play_status;
mod play_wires;
mod rss;
mod scatter;
mod script_runtime;
mod walk_arm;
mod walk_plan;
pub use walk_arm::{
    arm_walk_on, step_walk_arm_bank_fetch, walk_arm_bank_fetch_freezes_follow, NoPath, WalkArm,
    WalkArms,
};
mod play_scripts;
pub use play_scripts::{ScriptNavPaint, ScriptStartHandle};
mod play_slots;
#[cfg(test)]
use api::snapshot::{GameSnapshot, WorldTile};
#[cfg(test)]
use host::{wake_channel, DetectedRandom, Pump};
use host::{FrameBuf, SlotInput, SlotWake};
#[cfg(test)]
use nav::bank_fetch::BankStep;
#[cfg(test)]
use nav::router::{find_with, FindOptions, Route};
#[cfg(test)]
use nav::traveller::Traveller;
use nav::world::NavWorld;
#[cfg(test)]
use nav::WorldState;
#[cfg(test)]
use play_bootstrap::bot_client_config;
use play_bootstrap::PlayConnection;
pub use play_bootstrap::{
    default_pack_path, default_vault_path, default_vault_path_for, default_vault_rel,
    default_world_host, is_loopback_host, live_vault_passphrase, live_vault_passphrase_for,
    mint_live_entries, mint_live_entries_for_target, mint_live_names, open_vault,
    play_endpoint_for, profile_password, profile_password_for, run, run_channels,
    run_prepared_template, run_with_io, run_with_profile, run_with_template, validate_play_host,
    world_host_for_bot_target, PlayOptions, ProfilePlayOptions, SharedClientTemplate,
    ValidatedTemplate,
};
pub use play_login::SlotArm;
use play_login::{apply_queue_wait, sync_profile_arm, SharedLoginQueue};
#[cfg(test)]
use play_login::{
    configure_slot_world, drop_queue_place, enqueue_queue_place, granted_permit_may_start_login,
    granted_permit_world_is_current, login_and_acknowledge_permit, login_retry_wait,
    on_login_success, permit_wait_cancelled, publish_login_latched, publish_login_latched_from_arm,
    refresh_slot_world_preference, should_handshake, tick_flags, wait_for_permit,
    wait_for_transfer_response, GrantedReservation, PermitWait,
};
use play_slots::SlotFrame;
#[cfg(test)]
use play_slots::{observe_slot_catalog_and_paired, reset_slot_session_work};
use play_status::clear_startup_progress;
#[cfg(test)]
use play_status::{
    apply_startup_phase, mark_login_started, publish_session_boundary_status,
    publish_startup_phase, publish_startup_progress, record_login_error, set_startup_phase,
    startup_phase_after_observation,
};
pub use play_status::{
    copy_stream_bytes, owned_terminal_startup_error, player_here_tile, player_world_tile,
    SlotStatus, StartupPhase,
};
#[cfg(test)]
use play_wires::dispatch_wires;
pub use play_wires::WireCmd;
pub use rss::{count_tcp_to, current_resident_bytes, parse_lsof_established, sample_process};
pub use scatter::{scatter_tile_for, tele_args};
use script_runtime::*;
pub use walk_plan::PendingBankFetch;
use walk_plan::{route_or_bank_fetch, RouteOutcome};

#[cfg(test)]
use script::ScriptCtx;
use script::SlotScript;
use vault::Profile;
#[cfg(test)]
use vault::VaultError;

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
    /// Stop-requested workers awaiting a non-blocking [`JoinHandle::is_finished`]
    /// reap. UI callers never join these while they are live.
    retiring: HashMap<String, thread::JoinHandle<()>>,
    connection: PlayConnection,
    auto_world: Option<u16>,
    /// Generated facts only when the profile cache matches a checked-in asset.
    game_data: Option<Arc<api::game_data::SelectedGameData>>,
    /// Frozen roster resolved once from selected access placements and the
    /// bound walk surface; the selector and isolates share these exact rows.
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
}

/// Stop every worker before joining any of them, so one slot stuck in
/// cooperative shutdown cannot delay the stop signal for the rest.
impl Drop for Play {
    fn drop(&mut self) {
        let names: Vec<String> = self.handles.keys().cloned().collect();
        for name in &names {
            self.signal_slot_stop(name);
            self.wake(name);
        }
        for name in names {
            self.stop_slot(&name);
        }
        for (_, handle) in self.retiring.drain() {
            let _ = handle.join();
        }
    }
}

#[cfg(feature = "memory-profile")]
pub mod memory;

#[cfg(feature = "memory-profile")]
mod memory_diagnostics;

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
