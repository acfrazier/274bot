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

/// Per-uid script cell on the wall. Encode/post/drain take the slot lock
/// only — the wall map lock is held briefly for lookup/insert.
type ScriptSlot = Arc<Mutex<SlotScript>>;
/// Lock order: wall before slot; never hold slot then wall.
type ScriptWall = Arc<Mutex<HashMap<String, ScriptSlot>>>;

fn script_slot(wall: &ScriptWall, name: &str) -> Option<ScriptSlot> {
    wall.lock().unwrap().get(name).cloned()
}

fn script_slot_or_insert(wall: &ScriptWall, name: &str) -> ScriptSlot {
    wall.lock()
        .unwrap()
        .entry(name.to_string())
        .or_insert_with(|| Arc::new(Mutex::new(SlotScript::new())))
        .clone()
}

/// Drain isolate `this.log` / tick-error lines onto stderr when
/// `BOT_DEBUG=1`. Tick errors also become [`SlotScript::last_error`].
fn emit_script_debug_logs(slot: &mut SlotScript, name: &str) {
    let logs = slot.drain_logs();
    #[cfg(feature = "memory-profile")]
    memory_diagnostics::logs(name, &logs);
    if !debug_enabled() {
        return;
    }
    for line in logs {
        eprintln!("[script {name}] {line}");
    }
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
fn script_observe_with_npc_boxes(
    driver: &mut dyn Driver,
    name: &str,
    up: bool,
    tick_edge: bool,
    tick: u64,
    here: Option<(i32, i32, i32)>,
    inv: Option<&[(i32, i32)]>,
    state: Option<WorldState>,
    snapshot: Option<&GameSnapshot>,
    npc_boxes: Option<&[script::isolate_fb::NpcBoxInput]>,
    obj_names: Option<&api::obj_names::ObjNames>,
    scripts: &ScriptWall,
    cheats: &Arc<Mutex<HashMap<String, VecDeque<String>>>>,
    navs: &Arc<Mutex<HashMap<String, NavBot>>>,
    world: &Option<Arc<NavWorld>>,
    hold: bool,
    ours: bool,
    canlight: Option<&[u64]>,
    slot_input: Option<&SlotInput>,
) -> bool {
    if let Some(inp) = slot_input {
        inp.set_host_consume_allowed(up && !hold);
    }
    let mut wrote = false;
    let mut interact = Vec::new();
    let mut pending_withdraw_x_active = false;
    let mut pending_bank_op_active = false;
    let mut slot_work_epoch = None;
    if let Some(slot) = script_slot(scripts, name) {
        let mut slot = slot.lock().unwrap();
        // Reap a script-requested Stop before advancing host continuations.
        emit_script_debug_logs(&mut slot, name);
        slot.on_is_up(up);
        slot_work_epoch = Some(slot.work_epoch());
        if let Some(pending) = slot.pending_bank_op() {
            if hold || slot.state() == script::RunState::Paused {
                slot.freeze_pending_bank_op();
                pending_bank_op_active = true;
            } else {
                slot.resume_pending_bank_op();
                let valid_session = snapshot.is_some_and(|snap| {
                    snap.bank_component_id() >= 0
                        && snap.bank_loaded()
                        && snap.bank_session_generation() == pending.bank_generation
                });
                if !valid_session {
                    slot.complete_bank_op(false);
                } else if matches!(
                    pending.kind,
                    script::slot::PendingBankOpKind::WithdrawXAction
                ) {
                    // Raw open-only `Withdraw X`: the accepted menu action is
                    // the result the frozen caller awaits (it types the amount
                    // + Enter itself), so no bank or inventory delta can exist
                    // yet. Acknowledge on the same generation/bank-open gate
                    // the counted ops settle through.
                    slot.complete_bank_op(true);
                } else {
                    let current = snapshot.map_or(0, |snap| match pending.kind {
                        script::slot::PendingBankOpKind::Deposit => snap
                            .bank_side()
                            .iter()
                            .filter(|item| item.def.id == pending.item_id)
                            .map(|item| item.count)
                            .sum(),
                        // The open-only X kind never reaches the delta
                        // branch: it acknowledged on the session gate above.
                        script::slot::PendingBankOpKind::Withdraw
                        | script::slot::PendingBankOpKind::WithdrawXAction => snap
                            .bank()
                            .iter()
                            .filter(|item| item.def.id == pending.item_id)
                            .map(|item| item.count)
                            .sum(),
                    });
                    let inventory_increased =
                        matches!(pending.kind, script::slot::PendingBankOpKind::Withdraw)
                            && inv.is_some_and(|items| {
                                items
                                    .iter()
                                    .filter(|(id, _)| *id == pending.item_id)
                                    .map(|(_, count)| *count)
                                    .sum::<i32>()
                                    > pending.before_inventory_count
                            });
                    if current < pending.before_count || inventory_increased {
                        slot.complete_bank_op(true);
                    } else if pending.expired() {
                        slot.complete_bank_op(false);
                    } else {
                        pending_bank_op_active = true;
                    }
                }
            }
        }
        if let Some(pending) = slot.pending_withdraw_x() {
            if hold || slot.state() == script::RunState::Paused {
                slot.freeze_pending_withdraw_x();
                pending_withdraw_x_active = true;
            } else {
                slot.resume_pending_withdraw_x();
                let valid_session = snapshot.is_some_and(|snap| {
                    snap.bank_component_id() >= 0
                        && snap.bank_loaded()
                        && snap.bank_session_generation() == pending.bank_generation
                });
                if !valid_session {
                    slot.complete_current_withdrawal(false);
                } else {
                    match pending.phase {
                        script::slot::PendingWithdrawXPhase::Dialog => {
                            if pending.expired() {
                                slot.complete_current_withdrawal(false);
                            } else if snapshot.is_some_and(GameSnapshot::count_dialog_open) {
                                let sent = snapshot.is_some_and(|snap| {
                                    matches!(
                                        api::interact::Interactions::new(snap, driver)
                                            .answer_count(pending.count),
                                        api::interact::SendResult::Sent { .. }
                                    )
                                });
                                if sent {
                                    wrote = true;
                                    slot.set_pending_withdraw_x(Some(pending.waiting_settlement()));
                                    pending_withdraw_x_active = true;
                                } else {
                                    slot.complete_current_withdrawal(false);
                                }
                            } else {
                                pending_withdraw_x_active = true;
                            }
                        }
                        script::slot::PendingWithdrawXPhase::Settlement => {
                            let (current, full) = if let Some(rows) = inv {
                                (
                                    rows.iter()
                                        .filter(|(id, _)| *id == pending.item_id)
                                        .map(|(_, count)| *count)
                                        .sum::<i32>(),
                                    snapshot.is_some_and(|snap| {
                                        snap.inventory_size() > 0
                                            && rows.len() >= snap.inventory_size() as usize
                                    }),
                                )
                            } else {
                                snapshot.map_or((0, false), |snap| {
                                    (
                                        snap.inventory()
                                            .iter()
                                            .filter(|item| item.def.id == pending.item_id)
                                            .map(|item| item.count)
                                            .sum::<i32>(),
                                        snap.inventory_size() > 0
                                            && snap.inventory().len()
                                                >= snap.inventory_size() as usize,
                                    )
                                })
                            };
                            let load_settled = pending.fill.is_some_and(|fill| {
                                let (used, count) = inv.map_or_else(
                                    || {
                                        snapshot.map_or((0, 0), |snap| {
                                            let inventory = snap.inventory();
                                            (
                                                inventory.len(),
                                                inventory
                                                    .iter()
                                                    .filter(|item| item.def.id == fill.bank_item_id)
                                                    .map(|item| item.count)
                                                    .sum::<i32>(),
                                            )
                                        })
                                    },
                                    |rows| {
                                        (
                                            rows.len(),
                                            rows.iter()
                                                .filter(|(id, _)| *id == fill.bank_item_id)
                                                .map(|(_, count)| *count)
                                                .sum::<i32>(),
                                        )
                                    },
                                );
                                let stock_decreased = snapshot
                                    .and_then(|snap| {
                                        snap.bank()
                                            .iter()
                                            .find(|item| item.def.id == fill.bank_item_id)
                                    })
                                    .is_some_and(|item| item.count < fill.before_stock);
                                used > fill.before_used
                                    || count > fill.before_count
                                    || stock_decreased
                            });
                            if load_settled
                                || (pending.fill.is_none()
                                    && (current >= pending.target
                                        || (current > pending.before && full)))
                            {
                                slot.complete_current_withdrawal(true);
                            } else if pending.expired() {
                                slot.complete_current_withdrawal(false);
                            } else {
                                pending_withdraw_x_active = true;
                            }
                        }
                    }
                }
            }
        }
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
            // Guardian `hold` freezes clocks/nav and cancels owned recovery.
            // Recovery hold is host-owned and must still reach the isolate so
            // loop/pump freeze, without being treated as that external freeze.
            let recovery_hold = slot.load_active() && slot.watchdog().holds_script_actions();
            let isolate_hold = hold || recovery_hold;
            if slot.load_active() {
                let (
                    teleports_enabled,
                    walk_seq,
                    walk_generation,
                    walk_request_id,
                    walk_failed,
                    walk_x,
                    walk_z,
                    walk_level,
                    walk_radius,
                    walk_allow_teleports,
                ) = {
                    let mut all = navs.lock().unwrap();
                    match all.get_mut(name) {
                        Some(b) => {
                            let posted = (
                                b.allow_teleports,
                                b.walk_outcome_seq,
                                b.walk_outcome_generation,
                                b.walk_outcome_request_id,
                                b.walk_outcome_failed,
                                b.walk_outcome_x,
                                b.walk_outcome_z,
                                b.walk_outcome_level,
                                b.walk_outcome_radius,
                                b.walk_outcome_allow_teleports,
                            );
                            b.mark_walk_outcome_posted();
                            posted
                        }
                        None => (false, 0, 0, 0, false, 0, 0, 0, 0, false),
                    }
                };
                let (withdraw_x_result_seq, withdraw_x_result) = slot.withdraw_x_result();
                let (withdraw_load_result_seq, withdraw_load_result) = slot.withdraw_load_result();
                let (bank_op_result_seq, bank_op_result) = slot.bank_op_result();
                let bytes = with_script_snapshot_input(
                    tick,
                    here,
                    up,
                    inv,
                    snapshot,
                    obj_names,
                    world.as_deref(),
                    npc_boxes,
                    isolate_hold,
                    ours,
                    teleports_enabled,
                    withdraw_x_result_seq,
                    withdraw_x_result,
                    withdraw_load_result_seq,
                    withdraw_load_result,
                    bank_op_result_seq,
                    bank_op_result,
                    canlight,
                    PostedWalkOutcome {
                        seq: walk_seq,
                        generation: walk_generation,
                        request_id: walk_request_id,
                        failed: walk_failed,
                        x: walk_x,
                        z: walk_z,
                        level: walk_level,
                        radius: walk_radius,
                        allow_teleports: walk_allow_teleports,
                    },
                    |input, native| {
                        slot.encode_snapshot_delta_with_native(input, native, force_banks)
                    },
                );
                slot.post_snapshot(bytes);
                slot.store_last_world_id(world_id);
            }
            if isolate_hold {
                // Isolate: tick for onPaint only (hold gate inside V8 via
                // snapshot.hold). Recovery hold is distinct from guardian
                // hold: nav follow of the owned recovery route continues,
                // but script walk arms are not provided. Compiled: skip on
                // guardian hold.
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
        emit_script_debug_logs(&mut slot, name);
        // Fold forwarded shim requests on running frames. Pause leaves the
        // isolate queue untouched so Resume can dispatch it; guardian hold
        // retains the existing drain/drop policy below.
        if slot.load_active() {
            let mut lifecycle = Vec::new();
            if slot.state() == script::RunState::Running {
                lifecycle.extend(slot.drain_lifecycle());
            }
            let ready = up
                && here.is_some()
                && snapshot.is_some_and(|snap| snap.ingame() && snap.scene_state() == 2);
            let frozen = hold || !ready || slot.state() == script::RunState::Paused;
            let xp: Vec<i32> = snapshot
                .map(|snap| snap.stats().iter().map(|stat| stat.xp).collect())
                .unwrap_or_default();
            let now = Instant::now();
            if frozen {
                if slot.watchdog().recovering_anchor().is_some() {
                    if hold {
                        let _ = slot.notify_hold_during_walk();
                    } else {
                        let _ = slot.abort_owned_recovery();
                    }
                    abort_script_walk(navs, name);
                }
            } else if slot.watchdog().recovering_anchor().is_some() {
                let far = here
                    .map(|(x, z, _)| {
                        slot.watchdog().recovering_anchor().is_some_and(|anchor| {
                            script::watchdog::chebyshev_xz((x, z), (anchor.x, anchor.z))
                                > script::watchdog::WALK_RADIUS
                        })
                    })
                    .unwrap_or(true);
                if far && recovery_walk_idle(navs, name) {
                    match slot.notify_walk_failed(now) {
                        script::WatchdogAction::Restart { .. } => {
                            interact.clear();
                            if let Err(e) = slot.restart_load_from_identity(now) {
                                eprintln!("[script {name}] watchdog restart failed: {e}");
                            }
                        }
                        other => apply_watchdog_nav_action(
                            other,
                            driver,
                            snapshot,
                            here,
                            navs,
                            world,
                            state.clone(),
                            name,
                        ),
                    }
                }
            }
            let running = slot.state() == script::RunState::Running;
            let action = slot.feed_watchdog(now, here, &xp, frozen, running, &lifecycle);
            match action {
                script::WatchdogAction::Restart { .. } => {
                    interact.clear();
                    if frozen {
                        eprintln!("[script {name}] watchdog restart ignored: frozen");
                    } else if let Err(e) = slot.restart_load_from_identity(now) {
                        eprintln!("[script {name}] watchdog restart failed: {e}");
                    }
                }
                script::WatchdogAction::RequestAnchor => slot.request_recovery_anchor(),
                script::WatchdogAction::WarnHungLoop => {
                    if debug_enabled() {
                        eprintln!("[script {name}] watchdog hung loop (10s)");
                    }
                }
                other => apply_watchdog_nav_action(
                    other,
                    driver,
                    snapshot,
                    here,
                    navs,
                    world,
                    state.clone(),
                    name,
                ),
            }
            slot.sync_native_input_gate();
            if slot.state() == script::RunState::Running {
                if slot.watchdog().holds_script_actions() {
                    let _dropped = slot.drain_interacts();
                } else {
                    interact.extend(take_script_interacts(slot.drain_interacts(), slot_input));
                }
            }
        } else if slot.state() == script::RunState::Running {
            interact.extend(take_script_interacts(slot.drain_interacts(), slot_input));
        }
    }
    // Dispatch the shim's interact requests through the slot's own Driver
    // (open/deposit/withdraw) and the shared walk arm (bank-stand walks
    // with default FindOptions, so wilderness/quest gates fail closed).
    // The guardian's hold drops them: the script's parked wait stays
    // frozen, and a later retry re-queues what still matters.
    if up && !hold && !interact.is_empty() {
        if let Some(snapshot) = snapshot {
            let mut dispatchable = Vec::with_capacity(interact.len());
            let mut armed = None;
            let mut armed_bank_op = None;
            let mut rejected_withdraw_x = 0usize;
            let mut rejected_withdraw_load = 0usize;
            let mut rejected_bank_op = 0usize;
            for req in interact {
                match req {
                    req @ (script::shim::InteractReq::Deposit { .. }
                    | script::shim::InteractReq::Withdraw { .. }) => {
                        if pending_bank_op_active || armed_bank_op.is_some() {
                            rejected_bank_op += 1;
                        } else if let Some(pending) =
                            dispatch_observed_bank_op(driver, snapshot, obj_names, inv, &req)
                        {
                            armed_bank_op = Some(pending);
                            pending_bank_op_active = true;
                            wrote = true;
                        } else {
                            rejected_bank_op += 1;
                        }
                    }
                    script::shim::InteractReq::WithdrawX {
                        name: item_name,
                        count,
                        bank_item_id,
                        lands_as_id,
                        action,
                        bank_generation,
                    } if armed.is_none()
                        && !pending_withdraw_x_active
                        && count > 0
                        && snapshot.bank_component_id() >= 0
                        && snapshot.bank_loaded()
                        && !snapshot.count_dialog_open()
                        && snapshot.bank_session_generation() == bank_generation =>
                    {
                        let mut accepted = false;
                        let wanted = item_name.to_lowercase();
                        let mut ix = api::interact::Interactions::new(snapshot, driver);
                        if let Some(item) = snapshot.bank().iter().find(|item| {
                            item.def.id == bank_item_id
                                && obj_names
                                    .and_then(|names| names.name(item.def.id))
                                    .is_some_and(|name| name.eq_ignore_ascii_case(&wanted))
                        }) {
                            if let Some(op) = action_slot(&item.actions, &action) {
                                let sent = matches!(
                                    ix.interact(
                                        api::interact::OpTarget::Item(item),
                                        api::interact::ActionSpec::Operation(op),
                                    ),
                                    api::interact::SendResult::Sent { .. }
                                );
                                if sent {
                                    let before = inv.map_or_else(
                                        || {
                                            snapshot
                                                .inventory()
                                                .iter()
                                                .filter(|held| held.def.id == lands_as_id)
                                                .map(|held| held.count)
                                                .sum()
                                        },
                                        |rows| {
                                            rows.iter()
                                                .filter(|(id, _)| *id == lands_as_id)
                                                .map(|(_, count)| *count)
                                                .sum()
                                        },
                                    );
                                    wrote = true;
                                    let pending = script::slot::PendingWithdrawX::waiting_dialog(
                                        lands_as_id,
                                        count,
                                        before,
                                        before.saturating_add(count.min(item.count)),
                                        bank_generation,
                                    );
                                    armed = Some(
                                        if matches!(count, 1 | 5 | 10)
                                            && action_slot(
                                                &item.actions,
                                                &format!("Withdraw {count}"),
                                            ) == Some(op)
                                        {
                                            pending.waiting_settlement()
                                        } else {
                                            pending
                                        },
                                    );
                                    pending_withdraw_x_active = true;
                                    accepted = true;
                                }
                            }
                        }
                        if !accepted {
                            rejected_withdraw_x += 1;
                        }
                    }
                    script::shim::InteractReq::WithdrawX { .. } => {
                        rejected_withdraw_x += 1;
                    }
                    script::shim::InteractReq::WithdrawLoad {
                        name: item_name,
                        bank_generation,
                    } if armed.is_none()
                        && !pending_withdraw_x_active
                        && snapshot.bank_component_id() >= 0
                        && snapshot.bank_loaded()
                        && !snapshot.count_dialog_open()
                        && snapshot.bank_session_generation() == bank_generation =>
                    {
                        let capacity = snapshot.inventory_size().max(0) as usize;
                        let used = inv.map_or_else(|| snapshot.inventory().len(), <[_]>::len);
                        let free = capacity.saturating_sub(used);
                        let mut accepted = false;
                        if free > 0 {
                            let mut ix = api::interact::Interactions::new(snapshot, driver);
                            let wanted = item_name.to_lowercase();
                            if let Some(item) = snapshot.bank().iter().find(|item| {
                                item.count > 0
                                    && obj_names
                                        .and_then(|names| names.name(item.def.id))
                                        .is_some_and(|name| name.eq_ignore_ascii_case(&wanted))
                            }) {
                                let bank_item_id = item.def.id;
                                let count = item.count.min(free as i32);
                                if let Some((op, needs_dialog)) =
                                    fill_withdraw_action(&item.actions, count, item.count)
                                {
                                    let sent = matches!(
                                        ix.interact(
                                            api::interact::OpTarget::Item(item),
                                            api::interact::ActionSpec::Operation(op),
                                        ),
                                        api::interact::SendResult::Sent { .. }
                                    );
                                    if sent {
                                        let before_count = snapshot
                                            .inventory()
                                            .iter()
                                            .filter(|held| held.def.id == bank_item_id)
                                            .map(|held| held.count)
                                            .sum();
                                        let pending =
                                            script::slot::PendingWithdrawX::waiting_load_dialog(
                                                bank_item_id,
                                                count,
                                                used,
                                                before_count,
                                                item.count,
                                                bank_generation,
                                            );
                                        armed = Some(if needs_dialog {
                                            pending
                                        } else {
                                            pending.waiting_settlement()
                                        });
                                        wrote = true;
                                        pending_withdraw_x_active = true;
                                        accepted = true;
                                    }
                                }
                            }
                        }
                        if !accepted {
                            rejected_withdraw_load += 1;
                        }
                    }
                    script::shim::InteractReq::WithdrawLoad { .. } => {
                        rejected_withdraw_load += 1;
                    }
                    req => dispatchable.push(req),
                }
            }
            wrote |= dispatch_script_interact(
                driver,
                snapshot,
                obj_names,
                here,
                navs,
                world,
                state.clone(),
                name,
                dispatchable,
            );
            if armed.is_some()
                || armed_bank_op.is_some()
                || rejected_withdraw_x != 0
                || rejected_withdraw_load != 0
                || rejected_bank_op != 0
            {
                if let Some(slot) = script_slot(scripts, name) {
                    let mut slot = slot.lock().unwrap();
                    if Some(slot.work_epoch()) == slot_work_epoch {
                        for _ in 0..rejected_withdraw_x {
                            slot.complete_withdraw_x(false);
                        }
                        for _ in 0..rejected_withdraw_load {
                            slot.complete_withdraw_load(false);
                        }
                        for _ in 0..rejected_bank_op {
                            slot.complete_bank_op(false);
                        }
                        if let Some(pending) = armed_bank_op {
                            if matches!(
                                slot.state(),
                                script::RunState::Running | script::RunState::Paused
                            ) {
                                slot.set_pending_bank_op(Some(pending));
                                if slot.state() == script::RunState::Paused {
                                    slot.freeze_pending_bank_op();
                                }
                            }
                        }
                        if let Some(pending) = armed {
                            if matches!(
                                slot.state(),
                                script::RunState::Running | script::RunState::Paused
                            ) {
                                slot.set_pending_withdraw_x(Some(pending));
                                if slot.state() == script::RunState::Paused {
                                    slot.freeze_pending_withdraw_x();
                                }
                            }
                        }
                    }
                }
            }
        } else {
            let rejected_x = interact
                .iter()
                .filter(|req| matches!(req, script::shim::InteractReq::WithdrawX { .. }))
                .count();
            let rejected_load = interact
                .iter()
                .filter(|req| matches!(req, script::shim::InteractReq::WithdrawLoad { .. }))
                .count();
            let rejected_bank = interact
                .iter()
                .filter(|req| {
                    matches!(
                        req,
                        script::shim::InteractReq::Deposit { .. }
                            | script::shim::InteractReq::Withdraw { .. }
                    )
                })
                .count();
            if rejected_x != 0 || rejected_load != 0 || rejected_bank != 0 {
                if let Some(slot) = script_slot(scripts, name) {
                    let mut slot = slot.lock().unwrap();
                    if Some(slot.work_epoch()) == slot_work_epoch {
                        for _ in 0..rejected_x {
                            slot.complete_withdraw_x(false);
                        }
                        for _ in 0..rejected_load {
                            slot.complete_withdraw_load(false);
                        }
                        for _ in 0..rejected_bank {
                            slot.complete_bank_op(false);
                        }
                    }
                }
            }
        }
    } else if !interact.is_empty() {
        let rejected_x = interact
            .iter()
            .filter(|req| matches!(req, script::shim::InteractReq::WithdrawX { .. }))
            .count();
        let rejected_load = interact
            .iter()
            .filter(|req| matches!(req, script::shim::InteractReq::WithdrawLoad { .. }))
            .count();
        let rejected_bank = interact
            .iter()
            .filter(|req| {
                matches!(
                    req,
                    script::shim::InteractReq::Deposit { .. }
                        | script::shim::InteractReq::Withdraw { .. }
                )
            })
            .count();
        if rejected_x != 0 || rejected_load != 0 || rejected_bank != 0 {
            if let Some(slot) = script_slot(scripts, name) {
                let mut slot = slot.lock().unwrap();
                if Some(slot.work_epoch()) == slot_work_epoch {
                    for _ in 0..rejected_x {
                        slot.complete_withdraw_x(false);
                    }
                    for _ in 0..rejected_load {
                        slot.complete_withdraw_load(false);
                    }
                    for _ in 0..rejected_bank {
                        slot.complete_bank_op(false);
                    }
                }
            }
        }
    }
    let cmds = {
        let mut all = cheats.lock().unwrap();
        all.get_mut(name).map(std::mem::take).unwrap_or_default()
    };
    for cmd in cmds.into_iter().filter(|_| up) {
        api::interact::cheat(driver, &cmd);
        wrote = true;
    }
    wrote
}

#[cfg(test)]
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
    scripts: &ScriptWall,
    cheats: &Arc<Mutex<HashMap<String, VecDeque<String>>>>,
    navs: &Arc<Mutex<HashMap<String, NavBot>>>,
    world: &Option<Arc<NavWorld>>,
    hold: bool,
    ours: bool,
) -> bool {
    script_observe_with_npc_boxes(
        driver, name, up, tick_edge, tick, here, inv, state, snapshot, None, obj_names, scripts,
        cheats, navs, world, hold, ours, None, None,
    )
}

fn take_script_interacts(
    reqs: Vec<script::shim::InteractReq>,
    slot_input: Option<&SlotInput>,
) -> Vec<script::shim::InteractReq> {
    let mut out = Vec::with_capacity(reqs.len());
    for req in reqs {
        match req {
            script::shim::InteractReq::Mouse {
                down,
                x,
                y,
                button,
                identity,
            } => {
                if let Some(inp) = slot_input {
                    inp.enqueue_script_mouse_at(identity, down, x, y, button);
                }
            }
            other => out.push(other),
        }
    }
    out
}

fn dispatch_observed_bank_op(
    driver: &mut dyn Driver,
    snapshot: &GameSnapshot,
    obj_names: Option<&api::obj_names::ObjNames>,
    inventory: Option<&[(i32, i32)]>,
    req: &script::shim::InteractReq,
) -> Option<script::slot::PendingBankOp> {
    use api::interact::{ActionSpec, Interactions, OpTarget, SendResult};
    use script::shim::InteractReq;
    use script::slot::{PendingBankOp, PendingBankOpKind};

    if snapshot.bank_component_id() < 0 || !snapshot.bank_loaded() {
        return None;
    }
    let generation = snapshot.bank_session_generation();
    let inventory_count = |id| {
        inventory.map_or(0, |items| {
            items
                .iter()
                .filter(|(item_id, _)| *item_id == id)
                .map(|(_, count)| *count)
                .sum()
        })
    };
    match req {
        InteractReq::Deposit { name } => {
            let item = snapshot.bank_side().iter().find(|item| {
                obj_names
                    .and_then(|names| names.name(item.def.id))
                    .is_some_and(|actual| actual.eq_ignore_ascii_case(name))
            })?;
            let op = all_slot(&item.actions)?;
            let before_count = snapshot
                .bank_side()
                .iter()
                .filter(|row| row.def.id == item.def.id)
                .map(|row| row.count)
                .sum();
            matches!(
                Interactions::new(snapshot, driver)
                    .interact(OpTarget::Item(item), ActionSpec::Operation(op)),
                SendResult::Sent { .. }
            )
            .then(|| {
                PendingBankOp::new(
                    PendingBankOpKind::Deposit,
                    item.def.id,
                    before_count,
                    inventory_count(item.def.id),
                    generation,
                )
            })
        }
        InteractReq::Withdraw { name, action } => {
            let item = snapshot.bank().iter().find(|item| {
                obj_names
                    .and_then(|names| names.name(item.def.id))
                    .is_some_and(|actual| actual.eq_ignore_ascii_case(name))
            })?;
            let op = action_slot(&item.actions, action)?;
            let kind = if open_only_withdraw_x(&item.actions, op) {
                PendingBankOpKind::WithdrawXAction
            } else {
                PendingBankOpKind::Withdraw
            };
            let before_count = snapshot
                .bank()
                .iter()
                .filter(|row| row.def.id == item.def.id)
                .map(|row| row.count)
                .sum();
            matches!(
                Interactions::new(snapshot, driver)
                    .interact(OpTarget::Item(item), ActionSpec::Operation(op)),
                SendResult::Sent { .. }
            )
            .then(|| {
                PendingBankOp::new(
                    kind,
                    item.def.id,
                    before_count,
                    inventory_count(item.def.id),
                    generation,
                )
            })
        }
        _ => None,
    }
}

fn nearest_bank_booth(world: &NavWorld, (x, z, level): (i32, i32, i32)) -> Option<WorldTile> {
    world
        .banks()
        .iter()
        .filter(|stand| matches!(stand.access, nav::pack::BankAccess::Booth { .. }))
        .min_by_key(|stand| {
            let distance = stand.tile.x.abs_diff(x).max(stand.tile.z.abs_diff(z));
            if stand.tile.level == level {
                u64::from(distance)
            } else {
                (u64::MAX / 2).saturating_add(u64::from(distance))
            }
        })
        .map(|stand| stand.tile)
}

/// Dispatch one isolate's shim interact requests. Open/close/deposit/
/// withdraw run through [`api::interact::Interactions`] on the slot's
/// snapshot + Driver — a request whose target is missing (no loc at the
/// tile, no bank-side row with the resolved name, no bank open) fails
/// closed with no send. `walk` / `walk-near` route through the shared
/// [`ScriptWalkArm`] with the request's three FindOptions bits (serde/old
/// wire default off). `walk-to` (scene `DirectNavigator`) is the
/// [`Interactions::walk`] packet. Returns whether the driver's out buffer
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
    #[cfg(feature = "memory-profile")]
    memory_diagnostics::requests(name, &reqs);
    let mut wrote = false;
    let camera_yaws: Vec<i32> = reqs
        .iter()
        .filter_map(|req| {
            if let InteractReq::SetCameraYaw { yaw } = req {
                Some(*yaw)
            } else {
                None
            }
        })
        .collect();
    let mut ix = api::interact::Interactions::new(snapshot, driver);
    if debug_enabled() {
        for req in &reqs {
            eprintln!("[script {name}] interact {req:?}");
        }
    }
    for req in reqs {
        match req {
            InteractReq::OpenBooth {
                x,
                z,
                level,
                id,
                name: booth_name,
                action,
            } => {
                let target = api::snapshot::WorldTile { x, z, level };
                let accepted = match (booth_name, action) {
                    (Some(name), Some(action)) => matches!(
                        ix.open_named_booth_at(target, id, &name, &action),
                        SendResult::Sent { .. }
                    ),
                    (None, None) => {
                        matches!(ix.open_booth_at(target, id), SendResult::Sent { .. })
                    }
                    _ => false,
                };
                if accepted {
                    // The interaction supersedes any earlier scripted walk.
                    // Cancel after validation/dispatch only: rejected or
                    // malformed requests must not disturb an armed route.
                    abort_script_walk(navs, name);
                }
                wrote |= accepted;
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
                    let loc = snapshot.locs().iter().find(|loc| {
                        loc.tile.x == x
                            && loc.tile.z == z
                            && loc.tile.level == level
                            && name.as_deref().is_none_or(|wanted| {
                                loc.name
                                    .as_deref()
                                    .is_some_and(|actual| actual.eq_ignore_ascii_case(wanted))
                            })
                    });
                    if let Some(loc) = loc {
                        let op = stand_op.or_else(|| action_slot(&loc.actions, "Use-quickly"));
                        if let Some(op) = op {
                            wrote |= matches!(
                                ix.interact(OpTarget::Loc(loc), ActionSpec::Operation(op)),
                                SendResult::Sent { .. }
                            );
                        }
                    }
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
            InteractReq::Walk {
                x,
                z,
                level,
                allow_teleports,
                allow_wilderness,
                allow_bank_fetch,
                request_id,
            } => {
                let bank_rows: Vec<(i32, i32)> = snapshot
                    .bank()
                    .iter()
                    .map(|it| (it.def.id, it.count))
                    .collect();
                let arm = ScriptWalkArm {
                    here,
                    world: world.clone(),
                    navs: Arc::clone(navs),
                    name: name.to_string(),
                    state: state.clone(),
                    bank: bank_rows,
                };
                wrote |= arm.queue_route(
                    x,
                    z,
                    level,
                    FindOptions {
                        allow_teleports,
                        allow_wilderness,
                        allow_bank_fetch,
                        ..FindOptions::default()
                    },
                    0,
                    false,
                    request_id,
                );
            }
            InteractReq::WalkNear {
                x,
                z,
                level,
                radius,
                allow_teleports,
                allow_wilderness,
                allow_bank_fetch,
                request_id,
            } => {
                let arm = ScriptWalkArm {
                    here,
                    world: world.clone(),
                    navs: Arc::clone(navs),
                    name: name.to_string(),
                    state: state.clone(),
                    bank: snapshot
                        .bank()
                        .iter()
                        .map(|it| (it.def.id, it.count))
                        .collect(),
                };
                wrote |= arm.queue_route(
                    x,
                    z,
                    level,
                    FindOptions {
                        allow_teleports,
                        allow_wilderness,
                        allow_bank_fetch,
                        ..FindOptions::default()
                    },
                    radius,
                    true,
                    request_id,
                );
            }
            InteractReq::WalkNearestBank => {
                if let (Some((hx, hz, hl)), Some(nav_world)) = (here, world.as_deref()) {
                    if let Some(tile) = nearest_bank_booth(nav_world, (hx, hz, hl)) {
                        let arm = ScriptWalkArm {
                            here,
                            world: world.clone(),
                            navs: Arc::clone(navs),
                            name: name.to_string(),
                            state: state.clone(),
                            bank: snapshot
                                .bank()
                                .iter()
                                .map(|item| (item.def.id, item.count))
                                .collect(),
                        };
                        wrote |= arm.route_with_radius(
                            tile.x,
                            tile.z,
                            tile.level,
                            FindOptions::default(),
                            1,
                        );
                    }
                }
            }
            InteractReq::WalkTo { x, z, level } => {
                wrote |= matches!(ix.walk(WorldTile { x, z, level }), SendResult::Sent { .. });
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
                        let res = ix.interact(OpTarget::Item(item), ActionSpec::Operation(op));
                        if host::debug_enabled() {
                            let outcome = match &res {
                                SendResult::Sent { .. } => "sent".to_string(),
                                SendResult::Refused { reason, .. } => {
                                    format!("refused {reason:?}")
                                }
                            };
                            eprintln!("[shim-withdraw] {name} {action} -> {outcome}");
                        }
                        wrote |= matches!(res, SendResult::Sent { .. });
                    } else if host::debug_enabled() {
                        eprintln!("[shim-withdraw] {name} {action} -> no op slot");
                    }
                } else if host::debug_enabled() {
                    eprintln!("[shim-withdraw] {name} {action} -> no bank row");
                }
            }
            // `script_observe` consumes this variant so it can arm the
            // slot-owned continuation only after the X action was sent.
            // Direct callers cannot safely create host pending state.
            InteractReq::WithdrawX { .. } | InteractReq::WithdrawLoad { .. } => {}
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
                    let res = ix.interact(OpTarget::Item(item), ActionSpec::Label(action.clone()));
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
            InteractReq::InvButton {
                id,
                slot,
                component,
                operation,
                bank_generation,
            } => {
                // Selected bank withdraw or bank-side deposit identity, or the
                // open trade offer/side row. Exact id/slot/component only —
                // no same-name fallback, no count-dialog answer.
                if snapshot.bank_component_id() >= 0
                    && snapshot.bank_loaded()
                    && snapshot.bank_session_generation() == bank_generation
                {
                    if let Some(item) = snapshot
                        .bank()
                        .iter()
                        .chain(snapshot.bank_side().iter())
                        .find(|item| {
                            item.def.id == id && item.slot == slot && item.component_id == component
                        })
                    {
                        wrote |= matches!(
                            ix.interact(OpTarget::Item(item), ActionSpec::Operation(operation)),
                            SendResult::Sent { .. }
                        );
                    }
                } else if snapshot.trade().offer_open {
                    if let Some(item) = snapshot
                        .trade()
                        .side_pack
                        .iter()
                        .chain(snapshot.trade().my_offer.iter())
                        .find(|item| {
                            item.def.id == id && item.slot == slot && item.component_id == component
                        })
                    {
                        wrote |= matches!(
                            ix.interact(OpTarget::Item(item), ActionSpec::Operation(operation)),
                            SendResult::Sent { .. }
                        );
                    }
                }
            }
            InteractReq::ShopButton {
                kind,
                name,
                id,
                slot,
                component,
                chunk,
            } => {
                // Sell acts on the shop's own player pack and Buy on its
                // stock: never the other container, never the backpack. The
                // exact posted row must still be there (no same-name
                // fallback), then the api op re-resolves and sends it.
                let rows: &[api::snapshot::ItemView] = if kind == "sell" {
                    &snapshot.shop().player
                } else {
                    &snapshot.shop().stock
                };
                let present = rows
                    .iter()
                    .any(|it| it.def.id == id && it.slot == slot && it.component_id == component);
                if present {
                    let sent = if kind == "sell" {
                        ix.shop_sell(&name, chunk)
                    } else {
                        ix.shop_buy(&name, chunk)
                    };
                    wrote |= matches!(sent, SendResult::Sent { .. });
                }
            }
            InteractReq::MakePanel {
                id,
                slot,
                component,
                operation,
            } => {
                // Anvil/main skill-multi identity only. Do not fall back to
                // another same-name row, the backpack, or a count dialog.
                let present = snapshot.main_make().iter().any(|item| {
                    item.def.id == id && item.slot == slot && item.component_id == component
                });
                if present {
                    if let Some(item) = snapshot.main_make().iter().find(|item| {
                        item.def.id == id && item.slot == slot && item.component_id == component
                    }) {
                        wrote |= matches!(
                            ix.interact(OpTarget::Item(item), ActionSpec::Operation(operation)),
                            SendResult::Sent { .. }
                        );
                    }
                }
            }
            InteractReq::Close => {
                let res = ix.close_modal();
                if host::debug_enabled() {
                    match &res {
                        SendResult::Sent { .. } => eprintln!("[shim-close] sent"),
                        SendResult::Refused { reason, .. } => {
                            eprintln!("[shim-close] refused {reason:?}")
                        }
                    }
                }
                wrote |= matches!(res, SendResult::Sent { .. });
            }
            InteractReq::Npc {
                name,
                action,
                index,
            } => {
                let wanted = name.to_lowercase();
                let npc = snapshot.npcs().iter().find(|n| {
                    if let Some(i) = index {
                        n.index as i32 == i
                    } else {
                        n.name
                            .as_deref()
                            .is_some_and(|n| n.eq_ignore_ascii_case(&wanted))
                    }
                });
                if let Some(npc) = npc {
                    wrote |= matches!(
                        ix.interact(OpTarget::Npc(npc), ActionSpec::Label(action.clone())),
                        SendResult::Sent { .. }
                    );
                }
            }
            InteractReq::Loc {
                x,
                z,
                level,
                action,
                id,
            } => {
                // Selected `id` must match a current row at this tile.
                // Do not fall back to another co-located loc. Absent id
                // keeps the previous first-row coordinate match.
                let loc = snapshot.locs().iter().find(|l| {
                    l.tile.x == x
                        && l.tile.z == z
                        && l.tile.level == level
                        && id.is_none_or(|wanted| l.id == wanted)
                });
                if let Some(loc) = loc {
                    wrote |= matches!(
                        ix.interact(OpTarget::Loc(loc), ActionSpec::Label(action.clone())),
                        SendResult::Sent { .. }
                    );
                }
            }
            InteractReq::Obj {
                x,
                z,
                level,
                name,
                action,
            } => {
                let obj = snapshot.ground_items().iter().find(|it| {
                    it.tile.x == x
                        && it.tile.z == z
                        && it.tile.level == level
                        && name.as_deref().is_none_or(|wanted| {
                            obj_names
                                .and_then(|n| n.name(it.def.id))
                                .is_some_and(|n| n.eq_ignore_ascii_case(wanted))
                        })
                });
                if let Some(obj) = obj {
                    wrote |= matches!(
                        ix.interact(OpTarget::GroundItem(obj), ActionSpec::Label(action.clone())),
                        SendResult::Sent { .. }
                    );
                }
            }
            InteractReq::Player { name, action } => {
                let wanted = name.to_lowercase();
                let player = snapshot.players().iter().find(|p| {
                    p.actor
                        .name
                        .as_deref()
                        .is_some_and(|n| n.eq_ignore_ascii_case(&wanted))
                });
                if let Some(player) = player {
                    wrote |= matches!(
                        ix.interact(OpTarget::Player(player), ActionSpec::Label(action.clone())),
                        SendResult::Sent { .. }
                    );
                }
            }
            InteractReq::UseOn {
                name,
                kind,
                target_name,
                x,
                z,
                level,
                index,
                source_item_id,
                source_item_slot,
                target_item_id,
                target_item_slot,
            } => {
                let item = resolve_inventory_item(
                    snapshot,
                    obj_names,
                    Some(&name),
                    source_item_id,
                    source_item_slot,
                );
                if let Some(item) = item {
                    if let Some(target) = resolve_op_target(
                        snapshot,
                        obj_names,
                        &kind,
                        target_name.as_deref(),
                        target_item_id,
                        target_item_slot,
                        x,
                        z,
                        level,
                        index,
                    ) {
                        wrote |= matches!(ix.use_item_on(item, target), SendResult::Sent { .. });
                    }
                }
            }
            InteractReq::UseWidgetOn {
                component_id,
                kind,
                target_name,
                x,
                z,
                level,
                index,
            } => {
                let ctx = api::snapshot::ReadContext::new(snapshot);
                if let Some(widget) = ctx.component(component_id) {
                    if let Some(target) = resolve_op_target(
                        snapshot,
                        obj_names,
                        &kind,
                        target_name.as_deref(),
                        None,
                        None,
                        x,
                        z,
                        level,
                        index,
                    ) {
                        wrote |=
                            matches!(ix.use_widget_on(widget, target), SendResult::Sent { .. });
                    }
                }
            }
            InteractReq::ContinueDialog => {
                wrote |= matches!(ix.continue_dialog(), SendResult::Sent { .. });
            }
            InteractReq::Answer { option } => {
                wrote |= matches!(ix.answer_choice(option), SendResult::Sent { .. });
            }
            InteractReq::IfButton { component_id } => {
                let ctx = api::snapshot::ReadContext::new(snapshot);
                if let Some(widget) = ctx.component(component_id) {
                    wrote |= matches!(ix.press(widget), SendResult::Sent { .. });
                }
            }
            InteractReq::CloseModal => {
                wrote |= matches!(ix.close_modal(), SendResult::Sent { .. });
            }
            InteractReq::AnswerCount { value } => {
                let res = ix.answer_count(value);
                if host::debug_enabled() {
                    match &res {
                        SendResult::Sent { .. } => {
                            eprintln!("[shim-count] {value} sent")
                        }
                        SendResult::Refused { reason, .. } => {
                            eprintln!("[shim-count] {value} refused {reason:?}")
                        }
                    }
                }
                wrote |= matches!(res, SendResult::Sent { .. });
            }
            InteractReq::SideTab { tab } => {
                wrote |= matches!(ix.click_side_tab(tab), SendResult::Sent { .. });
            }
            InteractReq::Wear { name } => {
                let wanted = name.to_lowercase();
                let id = snapshot.inventory().iter().find_map(|it| {
                    obj_names
                        .and_then(|n| n.name(it.def.id))
                        .filter(|n| n.eq_ignore_ascii_case(&wanted))
                        .map(|_| it.def.id)
                });
                if let Some(id) = id {
                    let res = ix.wear(id);
                    if host::debug_enabled() {
                        let outcome = match &res {
                            SendResult::Sent { .. } => "sent".to_string(),
                            SendResult::Refused { reason, .. } => format!("refused {reason:?}"),
                        };
                        eprintln!(
                            "[shim-wear] {name} id={id} inv={} zip={} -> {outcome}",
                            snapshot.inventory().len(),
                            snapshot.inv().len()
                        );
                    }
                    wrote |= matches!(res, SendResult::Sent { .. });
                } else if host::debug_enabled() {
                    eprintln!(
                        "[shim-wear] {name} no inventory() row inv={} zip={:?}",
                        snapshot.inventory().len(),
                        snapshot.inv()
                    );
                }
            }
            InteractReq::SetRun { on } => {
                wrote |= matches!(ix.set_run(on), SendResult::Sent { .. });
            }
            InteractReq::SetRetaliate { on } => {
                wrote |= matches!(ix.set_retaliate(on), SendResult::Sent { .. });
            }
            InteractReq::SetNoteMode { on } => {
                let res = ix.set_note_mode(on);
                if host::debug_enabled() {
                    match &res {
                        SendResult::Sent { .. } => eprintln!("[shim-note] on={on} sent"),
                        SendResult::Refused { reason, .. } => {
                            eprintln!("[shim-note] on={on} refused {reason:?}")
                        }
                    }
                }
                wrote |= matches!(res, SendResult::Sent { .. });
            }
            InteractReq::Key { down, key, .. } => {
                wrote |= ix.apply_amount_key(down, &key);
            }
            InteractReq::Mouse { .. } => {}
            InteractReq::SetCameraYaw { .. } => {}
            InteractReq::NoteProgress
            | InteractReq::LoopSettled
            | InteractReq::WaitEnqueued
            | InteractReq::WaitSettled
            | InteractReq::RecoveryAnchor { .. }
            | InteractReq::RecoveryAnchorNone => {}
        }
    }
    for yaw in camera_yaws {
        wrote |= driver.set_orbit_camera_yaw(yaw);
    }
    #[cfg(feature = "memory-profile")]
    memory_diagnostics::sent(name, wrote);
    wrote
}

fn abort_script_walk(navs: &Arc<Mutex<HashMap<String, NavBot>>>, name: &str) {
    if let Some(bot) = navs.lock().unwrap().get_mut(name) {
        bot.route_generation = bot.route_generation.wrapping_add(1);
        bot.route = None;
        bot.route_worker = None;
        bot.pending_route = None;
        bot.requested_route = None;
        bot.walk_request_id = 0;
        bot.clear_walk_outcome();
    }
}

fn recovery_walk_idle(navs: &Arc<Mutex<HashMap<String, NavBot>>>, name: &str) -> bool {
    let all = navs.lock().unwrap();
    let Some(bot) = all.get(name) else {
        return true;
    };
    bot.route_worker.is_none() && bot.route.is_none() && bot.pending_route.is_none()
}

fn apply_watchdog_nav_action(
    action: script::WatchdogAction,
    _driver: &mut dyn Driver,
    snapshot: Option<&GameSnapshot>,
    here: Option<(i32, i32, i32)>,
    navs: &Arc<Mutex<HashMap<String, NavBot>>>,
    world: &Option<Arc<NavWorld>>,
    state: Option<WorldState>,
    name: &str,
) {
    match action {
        script::WatchdogAction::AbortWalk => abort_script_walk(navs, name),
        script::WatchdogAction::ArmWalk { x, z, level } => {
            let Some(snapshot) = snapshot else {
                return;
            };
            let arm = ScriptWalkArm {
                here,
                world: world.clone(),
                navs: Arc::clone(navs),
                name: name.to_string(),
                state,
                bank: snapshot
                    .bank()
                    .iter()
                    .map(|item| (item.def.id, item.count))
                    .collect(),
            };
            let armed = arm.route_with_radius(
                x,
                z,
                level,
                FindOptions::default(),
                script::watchdog::WALK_RADIUS,
            );
            if !armed {
                abort_script_walk(navs, name);
            }
        }
        _ => {}
    }
}

#[allow(clippy::too_many_arguments)]
fn resolve_op_target<'a>(
    snapshot: &'a GameSnapshot,
    obj_names: Option<&api::obj_names::ObjNames>,
    kind: &str,
    target_name: Option<&str>,
    target_item_id: Option<i32>,
    target_item_slot: Option<i32>,
    x: i32,
    z: i32,
    level: i32,
    index: Option<i32>,
) -> Option<api::interact::OpTarget<'a>> {
    use api::interact::OpTarget;
    match kind {
        "npc" => {
            let wanted = target_name.map(|n| n.to_lowercase());
            snapshot
                .npcs()
                .iter()
                .find(|n| {
                    if let Some(i) = index {
                        n.index as i32 == i
                    } else {
                        wanted.as_ref().is_some_and(|w| {
                            n.name.as_deref().is_some_and(|n| n.eq_ignore_ascii_case(w))
                        })
                    }
                })
                .map(OpTarget::Npc)
        }
        "loc" => snapshot
            .locs()
            .iter()
            .find(|l| l.tile.x == x && l.tile.z == z && l.tile.level == level)
            .map(OpTarget::Loc),
        "obj" => snapshot
            .ground_items()
            .iter()
            .find(|it| {
                it.tile.x == x
                    && it.tile.z == z
                    && it.tile.level == level
                    && target_name.is_none_or(|wanted| {
                        obj_names
                            .and_then(|n| n.name(it.def.id))
                            .is_some_and(|n| n.eq_ignore_ascii_case(wanted))
                    })
            })
            .map(OpTarget::GroundItem),
        "player" => {
            let wanted = target_name.map(|n| n.to_lowercase());
            snapshot
                .players()
                .iter()
                .find(|p| {
                    wanted.as_ref().is_some_and(|w| {
                        p.actor
                            .name
                            .as_deref()
                            .is_some_and(|n| n.eq_ignore_ascii_case(w))
                    })
                })
                .map(OpTarget::Player)
        }
        "inv" | "held" => resolve_inventory_item(
            snapshot,
            obj_names,
            target_name,
            target_item_id,
            target_item_slot,
        )
        .map(OpTarget::Item),
        _ => None,
    }
}

fn resolve_inventory_item<'a>(
    snapshot: &'a GameSnapshot,
    obj_names: Option<&api::obj_names::ObjNames>,
    name: Option<&str>,
    item_id: Option<i32>,
    item_slot: Option<i32>,
) -> Option<&'a api::snapshot::ItemView> {
    match (item_id, item_slot) {
        (Some(id), Some(slot)) => snapshot
            .inventory()
            .iter()
            .find(|item| item.def.id == id && item.slot == slot),
        (None, None) => {
            let wanted = name?.to_lowercase();
            snapshot.inventory().iter().find(|item| {
                obj_names
                    .and_then(|names| names.name(item.def.id))
                    .is_some_and(|actual| actual.eq_ignore_ascii_case(&wanted))
            })
        }
        _ => None,
    }
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

/// Whether the resolved bank op is the open-only `Withdraw X` label: it
/// opens the amount dialog and cannot move inventory on its own, so the
/// accepted send *is* the result (frozen `Bank.withdraw` returns
/// `Input.invButton` → `actions.menuAction`). Every other withdraw label
/// settles on the observed delta.
fn open_only_withdraw_x(actions: &[Option<String>], op: i32) -> bool {
    let index = match usize::try_from(op) {
        Ok(index) if index >= 1 => index - 1,
        _ => return false,
    };
    actions
        .get(index)
        .and_then(|label| label.as_deref())
        .is_some_and(|label| norm_action(label) == norm_action("Withdraw X"))
}

/// The bank-side op slot whose label contains "all" (Deposit All / the
/// deposit window's bulk op), 1-based.
fn all_slot(actions: &[Option<String>]) -> Option<i32> {
    actions
        .iter()
        .position(|a| a.as_deref().is_some_and(|s| norm_action(s).contains("all")))
        .map(|i| i as i32 + 1)
}

/// Select one bounded fill operation in compatibility order: an exact
/// amount, then All only when the requested fill consumes current stock,
/// then the host-owned X continuation.
fn fill_withdraw_action(actions: &[Option<String>], count: i32, stock: i32) -> Option<(i32, bool)> {
    action_slot(actions, &format!("Withdraw {count}"))
        .map(|op| (op, false))
        .or_else(|| {
            (count == stock)
                .then(|| all_slot(actions).map(|op| (op, false)))
                .flatten()
        })
        .or_else(|| action_slot(actions, "Withdraw X").map(|op| (op, true)))
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
    teleports_enabled: bool,
) -> (Vec<u8>, script::isolate_fb::SnapshotFingerprint) {
    with_script_snapshot_input(
        tick,
        here,
        ingame,
        inv,
        snapshot,
        obj_names,
        world,
        None,
        hold,
        ours,
        teleports_enabled,
        0,
        false,
        0,
        false,
        0,
        false,
        None,
        PostedWalkOutcome::default(),
        |input, native| {
            script::isolate_fb::encode_snapshot_delta_with_native(last, input, native, force_banks)
        },
    )
}

/// Note-mode landing id for an unnoted obj (`ObjNames` reverse cert), else
/// the snapshot def's raw `certlink`.
fn posted_cert(
    obj_names: Option<&api::obj_names::ObjNames>,
    def: &api::obj_names::ItemDefView,
) -> i32 {
    obj_names
        .and_then(|n| n.item(def.id))
        .map(|d| d.certificate_link)
        .filter(|&c| c >= 0)
        .unwrap_or(def.certificate_link)
}

fn posted_cert_id(obj_names: Option<&api::obj_names::ObjNames>, id: i32) -> i32 {
    obj_names
        .and_then(|n| n.item(id))
        .map(|d| d.certificate_link)
        .filter(|&c| c >= 0)
        .unwrap_or(-1)
}

/// Host-published walk outcome copied onto the isolate snapshot.
#[derive(Clone, Copy, Default)]
struct PostedWalkOutcome {
    seq: u64,
    generation: u64,
    request_id: u64,
    failed: bool,
    x: i32,
    z: i32,
    level: i32,
    radius: i32,
    allow_teleports: bool,
}

/// Build the observed snapshot input and hand it to `f`. The live observe
/// path encodes through the slot's reusable [`script::isolate_fb::IsolateBuf`];
/// tests encode through a one-shot builder via [`script_snapshot_fb`].
#[allow(clippy::too_many_arguments, unused_assignments)]
fn with_script_snapshot_input<R>(
    tick: u64,
    here: Option<(i32, i32, i32)>,
    ingame: bool,
    inv: Option<&[(i32, i32)]>,
    snapshot: Option<&GameSnapshot>,
    obj_names: Option<&api::obj_names::ObjNames>,
    world: Option<&NavWorld>,
    npc_boxes: Option<&[script::isolate_fb::NpcBoxInput]>,
    hold: bool,
    ours: bool,
    teleports_enabled: bool,
    withdraw_x_result_seq: u64,
    withdraw_x_result: bool,
    withdraw_load_result_seq: u64,
    withdraw_load_result: bool,
    bank_op_result_seq: u64,
    bank_op_result: bool,
    canlight: Option<&[u64]>,
    walk_outcome: PostedWalkOutcome,
    f: impl FnOnce(
        &script::isolate_fb::SnapshotInput<'_>,
        script::isolate_fb::NativeFactsInput<'_>,
    ) -> R,
) -> R {
    use script::isolate_fb::{
        BankApproachInput, BankStandInput, ChatLineInput, ChatOptionInput, CombatStyleInput,
        ItemRowInput, MakeButtonInput, MakeProductInput, NativeFactsInput, NearestBoothInput,
        QuestStatusInput, ReachViewInput, SceneEntityInput, SideTabIfaceInput, SnapshotInput,
        StatInput, TileInput, VarpInput, WidgetTextInput,
    };

    let flood = snapshot.and_then(|s| {
        let (x, z, level) = here?;
        if !s.scene().available {
            return None;
        }
        api::query::SceneQuery::new(s.scene(), Some(WorldTile { x, z, level })).flood_reach()
    });
    let canlight_plane = canlight.and_then(|bits| {
        world.map(|w| api::query::CanlightPlane {
            bits,
            origin_x: w.collision.origin.x,
            origin_z: w.collision.origin.z,
            width: w.collision.width as i32,
            height: w.collision.height as i32,
        })
    });
    let reach_pack = snapshot
        .map(|s| api::query::pack_reach_query_plane(s.scene(), flood.as_ref(), canlight_plane))
        .unwrap_or_else(api::query::ReachQueryView::unavailable);
    let reach = ReachViewInput {
        available: reach_pack.available,
        base_x: reach_pack.base_x,
        base_z: reach_pack.base_z,
        level: reach_pack.level,
        width: reach_pack.width,
        height: reach_pack.height,
        walkable: &reach_pack.walkable,
        reachable: &reach_pack.reachable,
        reachable_adj: &reach_pack.reachable_adj,
        exact_rank: &reach_pack.exact_rank,
        adjacent_rank: &reach_pack.adjacent_rank,
        step: &reach_pack.step,
        canlight: &reach_pack.canlight,
    };
    let here = here.map(|(x, z, level)| TileInput { x, z, level });
    let entity_reach = |x: i32, z: i32, level: i32| -> (bool, bool) {
        flood
            .as_ref()
            .map(|f| f.at(&WorldTile { x, z, level }))
            .unwrap_or((false, false))
    };
    let scene_entity_target = |target: Option<&api::snapshot::ActorTargetView>| -> (i32, i32) {
        match target {
            None => (0, -1),
            Some(t) => {
                let kind = match t.kind {
                    api::snapshot::ActorKind::Npc => 1,
                    api::snapshot::ActorKind::Player => 2,
                };
                (kind, t.index as i32)
            }
        }
    };
    let inv_ops_store: Vec<Vec<String>>;
    let inv: Vec<ItemRowInput<'_>> = if let Some(s) = snapshot {
        if s.inventory().is_empty() {
            inv_ops_store = Vec::new();
            inv.map(|rows| {
                rows.iter()
                    .map(|(id, count)| ItemRowInput {
                        name: obj_names.and_then(|names| names.name(*id)),
                        count: *count,
                        id: *id,
                        ops: &[],
                        noted: false,
                        cert: posted_cert_id(obj_names, *id),
                        component_id: -1,
                        slot: -1,
                    })
                    .collect()
            })
            .unwrap_or_default()
        } else {
            inv_ops_store = s
                .inventory()
                .iter()
                .map(|it| {
                    it.actions
                        .iter()
                        .filter_map(|a| a.as_deref().map(str::to_string))
                        .collect()
                })
                .collect();
            s.inventory()
                .iter()
                .enumerate()
                .map(|(i, it)| ItemRowInput {
                    name: obj_names
                        .and_then(|names| names.name(it.def.id))
                        .or(it.def.name.as_deref()),
                    count: it.count,
                    id: it.def.id,
                    ops: &inv_ops_store[i],
                    noted: it.def.noted,
                    cert: posted_cert(obj_names, &it.def),
                    component_id: -1,
                    slot: it.slot,
                })
                .collect()
        }
    } else {
        inv_ops_store = Vec::new();
        inv.map(|rows| {
            rows.iter()
                .map(|(id, count)| ItemRowInput {
                    name: obj_names.and_then(|names| names.name(*id)),
                    count: *count,
                    id: *id,
                    ops: &[],
                    noted: false,
                    cert: posted_cert_id(obj_names, *id),
                    component_id: -1,
                    slot: -1,
                })
                .collect()
        })
        .unwrap_or_default()
    };
    let stats = snapshot.map(|s| {
        s.stats()
            .iter()
            .map(|st| StatInput {
                index: st.index,
                name: &st.name,
                xp: st.xp,
                base: st.base,
                effective: st.effective,
            })
            .collect::<Vec<_>>()
    });
    let bank_ops_store: Vec<Vec<String>>;
    let bank: Vec<ItemRowInput<'_>> = if let Some(s) = snapshot {
        bank_ops_store = s
            .bank()
            .iter()
            .map(|it| {
                it.actions
                    .iter()
                    .filter_map(|a| a.as_deref().map(str::to_string))
                    .collect()
            })
            .collect();
        s.bank()
            .iter()
            .enumerate()
            .map(|(i, it)| ItemRowInput {
                name: obj_names
                    .and_then(|names| names.name(it.def.id))
                    .or(it.def.name.as_deref()),
                count: it.count,
                id: it.def.id,
                ops: &bank_ops_store[i],
                noted: it.def.noted,
                cert: posted_cert(obj_names, &it.def),
                component_id: it.component_id,
                slot: it.slot,
            })
            .collect()
    } else {
        bank_ops_store = Vec::new();
        Vec::new()
    };
    let bank_side_ops_store: Vec<Vec<String>>;
    let bank_side: Vec<ItemRowInput<'_>> = if let Some(s) = snapshot {
        bank_side_ops_store = s
            .bank_side()
            .iter()
            .map(|it| {
                it.actions
                    .iter()
                    .filter_map(|a| a.as_deref().map(str::to_string))
                    .collect()
            })
            .collect();
        s.bank_side()
            .iter()
            .enumerate()
            .map(|(i, it)| ItemRowInput {
                name: obj_names
                    .and_then(|names| names.name(it.def.id))
                    .or(it.def.name.as_deref()),
                count: it.count,
                id: it.def.id,
                ops: &bank_side_ops_store[i],
                noted: it.def.noted,
                cert: posted_cert(obj_names, &it.def),
                // Real deposit component (e.g. 2006 / fixture 701). Input.invButton
                // revalidates this id; posting -1 rejects every bank-side deposit.
                component_id: it.component_id,
                slot: it.slot,
            })
            .collect()
    } else {
        bank_side_ops_store = Vec::new();
        Vec::new()
    };
    let npc_action_store: Vec<Vec<String>>;
    let npcs: Vec<SceneEntityInput<'_>> = if let Some(s) = snapshot {
        npc_action_store = s
            .npcs()
            .iter()
            .map(|npc| {
                npc.actions
                    .iter()
                    .filter_map(|a| a.as_deref().map(str::to_string))
                    .collect::<Vec<String>>()
            })
            .collect();
        s.npcs()
            .iter()
            .enumerate()
            .map(|(i, npc)| {
                let (reachable, reachable_adj) =
                    entity_reach(npc.tile.x, npc.tile.z, npc.tile.level);
                let (target_kind, target_index) = scene_entity_target(npc.target.as_ref());
                SceneEntityInput {
                    index: npc.index as i32,
                    id: npc.r#type.map(|t| t as i32).unwrap_or(-1),
                    name: npc.name.as_deref(),
                    x: npc.tile.x,
                    z: npc.tile.z,
                    level: npc.tile.level,
                    distance: npc.distance,
                    health: npc.health,
                    max_health: npc.total_health,
                    in_combat: npc.in_combat,
                    animating: npc.moving || npc.animation != -1,
                    actions: &npc_action_store[i],
                    reachable,
                    reachable_adj,
                    combat_level: npc.level,
                    target_kind,
                    target_index,
                }
            })
            .collect()
    } else {
        Vec::new()
    };
    let loc_action_store: Vec<Vec<String>>;
    let locs: Vec<SceneEntityInput<'_>> = if let Some(s) = snapshot {
        loc_action_store = s
            .locs()
            .iter()
            .map(|loc| {
                loc.actions
                    .iter()
                    .filter_map(|a| a.as_deref().map(str::to_string))
                    .collect::<Vec<String>>()
            })
            .collect();
        s.locs()
            .iter()
            .enumerate()
            .map(|(i, loc)| {
                let (reachable, reachable_adj) =
                    entity_reach(loc.tile.x, loc.tile.z, loc.tile.level);
                SceneEntityInput {
                    index: loc.id,
                    id: loc.id,
                    name: loc.name.as_deref(),
                    x: loc.tile.x,
                    z: loc.tile.z,
                    level: loc.tile.level,
                    distance: loc.distance,
                    health: -1,
                    max_health: -1,
                    in_combat: false,
                    animating: loc.animation != -1,
                    actions: &loc_action_store[i],
                    reachable,
                    reachable_adj,
                    combat_level: 0,
                    target_kind: 0,
                    target_index: -1,
                }
            })
            .collect()
    } else {
        Vec::new()
    };
    let player_action_store: Vec<Vec<String>>;
    let players: Vec<SceneEntityInput<'_>> = if let Some(s) = snapshot {
        player_action_store = s
            .players()
            .iter()
            .map(|player| {
                player
                    .actor
                    .actions
                    .iter()
                    // FlatBuffers cannot carry null entries in a string
                    // vector. Keep each native op's position with the
                    // existing empty-string sentinel; the shim filters it
                    // from the public actions while opIndex still sees the
                    // original one-based slot numbers.
                    .map(|a| a.as_deref().unwrap_or_default().to_string())
                    .collect::<Vec<String>>()
            })
            .collect();
        s.players()
            .iter()
            .enumerate()
            .map(|(i, player)| {
                let (reachable, reachable_adj) = entity_reach(
                    player.actor.tile.x,
                    player.actor.tile.z,
                    player.actor.tile.level,
                );
                let (target_kind, target_index) = scene_entity_target(player.actor.target.as_ref());
                SceneEntityInput {
                    index: player.index as i32,
                    id: player.index as i32,
                    name: player.actor.name.as_deref(),
                    x: player.actor.tile.x,
                    z: player.actor.tile.z,
                    level: player.actor.tile.level,
                    distance: player.actor.distance,
                    health: player.actor.health,
                    max_health: player.actor.total_health,
                    in_combat: player.actor.in_combat,
                    animating: player.actor.moving || player.actor.animation != -1,
                    actions: &player_action_store[i],
                    reachable,
                    reachable_adj,
                    combat_level: player.combat_level,
                    target_kind,
                    target_index,
                }
            })
            .collect()
    } else {
        Vec::new()
    };
    let ground_action_store: Vec<Vec<String>>;
    let ground: Vec<SceneEntityInput<'_>> = if let Some(s) = snapshot {
        ground_action_store = s
            .ground_items()
            .iter()
            .map(|item| {
                item.actions
                    .iter()
                    .filter_map(|a| a.as_deref().map(str::to_string))
                    .collect::<Vec<String>>()
            })
            .collect();
        s.ground_items()
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let (reachable, reachable_adj) =
                    entity_reach(item.tile.x, item.tile.z, item.tile.level);
                SceneEntityInput {
                    index: item.def.id,
                    id: item.def.id,
                    name: obj_names.and_then(|names| names.name(item.def.id)),
                    x: item.tile.x,
                    z: item.tile.z,
                    level: item.tile.level,
                    distance: item.distance,
                    health: -1,
                    max_health: -1,
                    in_combat: false,
                    animating: false,
                    actions: &ground_action_store[i],
                    reachable,
                    reachable_adj,
                    combat_level: 0,
                    target_kind: 0,
                    target_index: -1,
                }
            })
            .collect()
    } else {
        Vec::new()
    };
    let equip_ops_store: Vec<Vec<String>>;
    let equipment: Vec<ItemRowInput<'_>> = if let Some(s) = snapshot {
        equip_ops_store = s
            .equipment()
            .iter()
            .map(|it| {
                it.actions
                    .iter()
                    .filter_map(|a| a.as_deref().map(str::to_string))
                    .collect()
            })
            .collect();
        s.equipment()
            .iter()
            .enumerate()
            .map(|(i, it)| ItemRowInput {
                name: obj_names
                    .and_then(|names| names.name(it.def.id))
                    .or(it.def.name.as_deref()),
                count: it.count,
                id: it.def.id,
                ops: &equip_ops_store[i],
                noted: it.def.noted,
                cert: posted_cert(obj_names, &it.def),
                component_id: -1,
                slot: it.slot,
            })
            .collect()
    } else {
        equip_ops_store = Vec::new();
        Vec::new()
    };
    let trade_mine_ops_store: Vec<Vec<String>>;
    let trade_mine: Vec<ItemRowInput<'_>> = if let Some(s) = snapshot {
        trade_mine_ops_store = s
            .trade()
            .my_offer
            .iter()
            .map(|it| {
                it.actions
                    .iter()
                    .filter_map(|a| a.as_deref().map(str::to_string))
                    .collect()
            })
            .collect();
        s.trade()
            .my_offer
            .iter()
            .enumerate()
            .map(|(i, it)| ItemRowInput {
                name: obj_names
                    .and_then(|names| names.name(it.def.id))
                    .or(it.def.name.as_deref()),
                count: it.count,
                id: it.def.id,
                ops: &trade_mine_ops_store[i],
                noted: it.def.noted,
                cert: posted_cert(obj_names, &it.def),
                component_id: it.component_id,
                slot: it.slot,
            })
            .collect()
    } else {
        trade_mine_ops_store = Vec::new();
        Vec::new()
    };
    let trade_theirs_ops_store: Vec<Vec<String>>;
    let trade_theirs: Vec<ItemRowInput<'_>> = if let Some(s) = snapshot {
        trade_theirs_ops_store = s
            .trade()
            .their_offer
            .iter()
            .map(|it| {
                it.actions
                    .iter()
                    .filter_map(|a| a.as_deref().map(str::to_string))
                    .collect()
            })
            .collect();
        s.trade()
            .their_offer
            .iter()
            .enumerate()
            .map(|(i, it)| ItemRowInput {
                name: obj_names
                    .and_then(|names| names.name(it.def.id))
                    .or(it.def.name.as_deref()),
                count: it.count,
                id: it.def.id,
                ops: &trade_theirs_ops_store[i],
                noted: it.def.noted,
                cert: posted_cert(obj_names, &it.def),
                component_id: it.component_id,
                slot: it.slot,
            })
            .collect()
    } else {
        trade_theirs_ops_store = Vec::new();
        Vec::new()
    };
    let trade_side_ops_store: Vec<Vec<String>>;
    let trade_side: Vec<ItemRowInput<'_>> = if let Some(s) = snapshot {
        trade_side_ops_store = s
            .trade()
            .side_pack
            .iter()
            .map(|it| {
                it.actions
                    .iter()
                    .filter_map(|a| a.as_deref().map(str::to_string))
                    .collect()
            })
            .collect();
        s.trade()
            .side_pack
            .iter()
            .enumerate()
            .map(|(i, it)| ItemRowInput {
                name: obj_names
                    .and_then(|names| names.name(it.def.id))
                    .or(it.def.name.as_deref()),
                count: it.count,
                id: it.def.id,
                ops: &trade_side_ops_store[i],
                noted: it.def.noted,
                cert: posted_cert(obj_names, &it.def),
                component_id: it.component_id,
                slot: it.slot,
            })
            .collect()
    } else {
        trade_side_ops_store = Vec::new();
        Vec::new()
    };
    let shop_player_ops_store: Vec<Vec<String>>;
    let shop_player_rows: Vec<ItemRowInput<'_>> = if let Some(s) = snapshot {
        shop_player_ops_store = s
            .shop()
            .player
            .iter()
            .map(|it| {
                it.actions
                    .iter()
                    .filter_map(|a| a.as_deref().map(str::to_string))
                    .collect()
            })
            .collect();
        s.shop()
            .player
            .iter()
            .enumerate()
            .map(|(i, it)| ItemRowInput {
                name: obj_names
                    .and_then(|names| names.name(it.def.id))
                    .or(it.def.name.as_deref()),
                count: it.count,
                id: it.def.id,
                ops: &shop_player_ops_store[i],
                noted: it.def.noted,
                cert: posted_cert(obj_names, &it.def),
                component_id: it.component_id,
                slot: it.slot,
            })
            .collect()
    } else {
        shop_player_ops_store = Vec::new();
        Vec::new()
    };
    // `None` = the shop side interface's player pack was not decoded: the
    // Sell path fails closed rather than acting on an empty stand-in.
    let shop_player = snapshot
        .filter(|s| s.shop().player_available)
        .map(|_| shop_player_rows.as_slice());
    let main_make_ops_store: Vec<Vec<String>>;
    let main_make_rows: Vec<ItemRowInput<'_>> = if let Some(s) = snapshot {
        main_make_ops_store = s
            .main_make()
            .iter()
            .map(|it| {
                it.actions
                    .iter()
                    .filter_map(|a| a.as_deref().map(str::to_string))
                    .collect()
            })
            .collect();
        s.main_make()
            .iter()
            .enumerate()
            .map(|(i, it)| ItemRowInput {
                name: obj_names
                    .and_then(|names| names.name(it.def.id))
                    .or(it.def.name.as_deref()),
                count: it.count,
                id: it.def.id,
                ops: &main_make_ops_store[i],
                noted: it.def.noted,
                cert: posted_cert(obj_names, &it.def),
                component_id: it.component_id,
                slot: it.slot,
            })
            .collect()
    } else {
        main_make_ops_store = Vec::new();
        Vec::new()
    };
    // Always decoded when a snapshot exists: empty means no Make TYPE_INV
    // on the open main modal, not an unpublished field.
    let main_make = snapshot.map(|_| main_make_rows.as_slice());
    let shop_stock_ops_store: Vec<Vec<String>>;
    let shop_stock: Vec<ItemRowInput<'_>> = if let Some(s) = snapshot {
        shop_stock_ops_store = s
            .shop()
            .stock
            .iter()
            .map(|it| {
                it.actions
                    .iter()
                    .filter_map(|a| a.as_deref().map(str::to_string))
                    .collect()
            })
            .collect();
        s.shop()
            .stock
            .iter()
            .enumerate()
            .map(|(i, it)| ItemRowInput {
                name: obj_names
                    .and_then(|names| names.name(it.def.id))
                    .or(it.def.name.as_deref()),
                count: it.count,
                id: it.def.id,
                ops: &shop_stock_ops_store[i],
                noted: it.def.noted,
                cert: posted_cert(obj_names, &it.def),
                component_id: it.component_id,
                slot: it.slot,
            })
            .collect()
    } else {
        shop_stock_ops_store = Vec::new();
        Vec::new()
    };
    let chat_options = snapshot.map(|s| {
        s.chat_options()
            .iter()
            .map(|o| ChatOptionInput { text: &o.text })
            .collect::<Vec<_>>()
    });
    let varps = snapshot.map(|s| {
        let magic = api::snapshot::ReadContext::new(s).varp(108);
        let energy = api::snapshot::ReadContext::new(s).varp(300);
        let armed = api::snapshot::ReadContext::new(s).varp(301);
        // attackstyle_magic is packed as varp 108 on both selected caches.
        // sa_energy/sa_attack are packed as 300/301. Always post them,
        // including 0, so Special.energy/armed and Autocast.armed/selected
        // observe real state instead of a missing-row default.
        let mut rows: Vec<VarpInput> = vec![
            VarpInput {
                index: 108,
                value: magic,
            },
            VarpInput {
                index: 300,
                value: energy,
            },
            VarpInput {
                index: 301,
                value: armed,
            },
        ];
        rows.extend(
            s.varps()
                .iter()
                .filter(|v| v.value != 0 && v.index != 108 && v.index != 300 && v.index != 301)
                .take(29)
                .map(|v| VarpInput {
                    index: v.index,
                    value: v.value,
                }),
        );
        rows
    });
    let combat_style_store = snapshot.map(|s| {
        let root = s
            .side_tabs()
            .iter()
            .find(|t| t.index == 0)
            .map(|t| t.root_component_id)
            .unwrap_or(-1);
        if root == -1 {
            Vec::new()
        } else {
            api::query::widget_search::combat_style_labels(s, root, 43)
        }
    });
    let combat_styles: Vec<CombatStyleInput<'_>> = combat_style_store
        .as_ref()
        .map(|labels| {
            labels
                .iter()
                .map(|l| CombatStyleInput {
                    mode: l.mode,
                    label: &l.label,
                    component_id: l.component_id,
                })
                .collect()
        })
        .unwrap_or_default();
    let local = snapshot.and_then(|s| s.local_player());
    let my_name = local.and_then(|lp| lp.player.actor.name.as_deref());
    let in_combat = local.is_some_and(|lp| lp.player.actor.in_combat);
    let attacked_by_player =
        local.is_some_and(|lp| api::snapshot::attacked_by_player(lp.player.actor.face_entity));
    let animating =
        local.is_some_and(|lp| lp.player.actor.moving || lp.player.actor.animation != -1);
    let modals = snapshot.map(|s| s.modals());
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
    let nearest_booth = snapshot.and_then(|s| {
        s.nearest_use_quickly_booth().map(|loc| NearestBoothInput {
            x: loc.tile.x,
            z: loc.tile.z,
            level: loc.tile.level,
            id: loc.id,
            name: loc.name.as_deref().unwrap_or("Bank booth"),
            op: "Use-quickly",
        })
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
    let mut make_button_store: Vec<Vec<MakeButtonInput>> = Vec::new();
    let make_products: Vec<MakeProductInput<'_>> = snapshot
        .map(|s| {
            make_button_store = s
                .make_products()
                .iter()
                .map(|p| {
                    p.buttons
                        .iter()
                        .map(|b| MakeButtonInput {
                            qty: b.quantity,
                            com_id: b.component_id,
                        })
                        .collect()
                })
                .collect();
            s.make_products()
                .iter()
                .enumerate()
                .map(|(i, p)| MakeProductInput {
                    object_id: p.object_id,
                    name: p.name.as_str(),
                    buttons: &make_button_store[i],
                })
                .collect()
        })
        .unwrap_or_default();
    let side_tab_ifaces: Vec<SideTabIfaceInput> = snapshot
        .map(|s| {
            s.side_tabs()
                .iter()
                .map(|t| SideTabIfaceInput {
                    index: t.index,
                    id: t.root_component_id,
                })
                .collect()
        })
        .unwrap_or_default();
    let spell_store = snapshot.map(|s| {
        let root = s
            .side_tabs()
            .iter()
            .find(|t| t.index == 6)
            .map(|t| t.root_component_id)
            .unwrap_or(-1);
        if root == -1 {
            Vec::new()
        } else {
            s.widgets()
                .iter()
                .chain(s.side_tabs().iter().flat_map(|t| t.widgets.iter()))
                .filter(|w| {
                    w.root_component_id == root
                        && w.button_type == 2
                        && w.target_base.as_deref().is_some_and(|t| !t.is_empty())
                })
                .map(|w| (w.target_base.clone().unwrap_or_default(), w.component_id))
                .collect::<Vec<_>>()
        }
    });
    let spell_buttons: Vec<CombatStyleInput<'_>> = spell_store
        .as_ref()
        .map(|rows| {
            rows.iter()
                .map(|(label, component_id)| CombatStyleInput {
                    mode: 0,
                    label,
                    component_id: *component_id,
                })
                .collect()
        })
        .unwrap_or_default();
    let chat_lines: Vec<ChatLineInput<'_>> = snapshot
        .map(|s| {
            s.chat_lines()
                .iter()
                .map(|l| ChatLineInput {
                    seq: l.sequence,
                    text: l.text.as_str(),
                    type_: l.type_,
                    username: l.username.as_deref(),
                })
                .collect()
        })
        .unwrap_or_default();
    let widget_store: Vec<(i32, String)> = snapshot
        .map(|s| {
            s.widgets()
                .iter()
                .filter_map(|w| w.text.as_ref().map(|text| (w.component_id, text.clone())))
                .collect()
        })
        .unwrap_or_default();
    let widgets: Vec<WidgetTextInput<'_>> = widget_store
        .iter()
        .map(|(component_id, text)| WidgetTextInput {
            component_id: *component_id,
            text,
        })
        .collect();
    let quest_statuses: Vec<QuestStatusInput<'_>> = snapshot
        .map(|s| {
            s.quest_statuses()
                .iter()
                .map(|quest| QuestStatusInput {
                    name: quest.name.as_str(),
                    status: quest.status().as_str(),
                })
                .collect()
        })
        .unwrap_or_default();
    let quest_statuses = snapshot
        .filter(|s| s.quest_statuses_available())
        .map(|_| quest_statuses.as_slice());
    let input = SnapshotInput {
        tick,
        here,
        ingame,
        inv: &inv,
        // The inv tab's slot count (28 when bound, 0 while the side icons
        // stay tutorial-locked): the `reader.inventorySize()` gate a
        // script's onStart parks on.
        inv_size: snapshot.map_or(0, |s| s.inventory_size()),
        stats: stats.as_deref().unwrap_or(&[]),
        booths: booths.as_deref().unwrap_or(&[]),
        nearest_booth,
        banks: banks.as_deref().unwrap_or(&[]),
        bank: &bank,
        bank_side: &bank_side,
        bank_open: snapshot.is_some_and(|s| s.bank_component_id() != -1),
        bank_loaded: snapshot.is_some_and(GameSnapshot::bank_loaded),
        bank_generation: snapshot.map_or(0, GameSnapshot::bank_session_generation),
        count_dialog_open: snapshot.is_some_and(GameSnapshot::count_dialog_open),
        withdraw_x_result_seq,
        withdraw_x_result,
        withdraw_load_result_seq,
        withdraw_load_result,
        bank_op_result_seq,
        bank_op_result,
        hold,
        ours,
        npcs: &npcs,
        locs: &locs,
        players: &players,
        ground: &ground,
        equipment: &equipment,
        chat_open: modals.is_some_and(|m| m.chat != -1),
        chat_continue: snapshot.is_some_and(|s| s.chat_continue_component_id() != -1),
        chat_text: snapshot.and_then(|s| s.chat()),
        chat_options: chat_options.as_deref().unwrap_or(&[]),
        side_tab: snapshot.map(|s| s.active_side_tab()).unwrap_or(-1),
        varps: varps.as_deref().unwrap_or(&[]),
        combat_styles: &combat_styles,
        run_energy: snapshot.map(|s| s.runenergy()).unwrap_or(0),
        run_enabled: snapshot.is_some_and(|s| api::snapshot::ReadContext::new(s).varp(173) != 0),
        retaliate_enabled: snapshot
            .is_some_and(|s| api::snapshot::ReadContext::new(s).varp(172) == 0),
        my_name,
        in_combat,
        animating,
        main_modal_id: modals.map(|m| m.main).unwrap_or(-1),
        chat_modal_id: modals.map(|m| m.chat).unwrap_or(-1),
        make_products: &make_products,
        side_tab_ifaces: &side_tab_ifaces,
        spell_buttons: &spell_buttons,
        chat_lines: &chat_lines,
        bank_note_on: snapshot
            .and_then(|s| s.bank_note_controls())
            .map(|c| c.on_component_id)
            .unwrap_or(-1),
        bank_note_off: snapshot
            .and_then(|s| s.bank_note_controls())
            .map(|c| c.off_component_id)
            .unwrap_or(-1),
        scene_state: snapshot.map(|s| s.scene_state()).unwrap_or(0),
        weight: snapshot
            .and_then(|s| s.local_player())
            .map(|lp| lp.weight)
            .unwrap_or(0),
        combat_level: snapshot
            .and_then(|s| s.local_player())
            .map(|lp| lp.player.combat_level)
            .unwrap_or(0),
        camera_yaw: snapshot.map(|s| s.camera().orbit_yaw).unwrap_or(0),
        camera_pitch: snapshot.map(|s| s.camera().orbit_pitch).unwrap_or(0),
        teleports_enabled,
        self_slot: snapshot.map(|s| s.self_slot()).unwrap_or(-1),
        trade_offer_open: snapshot.is_some_and(|s| s.trade().offer_open),
        trade_confirm_open: snapshot.is_some_and(|s| s.trade().confirm_open),
        trade_partner: snapshot.and_then(|s| s.trade().partner.as_deref()),
        trade_mine: &trade_mine,
        trade_theirs: &trade_theirs,
        trade_side: &trade_side,
        trade_accept_id: snapshot
            .map(|s| s.trade().accept_component_id)
            .unwrap_or(-1),
        trade_decline_id: snapshot
            .map(|s| s.trade().decline_component_id)
            .unwrap_or(-1),
        shop_open: snapshot.is_some_and(|s| s.shop().open),
        shop_stock: &shop_stock,
        reach,
        attacked_by_player,
        widgets: &widgets,
    };
    let bank_approach_store: Vec<BankApproachInput> = match (snapshot, here, flood.as_ref()) {
        (Some(s), Some(tile), Some(flood)) => s
            .locs()
            .iter()
            .filter_map(|loc| {
                let name = loc.name.as_deref()?;
                if !name
                    .as_bytes()
                    .windows(4)
                    .any(|word| word.eq_ignore_ascii_case(b"bank"))
                {
                    return None;
                }
                let approach = api::query::loc_approach::booth_approach(
                    loc,
                    s.scene(),
                    WorldTile {
                        x: tile.x,
                        z: tile.z,
                        level: tile.level,
                    },
                    flood,
                )?;
                Some(BankApproachInput {
                    loc_id: loc.id,
                    x: loc.tile.x,
                    z: loc.tile.z,
                    level: loc.tile.level,
                    can_operate: approach.can_operate,
                    dest_ok: approach.dest.is_some(),
                    dest_x: approach.dest.map(|t| t.x).unwrap_or(0),
                    dest_z: approach.dest.map(|t| t.z).unwrap_or(0),
                    dest_level: approach.dest.map(|t| t.level).unwrap_or(0),
                })
            })
            .collect(),
        _ => Vec::new(),
    };
    let native = NativeFactsInput {
        self_chat: snapshot.and_then(GameSnapshot::local_overhead_text),
        hint_tile: snapshot
            .and_then(GameSnapshot::hint_tile)
            .map(|tile| (tile.x, tile.z)),
        retaliate_controls: snapshot
            .and_then(GameSnapshot::retaliate_controls)
            .map(|controls| (controls.on_component_id, controls.off_component_id)),
        quest_statuses,
        npc_boxes,
        shop_player,
        main_make,
        bank_approaches: Some(&bank_approach_store),
        walk_outcome_seq: walk_outcome.seq,
        walk_outcome_generation: walk_outcome.generation,
        walk_outcome_request_id: walk_outcome.request_id,
        walk_outcome_failed: walk_outcome.failed,
        walk_outcome_x: walk_outcome.x,
        walk_outcome_z: walk_outcome.z,
        walk_outcome_level: walk_outcome.level,
        walk_outcome_radius: walk_outcome.radius,
        walk_outcome_allow_teleports: walk_outcome.allow_teleports,
    };
    f(&input, native)
}
/// Whether this observe pass needs a [`WorldState`] from the slot snapshot.
/// Built only for a Running script (walk arm) or an armed nav bot (route /
/// BankBudget session — interact dispatch may walk with gating facts).
fn nav_world_state_for_observe(
    here: Option<(i32, i32, i32)>,
    snapshot: &GameSnapshot,
    script_running: bool,
    nav_armed: bool,
    map_members: bool,
) -> Option<WorldState> {
    if here.is_some() && (script_running || nav_armed) {
        Some(WorldState::from_snapshot(snapshot).with_map_members(map_members))
    } else {
        None
    }
}

/// the per-observe inventory view (the observe re-checks the gate inside).
fn script_running(scripts: &ScriptWall, name: &str) -> bool {
    script_slot(scripts, name)
        .is_some_and(|s| s.lock().unwrap().state() == script::RunState::Running)
}

/// `name`'s slot script's latest paint frame, `None` for a slot with no
/// script or a script that has not painted. Copied onto the status row
/// each observe so the TUI can show paint-as-chat without a probe
/// round-trip (the isolate forwards the frame after each tick).
fn script_paint_of(scripts: &ScriptWall, name: &str) -> Option<script::shim::ScriptPaint> {
    script_slot(scripts, name).and_then(|s| s.lock().unwrap().paint())
}

/// Copy `paint` onto `status.script_paint` only when the frame changed.
fn publish_script_paint(status: &mut SlotStatus, paint: Option<&script::shim::ScriptPaint>) {
    match (&status.script_paint, paint) {
        (Some(cur), Some(next)) if cur == next => {}
        (None, None) => {}
        _ => status.script_paint = paint.cloned(),
    }
}

/// Per-uid nav state: the whole-world traveller plus the route it is
/// following. `ctx.walk` stores the route (found off-pump over the shared
/// [`NavWorld`]); the slot pump polls [`Traveller::follow`] with a clone of
/// it one step per player-info tick. `route` being set is the "armed"
/// gate the walk hook and the busy flag read. A pending BankBudget
/// session freezes follow until its steps finish.
#[derive(Default)]
struct NavBot {
    route_generation: u64,
    route_worker: Option<Arc<()>>,
    pending_route: Option<ScriptRouteRequest>,
    /// Dest, radius, allow_teleports, allow_wilderness, allow_bank_fetch.
    requested_route: Option<(WorldTile, i32, bool, bool, bool)>,
    traveller: Traveller,
    route: Option<Route>,
    bank_fetch: Option<PendingBankFetch>,
    /// Isolate-allocated walk request id for the armed / in-flight find.
    /// Distinct from `route_generation`, which remains the worker / retained-route token.
    walk_request_id: u64,
    /// Unpublished current-wait refusal id, distinct from `walk_request_id`.
    /// Older armed-route NoPath / mid-follow terminals must not overwrite this
    /// published outcome until a snapshot copies it.
    walk_live_refusal_id: u64,
    /// Last packed-walk `allow_teleports` opt-in (`Traversal.teleportsEnabled`).
    allow_teleports: bool,
    /// Bounded published walk outcome. Seq `0` means never published.
    walk_outcome_seq: u64,
    walk_outcome_generation: u64,
    walk_outcome_request_id: u64,
    walk_outcome_failed: bool,
    walk_outcome_x: i32,
    walk_outcome_z: i32,
    walk_outcome_level: i32,
    walk_outcome_radius: i32,
    walk_outcome_allow_teleports: bool,
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
        self.queue_route(x, z, level, opts, 0, false, 0)
    }
    /// Explicit WalkNear, including radius 0. Unlike [`Self::route`], an
    /// armed or in-flight route is replaced through the existing generation /
    /// pending-route coalescing path. A latched bank-fetch session still refuses.
    fn route_with_radius(
        &self,
        x: i32,
        z: i32,
        level: i32,
        opts: FindOptions,
        radius: i32,
    ) -> bool {
        self.queue_route(x, z, level, opts, radius, true, 0)
    }
    fn publish_refusal(&self, to: WorldTile, radius: i32, allow_teleports: bool, request_id: u64) {
        let mut navs = self.navs.lock().unwrap();
        let bot = navs.entry(self.name.clone()).or_default();
        // Do not bump route_generation or clear a retained route / bank-fetch.
        bot.note_failure(
            bot.route_generation,
            request_id,
            to,
            radius,
            allow_teleports,
        );
    }

    fn queue_route(
        &self,
        x: i32,
        z: i32,
        level: i32,
        mut opts: FindOptions,
        radius: i32,
        retarget: bool,
        request_id: u64,
    ) -> bool {
        let to = WorldTile { x, z, level };
        let Some((hx, hz, hl)) = self.here else {
            self.publish_refusal(to, radius, opts.allow_teleports, request_id);
            return false;
        };
        let Some(world) = self.world.as_ref() else {
            self.publish_refusal(to, radius, opts.allow_teleports, request_id);
            return false;
        };
        let from = WorldTile {
            x: hx,
            z: hz,
            level: hl,
        };
        let token = {
            let mut navs = self.navs.lock().unwrap();
            let bot = navs.entry(self.name.clone()).or_default();
            if bot.bank_fetch.is_some()
                || (!retarget && (bot.route.is_some() || bot.route_worker.is_some()))
            {
                bot.note_failure(
                    bot.route_generation,
                    request_id,
                    to,
                    radius,
                    opts.allow_teleports,
                );
                return false;
            }
            let key = (
                to,
                radius,
                opts.allow_teleports,
                opts.allow_wilderness,
                opts.allow_bank_fetch,
            );
            if bot.requested_route == Some(key)
                && (bot.route_worker.is_some()
                    || bot.route.is_some()
                    || bot.pending_route.is_some())
            {
                // Same-id retransmission and legacy request_id 0 keep the
                // in-flight find. A later distinct nonzero wait is refused
                // without restarting search or reassigning the armed id.
                if request_id != 0 && request_id != bot.walk_request_id {
                    bot.note_failure(
                        bot.route_generation,
                        request_id,
                        to,
                        radius,
                        opts.allow_teleports,
                    );
                    return false;
                }
                return true;
            }
            if let Some(ess) = bot.traveller.essence() {
                opts.essence = Some(ess);
            }
            bot.route_generation = bot.route_generation.wrapping_add(1);
            bot.walk_request_id = request_id;
            bot.requested_route = Some(key);
            bot.pending_route = Some(ScriptRouteRequest {
                generation: bot.route_generation,
                request_id,
                world: Arc::clone(world),
                from,
                to,
                radius,
                opts,
                state: self.state.clone(),
                bank: self.bank.clone(),
            });
            if bot.route_worker.is_some() {
                return true;
            }
            let token = Arc::new(());
            bot.route_worker = Some(Arc::clone(&token));
            token
        };
        let navs = Arc::clone(&self.navs);
        let name = self.name.clone();
        let worker_token = Arc::clone(&token);
        let spawned = thread::Builder::new()
            .name(format!("nav-find-{name}"))
            .spawn(move || loop {
                let request = {
                    let mut all = navs.lock().unwrap();
                    let Some(bot) = all.get_mut(&name) else {
                        return;
                    };
                    if !bot
                        .route_worker
                        .as_ref()
                        .is_some_and(|t| Arc::ptr_eq(t, &worker_token))
                    {
                        return;
                    }
                    let Some(request) = bot.pending_route.take() else {
                        bot.route_worker = None;
                        return;
                    };
                    request
                };
                let outcome = request.calculate();
                let mut all = navs.lock().unwrap();
                let Some(bot) = all.get_mut(&name) else {
                    return;
                };
                if !bot
                    .route_worker
                    .as_ref()
                    .is_some_and(|t| Arc::ptr_eq(t, &worker_token))
                {
                    return;
                }
                bot.publish_route(
                    request.generation,
                    request.request_id,
                    request.opts.allow_teleports,
                    outcome,
                );
            })
            .is_ok();
        if !spawned {
            if let Some(bot) = self.navs.lock().unwrap().get_mut(&self.name) {
                if bot
                    .route_worker
                    .as_ref()
                    .is_some_and(|t| Arc::ptr_eq(t, &token))
                {
                    if let Some((to, radius, allow_teleports, ..)) = bot.requested_route {
                        bot.note_failure(
                            bot.route_generation,
                            bot.walk_request_id,
                            to,
                            radius,
                            allow_teleports,
                        );
                    }
                    bot.route_worker = None;
                    bot.pending_route = None;
                    bot.requested_route = None;
                }
            }
        }
        spawned
    }
}

/// Mid-follow Stall / Refused / Blocked / GaveUp publish a failed outcome
/// with the armed walk's isolate request id so the matching wait returns
/// false. Arrival still settles from `here`. Only genuinely pending follow
/// work (None) keeps the caller timeout.
fn apply_nav_follow_outcome(
    bot: &mut NavBot,
    outcome: Option<nav::traveller::TravelOutcome>,
    walking_stand: bool,
) {
    match outcome {
        Some(nav::traveller::TravelOutcome::Arrived { .. }) => {
            bot.route = None;
        }
        Some(_) => {
            if let Some((to, radius, allow_teleports, ..)) = bot.requested_route {
                if bot.armed_outcome_may_publish(bot.walk_request_id) {
                    bot.note_failure(
                        bot.route_generation,
                        bot.walk_request_id,
                        to,
                        radius,
                        allow_teleports,
                    );
                }
            } else if let Some(route) = bot.route.as_ref() {
                if bot.armed_outcome_may_publish(bot.walk_request_id) {
                    bot.note_failure(
                        bot.route_generation,
                        bot.walk_request_id,
                        route.dest,
                        0,
                        bot.allow_teleports,
                    );
                }
            }
            bot.route = None;
            if walking_stand {
                bot.bank_fetch = None;
            }
        }
        None => {}
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
/// Mid-follow Stall / Refused / Blocked / GaveUp publish the armed walk's
/// isolate request id as a failed outcome. Arrival still settles from `here`.
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
    map_members: bool,
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
                step_bank_fetch_on_bot(driver, snapshot, bot, world, here, map_members);
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
        let follow_outcome = bot.traveller.follow(driver, snapshot, route, &mut options);
        apply_nav_follow_outcome(bot, follow_outcome, walking_stand);
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
    map_members: bool,
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
            } else if bot
                .route
                .as_ref()
                .is_some_and(|r| r.dest.x == x && r.dest.z == z && r.dest.level == level)
            {
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
                let state = WorldState::from_snapshot(snapshot).with_map_members(map_members);
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
            if snapshot.bank_loaded() {
                pending.steps.pop_front();
                false
            } else if snapshot.bank_component_id() == -1 {
                open_bank_at_here(driver, snapshot, here, world)
            } else {
                false
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
    if bot.bank_fetch.as_ref().is_some_and(|p| p.steps.is_empty()) {
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
            !bot.route
                .as_ref()
                .is_some_and(|r| r.dest.x == *x && r.dest.z == *z && r.dest.level == *level)
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

/// Borrow the current session's inventory only on running tick edges.
fn observe_script_inv(
    running: bool,
    tick_edge: bool,
    snapshot: &GameSnapshot,
) -> Option<&[(i32, i32)]> {
    (running && tick_edge).then(|| snapshot.inv())
}

/// Project only the client's bounded active-NPC list. A ready scene with no
/// projectable NPCs is an available empty update; a non-ready scene is
/// unavailable so isolates clear any prior geometry.
fn projected_npc_boxes(client: &Client) -> Option<Vec<script::isolate_fb::NpcBoxInput>> {
    if !client.ingame || client.scene_state != 2 {
        return None;
    }
    Some(
        client
            .npc_ids
            .iter()
            .take(usize::try_from(client.npc_count).unwrap_or(0))
            .filter_map(|&index| {
                let slot = usize::try_from(index).ok()?;
                client::render::npc_overlay_box(client, slot)
                    .map(|points| script::isolate_fb::NpcBoxInput { index, points })
            })
            .collect(),
    )
}

/// Evaluate native NPC projection only for a frame that can post an isolate
/// snapshot. Held running isolates still satisfy this gate because they post
/// the snapshot before dispatching their paint-only tick; idle, paused,
/// compiled-only, and non-tick frames do not consume one.
fn project_npc_boxes_for_isolate_snapshot<F>(
    scripts: &ScriptWall,
    name: &str,
    tick_edge: bool,
    project: F,
) -> Option<Vec<script::isolate_fb::NpcBoxInput>>
where
    F: FnOnce() -> Option<Vec<script::isolate_fb::NpcBoxInput>>,
{
    if !tick_edge {
        return None;
    }
    let consumes_snapshot = script_slot(scripts, name).is_some_and(|slot| {
        let slot = slot.lock().unwrap();
        slot.state() == script::RunState::Running && slot.load_active()
    });
    if consumes_snapshot {
        project()
    } else {
        None
    }
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
    if let Some(nav) = navs.lock().unwrap().get_mut(name) {
        nav.route_generation = nav.route_generation.wrapping_add(1);
        nav.route_worker = None;
        nav.pending_route = None;
        nav.requested_route = None;
        nav.traveller.clear();
        nav.route = None;
        nav.bank_fetch = None;
        nav.walk_request_id = 0;
        nav.clear_walk_outcome();
    }
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
                    bot_client_config(options, &profile), uid, slot_cache,
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
                        thread::sleep(Duration::from_millis(20));
                        continue;
                    }
                    let wait = wait_for_permit(&slot_queue, &slot_statuses, &username, uid, &arm);
                    if arm.stop.load(Ordering::Relaxed) {
                        slot_queue.lock().unwrap().leave(uid);
                        return;
                    }
                    // A withdrawal that lands on the granted poll spends the
                    // permit but must not start the handshake it cancelled.
                    if wait == PermitWait::Cancelled || !should_handshake(&arm, client.ingame) {
                        continue;
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
                            let script_lifecycle = script_slot(&slot_scripts, name)
                                .and_then(|slot| slot.lock().unwrap().lifecycle_receipt());
                            // `status` is last frame's client_frame publication.
                            // Snapshot observe runs before this frame copies it
                            // onto the slot row.
                            let guardian = if session_boundary {
                                catalog_core::BoundedGuardian::default()
                            } else {
                                bounded_guardian_fact(status)
                            };
                            obs_catalog_core.observe_snapshot_with_lifecycle(
                                name,
                                &nav_snapshot,
                                &slot_obj_names,
                                script_lifecycle,
                                guardian,
                                session_boundary,
                            );
                            obs_paired_core.observe_snapshot(name, &nav_snapshot, session_boundary);
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
                            script_observe_with_npc_boxes(
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

/// Outcome of [`wait_for_permit`]. `Granted` may handshake (the caller still
/// re-checks the intent); `Cancelled` means the request was withdrawn before
/// any handshake and neither its FIFO place nor its published `k of n`
/// survives.
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

/// Candidate destinations for an explicit radius request. Exact walks retain
/// their old routing behavior. Bound enumeration to the loaded scene size.
fn approach_tiles(world: &NavWorld, from: WorldTile, to: WorldTile, radius: i32) -> Vec<WorldTile> {
    let r = radius.clamp(0, 104);
    let mut tiles = Vec::new();
    for dx in -r..=r {
        for dz in -r..=r {
            let t = WorldTile {
                x: to.x + dx,
                z: to.z + dz,
                level: to.level,
            };
            if world.collision.standable(t) {
                tiles.push(t);
            }
        }
    }
    if world.collision.standable(to) {
        let connected = nav::router::local_step_component(&world.collision, to, r);
        tiles.retain(|tile| connected.contains(tile));
    }
    tiles.sort_by_key(|t| {
        (
            (t.x - from.x).abs().max((t.z - from.z).abs()),
            (t.x - to.x).abs().max((t.z - to.z).abs()),
            t.x,
            t.z,
        )
    });
    tiles
}

struct ScriptRouteRequest {
    generation: u64,
    request_id: u64,
    world: Arc<NavWorld>,
    from: WorldTile,
    to: WorldTile,
    radius: i32,
    opts: FindOptions,
    state: Option<WorldState>,
    bank: Vec<(i32, i32)>,
}
impl ScriptRouteRequest {
    fn calculate(&self) -> RouteOutcome {
        let empty = WorldState::empty();
        let state = self.state.as_ref().unwrap_or(&empty);
        if self.radius <= 0 {
            return route_or_bank_fetch(
                &self.world,
                self.from,
                self.to,
                self.opts,
                state,
                &self.bank,
            );
        }
        for target in approach_tiles(&self.world, self.from, self.to, self.radius) {
            let outcome =
                route_or_bank_fetch(&self.world, self.from, target, self.opts, state, &self.bank);
            if !matches!(outcome, RouteOutcome::NoPath) {
                return outcome;
            }
        }
        RouteOutcome::NoPath
    }
}
impl NavBot {
    fn bump_walk_outcome_seq(&mut self) {
        self.walk_outcome_seq = self.walk_outcome_seq.wrapping_add(1);
        if self.walk_outcome_seq == 0 {
            self.walk_outcome_seq = 1;
        }
    }

    /// Older armed-route results may publish only after the current wait
    /// refusal has been copied into a snapshot, or when they *are* that wait.
    fn armed_outcome_may_publish(&self, request_id: u64) -> bool {
        self.walk_live_refusal_id == 0 || request_id == self.walk_live_refusal_id
    }

    fn mark_walk_outcome_posted(&mut self) {
        self.walk_live_refusal_id = 0;
    }

    fn note_failure(
        &mut self,
        generation: u64,
        request_id: u64,
        to: WorldTile,
        radius: i32,
        allow_teleports: bool,
    ) {
        // Legacy request id 0 never settles a wait. Do not replace a live
        // nonzero refusal with it before the isolate can observe the refusal.
        if request_id == 0 && self.walk_live_refusal_id != 0 {
            return;
        }
        self.bump_walk_outcome_seq();
        self.walk_outcome_generation = generation;
        self.walk_outcome_request_id = request_id;
        self.walk_outcome_failed = true;
        self.walk_outcome_x = to.x;
        self.walk_outcome_z = to.z;
        self.walk_outcome_level = to.level;
        self.walk_outcome_radius = radius;
        self.walk_outcome_allow_teleports = allow_teleports;
        if request_id != 0 && request_id != self.walk_request_id {
            self.walk_live_refusal_id = request_id;
        } else {
            self.walk_live_refusal_id = 0;
        }
    }

    fn clear_walk_outcome(&mut self) {
        self.bump_walk_outcome_seq();
        self.walk_outcome_failed = false;
        self.walk_outcome_generation = self.route_generation;
        self.walk_outcome_request_id = 0;
        self.walk_outcome_x = 0;
        self.walk_outcome_z = 0;
        self.walk_outcome_level = 0;
        self.walk_outcome_radius = 0;
        self.walk_outcome_allow_teleports = false;
        self.walk_live_refusal_id = 0;
    }

    fn publish_route(
        &mut self,
        generation: u64,
        request_id: u64,
        allow_teleports: bool,
        outcome: RouteOutcome,
    ) {
        // Superseded workers and aborted/restarted runs keep a newer
        // route_generation. A late NoPath for the same dest must not publish.
        if self.route_generation != generation {
            return;
        }
        let (route, pending) = match outcome {
            RouteOutcome::Routed(route) => (route, None),
            RouteOutcome::BankSession { pending, route } => (route, Some(pending)),
            RouteOutcome::NoPath => {
                if let Some((to, radius, allow_teleports, ..)) = self.requested_route {
                    if self.armed_outcome_may_publish(request_id) {
                        self.note_failure(generation, request_id, to, radius, allow_teleports);
                    }
                }
                // The retained route belongs to the previous request. A later
                // request for this failed destination must be allowed to retry.
                self.requested_route = None;
                return;
            }
        };
        self.traveller.clear();
        self.route = Some(route);
        self.bank_fetch = pending;
        self.allow_teleports = allow_teleports;
    }
}
#[cfg(feature = "memory-profile")]
pub mod memory;

#[cfg(feature = "memory-profile")]
mod memory_diagnostics;

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
