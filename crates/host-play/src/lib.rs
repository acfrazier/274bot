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
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
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

/// [`client::bot_target::world_host_for`] from a `BOT_TARGET` string.
pub fn world_host_for_bot_target(target: Option<&str>) -> String {
    client::bot_target::world_host_for(client::bot_target::bot_target_from_env(target)).into()
}

/// Host and game port for a target. Prod is WSS `:443`; local is TCP `:43594`.
pub fn play_endpoint_for(target: BotTarget) -> (String, u16) {
    (
        client::world_host_for(target).into(),
        client::game_port_for(target),
    )
}

/// Active world host (`BOT_TARGET` / `--prod`).
pub fn default_world_host() -> String {
    client::world_host()
}

/// Mint `n` per-run usernames for a live boot (`live<token>_<i>`). The
/// engine auto-registers unknown names, so a minted name logs into a
/// fresh save instead of the shared `test` account. The engine enforces
/// the classic 12-character username limit. A randomly seeded counter
/// keeps consecutive runs distinct; its low base-36 digits fit the token
/// budget without truncating away the changing part of the counter.
/// Player saves accumulate under the engine's `player/` dir — wipe it to
/// reset.
pub fn mint_live_names(n: usize) -> Vec<String> {
    static NEXT: std::sync::OnceLock<std::sync::atomic::AtomicU64> = std::sync::OnceLock::new();
    let mut nonce = NEXT
        .get_or_init(|| std::sync::atomic::AtomicU64::new(OsRng.next_u64()))
        .fetch_add(1, Ordering::Relaxed);
    let slot_digits = n.saturating_sub(1).max(1).to_string().len();
    let max_token = 12usize.saturating_sub(4 + 1 + slot_digits).max(1);
    let mut token = vec![b'0'; max_token];
    for digit in token.iter_mut().rev() {
        *digit = b"0123456789abcdefghijklmnopqrstuvwxyz"[(nonce % 36) as usize];
        nonce /= 36;
    }
    let token = String::from_utf8(token).expect("base-36 token is ASCII");
    (0..n).map(|i| format!("live{token}_{i}")).collect()
}

/// High-entropy secret for prod profile passwords and live temp vaults.
fn mint_high_entropy_secret() -> String {
    let mut bytes = [0u8; 24];
    OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Default vault filename for a play target. Prod must not reuse the local
/// blob (username-as-password accounts).
pub fn default_vault_rel(target: BotTarget) -> &'static str {
    match target {
        BotTarget::Local => "vault",
        BotTarget::Prod => "vault-prod",
    }
}

/// `~/.274bot/vault` or `~/.274bot/vault-prod`.
pub fn default_vault_path_for(target: BotTarget) -> std::path::PathBuf {
    script::bot_file(default_vault_rel(target))
}

/// [`default_vault_path_for`] for the active [`client::bot_target`].
pub fn default_vault_path() -> std::path::PathBuf {
    default_vault_path_for(client::bot_target())
}

/// Profile login password for a freshly minted or missing account.
/// Local keeps username-as-password (Lost City auto-register); prod refuses it.
pub fn profile_password_for(username: &str, target: BotTarget) -> String {
    match target {
        BotTarget::Local => username.to_string(),
        BotTarget::Prod => mint_high_entropy_secret(),
    }
}

/// [`profile_password_for`] for the active [`client::bot_target`].
pub fn profile_password(username: &str) -> String {
    profile_password_for(username, client::bot_target())
}

/// Ephemeral live-vault passphrase. Local `--live` keeps the `"bot"` shim.
pub fn live_vault_passphrase_for(target: BotTarget) -> String {
    match target {
        BotTarget::Local => "bot".into(),
        BotTarget::Prod => mint_high_entropy_secret(),
    }
}

/// [`live_vault_passphrase_for`] for the active target.
pub fn live_vault_passphrase() -> String {
    live_vault_passphrase_for(client::bot_target())
}

/// `(username, password)` pairs for a live boot from minted names.
pub fn mint_live_entries_for_target(names: &[String], target: BotTarget) -> Vec<(String, String)> {
    names
        .iter()
        .map(|u| (u.clone(), profile_password_for(u, target)))
        .collect()
}

/// [`mint_live_entries_for_target`] for the active target.
pub fn mint_live_entries(names: &[String]) -> Vec<(String, String)> {
    mint_live_entries_for_target(names, client::bot_target())
}

/// Whether `host` is loopback (local engine RSA is safe).
pub fn is_loopback_host(host: &str) -> bool {
    matches!(
        host.trim()
            .trim_end_matches('.')
            .to_ascii_lowercase()
            .as_str(),
        "127.0.0.1" | "localhost" | "::1"
    )
}

/// Refuse a non-loopback play host while local (well-known Java) RSA is active.
pub fn validate_play_host(host: &str, target: BotTarget) -> Result<(), &'static str> {
    if target == BotTarget::Local && !is_loopback_host(host) {
        Err("host-play: non-loopback --host requires --prod (local RSA is loopback-only)")
    } else {
        Ok(())
    }
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

/// Production launch input. Connection fields come only from the immutable
/// profile; `PlayOptions` remains the legacy 274 caller interface.
#[derive(Clone)]
pub struct ProfilePlayOptions {
    pub profile: Arc<ServerProfile>,
    pub mainland: bool,
}

/// One validated asset decode for a process profile. Preparing clients clones
/// only the shared Arcs; mutable interface overlays remain per client.
pub struct SharedClientTemplate {
    profile: Arc<ServerProfile>,
    game_data: Option<Arc<api::game_data::SelectedGameData>>,
    cache: Arc<Cache>,
    ifaces: Arc<Vec<Option<Box<IfType>>>>,
    ifaces_mut: Arc<Vec<Option<Arc<IfTypeMut>>>>,
    world: Option<Arc<NavWorld>>,
    scatter: std::sync::OnceLock<Vec<WorldTile>>,
}

/// One-use proof that the template's frozen resource identities were checked
/// immediately before Play construction. Private fields prevent callers from
/// bypassing [`ServerProfile::validate_resources`].
pub struct ValidatedTemplate {
    template: Arc<SharedClientTemplate>,
}

impl ValidatedTemplate {
    /// The exact template whose profile produced this validation proof.
    pub fn template(&self) -> &Arc<SharedClientTemplate> {
        &self.template
    }
}

impl SharedClientTemplate {
    pub fn load(profile: Arc<ServerProfile>) -> Result<Arc<Self>, String> {
        Self::load_with_progress(profile, &progress::ProfileProgressObserver::default())
    }

    pub fn load_with_progress(
        profile: Arc<ServerProfile>,
        observer: &progress::ProfileProgressObserver,
    ) -> Result<Arc<Self>, String> {
        profile.validate_resources_with_progress(observer)?;
        observer.report(progress::ProfileProgress::steps(
            progress::ProfileProgressStage::LoadingGameData,
            0,
            4,
        ));
        let (cache, ifaces, ifaces_mut) =
            load_template_checked(profile.client().cache_dir(), observer)?;
        let game_data = profile.game_data();
        observer.report(progress::ProfileProgress::steps(
            progress::ProfileProgressStage::LoadingGameData,
            4,
            4,
        ));
        let world = profile.world();
        Ok(Arc::new(Self {
            profile,
            game_data,
            cache: Arc::new(cache),
            ifaces: Arc::new(ifaces),
            ifaces_mut: Arc::new(ifaces_mut),
            world,
            scatter: std::sync::OnceLock::new(),
        }))
    }

    pub fn profile(&self) -> &Arc<ServerProfile> {
        &self.profile
    }
    pub fn game_data(&self) -> Option<Arc<api::game_data::SelectedGameData>> {
        self.game_data.clone()
    }
    pub fn world(&self) -> Option<Arc<NavWorld>> {
        self.world.clone()
    }

    /// Revalidate the selected cache identity and return the consuming
    /// handoff required by [`run_prepared_template`]. Navigation is the
    /// already-loaded world; a post-load disk edit does not replace it.
    pub fn validate_for_play(self: &Arc<Self>) -> Result<ValidatedTemplate, String> {
        self.validate_for_play_with_progress(&progress::ProfileProgressObserver::default())
    }

    pub fn validate_for_play_with_progress(
        self: &Arc<Self>,
        observer: &progress::ProfileProgressObserver,
    ) -> Result<ValidatedTemplate, String> {
        self.profile.validate_resources_with_progress(observer)?;
        observer.report(progress::ProfileProgress::steps(
            progress::ProfileProgressStage::FinalChecks,
            1,
            1,
        ));
        Ok(ValidatedTemplate {
            template: Arc::clone(self),
        })
    }

    /// Stable account scatter over this template's selected navigation world.
    /// The seed vector is shared once per template, as are the world tables.
    pub fn scatter_tile_for(&self, uid: i32) -> WorldTile {
        let tiles = self
            .scatter
            .get_or_init(|| scatter::shuffled_for_world(self.world.as_deref()));
        tiles[(uid.unsigned_abs() as usize) % tiles.len()]
    }

    /// This constructor is usable for protocol qualification before bot action
    /// support is released. It does not start a host loop or any bot policy.
    pub fn prepare_client(&self, uid: i32, lowmem: bool) -> Result<Client, String> {
        host::prepare_client_with_profile(
            Arc::clone(self.profile.client()),
            uid,
            true,
            lowmem,
            Arc::clone(&self.cache),
            Arc::clone(&self.ifaces),
            Arc::clone(&self.ifaces_mut),
        )
    }
}

#[derive(Clone)]
enum PlayConnection {
    Legacy(PlayOptions),
    Bound {
        template: Arc<SharedClientTemplate>,
        mainland: bool,
    },
}

impl PlayConnection {
    fn profile(&self) -> Option<&Arc<ServerProfile>> {
        match self {
            Self::Legacy(_) => None,
            Self::Bound { template, .. } => Some(template.profile()),
        }
    }

    fn require_bot_operation(&self) -> Result<(), String> {
        self.profile().map_or(Ok(()), |p| p.require_bot_operation())
    }

    fn target(&self) -> BotTarget {
        self.profile()
            .map_or_else(client::bot_target, |p| p.target())
    }
}

/// Pollable per-slot view; the slot threads update it after each frame.
#[derive(Debug, Clone)]
pub struct SlotStatus {
    pub username: String,
    /// Native lifecycle phase; `ingame` is producer-gated and cannot
    /// distinguish login from a scene rebuild.
    pub startup_phase: StartupPhase,
    /// Monotonic instant at which `startup_phase` began.
    pub startup_phase_started: Instant,
    /// Client asset initialization progress; cleared when initialization ends.
    pub startup_progress_percent: Option<i32>,
    pub startup_progress_message: String,
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
    /// Local-player plane (`Client::minusedlevel` in observe).
    pub tile_level: i32,
    /// Local-player name, empty until `PLAYER_INFO` lands.
    pub player: String,
    /// `Client.main_modal_id` (open modal interface, -1 when none).
    pub main_modal_id: i32,
    /// Host login-readiness: script work is held while the native welcome
    /// modal is open or a bounded dismiss failed.
    pub welcome_hold: bool,
    /// Visible bounded dismiss failure; `None` while idle/settled.
    pub welcome_failure: Option<String>,
    /// Last welcome phase line for panel/TUI logs.
    pub welcome_notice: Option<String>,
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
    /// Intentional logout latch ([`SlotArm::latch`]): the slot is parked on
    /// the title until the operator explicitly arms login again.
    pub login_latched: bool,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupPhase {
    Preparing,
    Queueing,
    Connecting,
    LoadingScene,
    Ready,
    Error,
}

impl SlotStatus {
    /// Wall member is online: every slot is a full `Client` now (no lean
    /// special case), so a slot is up when the scene is built.
    pub fn is_up(&self) -> bool {
        self.ingame && self.scene_state == 2
    }

    /// A preparation failure that is terminal for the current slot lifetime.
    /// Login errors also use `StartupPhase::Error`, but are retryable and do
    /// not carry this producer-owned asset-initialization fact.
    pub fn terminal_startup_error(&self) -> Option<&str> {
        if self.startup_phase != StartupPhase::Error {
            return None;
        }
        self.error
            .as_deref()
            .filter(|error| error.starts_with("profile asset initialization failed:"))
    }
}

/// Find a terminal asset-init failure among the slots owned by one run.
/// Ownership is explicit so unrelated user slots cannot fail a live watch.
pub fn owned_terminal_startup_error(
    statuses: &[SlotStatus],
    owned_names: &[String],
) -> Option<String> {
    statuses
        .iter()
        .filter(|slot| owned_names.iter().any(|name| name == &slot.username))
        .find_map(|slot| {
            slot.terminal_startup_error()
                .map(|error| format!("{}: {error}", slot.username))
        })
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

/// The local player's world tile `(x, z, level)` from the scene origin,
/// route head, and [`Client::minusedlevel`] — the same level
/// [`GameSnapshot::tile`] and [`nav::traveller::Traveller::follow`] use.
pub fn player_here_tile(c: &Client) -> Option<(i32, i32, i32)> {
    c.local_player.as_ref().map(|lp| {
        let (tx, tz) = player_world_tile(
            c.map_build_base_x,
            c.map_build_base_z,
            lp.route_x[0],
            lp.route_z[0],
        );
        (tx, tz, c.minusedlevel)
    })
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
        Err(_) => match client::operator_home() {
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
            startup_phase: StartupPhase::Preparing,
            startup_phase_started: Instant::now(),
            startup_progress_percent: None,
            startup_progress_message: String::new(),
            login_started: None,
            ingame: false,
            scene_state: 0,
            error: None,
            runenergy: 0,
            run_sends: 0,
            tile_x: 0,
            tile_z: 0,
            tile_level: 0,
            player: String::new(),
            main_modal_id: 0,
            welcome_hold: false,
            welcome_failure: None,
            welcome_notice: None,
            walk_x: -1,
            walk_z: -1,
            walk_level: -1,
            queue_position: -1,
            queue_total: -1,
            login_latched: false,
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
    /// Live lamp auto-use toggle (`ProfileSettings.lamp_auto`).
    pub lamp_auto: Arc<AtomicBool>,
    /// Live lamp skill choice (`ProfileSettings.lamp_skill`).
    pub lamp_skill: Arc<Mutex<String>>,
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
            lamp_auto: Arc::new(AtomicBool::new(true)),
            lamp_skill: Arc::new(Mutex::new("strength".to_string())),
            reconnect: Arc::new(AtomicBool::new(false)),
        })
    }
}

/// Whether the slot may start a login handshake: on the title (not ingame)
/// and the arm wants a login that is not latched by an intentional logout.
fn should_handshake(arm: &SlotArm, ingame: bool) -> bool {
    !ingame && arm.want_login.load(Ordering::Relaxed) && !arm.latch.load(Ordering::Relaxed)
}

/// Claim the boundary between a granted reservation and `client.login`.
/// Cancellation or stop observed here abandons only this unused permit; once
/// this returns true, the caller must acknowledge the login return instead.
fn granted_permit_may_start_login(
    queue: &Arc<Mutex<LoginQueue>>,
    uid: i32,
    arm: &SlotArm,
    ingame: bool,
) -> bool {
    if !arm.stop.load(Ordering::Relaxed) && should_handshake(arm, ingame) {
        return true;
    }
    let abandoned = queue.lock().unwrap().abandon_permit(uid);
    debug_assert!(abandoned, "granted permit must be abandoned exactly once");
    false
}

/// Run the login call that owns a granted permit, then acknowledge its return
/// before success/error handling can branch. Errors are counted deliberately:
/// the client may have sent the attempt before returning either result.
fn login_and_acknowledge_permit<T, E>(
    queue: &Arc<Mutex<LoginQueue>>,
    uid: i32,
    login: impl FnOnce() -> Result<T, E>,
) -> Result<T, E> {
    let result = login();
    let acknowledged = queue
        .lock()
        .unwrap()
        .acknowledge_login_return(uid, Instant::now());
    debug_assert!(
        acknowledged,
        "each client.login return acknowledges one granted permit"
    );
    result
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
    if let Some(SessionExitObservation::ServerLogoutAfterLocalIdleRequest) =
        client.take_session_exit_observation()
    {
        arm.latch.store(true, Ordering::Relaxed);
        arm.want_login.store(false, Ordering::Relaxed);
    }
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
}

impl Play {
    /// Shared construction for the public `run*` entry points: one login
    /// FIFO, one shared cache/iface template, and the per-slot script/cheat
    /// maps. `per_frame` starts as a no-op — slots stay draw-off (headless)
    /// until a caller's own per-frame hook turns a slot's renderer on.
    fn new(options: &PlayOptions) -> Play {
        let (cache, ifaces, ifaces_mut_template) = load_template(&options.cache_dir);
        let cache = Arc::new(cache);
        Self::assemble(
            PlayConnection::Legacy(options.clone()),
            None,
            cache,
            Arc::new(ifaces),
            Arc::new(ifaces_mut_template),
            NavWorld::load_pack(&default_pack_path()).ok().map(Arc::new),
        )
    }

    fn from_template(template: Arc<SharedClientTemplate>, mainland: bool) -> Play {
        Self::assemble(
            PlayConnection::Bound {
                template: Arc::clone(&template),
                mainland,
            },
            template.game_data.clone(),
            Arc::clone(&template.cache),
            Arc::clone(&template.ifaces),
            Arc::clone(&template.ifaces_mut),
            template.world.clone(),
        )
    }

    fn assemble(
        connection: PlayConnection,
        game_data: Option<Arc<api::game_data::SelectedGameData>>,
        cache: Arc<Cache>,
        ifaces: Arc<Vec<Option<Box<IfType>>>>,
        ifaces_mut_template: Arc<Vec<Option<Arc<IfTypeMut>>>>,
        world: Option<Arc<NavWorld>>,
    ) -> Play {
        let obj_names = Arc::new(api::obj_names::ObjNames::from_objs(&cache.objs));
        let named_banks = Arc::new(
            world
                .as_deref()
                .map(|world| world.named_bank_facts(script::content::BANK_ALIASES))
                .unwrap_or_default(),
        );
        Play {
            statuses: Arc::new(Mutex::new(Vec::new())),
            handles: HashMap::new(),
            connection,
            game_data,
            named_banks,
            cache,
            obj_names,
            catalog_core: catalog_core::CoreWatch::default(),
            paired_core: paired_core::PairWatch::default(),
            ifaces,
            ifaces_mut_template,
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
            world,
            wakes: HashMap::new(),
        }
    }

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
        let uid = self
            .arms
            .get(name)
            .map(|arm| arm.uid.load(Ordering::Relaxed));
        self.queue.lock().unwrap().set_preferred(uid);
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

    /// Keep vault credentials for a later [`Play::spawn_slot`] /
    /// reconnect.
    pub fn remember_profile(&mut self, profile: Profile) {
        self.profiles.insert(profile.username.clone(), profile);
    }

    /// Move `uid` to the front of the login FIFO so the TV head handshakes
    /// before slots that already queued. Mirrors the place onto the status row
    /// so the queue card can show *k of n* during maininit (the slot has not
    /// entered [`wait_for_permit`] yet). A slot that cannot wait — already
    /// ingame, or a thread that already returned — only gets the precedence
    /// remembered: reserving a place for it would be an orphan FIFO entry that
    /// strands real waiters behind a phantom and publishes *k of n* for a
    /// running bot.
    pub fn prefer_login(&self, uid: i32) {
        let name = self
            .arms
            .iter()
            .find(|(_, arm)| arm.uid.load(Ordering::Relaxed) == uid)
            .map(|(n, _)| n.clone());
        let reserve = name.as_deref().is_some_and(|name| self.slot_can_wait(name));
        let mut q = self.queue.lock().unwrap();
        if reserve {
            q.prefer(uid);
            let pos = q.status(uid);
            drop(q);
            if let Some(name) = name {
                apply_queue_wait(&mut self.statuses.lock().unwrap(), &name, pos);
            }
            return;
        }
        // Keep the head's precedence without a place: `request_permit`
        // pushes a preferred uid to the front when it really asks.
        q.set_preferred(Some(uid));
        q.leave(uid);
        drop(q);
        if let Some(name) = name {
            apply_queue_wait(&mut self.statuses.lock().unwrap(), &name, None);
        }
    }

    /// Whether `name`'s slot thread can still enter [`wait_for_permit`]: a
    /// starting slot (its row is published by the thread before maininit) or
    /// a title-screen slot. An ingame slot waits for a disconnect first, and a
    /// slot whose prepare/login gave up has no thread left to ask.
    fn slot_can_wait(&self, name: &str) -> bool {
        let all = self.statuses.lock().unwrap();
        match all.iter().find(|s| s.username == name) {
            None => true,
            Some(row) => !row.ingame && row.startup_phase != StartupPhase::Error,
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
        script_slot_or_insert(&self.scripts, name)
            .lock()
            .unwrap()
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
        script_slot(&self.scripts, name)
            .map(|slot| slot.lock().unwrap().take_pending_logs())
            .unwrap_or_default()
    }

    /// Queue `cmd` (the `::` part only) for `user`'s slot: its own thread
    /// writes `CLIENT_CHEAT` through the slot's Driver and flushes. No-op
    /// when the user is not a running slot, or when the target is Prod.
    pub fn cheat(&self, user: &str, cmd: &str) {
        if self.connection.require_bot_operation().is_err()
            || !api::interact::cheat_allowed(self.connection.target())
        {
            return;
        }
        let statuses = self.statuses.lock().unwrap();
        if !statuses
            .iter()
            .any(|status| status.username == user && status.ingame)
        {
            return;
        }
        if let Some(q) = self.cheats.lock().unwrap().get_mut(user) {
            q.push_back(cmd.to_string());
        }
        drop(statuses);
        self.wake(user);
    }

    /// Queue a chat or one-tile movement command for a connected slot.
    /// Hold the published-session lock through enqueue so a disconnect reset
    /// cannot clear the queue and then receive an old producer's command.
    pub fn queue_wire(&self, user: &str, cmd: WireCmd) {
        if self.connection.require_bot_operation().is_err() {
            return;
        }
        let statuses = self.statuses.lock().unwrap();
        if !statuses
            .iter()
            .any(|status| status.username == user && status.ingame)
        {
            return;
        }
        if let Some(q) = self.wires.lock().unwrap().get_mut(user) {
            q.push_back(cmd);
        }
        drop(statuses);
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
            self.queue.lock().unwrap().set_preferred(None);
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
        // Keep the vault credentials on the wall for later spawns and
        // DC-reconnect re-handshakes.
        self.profiles
            .insert(profile.username.clone(), profile.clone());
        if !self.spawned.insert(profile.username.clone()) {
            return Ok(());
        }
        let arm = arm.unwrap_or_else(|| SlotArm::new(profile.uid, true));
        // `stop_slot` leaves the FIFO by `arm.uid`; force it from the
        // profile at spawn. The store goes through the shared inner field
        // so a caller's own clone cannot keep a stale uid.
        arm.uid.store(profile.uid, Ordering::Relaxed);
        arm.random_events
            .store(profile.settings.random_events, Ordering::Relaxed);
        arm.lamp_auto
            .store(profile.settings.lamp_auto, Ordering::Relaxed);
        *arm.lamp_skill.lock().unwrap() = profile.settings.lamp_skill.clone();
        self.arms.insert(profile.username.clone(), Arc::clone(&arm));
        // The control wake: `Play::wake` kicks the parked slot thread on
        // focus/draw/stop/spawn changes; the slot thread polls the park end.
        let (wake, park) = wake_channel();
        self.wakes.insert(profile.username.clone(), wake);
        spawn_slot_thread(
            &self.connection,
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

/// Checked production path. Callers can load the template before opening a
/// vault, then hand the same shared resources to this function after unlock.
pub fn run_with_template<F, G>(
    template: Arc<SharedClientTemplate>,
    mainland: bool,
    profiles: Vec<Profile>,
    per_slot: F,
    per_frame: G,
) -> Result<Play, String>
where
    F: Fn(&str) -> (Option<Arc<SlotInput>>, Option<Arc<FrameBuf>>),
    G: Fn(&mut Client, &str, bool) + Send + Sync + 'static,
{
    if !profiles.is_empty() {
        template.profile().require_bot_operation()?;
    }
    let validated = template.validate_for_play()?;
    run_prepared_template(validated, mainland, profiles, per_slot, per_frame)
}

/// Construct Play from a just-validated, one-use template handoff. The public
/// checked convenience entry remains [`run_with_template`]; callers cannot
/// construct this function's ticket without running the final validation.
pub fn run_prepared_template<F, G>(
    validated: ValidatedTemplate,
    mainland: bool,
    profiles: Vec<Profile>,
    per_slot: F,
    per_frame: G,
) -> Result<Play, String>
where
    F: Fn(&str) -> (Option<Arc<SlotInput>>, Option<Arc<FrameBuf>>),
    G: Fn(&mut Client, &str, bool) + Send + Sync + 'static,
{
    let template = validated.template;
    if !profiles.is_empty() {
        template.profile().require_bot_operation()?;
    }
    let mut play = Play::from_template(template, mainland);
    play.per_frame = Arc::new(per_frame);
    for profile in profiles {
        let (input, mailbox) = per_slot(&profile.username);
        play.try_spawn_slot(profile, input, mailbox, None)?;
    }
    Ok(play)
}

pub fn run_with_profile(
    options: &ProfilePlayOptions,
    profiles: Vec<Profile>,
) -> Result<Play, String> {
    if !profiles.is_empty() {
        options.profile.require_bot_operation()?;
    }
    let template = SharedClientTemplate::load(Arc::clone(&options.profile))?;
    run_with_template(
        template,
        options.mainland,
        profiles,
        |_| (None, None),
        |_, _, _| {},
    )
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
        s.startup_phase = StartupPhase::Connecting;
        s.startup_phase_started = Instant::now();
        s.error = None;
    }
}

fn publish_startup_phase(statuses: &Arc<Mutex<Vec<SlotStatus>>>, name: &str, message: &str) {
    let mut all = statuses.lock().unwrap();
    if let Some(s) = all.iter_mut().find(|s| s.username == name) {
        s.startup_phase = StartupPhase::Preparing;
        s.startup_phase_started = Instant::now();
        s.startup_progress_percent = None;
        s.startup_progress_message.clear();
        s.startup_progress_message.push_str(message);
    }
}

fn publish_startup_progress(
    statuses: &Arc<Mutex<Vec<SlotStatus>>>,
    name: &str,
    message: &str,
    percent: i32,
) {
    let mut all = statuses.lock().unwrap();
    if let Some(s) = all.iter_mut().find(|s| s.username == name) {
        s.startup_progress_percent = Some(percent.clamp(0, 100));
        s.startup_progress_message.clear();
        s.startup_progress_message.push_str(message);
    }
}

fn clear_startup_progress(statuses: &Arc<Mutex<Vec<SlotStatus>>>, name: &str) {
    let mut all = statuses.lock().unwrap();
    if let Some(s) = all.iter_mut().find(|s| s.username == name) {
        s.startup_progress_percent = None;
        s.startup_progress_message.clear();
    }
}

fn set_startup_phase(statuses: &Arc<Mutex<Vec<SlotStatus>>>, name: &str, phase: StartupPhase) {
    let mut all = statuses.lock().unwrap();
    if let Some(s) = all.iter_mut().find(|s| s.username == name) {
        if s.startup_phase != phase {
            s.startup_phase = phase;
            s.startup_phase_started = Instant::now();
            if debug_enabled() {
                eprintln!("[host-play] slot {name}: startup phase {phase:?}");
            }
        }
    }
}

fn startup_phase_after_observation(
    current: StartupPhase,
    ready: bool,
    client_ingame: bool,
) -> Option<StartupPhase> {
    if matches!(current, StartupPhase::Preparing | StartupPhase::Error) {
        return None;
    }
    if !client_ingame {
        return Some(StartupPhase::Queueing);
    }
    Some(if ready {
        StartupPhase::Ready
    } else {
        StartupPhase::LoadingScene
    })
}

fn apply_startup_phase(s: &mut SlotStatus, name: &str, ready: bool, client_ingame: bool) {
    if let Some(next_phase) = startup_phase_after_observation(s.startup_phase, ready, client_ingame)
    {
        if s.startup_phase != next_phase {
            s.startup_phase = next_phase;
            s.startup_phase_started = Instant::now();
            if debug_enabled() {
                eprintln!("[host-play] slot {name}: startup phase {next_phase:?}");
            }
        }
    }
}

fn record_login_error(statuses: &Arc<Mutex<Vec<SlotStatus>>>, name: &str, e: &LoginError) {
    let msg = format!("code {}: {}", e.code, e.mes2);
    if debug_enabled() {
        eprintln!("[host-play] slot {name}: login {msg}");
    }
    let mut all = statuses.lock().unwrap();
    if let Some(s) = all.iter_mut().find(|s| s.username == name) {
        s.startup_phase = StartupPhase::Error;
        s.startup_phase_started = Instant::now();
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

/// Drop producer-gated observation without changing the display phase.
/// A successful login/reconnect (`Pump` session gen) must reset snapshot
/// fields without mislabeling the new session as a queue wait.
fn reset_slot_observation(s: &mut SlotStatus) {
    s.ingame = false;
    s.scene_state = 0;
    s.runenergy = 0;
    s.main_modal_id = -1;
    s.welcome_hold = false;
    s.welcome_failure = None;
    s.welcome_notice = None;
    s.tile_x = -1;
    s.tile_z = -1;
    s.tile_level = -1;
    s.player.clear();
    s.chat_head.clear();
    s.walk_x = -1;
    s.walk_z = -1;
    s.walk_level = -1;
    s.random = RandomStatus::default();
}

fn publish_slot_observation_reset(statuses: &Arc<Mutex<Vec<SlotStatus>>>, name: &str) {
    if let Some(s) = statuses
        .lock()
        .unwrap()
        .iter_mut()
        .find(|s| s.username == name)
    {
        reset_slot_observation(s);
    }
}

/// Close the producer gate before clearing any queued work. Never hold this
/// lock while locking a script: script -> statuses is the established order.
fn publish_slot_disconnected(statuses: &Arc<Mutex<Vec<SlotStatus>>>, name: &str) {
    if let Some(s) = statuses
        .lock()
        .unwrap()
        .iter_mut()
        .find(|s| s.username == name)
    {
        reset_slot_observation(s);
        s.login_started = None;
        if s.startup_phase != StartupPhase::Error && s.startup_phase != StartupPhase::Queueing {
            s.startup_phase = StartupPhase::Queueing;
            s.startup_phase_started = Instant::now();
            if debug_enabled() {
                eprintln!(
                    "[host-play] slot {name}: startup phase {:?}",
                    StartupPhase::Queueing
                );
            }
        }
    }
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
        );
    }
    if paired.configured() {
        paired.observe_snapshot(account, snapshot, session_boundary);
    }
}

/// `session_changed` is a successful login/reconnect, not a drop. Close the
/// producer gate in both cases; only a native logout returns the banner to
/// queue/connect wait.
fn publish_session_boundary_status(
    statuses: &Arc<Mutex<Vec<SlotStatus>>>,
    name: &str,
    session_changed: bool,
    client_ingame: bool,
    snapshot_ingame: bool,
) -> bool {
    let session_boundary = session_changed || (!client_ingame && snapshot_ingame);
    if session_boundary {
        if client_ingame {
            publish_slot_observation_reset(statuses, name);
        } else {
            publish_slot_disconnected(statuses, name);
        }
    }
    session_boundary
}

/// Every profile spawns one slot thread; shared handles are threaded through
/// because the closure moves most of them (allowed: see `script_observe`).
#[allow(clippy::too_many_arguments)]
fn spawn_slot_thread(
    connection: &PlayConnection,
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
            script_slot_or_insert(&slot_scripts, &username);
            let slot_input = slot_input.unwrap_or_else(SlotInput::new);
            if let Some(slot) = script_slot(&slot_scripts, &username) {
                slot.lock()
                    .unwrap()
                    .bind_native_input(slot_input.authority());
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
            let mut script_tick: u64 = 0;
            loop {
                if arm.stop.load(Ordering::Relaxed) {
                    slot_queue.lock().unwrap().leave(uid);
                    return;
                }
                if !client.ingame {
                    if !should_handshake(&arm, client.ingame) {
                        // No pending intent (title hold, latched logout, a
                        // withdrawn wait): a parked slot holds no FIFO place
                        // and publishes no `k of n`, so a reservation
                        // (`Play::prefer_login` for the TV head) that
                        // outlived its intent cannot strand later members.
                        drop_queue_place(&slot_queue, &slot_statuses, &username, uid);
                        publish_login_latched_from_arm(&slot_statuses, &username, &arm);
                        thread::sleep(Duration::from_millis(20));
                        continue;
                    }
                    // Leaving title park for handshake: refresh latch from the
                    // arm so an explicit Log in cannot keep a stale TRUE from
                    // the last park publish through queue/connect.
                    publish_login_latched_from_arm(&slot_statuses, &username, &arm);
                    let wait = wait_for_permit(&slot_queue, &slot_statuses, &username, uid, &arm);
                    if wait == PermitWait::Cancelled {
                        if arm.stop.load(Ordering::Relaxed) {
                            slot_queue.lock().unwrap().leave(uid);
                            return;
                        }
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
                            slot_queue.lock().unwrap().leave(uid);
                            return;
                        }
                        continue;
                    }
                    mark_login_started(&slot_statuses, &username);
                    let reconnect = arm.reconnect.load(Ordering::Relaxed);
                    if debug_enabled() {
                        eprintln!(
                            "[host-play] slot {username}: handshake begin reconnect={reconnect}"
                        );
                    }
                    let login = login_and_acknowledge_permit(&slot_queue, uid, || {
                        client.login(&username, &password, reconnect)
                    });
                    match login {
                        Ok(()) => {
                            backoff.reset();
                            on_login_success(&arm);
                            set_startup_phase(&slot_statuses, &username, StartupPhase::LoadingScene);
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
                            let inspect = if obs_catalog_core.copies_route_inspect() {
                                slot_navs.lock().unwrap().get(name).map(|bot| {
                                    bot.inspect.published_core_facts()
                                })
                            } else {
                                None
                            };
                            let catalog_paint = if obs_catalog_core.copies_line_of_sight()
                                || obs_catalog_core.copies_actor_observation()
                                || obs_catalog_core.copies_fight_field()
                                || obs_catalog_core.copies_hold_spot()
                                || obs_catalog_core.copies_retreat_spot()
                                || obs_catalog_core.copies_walk_spot()
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
                                catalog_paint.as_ref(),
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
                                        publish_script_paint(s, paint.as_ref());
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
                    slot_queue.lock().unwrap().leave(uid);
                    return;
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
fn load_template_checked(
    cache_dir: &Path,
    observer: &progress::ProfileProgressObserver,
) -> Result<IfaceTables, String> {
    let config =
        std::fs::read(cache_dir.join("config")).map_err(|e| format!("config archive: {e}"))?;
    let interface = std::fs::read(cache_dir.join("interface"))
        .map_err(|e| format!("interface archive: {e}"))?;
    observer.report(progress::ProfileProgress::steps(
        progress::ProfileProgressStage::LoadingGameData,
        1,
        4,
    ));
    std::panic::catch_unwind(AssertUnwindSafe(|| {
        let cache = Cache::unpack(&JagFile::new(config));
        observer.report(progress::ProfileProgress::steps(
            progress::ProfileProgressStage::LoadingGameData,
            2,
            4,
        ));
        let (ifaces, mutable) = IfType::unpack(&JagFile::new(interface));
        observer.report(progress::ProfileProgress::steps(
            progress::ProfileProgressStage::LoadingGameData,
            3,
            4,
        ));
        if cache.objs.is_empty()
            || cache.npcs.is_empty()
            || cache.locs.is_empty()
            || !ifaces.iter().any(Option::is_some)
        {
            return Err(format!(
                "missing required config/interface tables at {}",
                cache_dir.display()
            ));
        }
        Ok((
            cache,
            ifaces,
            mutable
                .into_iter()
                .map(|v| v.map(|b| Arc::new(*b)))
                .collect(),
        ))
    }))
    .map_err(|_| {
        format!(
            "invalid config/interface archives at {}",
            cache_dir.display()
        )
    })?
}

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
fn publish_login_latched(statuses: &Arc<Mutex<Vec<SlotStatus>>>, name: &str, latched: bool) {
    if let Some(s) = statuses
        .lock()
        .unwrap()
        .iter_mut()
        .find(|s| s.username == name)
    {
        s.login_latched = latched;
    }
}

fn publish_login_latched_from_arm(
    statuses: &Arc<Mutex<Vec<SlotStatus>>>,
    name: &str,
    arm: &SlotArm,
) {
    publish_login_latched(
        statuses,
        name,
        arm.latch.load(Ordering::Relaxed),
    );
}

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

/// Refresh cadence for a waiting slot's published place. A window-blocked
/// head can sleep a whole 60 s deadline in one wait, so the card would
/// otherwise miss members that queue behind it. Read-only: the refresh
/// re-reads `status(uid)` and never requests a permit.
const QUEUE_PUBLISH: Duration = Duration::from_millis(200);

/// Outcome of [`wait_for_permit`]. `Granted` owns a pending reservation that
/// the caller must either pass into `client.login` and acknowledge on return,
/// or abandon if its final intent check fails. `Cancelled` owns no reservation:
/// the request was withdrawn before a grant and neither its FIFO place nor its
/// published `k of n` survives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PermitWait {
    Granted,
    Cancelled,
}

/// Drop `uid`'s login-FIFO place and the slot's published `k of n`.
/// [`LoginQueue::leave`] reports whether a place was really held, so a slot
/// that never queued leaves its row alone. Guessing here would blank a card
/// the panel legitimately shows through its FIFO-head fallback.
fn drop_queue_place(
    queue: &Arc<Mutex<LoginQueue>>,
    statuses: &Arc<Mutex<Vec<SlotStatus>>>,
    username: &str,
    uid: i32,
) {
    let removed = queue.lock().unwrap().leave(uid);
    if removed {
        apply_queue_wait(&mut statuses.lock().unwrap(), username, None);
    }
}

/// Whether a pending permit wait must be withdrawn before any handshake:
/// `stop` (rail ✕ / slot removal), the intent itself (`want_login` cleared by
/// an intentional logout or by the wall re-applying auto-login), the logout
/// latch, or the auto-login checkbox that armed the intent being cleared.
/// `auto_sourced` latches once the pending intent is observed armed by
/// auto-login — [`SlotArm::new`] seeds `want_login = auto_login`, so clearing
/// the checkbox withdraws the wait it armed, while an explicit Log in /
/// Login all intent (armed with auto-login off) survives the same toggle.
fn permit_wait_cancelled(arm: &SlotArm, auto_sourced: &mut bool) -> bool {
    let want = arm.want_login.load(Ordering::Relaxed);
    let auto = arm.auto_login.load(Ordering::Relaxed);
    if want && auto {
        *auto_sourced = true;
    }
    arm.stop.load(Ordering::Relaxed)
        || !want
        || arm.latch.load(Ordering::Relaxed)
        || (*auto_sourced && !auto)
}

/// Block until the login queue grants `uid` a handshake permit, mirroring
/// the queue position onto the slot's status row while it waits. Every
/// withdrawal is observed each poll **before** `request_permit`, so a `leave`
/// from [`Play::stop_slot`] or a cancelled login intent is never undone by a
/// re-enqueue; a withdrawn wait also clears `want_login` so the caller's loop
/// does not re-enter the queue for an intent the operator dropped.
fn wait_for_permit(
    queue: &Arc<Mutex<LoginQueue>>,
    statuses: &Arc<Mutex<Vec<SlotStatus>>>,
    username: &str,
    uid: i32,
    arm: &SlotArm,
) -> PermitWait {
    let mut auto_sourced = false;
    // Withdraw a dead request: clear `want_login` so the caller's loop does
    // not re-enter the queue for an intent the operator dropped, drop the
    // FIFO place and clear the published `k of n`.
    let withdraw = || {
        arm.want_login.store(false, Ordering::Relaxed);
        drop_queue_place(queue, statuses, username, uid);
        if debug_enabled() {
            eprintln!("[host-play] slot {username}: permit wait withdrawn");
        }
    };
    loop {
        // Before `request_permit`: it enqueues, so a place dropped by
        // `stop_slot` (or by a withdrawn intent) must not be recreated.
        if permit_wait_cancelled(arm, &mut auto_sourced) {
            withdraw();
            return PermitWait::Cancelled;
        }
        let wait = {
            let mut q = queue.lock().unwrap();
            match q.request_permit(uid, Instant::now()) {
                Permit::Grant => {
                    drop(q);
                    let mut all = statuses.lock().unwrap();
                    apply_queue_wait(&mut all, username, None);
                    return PermitWait::Granted;
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
        // Interruptible sleep: a withdrawal must not wait out a 60 s address
        // deadline, and the published `k of n` must not freeze for that long
        // either — a window-blocked head sleeping its deadline would not show
        // members that queue behind it. This is a read-only re-read of the
        // place (no `request_permit`), so no grant can happen here.
        let deadline = Instant::now() + wait;
        let mut next_publish = Instant::now() + QUEUE_PUBLISH;
        while Instant::now() < deadline && !permit_wait_cancelled(arm, &mut auto_sourced) {
            let now = Instant::now();
            if now >= next_publish {
                let pos = queue.lock().unwrap().status(uid);
                apply_queue_wait(&mut statuses.lock().unwrap(), username, pos);
                next_publish = now + QUEUE_PUBLISH;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            thread::sleep(left.min(Duration::from_millis(20)));
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
