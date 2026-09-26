//! `tui-play`: the headless operator panel binary. Same flag spirit as
//! `host-play` / `panel-play` (`--vault`, `--vault-pass` / `BOT_VAULT_PASS`,
//! `--host`, `--port`, `--cache`, `--user`, `--live script_<name>`).
//!
//! The operator lifecycle (vault, `host_play::Play`, fleet membership,
//! selection, Load/Log in/Log out/Remove, script Start/Stop settlement and
//! status polling) lives in the shared [`frontend_core::OperatorSession`];
//! the binary adds a per-frame hook that publishes each slot's snapshot and
//! steps the focused slot's walk arm, and the UI loop polls the core,
//! refreshes [`TuiApp`], routes keys/clicks, and dispatches the returned
//! [`AppAction`] —
//! map Walk-confirm consumes a revision-bound `MapCommand` through
//! `host_play::Play::map_walk`, chat Continue/Answer and WASD walks go
//! through `host_play::WireCmd`, and the settings popup writes
//! `ProfileSettings` on the focused profile.
//!
//! **Raster Off:** every profile is spawned with `RasterMode::Off` and the
//! TUI never attaches a `Renderer` (no `panel` / imgui / wgpu anywhere).
//! `--live script_<name>` mints per-run usernames into an ephemeral vault
//! (like the panel) and PASS/FAIL comes from the scenario runner, not a
//! screenshot.

use std::collections::{HashMap, HashSet};
use std::env;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyEventKind, MouseEventKind};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
#[cfg(test)]
use host_play::arm_walk_on;
use host_play::walk_map::{
    observed_services, ActionError, ActionKind, Catalogue, MapContext, WalkExclude,
    WalkSlotRequest, WalkSlotStatus,
};
use host_play::{
    background_ack_text, background_bots_ack_error, background_bots_acked,
    clear_background_bots_ack_error, live_vault_passphrase_for, load_navpois, map_ready_catalogue,
    mint_live_entries_for_target, mint_live_names, open_vault, parse_profile_args,
    peek_map_catalogue, persist_background_bots_ack, player_here_tile, profile_password_for,
    run_with_io, run_with_template, step_walk_arm_bank_fetch, walk_arm_bank_fetch_freezes_follow,
    MapDemandHandle, MapJobStatus, MapStage, PlayOptions, ProfileOptions, ReadyCatalogue,
    ResourceSampler, ResourceView, ServerProfile, SharedClientTemplate, WalkArm, WireCmd,
};
use nav::map::identity::Digest;
use nav::tile::Tile;
use nav::traveller::{TravelOptions, TravelOutcome};
use nav::WorldState;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use vault::{Profile, Vault};

use frontend_core::{
    load_map_bake_choice, persist_map_bake_choice, HeadlessSurface, MapBakeGate, OperatorSession,
    ScriptStart,
};
use host_play::map_cache::MapDemand;

use crate::app::{AppAction, ChatData, MapCatalogueStatus, TuiApp};
use crate::chat::ChatAction;
use crate::script_shape::{
    categories_present, resolve_category_order, rs2b0t_root_has_index, BrowseCard,
};

/// What `tui-play` should do this run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunMode {
    /// Interactive operator panel.
    Interactive,
    /// `--live script_<name>`: PASS/FAIL from the scenario runner.
    Live(String),
}

/// Parsed `tui-play` flags.
#[derive(Debug, Clone)]
pub struct Args {
    pub pass: Option<String>,
    pub users: Vec<String>,
    pub live: Option<String>,
    pub world: Option<u16>,
    pub profile: ProfileOptions,
}

fn usage() -> ! {
    eprintln!(
        "usage: tui-play [--profile local-274|local-289|public-289] [--revision 274|289] \
         [--prod] [--host HOST] [--port PORT] [--asset-host HOST] [--http-port PORT] \
         [--engine DIR] [--cache DIR] [--unpack DIR] [--nav-pack PATH] [--nav-flags PATH] \
         [--content DIR] [--vault PATH] [--catalog DIR] [--cache-manifest PATH] [--vault-pass PASS] \
         [--world N] [--live script_<name>] [--user USER]... (default user: first vault profile)"
    );
    std::process::exit(2);
}

fn need_value(
    it: &mut impl Iterator<Item = impl AsRef<str>>,
    flag: &str,
) -> Result<String, String> {
    it.next()
        .map(|s| s.as_ref().to_string())
        .ok_or_else(|| format!("tui-play: {flag} needs a value"))
}

/// `--live NAME` wins over `BOT_LIVE`; empty env is ignored.
/// `--help`/`-h` print the usage line (exit 2, the CLI family's
/// convention). Shared server/profile flags are consumed first by host-play.
pub fn parse_args() -> Args {
    match parse_args_from(env::args().skip(1)) {
        Ok(args) => args,
        Err(msg) => {
            if msg != "usage" {
                eprintln!("{msg}");
            }
            usage();
        }
    }
}

/// Testable CLI parse. Does not flip [`client::set_bot_target`].
pub fn parse_args_from(args: impl IntoIterator<Item = impl AsRef<str>>) -> Result<Args, String> {
    let (profile, rest) = parse_profile_args(args).map_err(|e| format!("tui-play: {e}"))?;
    let mut parsed = Args {
        pass: env::var("BOT_VAULT_PASS").ok(),
        world: None,
        users: Vec::new(),
        live: env::var("BOT_LIVE").ok().filter(|s| !s.is_empty()),
        profile,
    };
    let mut it = rest.into_iter();
    while let Some(arg) = it.next() {
        match arg.as_ref() {
            "--vault-pass" => parsed.pass = Some(need_value(&mut it, "--vault-pass")?),
            "--user" => parsed.users.push(need_value(&mut it, "--user")?),
            "--world" => {
                let value = need_value(&mut it, "--world")?;
                parsed.world = Some(value.parse::<u16>().ok().filter(|n| *n != 0).ok_or_else(
                    || "tui-play: --world needs a positive world number".to_string(),
                )?);
            }
            "--live" => parsed.live = Some(need_value(&mut it, "--live")?),
            "--help" | "-h" => return Err("usage".into()),
            other => return Err(format!("tui-play: unknown {other}")),
        }
    }
    Ok(parsed)
}

/// Refuse a non-loopback `--host` while local RSA is active (same bind as host-play).
pub fn validate_startup_host(host: &str) -> Result<(), &'static str> {
    host_play::validate_play_host(host, client::bot_target())
}

/// The `--live` scenario, `Err` when the name is not a `script_<name>`.
/// Unknown names fail the run (same usage contract as panel-play).
pub fn live_scenario(name: &str) -> Result<scenario::Scenario, String> {
    let script = name
        .strip_prefix("script_")
        .ok_or_else(|| format!("tui-play: --live {name}: only script_<name> is supported"))?;
    scenario::get(script).ok_or_else(|| format!("tui-play: --live {name}: unknown scenario"))
}

/// Throwaway encrypted vault for `--live` (minted names, target-aware pass).
fn temp_live_vault(entries: &[(String, String)], vault_pass: &str) -> PathBuf {
    static SERIAL: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let serial = SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = env::temp_dir().join(format!(
        "274bot-tui-live-{}-{}-{serial}",
        std::process::id(),
        entries.len()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("vault");
    if path.exists() {
        std::fs::remove_file(&path).unwrap();
    }
    let mut vault = Vault::create(&path, vault_pass).unwrap();
    for (i, (user, pass)) in entries.iter().enumerate() {
        vault
            .upsert(Profile {
                username: user.clone(),
                password: pass.clone(),
                uid: 274_000_001 + i as i32,
                settings: vault::ProfileSettings {
                    auto_login: true,
                    ..vault::ProfileSettings::default()
                },
            })
            .unwrap();
    }
    path
}

/// The walk-arm step latch key: `(player gen, here)` per username, so a
/// hop is sent once per server tick, not every 20 ms frame.
type NavStepLatch = HashMap<String, (u64, (i32, i32, i32))>;

type SlotTravellers = Arc<Mutex<HashMap<String, Arc<Mutex<WalkArm>>>>>;

fn reset_frontend_slot_session(
    name: &str,
    travellers: &SlotTravellers,
    tick_latch: &Arc<Mutex<NavStepLatch>>,
) -> bool {
    tick_latch.lock().unwrap().remove(name);
    travellers.lock().unwrap().remove(name).is_some()
}

fn reset_frontend_slot_lifetime(
    name: &str,
    gens: &Arc<Mutex<HashMap<String, client::client::ClientGens>>>,
    snapshots: &Arc<Mutex<HashMap<String, api::snapshot::GameSnapshot>>>,
    travellers: &SlotTravellers,
    tick_latch: &Arc<Mutex<NavStepLatch>>,
) -> bool {
    gens.lock().unwrap().remove(name);
    snapshots.lock().unwrap().remove(name);
    reset_frontend_slot_session(name, travellers, tick_latch)
}

fn publish_frontend_slot(
    name: &str,
    client: &client::client::Client,
    gens: &Arc<Mutex<HashMap<String, client::client::ClientGens>>>,
    snapshots: &Arc<Mutex<HashMap<String, api::snapshot::GameSnapshot>>>,
    travellers: &SlotTravellers,
    tick_latch: &Arc<Mutex<NavStepLatch>>,
) -> bool {
    let mut gens_guard = gens.lock().unwrap();
    let last = gens_guard.entry(name.to_string()).or_default();
    let mut snapshot_guard = snapshots.lock().unwrap();
    let snapshot = snapshot_guard.entry(name.to_string()).or_default();
    let publication = host_play::Host::publish_frontend_snapshot(last, snapshot, client);
    drop(snapshot_guard);
    if publication.session_boundary {
        reset_frontend_slot_session(name, travellers, tick_latch);
    }
    publication.session_boundary
}

/// Claim the one mainland seed for a slot's cold login. Reconnects retain
/// scenario-owned position, while an ordinary interactive boot stays opt-in.
fn take_mainland_seed(
    sent: &Mutex<HashSet<String>>,
    name: &str,
    enabled: bool,
    last_login_reconnect: Option<bool>,
) -> bool {
    enabled && last_login_reconnect != Some(true) && sent.lock().unwrap().insert(name.to_string())
}

fn mainland_seed_options(options: &PlayOptions) -> (bool, PlayOptions) {
    let enabled = options.mainland;
    let mut host_options = options.clone();
    host_options.mainland = false;
    (enabled, host_options)
}

fn seed_mainland_on_ready<D: api::interact::Driver>(
    driver: &mut D,
    sent: &Mutex<HashSet<String>>,
    name: &str,
    enabled: bool,
    ready: bool,
    last_login_reconnect: Option<bool>,
) -> bool {
    if ready && take_mainland_seed(sent, name, enabled, last_login_reconnect) {
        api::interact::mainland_hop(driver);
        true
    } else {
        false
    }
}

/// Panel-parity walk-arm tick: BankBudget session first, then route follow.
fn step_walk_arm_follow<D: api::interact::Driver>(
    driver: &mut D,
    snapshot: &api::snapshot::GameSnapshot,
    arm: &mut WalkArm,
    world: Option<&nav::world::NavWorld>,
    here: (i32, i32, i32),
    map_members: bool,
) -> bool {
    if arm.bank_fetch.is_some() {
        step_walk_arm_bank_fetch(driver, snapshot, arm, world, Some(here), map_members);
        if walk_arm_bank_fetch_freezes_follow(arm) {
            return false;
        }
    }
    let Some(route) = arm.route.clone() else {
        return false;
    };
    let walking_stand = arm.bank_fetch.as_ref().is_some_and(|p| {
        matches!(
            p.steps.front(),
            Some(nav::bank_fetch::BankStep::Walk { x, z, level })
                if route.dest.x == *x
                    && route.dest.z == *z
                    && route.dest.level == *level
        )
    });
    let mut options = TravelOptions {
        close_enough: 0,
        teleports: world.map(|w| w.graph.teleports.as_slice()),
        edges: world.map(|w| w.graph.edges.as_slice()),
        ..TravelOptions::default()
    };
    let outcome = arm.traveller.follow(driver, snapshot, route, &mut options);
    if walking_stand
        && matches!(
            &outcome,
            Some(o) if !matches!(o, TravelOutcome::Arrived { .. })
        )
    {
        arm.bank_fetch = None;
        arm.route = None;
        return false;
    }
    if outcome.is_some() {
        arm.route = None;
        true
    } else {
        false
    }
}

/// Catalog card live_prepare stashes so the StartScript pump can
/// `script_start_load` once after seed waits.
struct PendingCatalogStart {
    slot: String,
    js: String,
    shape: script::LoadShape,
    bag: Option<serde_json::Map<String, serde_json::Value>>,
    siblings: Vec<(String, String)>,
    loadouts: Vec<script::Loadout>,
    compiled: Option<script::CompiledId>,
}

fn scenario_fixture_loadouts(settings: &scenario::ScenarioSettings) -> Vec<script::Loadout> {
    settings
        .fixture_loadouts
        .unwrap_or(&[])
        .iter()
        .map(|row| {
            row.carry
                .iter()
                .fold(script::Loadout::new(row.name), |loadout, &(item, qty)| {
                    loadout.with_carry(item, qty)
                })
        })
        .collect()
}

fn start_stashed_catalog_card(
    handle: &host_play::ScriptStartHandle,
    card: &PendingCatalogStart,
) -> Result<(), String> {
    if let Some(id) = card.compiled {
        return handle.start_compiled(&card.slot, id);
    }
    let result = if card.loadouts.is_empty() {
        handle.start_load(
            &card.slot,
            card.js.clone(),
            card.shape,
            card.bag.clone(),
            card.siblings.clone(),
        )
    } else {
        handle.start_load_with_loadouts(
            &card.slot,
            card.js.clone(),
            card.shape,
            card.bag.clone(),
            card.siblings.clone(),
            &card.loadouts,
        )
    };
    if result.is_ok() && std::env::var_os("BOT_DEBUG").is_some() {
        let fixture_names = card
            .loadouts
            .iter()
            .map(|loadout| loadout.name.as_str())
            .collect::<Vec<_>>();
        let selected_loadout = card
            .bag
            .as_ref()
            .and_then(|bag| bag.get("loadout"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("<unset>");
        eprintln!(
            "[tui-play] catalog start fixtures={} names={fixture_names:?} loadout={selected_loadout:?}",
            card.loadouts.len()
        );
    }
    result
}

/// When the runner is on [`scenario::StepKind::StartScript`], start the
/// stashed catalog isolate once. Returns false when Start was attempted
/// and failed, so the pump must not consume the one-tick wait.
fn fire_pending_catalog_start(
    pending: &Mutex<Option<PendingCatalogStart>>,
    handle: &Mutex<Option<host_play::ScriptStartHandle>>,
    runner: &scenario::ScenarioRunner,
) -> bool {
    if !runner.on_start_script() {
        return true;
    }
    let mut pending = pending.lock().unwrap();
    let Some(card) = pending.as_ref() else {
        return true;
    };
    let handle = handle.lock().unwrap();
    let Some(h) = handle.as_ref() else {
        return false;
    };
    if start_stashed_catalog_card(h, card).is_ok() {
        pending.take();
        true
    } else {
        false
    }
}

/// The TUI session: the running play, the vault, the per-slot snapshot
/// publication, and the per-username walk arms (the same `WalkArm` map
/// `host_play::arm_walk_on` latches and the panel drives).
pub struct TuiSession {
    #[cfg(feature = "memory-profile")]
    memory: Option<host_play::memory::Run>,
    /// Shared operator lifecycle (vault, play, fleet, selection, removals,
    /// status polling and operation results). Headless: no per-slot IO.
    core: OperatorSession<()>,
    auto_world: Option<u16>,
    pub error: Option<String>,
    /// All profile names (for the strip's slot list), in vault order.
    names: Vec<String>,
    /// Legacy-only endpoint bag retained for unit-test constructors.
    options: PlayOptions,
    /// Checked production assets and immutable server identity.
    template: Option<Arc<SharedClientTemplate>>,
    server_profile: Option<Arc<ServerProfile>>,
    /// Catalogue-only demand lease. Dropped on MapClose; never requests PNGs.
    map_demand: Option<MapDemandHandle>,
    /// Map demand goes through the shared bake consent (catalogue-only here,
    /// so it never asks); the remembered choice is edited in settings.
    map_bake: MapBakeGate,
    map_catalogue_named: bool,
    /// The username the settings popup currently edits; reload
    /// `ProfileSettings` into the app when it changes.
    last_focused: Option<String>,
    /// Per-username snapshots, rebuilt by the per-frame hook.
    snapshots: Arc<Mutex<HashMap<String, api::snapshot::GameSnapshot>>>,
    /// Host publication cursor per slot. PLAYER_INFO's tick edge is tracked
    /// separately from the snapshot's family generations.
    frontend_gens: Arc<Mutex<HashMap<String, client::client::ClientGens>>>,
    /// Per-username walk arms; the focused arm's route paints the map and
    /// the per-frame hook steps it via `Traveller::follow`.
    travellers: SlotTravellers,
    /// Last `(player gen, here)` ticked per username, so a walk hop is
    /// sent once per server tick, not every 20 ms frame.
    tick_latch: Arc<Mutex<NavStepLatch>>,
    /// Slot threads set this when a traveller returns Arrived/Budget so
    /// the UI can clear the picked walk dest.
    walk_clear: Arc<AtomicBool>,
    /// The shared nav world (`Play::world`), read by the per-frame walk
    /// step and the app thread's route arms.
    nav_world: Arc<Mutex<Option<Arc<nav::world::NavWorld>>>>,
    /// The shared `--live script_*` runner (slot threads tick it from the
    /// per-frame hook; the UI loop reads its status/evidence).
    scenario: Arc<Mutex<Option<scenario::ScenarioRunner>>>,
    /// Catalog card `live_prepare_script` stashes; the live pump Starts
    /// it once on [`scenario::StepKind::StartScript`].
    pending_script: Arc<Mutex<Option<PendingCatalogStart>>>,
    /// Isolate-start handle the per-frame hook uses (filled after Play).
    script_start_handle: Arc<Mutex<Option<host_play::ScriptStartHandle>>>,
    /// The `--live` run's scenario name, for the PASS/FAIL label.
    live_name: Option<String>,
    /// `BUDGET_S` soak: keep pumping after proof PASS until this instant.
    live_soak_until: Option<Instant>,
    live_announced_pass: bool,
    live_wait_script_stop: Option<&'static str>,
    live_stop_wait_started: Option<Instant>,
    /// The Browse-selected card (catalog Start after seed, or operator Start).
    script_sel: Option<script::ScriptSel>,
    /// The out-of-tree JS library: the Browse picker's cards and the
    /// Load/Start source for the focused slot (same store the panel
    /// persists to).
    js: script::JsLibrary,
    /// The `$RS2B0T` registry cards were filled into `js` once (first
    /// Browse/Load, like the panel).
    rs2b0t_filled: bool,
    /// First-run rs2b0t clone-root folder browser.
    rs2b0t_catalog_open: bool,
    rs2b0t_catalog_dir: PathBuf,
    /// Browse category order keys (in-memory; panel persists to panel-ui.json).
    script_category_order: Vec<String>,
    /// Operator script-parameter overrides (`~/.274bot/script-settings.json`).
    script_settings: script::ScriptSettingsStore,
    /// Process-wide loadout presets (`~/.274bot/loadouts.json`).
    loadouts: script::LoadoutsStore,
    /// Scenario/live inject merged last on Start.
    script_settings_inject: Option<serde_json::Map<String, serde_json::Value>>,
    /// Last directory visited in the out-of-tree Load file browser.
    script_load_last_dir: Option<PathBuf>,
    /// Load Starts whose isolate setup has not settled, by profile: Start
    /// returns before V8 setup, so the card's load diagnostic is recorded
    /// or cleared when [`TuiSession::settle_script_starts`] observes it.
    pending_starts: HashMap<String, script::JsCard>,
    resource_sampler: ResourceSampler,
    persist_ui: bool,
    background_bots_acked: bool,
    ack_checked_at: Option<Instant>,
    notice_sig: Option<(usize, ResourceView)>,
}

#[cfg(test)]
impl TuiSession {
    /// Inject a `Play` for unit tests (no vault / slot threads).
    fn inject_play(&mut self, play: host_play::Play) {
        self.core.set_play(Some(play));
    }

    fn expire_ack_cache(&mut self) {
        self.ack_checked_at = None;
    }
}

impl TuiSession {
    /// Empty session over the default engine options.
    #[cfg(test)]
    fn new(options: PlayOptions) -> Self {
        Self::with_instance(options, host_play::InstancePermit::SkipLock)
    }

    fn with_instance(options: PlayOptions, _instance: host_play::InstancePermit) -> Self {
        #[cfg(test)]
        script::IsolatedEnv::ensure_thread();
        let mut js = script::JsLibrary::new(script::default_js_store());
        let _ = js.restore(); // missing/broken store is not fatal here
        Self {
            #[cfg(feature = "memory-profile")]
            memory: None,
            core: OperatorSession::new(_instance),
            auto_world: None,
            error: None,
            names: Vec::new(),
            options,
            template: None,
            server_profile: None,
            map_demand: None,
            map_bake: MapBakeGate::new(load_map_bake_choice()),
            map_catalogue_named: false,
            last_focused: None,
            snapshots: Arc::new(Mutex::new(HashMap::new())),
            frontend_gens: Arc::new(Mutex::new(HashMap::new())),
            travellers: Arc::new(Mutex::new(HashMap::new())),
            tick_latch: Arc::new(Mutex::new(HashMap::new())),
            walk_clear: Arc::new(AtomicBool::new(false)),
            nav_world: Arc::new(Mutex::new(None)),
            scenario: Arc::new(Mutex::new(None)),
            pending_script: Arc::new(Mutex::new(None)),
            script_start_handle: Arc::new(Mutex::new(None)),
            live_name: None,
            live_soak_until: None,
            live_announced_pass: false,
            live_wait_script_stop: None,
            live_stop_wait_started: None,
            script_sel: None,
            js,
            rs2b0t_filled: false,
            rs2b0t_catalog_open: false,
            rs2b0t_catalog_dir: Self::default_catalog_browse_dir(),
            script_category_order: Vec::new(),
            script_settings: script::ScriptSettingsStore::with_default_path(),
            loadouts: script::LoadoutsStore::with_default_path(),
            script_settings_inject: None,
            script_load_last_dir: None,
            pending_starts: HashMap::new(),
            resource_sampler: ResourceSampler::default(),
            persist_ui: true,
            background_bots_acked: background_bots_acked(),
            ack_checked_at: Some(Instant::now()),
            notice_sig: None,
        }
    }

    fn new_bound(template: Arc<SharedClientTemplate>, permit: host_play::InstancePermit) -> Self {
        let profile = Arc::clone(template.profile());
        let mut session = Self::with_instance(
            PlayOptions {
                host: profile.client().game_host().to_string(),
                port: profile.client().game_port(),
                cache_dir: profile.client().cache_dir().display().to_string(),
                lowmem: true,
                mainland: false,
            },
            permit,
        );
        session.template = Some(template);
        session.server_profile = Some(profile);
        session
    }

    fn target(&self) -> client::BotTarget {
        self.server_profile
            .as_ref()
            .map_or_else(client::bot_target, |profile| profile.target())
    }

    fn app_title(&self) -> String {
        let revision = self
            .server_profile
            .as_ref()
            .map_or(274, |profile| profile.revision().as_i32());
        format!("{revision}bot")
    }

    fn profile_label(&self) -> String {
        self.server_profile.as_ref().map_or_else(
            || "legacy local-274 · revision 274".into(),
            |profile| profile.label(),
        )
    }

    fn catalog_root(&self) -> Option<PathBuf> {
        match self.server_profile.as_ref() {
            Some(profile) => profile.catalog_root().map(Path::to_path_buf),
            None => script::rs2b0t_root(),
        }
    }

    fn merged_settings_bag(
        &self,
        source: script::ScriptSource,
        name: &str,
        schema: &[script::SettingDef],
    ) -> serde_json::Map<String, serde_json::Value> {
        self.script_settings
            .merged_bag(source, name, schema, self.script_settings_inject.as_ref())
    }

    /// Schema defaults + overrides + inject. Empty schema still keeps
    /// inject keys (Thiever `target: Guard`). `None` only when the merged
    /// bag is empty.
    fn pending_settings_bag(
        &self,
        source: script::ScriptSource,
        name: &str,
        schema: &[script::SettingDef],
    ) -> Option<serde_json::Map<String, serde_json::Value>> {
        let merged = self.merged_settings_bag(source, name, schema);
        if merged.is_empty() {
            None
        } else {
            Some(merged)
        }
    }

    fn default_catalog_browse_dir() -> PathBuf {
        let home = script::bot_home();
        if home.as_os_str() == "." {
            PathBuf::from("/")
        } else {
            home
        }
    }

    /// Unlock (or first-run create) the vault at `path` and start the play.
    fn unlock_at(&mut self, path: &Path, pass: &str) -> Result<(), String> {
        let vault = open_vault(path, pass).map_err(|e| e.to_string())?;
        self.start_play(vault)
    }

    /// Empty `Play` (shared cache + FIFO + per-frame hook) handed to the
    /// core; the boot then loads and logs in the focused profile only and
    /// `m` loads and logs in the rest.
    fn start_play(&mut self, vault: Vault) -> Result<(), String> {
        let snapshots = Arc::clone(&self.snapshots);
        let frontend_gens = Arc::clone(&self.frontend_gens);
        let travellers = Arc::clone(&self.travellers);
        let tick_latch = Arc::clone(&self.tick_latch);
        let walk_clear = Arc::clone(&self.walk_clear);
        let nav_world = Arc::clone(&self.nav_world);
        let scenario = Arc::clone(&self.scenario);
        let pending_script = Arc::clone(&self.pending_script);
        let script_start_handle = Arc::clone(&self.script_start_handle);
        let options = self.options.clone();
        let (mainland, host_options) = mainland_seed_options(&options);
        let mainland_sent = Arc::new(Mutex::new(HashSet::new()));
        let map_members = self
            .template
            .as_ref()
            .map(|t| t.profile().map_members())
            .unwrap_or(false);
        let per_frame = move |c: &mut client::client::Client, name: &str, hold: bool| {
            // Clear facts and externally armed work at the actual session
            // boundary before scenario/local-player/Guardian early returns.
            if publish_frontend_slot(
                name,
                c,
                &frontend_gens,
                &snapshots,
                &travellers,
                &tick_latch,
            ) {
                walk_clear.store(true, Ordering::Relaxed);
            }

            // TUI owns mainland seeding so host-play cannot re-arm it when
            // an intentional scenario logout starts a new run_client stretch.
            if seed_mainland_on_ready(
                c,
                &mainland_sent,
                name,
                mainland,
                c.ingame && c.scene_state == 2 && c.local_player.is_some(),
                c.last_login_reconnect,
            ) {
                api::host_log!(
                    api::hostlog::Category::Lifecycle,
                    api::hostlog::Level::Info,
                    slot = name,
                    "mainland hop queued"
                );
            }

            // The shared `--live script_*` runner: tick the driven
            // slot and its companions before the local-player gate
            // (seeding must observe frames with no player decode).
            // Hold freezes scenario follow like `step_nav_bot`.
            if let Some(runner) = scenario.lock().unwrap().as_mut() {
                if runner.drives(name) {
                    if fire_pending_catalog_start(&pending_script, &script_start_handle, runner) {
                        runner.tick_with_hold(c, hold);
                    }
                } else if let Some(index) = runner.companion_for(name) {
                    runner.companion_tick(index, c);
                }
            }

            let Some(here) = player_here_tile(c) else {
                return;
            };
            // Guardian hold freezes WalkArm follow; the armed route
            // stays latched and resumes when hold lifts.
            if !WalkArm::may_follow(hold) {
                return;
            }
            // Step the armed walk route one leg per player-info tick
            // (the panel's `tick_latch` pattern — a hop is sent once
            // per server tick, not re-sent every 20 ms frame).
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
                let snapshots = snapshots.lock().unwrap();
                let Some(snap) = snapshots.get(name) else {
                    return;
                };
                let mut arm = arm.lock().unwrap();
                let world = nav_world.lock().unwrap().clone();
                step_walk_arm_follow(c, snap, &mut arm, world.as_deref(), here, map_members)
            };
            if finished {
                walk_clear.store(true, Ordering::Relaxed);
            }
        };
        let mut play = match self.template.clone() {
            Some(template) => run_with_template(
                template,
                host_options.mainland,
                Vec::new(),
                |_| (None, None),
                per_frame,
            )?,
            None => run_with_io(&host_options, Vec::new(), |_| (None, None), per_frame),
        };
        if let Some(number) = self.auto_world {
            play.set_auto_world(number)?;
        }
        self.nav_world.lock().unwrap().clone_from(&play.world());
        if std::env::var_os("BOT_DEBUG").is_some() {
            let game_data = play.game_data();
            let lobster_heal = game_data
                .as_deref()
                .and_then(|data| data.fixed_food_heal("Lobster"));
            eprintln!(
                "[tui-play] script-start handle profile={:?} game_data={} lobster_fixed_heal={lobster_heal:?}",
                self.profile_label(),
                if game_data.is_some() {
                    "attached"
                } else {
                    "absent"
                }
            );
        }
        *self.script_start_handle.lock().unwrap() = Some(play.script_start_handle());
        self.core.start(vault, play);
        Ok(())
    }

    /// Load `name` into the fleet and log it in (the TUI boot and `--live`
    /// intent). `RasterMode::Off` always (the headless surface never
    /// attaches a `Renderer`); after a DC the slot re-handshakes only when
    /// the profile's `auto_login` is on.
    fn load_and_login(&mut self, name: &str) -> bool {
        let failure = {
            let (core, mut surface) = self.core_and_surface();
            let (load, _) = core.load(name, &mut surface);
            let login = core.login(name, &mut surface);
            core.failure(load).or_else(|| core.failure(login))
        };
        match failure {
            Some(error) => {
                self.error = Some(error);
                false
            }
            None => true,
        }
    }

    /// Split borrow: the core plus the headless surface, whose lifetime
    /// reset drops this slot's published snapshot and walk arm.
    fn core_and_surface(
        &mut self,
    ) -> (
        &mut OperatorSession<()>,
        HeadlessSurface<impl FnMut(&str) + '_>,
    ) {
        let gens = &self.frontend_gens;
        let snapshots = &self.snapshots;
        let travellers = &self.travellers;
        let tick_latch = &self.tick_latch;
        let walk_clear = &self.walk_clear;
        (
            &mut self.core,
            HeadlessSurface::with_reset(move |name: &str| {
                if reset_frontend_slot_lifetime(name, gens, snapshots, travellers, tick_latch) {
                    walk_clear.store(true, Ordering::Relaxed);
                }
            }),
        )
    }

    /// Log in the focused member (explicit handshake; recreates a terminal
    /// worker).
    fn login(&mut self, app: &mut TuiApp) {
        let Some(name) = app.focused_name() else {
            return;
        };
        let (core, mut surface) = self.core_and_surface();
        let op = core.login(&name, &mut surface);
        app.error = core.failure(op);
    }

    /// Log out the focused member; the slot stays loaded and latched.
    fn logout(&mut self, app: &mut TuiApp) {
        if let Some(name) = app.focused_name() {
            self.core.logout(&name);
        }
    }

    /// Log out every member and every other running slot.
    fn logout_all(&mut self) {
        self.core.logout_all();
    }

    /// Remove the focused member: clean logout, then its worker stops. The
    /// neighbour becomes focused.
    fn remove(&mut self, app: &mut TuiApp) {
        let Some(name) = app.focused_name() else {
            return;
        };
        let (core, mut surface) = self.core_and_surface();
        let removal = core.remove(&name, Instant::now(), &mut surface);
        if let Some(next) = removal.reselected {
            app.focused = app.names.iter().position(|n| n == &next);
        } else if removal.selection_cleared {
            app.focused = None;
        }
        // The strip drops the member on the next pump.
    }

    /// Load every vault profile and log in every member (the `m` key). A
    /// loaded, logged-out member is re-armed and a terminal worker
    /// recreated. Returns how many were newly loaded.
    fn load_and_login_all(&mut self) -> usize {
        let (added, failure) = {
            let (core, mut surface) = self.core_and_surface();
            let (load, added) = core.load_all(&mut surface);
            let login = core.login_all(&mut surface);
            (added, core.failure(load).or_else(|| core.failure(login)))
        };
        if failure.is_some() {
            self.error = failure;
        }
        added
    }

    /// Select `name` (pure bookkeeping: never a spawn or login).
    fn focus(&mut self, name: &str) {
        self.core.select(name);
    }

    /// `--live script_*` boot: minted ephemeral vault + spawn + runner.
    fn live_prepare_script(&mut self, scenario: scenario::Scenario) -> Result<(), String> {
        self.persist_ui = false;
        let name = scenario.name.to_string();
        let start_script = scenario.settings.start_script;
        let start_file = scenario.settings.start_file;
        let wait_script_stop = scenario.settings.wait_script_stop;
        let settings_inject = scenario.settings.script_settings_inject;
        let fixture_loadouts = scenario_fixture_loadouts(&scenario.settings);
        let names = mint_live_names(scenario.seed.profiles.len());
        let entries = mint_live_entries_for_target(&names, self.target());
        let pass = live_vault_passphrase_for(self.target());
        let path = temp_live_vault(&entries, &pass);
        self.unlock_at(&path, &pass)?;
        self.live_name = Some(name);
        self.live_wait_script_stop = wait_script_stop;
        self.live_stop_wait_started = None;
        let world = self.core.play().and_then(|play| play.world());
        let mut runner = scenario::ScenarioRunner::with_world(scenario, world);
        if let Some(budget) = scenario::budget_s_from_env() {
            runner.set_deadline(budget);
            self.live_soak_until = Some(Instant::now() + budget);
            self.live_announced_pass = false;
        }
        runner.set_live_names(&names);
        if let Some(play) = self.core.play() {
            runner.set_obj_names(play.obj_names());
        }
        *self.scenario.lock().unwrap() = Some(runner);
        self.script_settings_inject = scenario::settings_inject_map(settings_inject);
        self.names = names.clone();
        for n in &names {
            self.load_and_login(n);
        }
        self.focus(&names[0]);
        // A scenario that names a script card selects the real `$RS2B0T`
        // catalog script on the driven slot (same as the panel): fill the
        // catalog from `$RS2B0T`, then Start on StartScript after seed.
        // Exact example files Load as File cards and select by identity_id.
        if let Some(file_name) = start_file {
            let path = script::live_example_path(file_name)
                .ok_or_else(|| format!("no in-tree example {file_name}"))?;
            let loaded = self
                .js
                .load(&path)
                .map_err(|e| format!("load {file_name}: {e}"))?;
            let identity = loaded.identity_id();
            let card = self
                .js
                .get(script::ScriptSource::File, &identity)
                .cloned()
                .ok_or_else(|| format!("file example {file_name} missing after identity load"))?;
            self.script_sel = Some(script::ScriptSel::Loaded(
                script::ScriptSource::File,
                identity.clone(),
            ));
            let bag = self.pending_settings_bag(
                script::ScriptSource::File,
                &identity,
                &card.settings_schema,
            );
            let siblings = script::resolve_sibling_modules(
                &card.path,
                &card.origin,
                self.js.cache(),
                script::CacheMeta {
                    kind: card.kind,
                    source: card.source,
                    shape: None,
                    api_family: Some(card.api_family.as_str().into()),
                },
            )?;
            *self.pending_script.lock().unwrap() = Some(PendingCatalogStart {
                slot: names[0].clone(),
                js: card.js.clone(),
                shape: card.shape,
                bag,
                siblings,
                loadouts: fixture_loadouts.clone(),
                compiled: None,
            });
        } else if let Some(card_name) = start_script {
            if let Some(id) = script::compiled_id(card_name) {
                self.script_sel = Some(script::ScriptSel::Compiled(id));
                *self.pending_script.lock().unwrap() = Some(PendingCatalogStart {
                    slot: names[0].clone(),
                    js: String::new(),
                    shape: script::LoadShape::Reject,
                    bag: None,
                    siblings: Vec::new(),
                    loadouts: Vec::new(),
                    compiled: Some(id),
                });
            } else {
                self.fill_rs2b0t_cards_once();
                self.js
                    .ensure_js(script::ScriptSource::Catalog, card_name)
                    .map_err(|e| format!("transpile {card_name}: {e}"))?;
                let card = self
                    .js
                    .get(script::ScriptSource::Catalog, card_name)
                    .cloned()
                    .ok_or_else(|| {
                        format!("$RS2B0T catalog has no {card_name} card (is $RS2B0T set?)")
                    })?;
                self.script_sel = Some(script::ScriptSel::Loaded(
                    script::ScriptSource::Catalog,
                    card_name.to_string(),
                ));
                let bag = self.pending_settings_bag(
                    script::ScriptSource::Catalog,
                    card_name,
                    &card.settings_schema,
                );
                let siblings = script::resolve_sibling_modules(
                    &card.path,
                    &card.origin,
                    self.js.cache(),
                    script::CacheMeta {
                        kind: card.kind,
                        source: card.source,
                        shape: None,
                        api_family: Some(card.api_family.as_str().into()),
                    },
                )?;
                *self.pending_script.lock().unwrap() = Some(PendingCatalogStart {
                    slot: names[0].clone(),
                    js: card.js.clone(),
                    shape: card.shape,
                    bag,
                    siblings,
                    loadouts: fixture_loadouts,
                    compiled: None,
                });
            }
        }

        Ok(())
    }

    fn map_members(&self) -> bool {
        self.core
            .play()
            .map(|p| p.map_members())
            .or_else(|| self.template.as_ref().map(|t| t.profile().map_members()))
            .unwrap_or(false)
    }

    /// The focused slot's last published [`WorldState`] (inv/equipment/
    /// stats/varps/quests), or the fail-closed empty state when the slot
    /// has not published yet.
    fn focused_walk_state(&self, name: &Option<String>) -> WorldState {
        name.as_deref()
            .and_then(|n| {
                self.snapshots
                    .lock()
                    .unwrap()
                    .get(n)
                    .map(|s| WorldState::from_snapshot(s).with_map_members(self.map_members()))
            })
            .unwrap_or_else(|| WorldState::empty().with_map_members(self.map_members()))
    }

    fn map_context(&self, app: &TuiApp) -> Result<MapContext, ActionError> {
        let name = app.focused_name().ok_or(ActionError::NoFocus)?;
        let focus = self.core.play().and_then(|play| play.map_focus(&name));
        let nav = self
            .server_profile
            .as_ref()
            .and_then(|profile| profile.nav_identity())
            .and_then(|manifest| Digest::from_hex(&manifest.nav_sha256).ok())
            .ok_or(ActionError::NoNavigation)?;
        Ok(MapContext {
            focus,
            nav,
            overlay: app.map_host_catalogue.as_ref().map(|c| c.key()),
            generation: 1,
        })
    }

    fn bind_map_context(&self, app: &mut TuiApp) {
        if !app.map_active {
            app.map_model.close();
            return;
        }
        match self.map_context(app) {
            Ok(context) => {
                app.map_model.bind(context);
            }
            Err(error) => {
                app.map_model.close();
                app.error = Some(format!("map: {error}"));
            }
        }
    }

    fn open_map_catalogue(&mut self, app: &mut TuiApp) {
        let Some(profile) = self.server_profile.clone() else {
            return;
        };
        if self.map_demand.is_none() {
            match self
                .map_bake
                .open_profile(profile.as_ref(), MapDemand::CatalogueOnly)
            {
                Ok(handle) => self.map_demand = Some(handle),
                Err(error) => {
                    app.set_map_unavailable(format!(
                        "coverage: catalogue unavailable: {error}; terrain imagery unavailable"
                    ));
                    return;
                }
            }
        }
        if app.map_catalogue_status == MapCatalogueStatus::Unavailable
            || app.map_catalogue_status == MapCatalogueStatus::Inactive
        {
            app.map_catalogue_status = MapCatalogueStatus::ReadingCache;
            app.map_coverage = "coverage: reading cache".into();
        }
        self.poll_map_demand(app);
    }

    fn poll_map_demand(&mut self, app: &mut TuiApp) {
        if !app.map_active {
            return;
        }
        let Some(profile) = self.server_profile.clone() else {
            return;
        };
        let status = match &self.map_demand {
            Some(handle) => handle.status(),
            None => return,
        };
        match status {
            MapJobStatus::Ready => {
                let ready = self
                    .map_demand
                    .as_ref()
                    .and_then(|handle| map_ready_catalogue(handle).ok());
                if let Some(ready) = ready {
                    self.bind_ready_catalogue(app, ready);
                }
            }
            MapJobStatus::Queued => {
                app.map_catalogue_status = MapCatalogueStatus::ReadingCache;
                app.map_coverage = "coverage: queued".into();
                if let Some(ready) = peek_map_catalogue(profile.as_ref()) {
                    self.bind_ready_catalogue(app, ready);
                }
            }
            MapJobStatus::Running(progress) => {
                app.map_catalogue_status = match progress.stage {
                    MapStage::ReadingCache => MapCatalogueStatus::ReadingCache,
                    _ => MapCatalogueStatus::DerivingPois,
                };
                let stage = match progress.stage {
                    MapStage::ReadingCache => "reading cache",
                    MapStage::DerivingPois => "deriving POIs",
                    MapStage::BakingPlane => "baking plane",
                    MapStage::BuildingZoomLevels => "building zoom levels",
                    MapStage::Publishing => "publishing",
                };
                app.map_coverage = format!(
                    "coverage: {stage} ({}/{}) {}",
                    progress.completed, progress.total, progress.message
                );
                if let Some(ready) = peek_map_catalogue(profile.as_ref()) {
                    self.bind_ready_catalogue(app, ready);
                }
            }
            MapJobStatus::Failed(error) => {
                app.set_map_unavailable(format!(
                    "coverage: catalogue unavailable: {error}; terrain imagery unavailable"
                ));
            }
            MapJobStatus::Paused | MapJobStatus::Cancelled => {
                app.set_map_unavailable(
                    "coverage: catalogue unavailable: demand paused; terrain imagery unavailable",
                );
            }
        }
    }

    fn bind_ready_catalogue(&mut self, app: &mut TuiApp, ready: Arc<ReadyCatalogue>) {
        let Some(world) = app.world.clone().or_else(|| {
            self.core
                .play()
                .and_then(|play| play.world())
                .or_else(|| self.nav_world.lock().unwrap().clone())
        }) else {
            return;
        };
        let identity = ready.manifest().identity;
        let Some(profile) = self.server_profile.as_ref() else {
            return;
        };
        let Some(nav) = profile
            .nav_identity()
            .and_then(|manifest| Digest::from_hex(&manifest.nav_sha256).ok())
        else {
            return;
        };
        let data = profile
            .game_data()
            .or_else(|| self.core.play().and_then(|play| play.game_data()));
        let same = app.map_host_catalogue.as_ref().is_some_and(|catalogue| {
            catalogue.identity() == identity && catalogue.nav_identity() == nav
        });
        if same && (self.map_catalogue_named || data.is_none()) {
            return;
        }
        let services = load_navpois(profile, identity.content, nav);
        let Ok(catalogue) = Catalogue::from_ready(world, identity, nav, Some(ready), services)
        else {
            return;
        };
        self.map_catalogue_named = false;
        let catalogue = match data {
            Some(data) => match catalogue.with_game_data(data) {
                Ok(named) => {
                    self.map_catalogue_named = true;
                    named
                }
                Err(_) => return,
            },
            None => catalogue,
        };
        app.bind_host_catalogue(Arc::new(catalogue));
    }

    fn release_map_catalogue(&mut self) {
        self.map_demand = None;
        self.map_catalogue_named = false;
    }

    /// Map Walk-confirm consumes the shared, revision-bound command before
    /// handing it to `Play`; the panel and TUI therefore share stale-focus,
    /// origin and routing-option checks.
    fn arm_walk_on(&mut self, app: &mut TuiApp, dest: Tile) {
        self.walk_clear.store(false, Ordering::Relaxed);
        #[cfg(test)]
        if self.core.play().is_none() && self.server_profile.is_none() {
            // Headless arm tests have no Play/profile identity to capture.
            self.arm_walk_without_host(app, dest);
            return;
        }
        let _ = dest;
        let context = match self.map_context(app) {
            Ok(context) => context,
            Err(error) => {
                app.error = Some(format!("map: {error}"));
                return;
            }
        };
        let Some(from) = app.here.map(|h| Tile {
            x: h.x,
            z: h.z,
            level: h.level,
        }) else {
            app.error = Some(ActionError::NoOrigin.to_string());
            return;
        };
        let command = match app.map_model.confirm(
            ActionKind::Walk,
            &context,
            Some(from),
            app.nav.find_options(),
        ) {
            Ok(command) => command,
            Err(error) => {
                app.clear_consumed_map_selection();
                app.error = Some(format!("map: {error}"));
                return;
            }
        };
        app.clear_consumed_map_selection();
        let Some(play) = self.core.play() else {
            app.error = Some(ActionError::NoFocus.to_string());
            return;
        };
        let name = app.focused_name();
        let state = self.focused_walk_state(&name);
        let bank = name
            .as_deref()
            .and_then(|n| {
                self.snapshots.lock().unwrap().get(n).map(|snap| {
                    snap.bank()
                        .iter()
                        .map(|it| (it.def.id, it.count))
                        .collect::<Vec<_>>()
                })
            })
            .unwrap_or_default();
        let destination = command.destination();
        match play.map_walk(command, &context, &state, &bank, &self.travellers) {
            Ok(_) => {
                app.walk_dest = Some(destination);
                app.error = None;
            }
            Err(error) => app.error = Some(format!("map: {error}")),
        }
    }

    #[cfg(test)]
    fn arm_walk_without_host(&mut self, app: &mut TuiApp, dest: Tile) {
        let name = app.focused_name();
        let from = app.here.map(|h| Tile {
            x: h.x,
            z: h.z,
            level: h.level,
        });
        let world = self.nav_world.lock().unwrap().clone();
        let (Some(world), Some(from)) = (world, from) else {
            app.error = Some(ActionError::NoOrigin.to_string());
            return;
        };
        let state = self.focused_walk_state(&name);
        let bank = name
            .as_deref()
            .and_then(|n| {
                self.snapshots.lock().unwrap().get(n).map(|snap| {
                    snap.bank()
                        .iter()
                        .map(|it| (it.def.id, it.count))
                        .collect::<Vec<_>>()
                })
            })
            .unwrap_or_default();
        let routed = arm_walk_on(
            &world,
            from,
            dest,
            app.nav.find_options(),
            &state,
            &bank,
            &self.travellers,
            name.as_deref(),
        );
        match routed {
            Ok(_) => {
                app.walk_dest = Some(dest);
                app.error = None;
            }
            Err(_) => {
                app.error = Some(format!("no path to {} {} {}", dest.x, dest.z, dest.level));
            }
        }
    }

    fn map_teleport(&mut self, app: &mut TuiApp, _dest: Tile) {
        let Some(context) = self.map_context(app).ok() else {
            app.error = Some("map: teleport unavailable: stale host context".into());
            return;
        };
        let from = app.here.map(|h| Tile {
            x: h.x,
            z: h.z,
            level: h.level,
        });
        let command = match app.map_model.confirm(
            ActionKind::Teleport,
            &context,
            from,
            app.nav.find_options(),
        ) {
            Ok(command) => command,
            Err(error) => {
                app.clear_consumed_map_selection();
                app.error = Some(format!("map: {error}"));
                return;
            }
        };
        app.clear_consumed_map_selection();
        let Some(play) = self.core.play() else {
            app.error = Some(ActionError::NoFocus.to_string());
            return;
        };
        app.error = play
            .map_teleport(command, &context)
            .err()
            .map(|error| format!("map: {error}"));
    }

    fn map_walk_group(&mut self, app: &mut TuiApp) {
        use host_play::walk_map::WalkSlotOutcomeKind;
        let names: Vec<String> = app
            .walk_send
            .rows()
            .iter()
            .filter(|row| row.checked)
            .map(|row| row.name.clone())
            .collect();
        let context = match self.map_context(app) {
            Ok(context) => context,
            Err(error) => {
                app.error = Some(format!("map: {error}"));
                return;
            }
        };
        let plan = match app
            .map_model
            .confirm_walk_plan(&context, app.nav.find_options())
        {
            Ok(plan) => plan,
            Err(error) => {
                app.clear_consumed_map_selection();
                app.error = Some(format!("map: {error}"));
                return;
            }
        };
        app.clear_consumed_map_selection();
        let Some(play) = self.core.play() else {
            app.error = Some(ActionError::NoFocus.to_string());
            return;
        };
        let states: Vec<_> = names
            .iter()
            .map(|name| self.focused_walk_state(&Some(name.clone())))
            .collect();
        let banks: Vec<Vec<(i32, i32)>> = names
            .iter()
            .map(|name| {
                self.snapshots
                    .lock()
                    .unwrap()
                    .get(name)
                    .map(|snap| {
                        snap.bank()
                            .iter()
                            .map(|it| (it.def.id, it.count))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
            })
            .collect();
        let reqs: Vec<WalkSlotRequest<'_>> = names
            .iter()
            .enumerate()
            .map(|(i, name)| WalkSlotRequest {
                name,
                state: &states[i],
                bank: &banks[i],
            })
            .collect();
        let report = play.map_walk_group(plan, &context, &reqs, &self.travellers);
        {
            let mut latch = self.tick_latch.lock().unwrap();
            for outcome in &report.outcomes {
                if matches!(outcome.kind, WalkSlotOutcomeKind::Walking) {
                    latch.remove(&outcome.name);
                }
            }
        }
        self.walk_clear.store(false, Ordering::Relaxed);
        if report
            .outcomes
            .iter()
            .any(|outcome| matches!(outcome.kind, WalkSlotOutcomeKind::Walking))
        {
            app.walk_dest = Some(plan.destination());
        }
        app.error = Some(report.summary());
    }

    /// WASD one-tile walk: a direct `try_move` through the slot's wire
    /// queue (adjacent world tile; the client pathfinds the step).
    fn wasd_walk(&self, app: &TuiApp, dest: Tile) {
        let Some(name) = app.focused_name() else {
            return;
        };
        if let Some(play) = self.core.play() {
            play.queue_wire(
                &name,
                WireCmd::Walk {
                    x: dest.x,
                    z: dest.z,
                    level: dest.level,
                },
            );
        }
    }

    /// Chat modal advance: queue `continue_dialog` / `answer_choice` on
    /// the focused slot (the guardian's hold still lets chat through).
    fn chat_send(&self, app: &TuiApp, action: ChatAction) {
        let Some(name) = app.focused_name() else {
            return;
        };
        if let Some(play) = self.core.play() {
            match action {
                ChatAction::Continue => play.queue_wire(&name, WireCmd::Continue),
                ChatAction::Answer(option) => {
                    play.queue_wire(&name, WireCmd::Answer(option as i32))
                }
                ChatAction::PaintButton(index) => {
                    if let Some(paint) = app.chat_data.script_paint.as_ref() {
                        if let Some(id) = paint.buttons.get(index).map(|b| b.id.clone()) {
                            play.script_paint_click(&name, &id, paint.generation);
                        }
                    }
                }
                ChatAction::PaintChrome(index) => {
                    if let Some(paint) = app.chat_data.script_paint.as_ref() {
                        let rows = super::chat::paint_chrome_rows(paint);
                        if let Some((key, select_name, _)) = rows.get(index) {
                            play.script_paint_select(&name, key, select_name, paint.generation);
                        }
                    }
                }
                ChatAction::None => {}
            }
        }
    }

    /// Fill the JS library's cards from the `$RS2B0T` registry once when
    /// the env/persisted root is already known (live boot). Errors are debug-only.
    fn fill_rs2b0t_cards_once(&mut self) {
        if self.rs2b0t_filled {
            return;
        }
        self.rs2b0t_filled = true;
        if let Some(root) = self.catalog_root() {
            if let Err(e) = self
                .js
                .register_rs2b0t(&root, &script::default_rs2b0t_path_file())
            {
                if std::env::var("BOT_DEBUG").is_ok() {
                    eprintln!("[tui-play] $RS2B0T registry: {e}");
                }
            }
        }
    }

    /// Opening Browse: fill from `$RS2B0T`/persisted root, or prompt for a
    /// clone root, or honour a prior defer (panel parity).
    fn on_script_browse_open(&mut self, app: &mut TuiApp) {
        if self.rs2b0t_filled {
            return;
        }
        self.rs2b0t_filled = true;
        if let Some(root) = self.catalog_root() {
            if let Err(e) = self
                .js
                .register_rs2b0t(&root, &script::default_rs2b0t_path_file())
            {
                if std::env::var("BOT_DEBUG").is_ok() {
                    eprintln!("[tui-play] $RS2B0T registry: {e}");
                }
            }
            return;
        }
        if script::rs2b0t_import_deferred_at(&script::default_rs2b0t_import_file()) {
            return;
        }
        self.rs2b0t_catalog_open = true;
        self.rs2b0t_catalog_dir = Self::default_catalog_browse_dir();
        app.rs2b0t_catalog_open = true;
        app.rs2b0t_catalog_dir = self.rs2b0t_catalog_dir.clone();
        app.catalog_sel = 0;
    }

    fn defer_rs2b0t_catalog(&mut self, app: &mut TuiApp) {
        let _ = script::set_rs2b0t_import_deferred_at(&script::default_rs2b0t_import_file());
        self.rs2b0t_catalog_open = false;
        app.rs2b0t_catalog_open = false;
    }

    fn import_rs2b0t_catalog(&mut self, app: &mut TuiApp, root: &Path) -> Result<usize, String> {
        if !rs2b0t_root_has_index(root) {
            return Err(format!(
                "no catalog at {}",
                script::registry_index_path(root).display()
            ));
        }
        let n = self
            .js
            .register_rs2b0t(root, &script::default_rs2b0t_path_file())?;
        let _ = script::clear_rs2b0t_import_at(&script::default_rs2b0t_import_file());
        self.rs2b0t_catalog_open = false;
        app.rs2b0t_catalog_open = false;
        Ok(n)
    }

    /// Start the Browse-selected card on the focused slot.
    fn script_start(&mut self, app: &mut TuiApp, sel: &script::ScriptSel) {
        let Some(name) = app.focused_name() else {
            app.error = Some("script: no focused profile".into());
            return;
        };
        let result = match self.core.play() {
            Some(_) => match sel {
                script::ScriptSel::Loaded(source, card_name) => {
                    match self.js.get(*source, card_name) {
                        Some(card) if card.unloadable.is_some() => Err(format!(
                            "unloadable import: {}",
                            card.unloadable.as_deref().unwrap_or("")
                        )),
                        Some(_) => match self.js.ensure_js(*source, card_name) {
                            Err(e) => Err(e),
                            Ok(()) => match self.js.get(*source, card_name).cloned() {
                                Some(card) => {
                                    let bag = self.pending_settings_bag(
                                        *source,
                                        card_name,
                                        &card.settings_schema,
                                    );
                                    match script::resolve_sibling_modules(
                                        &card.path,
                                        &card.origin,
                                        self.js.cache(),
                                        script::CacheMeta {
                                            kind: card.kind,
                                            source: card.source,
                                            shape: None,
                                            api_family: Some(card.api_family.as_str().into()),
                                        },
                                    ) {
                                        Ok(siblings) => match self.core.start_script(
                                            &name,
                                            ScriptStart::Load {
                                                js: card.js.clone(),
                                                shape: card.shape,
                                                bag,
                                                siblings,
                                            },
                                            None,
                                        ) {
                                            Ok(_) => {
                                                self.pending_starts.insert(name.clone(), card);
                                                Ok(())
                                            }
                                            Err(e) => self.js.record_start_result(&card, Err(e)),
                                        },
                                        Err(e) => Err(e),
                                    }
                                }
                                None => Err(format!("no loaded script: {card_name}")),
                            },
                        },
                        None => Err(format!("no loaded script: {card_name}")),
                    }
                }
                script::ScriptSel::Compiled(id) => self
                    .core
                    .start_script(&name, ScriptStart::Compiled(*id), None)
                    .map(|_| ())
                    .map_err(|e| e.to_string()),
            },
            None => Err("no play".to_string()),
        };
        app.error = match result {
            Ok(()) => {
                if self.js.load_failures().is_empty() {
                    None
                } else {
                    Some(self.js.named_failure_output())
                }
            }
            Err(e) => Some(format!("script: {e}")),
        };
    }

    /// Record or clear the load diagnostic of every Start whose isolate
    /// setup has settled, and show the outcome the way a synchronous Start
    /// did: a failure is `script: <diagnostic>`, success the remaining
    /// failure list (or nothing). Called once per pump.
    fn settle_script_starts(&mut self, app: &mut TuiApp) {
        let settled = self.core.take_settled_starts();
        for frontend_core::StartSettled {
            slot: name,
            outcome,
            ..
        } in settled
        {
            let Some(card) = self.pending_starts.remove(&name) else {
                continue;
            };
            match outcome {
                Some(script::StartOutcome::Ready) => {
                    let _ = self.js.record_start_result(&card, Ok(()));
                    app.error = if self.js.load_failures().is_empty() {
                        None
                    } else {
                        Some(self.js.named_failure_output())
                    };
                }
                Some(script::StartOutcome::Failed(e)) => {
                    let diagnostic = self
                        .js
                        .record_start_result(
                            &card,
                            Err(script::StartLoadError::RuntimeLoad(e.clone())),
                        )
                        .err()
                        .unwrap_or(e);
                    app.error = Some(format!("script: {diagnostic}"));
                }
                Some(script::StartOutcome::Cancelled) | None => {}
            }
        }
    }

    /// Pause or resume the focused slot's script (toggle like the panel).
    fn script_toggle_pause(&mut self, app: &mut TuiApp) {
        if let Some(name) = app.focused_name() {
            self.core.toggle_pause(&name);
        }
    }

    /// Stop the focused slot's script.
    fn script_stop(&mut self, app: &mut TuiApp) {
        if let Some(name) = app.focused_name() {
            self.core.stop_script(&name);
        }
    }

    /// Load a local JS bot file into the library, select it for Start,
    /// and persist the store. Errors land on the strip.
    fn script_load(&mut self, app: &mut TuiApp, path: &Path) {
        match self.js.load(path) {
            Ok(card) => {
                app.script_sel = Some(script::ScriptSel::Loaded(card.source, card.name));
                app.error = if self.js.load_failures().is_empty() {
                    None
                } else {
                    Some(self.js.named_failure_output())
                };
                if let Some(parent) = path.parent() {
                    self.script_load_last_dir = Some(parent.to_path_buf());
                    app.script_load_last_dir = self.script_load_last_dir.clone();
                }
            }
            Err(e) => {
                app.error = if self.js.load_failures().len() > 1 {
                    Some(self.js.named_failure_output())
                } else {
                    Some(format!("script: {e}"))
                };
            }
        }
    }

    /// Persist the settings popup's changes onto the focused vault
    /// profile (the operator vault; `--live`'s temp vault is ephemeral)
    /// and mirror guardian settings onto a running slot's arm.
    fn persist_settings(&mut self, app: &mut TuiApp) {
        let Some(name) = app.focused_name() else {
            return;
        };
        // Field edit, not a whole-settings replacement: the popup owns only
        // the guardian fields, and the arm changes only after the vault
        // write succeeded.
        let settings = &app.settings;
        if let Err(e) = self.core.set_random_settings(
            &name,
            settings.random_events,
            &settings.lamp_skill,
            settings.lamp_auto,
        ) {
            app.error = Some(format!("settings: {e}"));
        }
    }

    /// The remembered terrain-bake choice, shared with the panel through
    /// `panel-ui.json`.
    fn persist_map_bake(&mut self, app: &mut TuiApp) {
        self.map_bake.set_choice(app.map_bake);
        if !self.persist_ui {
            return;
        }
        if let Err(e) = persist_map_bake_choice(app.map_bake) {
            app.error = Some(format!("settings: map bake: {e}"));
        }
    }

    /// Copy the focused slot's views into the app and poll the runner.
    fn pump(&mut self, app: &mut TuiApp) {
        #[cfg(feature = "memory-profile")]
        if let Some(run) = self.memory.as_mut() {
            app.focused = Some(run.focus_index());
            self.core.select(&run.names[run.focus_index()]);
            if let Some(play) = self.core.play_mut() {
                match run.poll(play) {
                    Ok(true) => {
                        restore_terminal();
                        eprintln!("PASS: memory tui observation complete");
                        std::process::exit(0);
                    }
                    Ok(false) => {}
                    Err(error) => {
                        restore_terminal();
                        eprintln!("FAIL: memory tui: {error}");
                        std::process::exit(1);
                    }
                }
            }
        }

        // Start/Stop return before the isolate is up or reaped. A slot that
        // is offline or queued for login has no observe of its own, so the
        // core resolves every slot (and advances removals), then the TUI
        // commits the Starts that settled.
        self.core.poll();
        self.settle_script_starts(app);

        // The strip is the fleet: a removed member leaves it at once (its
        // worker may still be logging out). Both copies reuse the app's
        // buffers, so a steady pump allocates nothing here.
        let members = self.core.members();
        if app.names.as_slice() != members {
            app.names.truncate(members.len());
            let kept = app.names.len();
            app.names.clone_from_slice(&members[..kept]);
            app.names.extend_from_slice(&members[kept..]);
        }
        app.focused = self
            .core
            .selected()
            .and_then(|selected| app.names.iter().position(|n| n == selected));
        self.core.copy_statuses_into(&mut app.statuses);
        for failure in self.core.take_write_failures() {
            app.error = Some(failure);
        }
        let now = Instant::now();
        let sampled = self.resource_sampler.due(now);
        if sampled {
            let focused = app.focused_name();
            match self.core.play() {
                Some(play) => self
                    .resource_sampler
                    .sample_play(now, play, focused.as_deref()),
                None => self
                    .resource_sampler
                    .sample(now, focused.as_deref(), std::iter::empty()),
            }
            app.resources.clone_from(self.resource_sampler.view());
        }
        self.refresh_background_notice(app, now);
        // The script pane's Browse picker lists library cards with registry fields.
        app.script_cards = self.js.cards().iter().map(BrowseCard::from).collect();
        let present = categories_present(&app.script_cards);
        let order = resolve_category_order(&self.script_category_order, &present);
        if order != self.script_category_order {
            self.script_category_order = order.clone();
        }
        app.script_category_order = self.script_category_order.clone();
        app.params_schema = match &app.script_sel {
            Some(script::ScriptSel::Loaded(source, name)) => self
                .js
                .get(*source, name)
                .map(|c| c.settings_schema.clone())
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        if app.params_state.open && app.params_schema.is_empty() {
            app.params_state.open = false;
        }
        app.rs2b0t_catalog_open = self.rs2b0t_catalog_open;
        app.script_load_last_dir = self.script_load_last_dir.clone();
        if !app.rs2b0t_catalog_open {
            app.rs2b0t_catalog_dir = self.rs2b0t_catalog_dir.clone();
        } else {
            self.rs2b0t_catalog_dir.clone_from(&app.rs2b0t_catalog_dir);
        }
        // The map paints `Play`'s packed collision, not the live loc
        // snapshot. Copy the session world each pump (an `Arc` clone)
        // so a loaded pack is not stuck behind the empty-state title.
        app.world = self.nav_world.lock().unwrap().clone();
        app.refresh();
        self.bind_map_context(app);
        if app.map_active {
            self.poll_map_demand(app);
            app.refresh_walk_send(|name| {
                self.core
                    .play()
                    .map(|p| p.walk_eligibility(name))
                    .unwrap_or(WalkSlotStatus::Excluded(WalkExclude::NotLoggedIn))
            });
        }

        // The settings popup edits the focused profile: reload when the
        // focus changes (a fresh focus must not show the old slot's
        // random toggle).
        let focused = app.focused_name();
        if self.last_focused.as_deref() != focused.as_deref() {
            self.last_focused = focused.clone();
            app.settings = focused
                .as_deref()
                .and_then(|n| self.core.vault().and_then(|v| v.get(n)))
                .map(|p| p.settings.clone())
                .unwrap_or_default();
            app.settings_state.open = false;
        }

        if let Some(name) = &focused {
            // The paint-as-chat toggle is operator state: rebuild the
            // pane data without losing it across pumps.
            let show_game_chat = app.chat_data.show_game_chat;
            // Borrow the focused slot's snapshot under the lock and copy
            // the small views the panes need (GameSnapshot is not Clone).
            let snap = self.snapshots.lock().unwrap();
            match snap.get(name) {
                Some(s) => {
                    app.chat_data = chat_data_from(s);
                    app.chat_data.show_game_chat = show_game_chat;
                    app.inv_items = s
                        .inventory()
                        .iter()
                        .map(|i| (i.def.name.clone().unwrap_or_else(|| "?".into()), i.count))
                        .collect();
                    app.stats_rows = s
                        .stats()
                        .iter()
                        .filter(|st| st.used)
                        .map(|st| (st.name.clone(), st.effective))
                        .collect();
                    let here = app.here;
                    app.locs_near = s
                        .locs()
                        .iter()
                        .filter_map(|l| {
                            let name = l.name.as_deref()?.to_string();
                            let d = here
                                .map(|h| (l.tile.x - h.x).abs().max((l.tile.z - h.z).abs()))
                                .unwrap_or(l.distance);
                            Some((d, name))
                        })
                        .collect();
                    app.locs_near.sort_by_key(|(d, _)| *d);
                    app.locs_near.truncate(3);
                    if app.map_active {
                        if let Ok(ctx) = self.map_context(app) {
                            match observed_services(s, ctx, ctx) {
                                Ok(records) => app.map_observed = records,
                                Err(_) => app.map_observed.clear(),
                            }
                        } else {
                            app.map_observed.clear();
                        }
                    }
                }
                None => {
                    // A slot with no published snapshot must not show the
                    // previous focused slot's chat / inventory.
                    app.chat_data = ChatData::default();
                    app.chat_data.show_game_chat = show_game_chat;
                    app.inv_items.clear();
                    app.stats_rows.clear();
                    app.locs_near.clear();
                    app.map_observed.clear();
                }
            }
            drop(snap);
            // The script's paint frame rides the status row (copied from
            // the isolate each observe); the chat pane shows it in place
            // of the game chat while it is non-empty.
            app.chat_data.script_paint =
                app.focused_status().and_then(|st| st.script_paint.clone());
            // Stop drops the isolate (and its paint with it); reset the
            // operator's game-chat toggle when no paint is showing so a
            // fresh Start shows the new paint by default instead of
            // hiding it until `p` again.
            if app.chat_data.script_paint.is_none() {
                app.chat_data.show_game_chat = false;
            }
            app.route = self
                .travellers
                .lock()
                .unwrap()
                .get(name)
                .and_then(|a| a.lock().unwrap().route.clone());
            if self.walk_clear.swap(false, Ordering::Relaxed) {
                app.walk_dest = None;
            }
            if let Some(play) = self.core.play() {
                app.script_state = play.script_state(name);
            }
        } else if app.map_active {
            app.map_observed.clear();
        }
        if app.settings_dirty {
            self.persist_settings(app);
            app.settings_dirty = false;
        }
        if app.map_bake_dirty {
            self.persist_map_bake(app);
            app.map_bake_dirty = false;
        }
    }

    fn script_self_stop_observed(&self, needle: &str) -> bool {
        let Some(name) = self.names.first() else {
            return false;
        };
        let Some(play) = self.core.play() else {
            return false;
        };
        matches!(play.script_state(name), script::RunState::Idle)
            && play
                .script_lifecycle_receipt(name)
                .is_some_and(|receipt| receipt.reason.contains(needle))
    }

    /// The `--live` terminal state: `Some(exit code)` when the runner
    /// passed (0) or failed (1); `None` while it runs. Proof lines are
    /// returned, not printed, so a headed loop can hold them until after
    /// alternate-screen restore.
    fn live_status(&mut self) -> (Option<i32>, Vec<ProofLine>) {
        let name = self.live_name.as_deref().unwrap_or("script");
        if let Some(message) = self.terminal_startup_failure() {
            return (
                Some(1),
                vec![
                    ProofLine::Stderr(format!("FAIL: live {name} startup: {message}")),
                    ProofLine::Stderr(format!("FAIL: {message}")),
                ],
            );
        }
        let status = self.scenario.lock().unwrap().as_ref().map(|r| r.status());
        if let (Some(scenario::RunnerStatus::Passed), Some(needle)) =
            (&status, self.live_wait_script_stop)
        {
            if !self.script_self_stop_observed(needle) {
                let started = self.live_stop_wait_started.get_or_insert_with(Instant::now);
                if started.elapsed() >= Duration::from_secs(45) {
                    return (
                        Some(1),
                        vec![
                            ProofLine::Stderr(format!(
                                "FAIL: live {name} timed out waiting for script Idle and clean stop reason {needle:?}"
                            )),
                            ProofLine::Stderr(format!(
                                "FAIL: timed out waiting for script Idle and clean stop reason {needle:?}"
                            )),
                        ],
                    );
                }
                return (None, Vec::new());
            }
        }
        let evidence = self
            .scenario
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|r| r.evidence().cloned())
            .map(|ev| ev.to_json())
            .unwrap_or_default();
        let soaking = self.live_soak_until.is_some_and(|t| Instant::now() < t);
        let (code, lines, announced) =
            live_proof(name, status, &evidence, soaking, self.live_announced_pass);
        self.live_announced_pass = announced;
        let mut lines = lines;
        if code == Some(1) && std::env::var_os("BOT_DEBUG").is_some() {
            if let (Some(slot), Some(play)) = (self.names.first(), self.core.play()) {
                if let Some(receipt) = play.script_lifecycle_receipt(slot) {
                    lines.push(ProofLine::Stderr(format!(
                        "[tui-play] script lifecycle slot={slot:?} generation={} state={:?} tick={} reason={:?}",
                        receipt.runtime_generation, receipt.state, receipt.tick, receipt.reason
                    )));
                }
            }
        }
        (code, lines)
    }

    /// Return a producer-marked terminal asset-init failure for a slot owned
    /// by this scenario. Retryable login errors intentionally remain pending.
    fn terminal_startup_failure(&self) -> Option<String> {
        let runner = self.scenario.lock().unwrap();
        let runner = runner.as_ref()?;
        let owned = runner.owned_profile_names();
        let statuses = self.core.play()?.statuses();
        host_play::owned_terminal_startup_error(&statuses, &owned)
    }

    fn ack_background_bots(&mut self, app: &mut TuiApp) {
        if self.persist_ui {
            match persist_background_bots_ack() {
                Ok(()) => {
                    self.background_bots_acked = true;
                    self.notice_sig = None;
                    app.background_notice = None;
                    clear_background_bots_ack_error(&mut app.error);
                }
                Err(e) => app.error = Some(background_bots_ack_error(&e)),
            }
        } else {
            self.notice_sig = None;
            app.background_notice = None;
        }
    }

    fn refresh_ack_cache(&mut self, now: Instant) {
        if self
            .ack_checked_at
            .is_none_or(|t| now.duration_since(t) >= Duration::from_secs(1))
        {
            self.background_bots_acked = background_bots_acked();
            self.ack_checked_at = Some(now);
        }
    }

    fn refresh_background_notice(&mut self, app: &mut TuiApp, now: Instant) {
        if !self.persist_ui {
            self.notice_sig = None;
            app.background_notice = None;
            return;
        }
        self.refresh_ack_cache(now);
        let focused = app.focused_name();
        let background = self
            .core
            .play()
            .map(|play| play.background_bot_count(focused.as_deref()))
            .unwrap_or(0);
        if self.background_bots_acked || background == 0 {
            self.notice_sig = None;
            app.background_notice = None;
            return;
        }
        let view = self.resource_sampler.view();
        match &self.notice_sig {
            Some((n, v)) if *n == background && v == view => {}
            _ => {
                app.background_notice = Some(background_ack_text(background, view));
                self.notice_sig = Some((background, view.clone()));
            }
        }
    }
}

/// One machine-readable `--live` proof line. PASS stays on stdout for
/// harness scrapers; FAIL stays on stderr and still exits 1.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ProofLine {
    Stdout(String),
    Stderr(String),
}

impl ProofLine {
    fn write(&self) {
        match self {
            Self::Stdout(line) => println!("{line}"),
            Self::Stderr(line) => eprintln!("{line}"),
        }
    }
}

fn write_proof_lines(lines: &[ProofLine]) {
    for line in lines {
        line.write();
    }
}

/// Proof text and exit for one `--live` poll. `announced_pass` is the
/// caller's latch so a `BUDGET_S` soak does not reprint PASS.
fn live_proof(
    name: &str,
    status: Option<scenario::RunnerStatus>,
    evidence: &str,
    soaking: bool,
    announced_pass: bool,
) -> (Option<i32>, Vec<ProofLine>, bool) {
    match status {
        Some(scenario::RunnerStatus::Passed) => {
            let mut lines = Vec::new();
            let announced = if announced_pass {
                true
            } else {
                lines.push(ProofLine::Stdout(format!("PASS: live {name} {evidence}")));
                true
            };
            if soaking {
                (None, lines, announced)
            } else {
                (Some(0), lines, announced)
            }
        }
        Some(scenario::RunnerStatus::Failed(msg)) => (
            Some(1),
            vec![
                ProofLine::Stderr(format!("FAIL: live {name} {evidence}")),
                ProofLine::Stderr(format!("FAIL: {msg}")),
            ],
            announced_pass,
        ),
        _ => (None, Vec::new(), announced_pass),
    }
}

/// The chat pane's owned data, copied from the focused snapshot each pump.
fn chat_data_from(s: &api::snapshot::GameSnapshot) -> ChatData {
    ChatData {
        lines: s.chat_lines().to_vec(),
        modal_texts: s.chat_modal_texts().to_vec(),
        options: s.chat_options().to_vec(),
        has_continue: s.chat_continue_component_id() != -1,
        // The paint rides the status row, not the snapshot; the pump
        // patches it in, and the toggle is operator state.
        script_paint: None,
        show_game_chat: false,
    }
}

fn prompt_instance_conflict(holder: &host_play::InstanceHolder) -> bool {
    eprintln!("{}", host_play::instance_conflict_message(holder));
    eprint!("Exit (default) or Continue anyway [E/c]: ");
    let _ = std::io::Write::flush(&mut std::io::stderr());
    let mut line = String::new();
    let _ = std::io::stdin().read_line(&mut line);
    matches!(
        line.trim(),
        "c" | "C" | "continue" | "Continue" | "Continue anyway"
    )
}

/// Run the interactive (or `--live`) TUI: unlock, load + log in, event loop.
fn run(args: &Args, mode: RunMode) -> Result<i32, String> {
    let memory = {
        #[cfg(feature = "memory-profile")]
        {
            host_play::memory::Config::from_env()
                .ok()
                .flatten()
                .is_some()
        }
        #[cfg(not(feature = "memory-profile"))]
        {
            false
        }
    };
    let skip_lock = !matches!(mode, RunMode::Interactive) || memory;
    let permit = match host_play::resolve_instance_permit(host_play::InstanceKind::Tui, skip_lock) {
        Ok(host_play::InstancePermitOutcome::Ready(permit)) => permit,
        Ok(host_play::InstancePermitOutcome::NeedsConfirm(holder)) => {
            if !prompt_instance_conflict(&holder) {
                return Ok(0);
            }
            host_play::InstancePermit::skip()
        }
        Err(e) => return Err(format!("instance lock: {e}")),
    };
    if frontend_core::log_file::session_log_setting() {
        frontend_core::log_file::apply_session_log(true);
    }
    let selection = args.profile.resolve(None)?;
    if let Some(number) = args.world {
        let worlds = selection
            .public_worlds()
            .ok_or("--world requires public-289")?;
        if worlds.by_number(number).is_none() {
            return Err(format!(
                "world {number} is not in the configured public worlds"
            ));
        }
    }
    let template = selection.prepare_template()?;
    let mut session = TuiSession::new_bound(template, permit);
    session.auto_world = args.world;
    session
        .server_profile
        .as_ref()
        .expect("bound session")
        .require_bot_operation()?;

    #[cfg(feature = "memory-profile")]
    if let Some(config) = host_play::memory::Config::from_env()? {
        host_play::memory::require_live_benchmark()?;
        session.persist_ui = false;
        // Vault/card first; unlock constructs Play once (single load_pack).
        let run = host_play::memory::Run::prepare_unseeded(config, "tui")?;
        session.options.mainland = true;
        session.unlock_at(&run.vault, &run.pass)?;
        run.bind_seed_nav(host_play::memory::SeedNav::FromPlay(
            session.core.play().and_then(|p| p.world()),
        ))?;
        session.names = run.names.clone();
        session.load_and_login_all();
        session.focus(&run.names[0]);
        let mut app = TuiApp::new(format!(
            "{} memory benchmark · {}",
            session.app_title(),
            session.profile_label()
        ));
        app.names = run.names.clone();
        app.focused = Some(0);
        session.memory = Some(run);
        return run_loop(session, app);
    }

    match mode {
        RunMode::Live(name) => {
            let scenario = live_scenario(&name)?;
            session.options.mainland = scenario.seed.mainland;
            session.live_prepare_script(scenario)?;
            let mut app = TuiApp::new(format!(
                "tui-play --live {name} · {}",
                session.profile_label()
            ));
            app.names = session.names.clone();
            app.focused = Some(0);
            run_loop(session, app)
        }
        RunMode::Interactive => {
            let Some(pass) = args.pass.clone() else {
                return Err("no vault passphrase (set BOT_VAULT_PASS or --vault-pass)".into());
            };
            let vault_path = session
                .server_profile
                .as_ref()
                .expect("bound session")
                .vault_path()
                .to_path_buf();
            let vault_exists = vault_path.is_file();
            if let Err(e) = session.unlock_at(&vault_path, &pass) {
                return Err(format!("vault {}: {e}", vault_path.display()));
            }
            if !vault_exists {
                // First run: create the default `test`/`test` profile so
                // unlock is not a dead end (host-play CLI convention).
                session.create_profile("test")?;
            }
            // `--user` names may not exist: create them like host-play
            // does (`password = username`, fresh uid).
            for u in &args.users {
                if session.core.vault().is_none_or(|v| v.get(u).is_none()) {
                    session.create_profile(u)?;
                }
            }
            session.names = session
                .core
                .vault()
                .map(|v| v.profiles().map(|p| p.username.clone()).collect())
                .unwrap_or_default();
            let focus = args
                .users
                .first()
                .cloned()
                .or_else(|| session.names.first().cloned());
            let Some(focus) = focus else {
                return Err("vault has no profiles (create one with host-play --user)".into());
            };
            session.load_and_login(&focus);
            session.focus(&focus);
            let mut app = TuiApp::new(format!(
                "{} headless · {}",
                session.app_title(),
                session.profile_label()
            ));
            app.names = session.names.clone();
            app.focused = session.names.iter().position(|n| n == &focus);
            run_loop(session, app)
        }
    }
}

impl TuiSession {
    /// Create a missing profile (host-play CLI convention: password =
    /// username, uid one past the vault's max, from the 274M base).
    fn create_profile(&mut self, username: &str) -> Result<(), String> {
        let uid = self
            .core
            .vault()
            .map(|v| v.profiles().map(|p| p.uid).max().unwrap_or(274_000_000) + 1)
            .unwrap_or(274_000_001);
        let profile = Profile {
            username: username.into(),
            password: profile_password_for(username, self.target()),
            uid,
            settings: vault::ProfileSettings::default(),
        };
        // Startup only (before the terminal loop): waiting on the write
        // here keeps a failed first-run profile a startup error.
        let op = self
            .core
            .save_profile(profile, frontend_core::ArmMirror::None, "profile")?;
        self.core.flush_writes();
        match self.core.failure(op) {
            Some(error) => Err(format!("profile: {error}")),
            None => Ok(()),
        }
    }
}

/// Raw mode was enabled for this process. `restore_terminal` clears it.
static RAW_MODE: AtomicBool = AtomicBool::new(false);
/// Alternate screen + mouse capture were entered. Cleared on restore.
static ALT_SCREEN: AtomicBool = AtomicBool::new(false);

/// Leave the headed TUI (raw mode / alt screen) if this process entered it.
/// Idempotent. `process::exit` skips Drop, so memory-profile paths call this
/// before printing PASS/FAIL.
fn restore_terminal() {
    if RAW_MODE.swap(false, Ordering::SeqCst) {
        disable_raw_mode().ok();
    }
    if ALT_SCREEN.swap(false, Ordering::SeqCst) {
        let mut out = std::io::stdout();
        let _ = crossterm::execute!(
            out,
            crossterm::event::DisableMouseCapture,
            LeaveAlternateScreen
        );
        let _ = out.flush();
    }
    crate::stderr_capture::restore();
}

pub(super) fn install_tui_panic_hook() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let main = std::thread::current().id();
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if std::thread::current().id() == main {
                restore_terminal();
            }
            previous(info);
        }));
    });
}

struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore_terminal();
    }
}

/// The crossterm event loop. `--live` runs headed when a controlling
/// terminal is available (the operator watches the panes) and degrades to
/// a headless pump loop otherwise, so the PASS/FAIL still lands in CI.
/// Headless prints proof immediately. Headed holds it until after restore
/// so the PASS JSON line cannot paint into Ratatui rows.
fn run_loop(mut session: TuiSession, mut app: TuiApp) -> Result<i32, String> {
    app.map_bake = session.map_bake.choice();
    if enable_raw_mode().is_err() {
        // No controlling terminal: pump the runner without drawing.
        loop {
            session.pump(&mut app);
            let (code, lines) = session.live_status();
            write_proof_lines(&lines);
            if let Some(code) = code {
                return Ok(code);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    RAW_MODE.store(true, Ordering::SeqCst);
    let _guard = TerminalGuard;
    let mut stdout = std::io::stdout();
    crossterm::execute!(
        stdout,
        EnterAlternateScreen,
        crossterm::event::EnableMouseCapture
    )
    .map_err(|e| format!("terminal setup: {e}"))?;
    ALT_SCREEN.store(true, Ordering::SeqCst);
    install_tui_panic_hook();
    crate::stderr_capture::capture();
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).map_err(|e| e.to_string())?;

    let mut deferred = Vec::new();
    let result = (|| loop {
        session.pump(&mut app);
        let (code, lines) = session.live_status();
        deferred.extend(lines);
        if let Some(code) = code {
            return Ok(code);
        }
        {
            let _profile_frame = client::profiling::UI_FRAME.start();
            let params_data = session.template.as_ref().and_then(|t| t.game_data());
            terminal
                .draw(|frame| {
                    let _profile_draw = client::profiling::UI_DRAW.start();
                    app.draw(frame);
                    app.draw_loadouts_overlay(frame, &mut session.loadouts);
                    app.draw_params_overlay(
                        frame,
                        &mut session.script_settings,
                        &session.loadouts,
                        params_data.as_deref(),
                    );
                })
                .map_err(|e| e.to_string())?;
        }
        if event::poll(Duration::from_millis(50)).map_err(|e| e.to_string())? {
            match event::read().map_err(|e| e.to_string())? {
                Event::Key(k) if k.kind == KeyEventKind::Press => {
                    if app.params_state.open {
                        let params_data = session.template.as_ref().and_then(|t| t.game_data());
                        app.params_on_key(
                            &mut session.script_settings,
                            &session.loadouts,
                            params_data.as_deref(),
                            k,
                        );
                    } else if app.loadouts_on_key(&mut session.loadouts, k) {
                    } else {
                        let action = app.on_key(k);
                        dispatch(&mut session, &mut app, action);
                    }
                }
                Event::Mouse(m) => {
                    if let MouseEventKind::Down(_) = m.kind {
                        let action = app.on_click(m.column, m.row);
                        dispatch(&mut session, &mut app, action);
                    }
                }
                _ => {}
            }
        }
        if app.quit {
            return Ok(0);
        }
    })();

    drop(terminal);
    restore_terminal();
    write_proof_lines(&deferred);
    result
}

/// Route one [`AppAction`] onto the session: map walks arm through
/// `host_play::arm_walk_on`, chat and WASD go through [`WireCmd`], the
/// script actions dispatch `Play::script_start_load` / pause / stop /
/// the JS library, and the settings popup persists on the next pump.
fn dispatch(session: &mut TuiSession, app: &mut TuiApp, action: AppAction) {
    match action {
        AppAction::Quit => app.quit = true,
        AppAction::Focus(name) => session.focus(&name),
        AppAction::MapOpen => {
            session.bind_map_context(app);
            session.open_map_catalogue(app);
        }
        AppAction::MapClose => {
            session.release_map_catalogue();
            app.map_model.close();
        }
        AppAction::ArmWalk(tile) => session.arm_walk_on(app, tile),
        AppAction::MapWalkGroup => session.map_walk_group(app),
        AppAction::WalkTile(tile) => session.wasd_walk(app, tile),
        AppAction::MapTeleport(tile) => session.map_teleport(app, tile),
        AppAction::Chat(action) => session.chat_send(app, action),
        AppAction::SpawnAll => multibox_key(session, app),
        AppAction::Login => session.login(app),
        AppAction::Logout => session.logout(app),
        AppAction::LogoutAll => session.logout_all(),
        AppAction::Remove => session.remove(app),
        AppAction::ScriptStart(sel) => session.script_start(app, &sel),
        AppAction::ScriptPause => session.script_toggle_pause(app),
        AppAction::ScriptStop => session.script_stop(app),
        AppAction::ScriptBrowse => {
            if app.script_browse_open {
                session.on_script_browse_open(app);
            }
        }
        AppAction::ScriptImportCatalog => {
            session.rs2b0t_catalog_open = true;
            session.rs2b0t_catalog_dir = app.rs2b0t_catalog_dir.clone();
            app.rs2b0t_catalog_open = true;
            app.catalog_sel = 0;
        }
        AppAction::ScriptDeferCatalog => session.defer_rs2b0t_catalog(app),
        AppAction::ScriptUseCatalog => {
            let root = app.rs2b0t_catalog_dir.clone();
            session.rs2b0t_catalog_dir = root.clone();
            app.error = match session.import_rs2b0t_catalog(app, &root) {
                Ok(_) => {
                    if session.js.load_failures().is_empty() {
                        None
                    } else {
                        Some(session.js.named_failure_output())
                    }
                }
                Err(e) => Some(e),
            };
        }
        AppAction::ScriptLoad(path) => session.script_load(app, &path),
        AppAction::ScriptParams => app.open_script_params(&session.script_settings),
        AppAction::AckBackground => session.ack_background_bots(app),
        AppAction::None => {}
    }
}

/// The `m` key loads every profile and logs every member in.
fn multibox_key(session: &mut TuiSession, app: &mut TuiApp) {
    let loaded = session.load_and_login_all();
    app.error = session
        .error
        .take()
        .or_else(|| (loaded > 0).then(|| format!("loaded {loaded} member(s)")));
}

pub fn main() -> ExitCode {
    let args = parse_args();
    let mode = match args.live.clone() {
        Some(name) => RunMode::Live(name),
        None => RunMode::Interactive,
    };
    match run(&args, mode) {
        Ok(0) => ExitCode::SUCCESS,
        Ok(code) => ExitCode::from(code as u8),
        Err(e) => {
            eprintln!("tui-play: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
#[path = "bin_tests.rs"]
mod tests;
