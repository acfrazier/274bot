//! `tui-play`: the headless operator panel binary. Same flag spirit as
//! `host-play` / `panel-play` (`--vault`, `--vault-pass-stdin`,
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

use crossterm::event::{self, Event, KeyEventKind};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
#[cfg(test)]
use host_play::arm_walk_on;
use host_play::catalog_core::CoreWatch;
use host_play::live_gate::{self, CoreGate, LiveCore, PassHold};
use host_play::live_start::{self, PendingCatalogStart, StartArming};
use host_play::paired_core::PairWatch;
use host_play::walk_map::{
    observed_services, ActionError, ActionKind, Catalogue, MapContext, WalkExclude, WalkSlotStatus,
};
use host_play::{
    background_bots_ack_error, background_bots_acked, clear_background_bots_ack_error,
    live_vault_passphrase, load_navpois, map_ready_catalogue, mint_live_entries, mint_live_names,
    open_vault, parse_profile_args, peek_map_catalogue, persist_background_bots_ack,
    player_here_tile, profile_password, run_with_io, run_with_template, MapDemandHandle,
    MapJobStatus, MapStage, PlayOptions, ProfileOptions, ReadyCatalogue, ServerProfile,
    SharedClientTemplate, WalkArm, WireCmd,
};
use nav::map::identity::Digest;
use nav::tile::Tile;
use nav::WorldState;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use vault::{Profile, Vault};

use frontend_core::resources::background_ack_text;
use frontend_core::{
    load_map_bake_choice, persist_map_bake_choice, HeadlessSurface, MapBakeGate, OperatorSession,
};
use host_play::map_cache::MapDemand;

use crate::app::{AppAction, ChatData, MapCatalogueStatus, TuiApp};
use crate::chat::ChatAction;
use crate::overlay::ConfirmKind;
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
    /// `--vault-pass-stdin`: read the vault passphrase from a piped stdin. It
    /// is never an argument or an environment variable; on a terminal it is
    /// asked for at a hidden prompt.
    pub pass_stdin: bool,
    pub users: Vec<String>,
    pub live: Option<String>,
    pub world: Option<u16>,
    pub profile: ProfileOptions,
    /// `--catalog-core`: qualify the `--live` run under the shared catalog
    /// core witness (the panel `catalog_watch` mode).
    pub catalog_core: bool,
    /// `--pair-core`: qualify a paired `--live` run under the shared pair
    /// witness (the panel `pair_watch` mode).
    pub pair_core: bool,
    /// `--map-bundle OUT`: release packaging only; bake the shipped WalkTo
    /// map terrain for `--revision`, `--cache` and `--unpack` under `OUT`
    /// and exit.
    pub map_bundle: Option<PathBuf>,
}

const USAGE: &str = "usage: tui-play [--profile NAME|--rs2b2t] [--revision 274|289] \
         [--host HOST] [--port PORT] [--asset-host HOST] [--http-port PORT] \
         [--engine DIR] [--cache DIR] [--unpack DIR] [--nav-pack PATH] [--nav-flags PATH] \
         [--content DIR] [--vault PATH] [--catalog DIR] [--cache-manifest PATH] [--vault-pass-stdin] \
         [--world N] [--live script_<name> [--catalog-core | --pair-core]] [--user USER]... \
         (default user: first vault profile)\n\
         release packaging: tui-play --map-bundle OUT --revision 274|289 --cache JAG_DIR \
         --unpack SNAPSHOT_ROOT (bakes the shipped WalkTo map terrain into OUT/map/<revision>)";

/// Usage on stderr for a bad invocation (exit 2, the CLI family's convention).
fn usage() -> ! {
    eprintln!("{USAGE}");
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

/// `BOT_LIVE_CORE=catalog|pair`, the environment form of `--catalog-core` /
/// `--pair-core` for harnesses that only pass environment. Empty is unset.
fn live_core_from_env(value: Option<&str>) -> Result<(bool, bool), String> {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok((false, false)),
        Some("catalog") => Ok((true, false)),
        Some("pair") => Ok((false, true)),
        Some(other) => Err(format!(
            "tui-play: BOT_LIVE_CORE={other:?}: expected catalog or pair"
        )),
    }
}

/// `--live NAME` wins over `BOT_LIVE`, and a core flag over `BOT_LIVE_CORE`;
/// empty env is ignored. `--help`/`-h` print the usage on stdout and exit 0;
/// a bad invocation prints it on stderr and exits 2. Shared server/profile
/// flags are consumed first by host-play.
pub fn parse_args() -> Args {
    match parse_args_from(env::args().skip(1)) {
        Ok(args) => args,
        Err(msg) if msg == "usage" => {
            println!("{USAGE}");
            std::process::exit(0);
        }
        Err(msg) => {
            eprintln!("{msg}");
            usage();
        }
    }
}

/// Testable CLI parse.
pub fn parse_args_from(args: impl IntoIterator<Item = impl AsRef<str>>) -> Result<Args, String> {
    let (profile, rest) = parse_profile_args(args).map_err(|e| format!("tui-play: {e}"))?;
    let (catalog_core, pair_core) = live_core_from_env(env::var("BOT_LIVE_CORE").ok().as_deref())?;
    let mut parsed = Args {
        pass_stdin: false,
        world: None,
        users: Vec::new(),
        live: env::var("BOT_LIVE").ok().filter(|s| !s.is_empty()),
        profile,
        catalog_core,
        pair_core,
        map_bundle: None,
    };
    let mut core_flags: Option<(bool, bool)> = None;
    let mut it = rest.into_iter();
    while let Some(arg) = it.next() {
        if host_play::passphrase::is_removed_flag(arg.as_ref()) {
            return Err(host_play::passphrase::removed_flag_error("tui-play"));
        }
        match arg.as_ref() {
            "--vault-pass-stdin" => parsed.pass_stdin = true,
            "--user" => parsed.users.push(need_value(&mut it, "--user")?),
            "--world" => {
                let value = need_value(&mut it, "--world")?;
                parsed.world = Some(value.parse::<u16>().ok().filter(|n| *n != 0).ok_or_else(
                    || "tui-play: --world needs a positive world number".to_string(),
                )?);
            }
            "--live" => parsed.live = Some(need_value(&mut it, "--live")?),
            "--map-bundle" => {
                parsed.map_bundle = Some(PathBuf::from(need_value(&mut it, "--map-bundle")?))
            }
            "--catalog-core" => core_flags.get_or_insert((false, false)).0 = true,
            "--pair-core" => core_flags.get_or_insert((false, false)).1 = true,
            "--help" | "-h" => return Err("usage".into()),
            other => return Err(format!("tui-play: unknown {other}")),
        }
    }
    if let Some((catalog_core, pair_core)) = core_flags {
        parsed.catalog_core = catalog_core;
        parsed.pair_core = pair_core;
    }
    if (parsed.catalog_core || parsed.pair_core) && parsed.live.is_none() {
        return Err(
            "tui-play: --catalog-core/--pair-core qualify a --live script_<name> run".into(),
        );
    }
    if parsed.map_bundle.is_some()
        && (parsed.profile.revision.is_none()
            || parsed.profile.cache_dir.is_none()
            || parsed.profile.unpack_dir.is_none()
            || parsed.live.is_some())
    {
        return Err(
            "tui-play: --map-bundle needs --revision, --cache and --unpack (and no --live)".into(),
        );
    }
    Ok(parsed)
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
                password: pass.clone().into(),
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
    let removed = travellers.lock().unwrap().remove(name);
    let Some(arm) = removed else {
        return false;
    };
    host_play::cancel_walk_arm(Some(name), &mut arm.lock().unwrap(), None, "SessionEnded");
    true
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

/// The isolate Starts a live card stashes: both slots with their role bags
/// under the pair gate, else the driven slot alone.
fn live_card_starts(
    live_core: LiveCore,
    names: &[String],
    card: &script::JsCard,
    bag: Option<serde_json::Map<String, serde_json::Value>>,
    siblings: Vec<(String, String)>,
    loadouts: Vec<script::Loadout>,
    pair_watch: &PairWatch,
) -> Result<Vec<PendingCatalogStart>, String> {
    match live_core {
        LiveCore::Pair(case) => live_start::pair_starts(
            names,
            case,
            card.js.clone(),
            card.shape,
            &card.settings_schema,
            siblings,
            pair_watch,
        ),
        LiveCore::Off | LiveCore::Catalog(_) => Ok(vec![PendingCatalogStart::load(
            names[0].clone(),
            card.js.clone(),
            card.shape,
            bag,
            siblings,
            loadouts,
        )]),
    }
}

/// The Play-owned witness handles a core-gated `--live` run polls, taken
/// once at prepare so each poll borrows them instead of cloning `Play`'s.
struct LiveWitnesses {
    catalog: CoreWatch,
    pair: PairWatch,
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
    /// them once on [`scenario::StepKind::StartScript`].
    pending_script: Arc<Mutex<Vec<PendingCatalogStart>>>,
    /// Isolate-start handle and core watches the per-frame hook arms Start
    /// with (filled after Play).
    start_arming: Arc<Mutex<StartArming>>,
    /// The `--live` run's scenario name, for the PASS/FAIL label.
    live_name: Option<String>,
    /// `--catalog-core` / `--pair-core`: the shared witness the next
    /// `--live` prepare arms.
    live_catalog_core: bool,
    live_pair_core: bool,
    /// The witnesses a `--catalog-core` / `--pair-core` run armed. `None`
    /// for interactive use and plain `--live` runs, which poll no gate.
    live_witnesses: Option<LiveWitnesses>,
    /// The armed witness's own ceiling (`None` without a core gate).
    live_core_deadline: Option<Instant>,
    /// `BUDGET_S` soak: keep pumping after proof PASS until this instant.
    live_soak_until: Option<Instant>,
    live_announced_pass: bool,
    live_wait_script_stop: Option<&'static str>,
    live_stop_wait_started: Option<Instant>,
    /// The Browse-selected card (catalog Start after seed, or operator Start).
    script_sel: Option<script::ScriptSel>,
    /// Shared script coordination (card library and catalog, assignment,
    /// per-profile parameters, Start/Stop all, reload, Apply to all): the
    /// same owner the panel uses.
    scripts: frontend_core::Scripts,
    /// First-run rs2b0t clone-root folder browser.
    rs2b0t_catalog_open: bool,
    rs2b0t_catalog_dir: PathBuf,
    /// Browse category order keys (in-memory; panel persists to panel-ui.json).
    script_category_order: Vec<String>,
    /// Process-wide loadout presets (`~/.274bot/loadouts.json`).
    loadouts: script::LoadoutsStore,
    /// Last directory visited in the out-of-tree Load file browser.
    script_load_last_dir: Option<PathBuf>,
    /// Core projection generations the app last copied (fleet rows, the
    /// whole view, the resource meter): a pump copies only what moved.
    copied_rows: Option<u64>,
    copied_view: Option<u64>,
    copied_meter: Option<u64>,
    persist_ui: bool,
    background_bots_acked: bool,
    ack_checked_at: Option<Instant>,
    /// Background count and resource generation the notice was built for.
    notice_sig: Option<(usize, u64)>,
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
        // A missing store is a first run; a refused or corrupt one is said out
        // loud (and nothing is saved over a refused one).
        if let Err(error) = js.restore() {
            eprintln!("tui-play: {error}");
        }
        let travellers: SlotTravellers = Arc::new(Mutex::new(HashMap::new()));
        let mut core = OperatorSession::new(_instance);
        // The fleet rows show the map walks this TUI arms.
        core.set_walk_arms(Arc::clone(&travellers));
        Self {
            #[cfg(feature = "memory-profile")]
            memory: None,
            core,
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
            travellers,
            tick_latch: Arc::new(Mutex::new(HashMap::new())),
            walk_clear: Arc::new(AtomicBool::new(false)),
            nav_world: Arc::new(Mutex::new(None)),
            scenario: Arc::new(Mutex::new(None)),
            pending_script: Arc::new(Mutex::new(Vec::new())),
            start_arming: Arc::new(Mutex::new(StartArming::default())),
            live_name: None,
            live_catalog_core: false,
            live_pair_core: false,
            live_witnesses: None,
            live_core_deadline: None,
            live_soak_until: None,
            live_announced_pass: false,
            live_wait_script_stop: None,
            live_stop_wait_started: None,
            script_sel: None,
            scripts: frontend_core::Scripts::new(
                js,
                script::ScriptSettingsStore::with_default_path(),
            ),
            rs2b0t_catalog_open: false,
            rs2b0t_catalog_dir: Self::default_catalog_browse_dir(),
            script_category_order: Vec::new(),
            loadouts: script::LoadoutsStore::with_default_path(),
            script_load_last_dir: None,
            copied_rows: None,
            copied_view: None,
            copied_meter: None,
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
                transport: profile.transport(),
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

    #[cfg(feature = "memory-profile")]
    fn profile_class(&self) -> host_play::ProfileClass {
        self.server_profile
            .as_ref()
            .map_or(host_play::ProfileClass::Remote, |profile| {
                profile.profile_class()
            })
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

    /// Live harness bag: schema defaults + legacy overrides + inject.
    /// Empty schema still keeps inject keys (Thiever `target: Guard`).
    /// `None` only when the merged bag is empty.
    fn pending_settings_bag(
        &self,
        source: script::ScriptSource,
        name: &str,
        schema: &[script::SettingDef],
    ) -> Option<serde_json::Map<String, serde_json::Value>> {
        let merged = self.scripts.legacy_bag(source, name, schema);
        (!merged.is_empty()).then_some(merged)
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
        let start_arming = Arc::clone(&self.start_arming);
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
                    if runner.wants_script_paint() {
                        let paint = start_arming
                            .lock()
                            .unwrap()
                            .handle
                            .as_ref()
                            .and_then(|handle| handle.paint(name));
                        if let Some(paint) = paint {
                            runner.observe_script_paint(paint.lines.iter().map(String::as_str));
                        }
                    }
                    let pump = live_start::fire_pending_catalog_start(
                        &mut pending_script.lock().unwrap(),
                        runner.on_start_script(),
                        runner.on_stop_script(),
                        || start_arming.lock().unwrap().clone(),
                    );
                    match pump {
                        live_start::StartScriptPump::Hold => {}
                        live_start::StartScriptPump::CompiledFailed(error) => {
                            runner.fail_start(&error);
                        }
                        live_start::StartScriptPump::Continue
                        | live_start::StartScriptPump::CompiledRunning => {
                            if pump == live_start::StartScriptPump::CompiledRunning {
                                runner.observe_script_running();
                            }
                            if runner.on_stop_script() {
                                runner.observe_script_idle(start_arming.lock().is_ok_and(
                                    |arming| {
                                        arming
                                            .handle
                                            .as_ref()
                                            .is_some_and(|handle| handle.idle(name))
                                    },
                                ));
                            }
                            runner.tick_with_hold(c, hold);
                            // A relog can enter Start on the final off-world frame.
                            // Arm it now rather than after the reconnect posts colour.
                            if runner.on_start_script() {
                                if let Ok(mut pending) = pending_script.lock() {
                                    let pump = live_start::fire_pending_catalog_start(
                                        &mut pending,
                                        true,
                                        false,
                                        || {
                                            start_arming
                                                .lock()
                                                .map(|arming| arming.clone())
                                                .unwrap_or_default()
                                        },
                                    );
                                    match pump {
                                        live_start::StartScriptPump::CompiledRunning => {
                                            runner.observe_script_running();
                                        }
                                        live_start::StartScriptPump::CompiledFailed(error) => {
                                            runner.fail_start(&error);
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }
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
                host_play::step_walk_arm_follow(
                    c,
                    snap,
                    &mut arm,
                    world.as_deref(),
                    here,
                    map_members,
                    Some(name),
                )
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
        *self.start_arming.lock().unwrap() = StartArming {
            handle: Some(play.script_start_handle()),
            catalog: Some(play.catalog_core_watch()),
            pair: Some(play.paired_core_watch()),
        };
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
            self.scripts.cancel_queued_as(&name, "logged out");
            self.scripts.publish_start_places(&mut self.core);
            self.apply_script_notice(app);
            self.core.logout(&name);
        }
    }

    /// Log out every member and every other running slot.
    fn logout_all(&mut self) {
        self.core.logout_all();
    }

    /// Settings-popup `r`: Relog-now for a memory-mode switch. Confirms
    /// first when a running script would be interrupted, relogs directly
    /// otherwise (the same shared command the panel's mem picker calls).
    fn memory_relog_request(&mut self, app: &mut TuiApp, name: &str) {
        if self.core.memory_relog_warning(name) {
            app.confirm(ConfirmKind::MemoryRelog(name.to_string()));
        } else {
            self.memory_relog_now(app, name);
        }
    }

    /// Relog `name` now: log out and back in through the login FIFO so
    /// the toggled memory mode reaches the server tabs and sound.
    fn memory_relog_now(&mut self, app: &mut TuiApp, name: &str) {
        self.scripts.cancel_queued_as(name, "logged out");
        self.scripts.publish_start_places(&mut self.core);
        self.apply_script_notice(app);
        let (core, mut surface) = self.core_and_surface();
        let op = core.request_memory_relog(name, &mut surface);
        app.error = core.failure(op);
    }

    /// Remove `name` (frozen when the operator confirmed): clean logout,
    /// then its worker stops. The neighbour becomes selected when it was.
    fn remove(&mut self, app: &mut TuiApp, name: &str) {
        self.scripts.cancel_queued_as(name, "removed");
        self.scripts.publish_start_places(&mut self.core);
        self.apply_script_notice(app);
        let (core, mut surface) = self.core_and_surface();
        let removal = core.remove(name, Instant::now(), &mut surface);
        if let Some(next) = removal.reselected {
            app.focused = app.names.iter().position(|n| n == &next);
        } else if removal.selection_cleared {
            app.focused = None;
        }
        // The fleet table drops the member on the next pump.
    }

    /// Load every vault profile and log in every member (Load+login all). A
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

    /// `--live script_*` boot: minted ephemeral vault + spawn + runner. With
    /// `--catalog-core` / `--pair-core` the Play-owned witness is armed
    /// before either slot publishes, exactly like the panel watches.
    fn live_prepare_script(&mut self, scenario: scenario::Scenario) -> Result<(), String> {
        self.persist_ui = false;
        let live_core = LiveCore::resolve(
            scenario.name,
            scenario.seed.profiles.len(),
            self.live_catalog_core,
            self.live_pair_core,
        )?;
        let name = scenario.name.to_string();
        let start_script = scenario.settings.start_script;
        let fleet_start = scenario.settings.fleet_start;
        let start_file = scenario.settings.start_file;
        let wait_script_stop = scenario.settings.wait_script_stop;
        let settings_inject = scenario.settings.script_settings_inject;
        let scenario_deadline = scenario.settings.deadline;
        let fixture_loadouts = scenario_fixture_loadouts(&scenario.settings);
        let names = mint_live_names(scenario.seed.profiles.len());
        let entries = mint_live_entries(&names);
        let pass = live_vault_passphrase();
        let path = temp_live_vault(&entries, &pass);
        self.unlock_at(&path, &pass)?;
        let play = self
            .core
            .play()
            .ok_or_else(|| "live prepare needs the unlocked play".to_string())?;
        live_core.configure(play, &names)?;
        let pair_watch = play.paired_core_watch();
        self.live_witnesses = (live_core != LiveCore::Off).then(|| LiveWitnesses {
            catalog: play.catalog_core_watch(),
            pair: pair_watch.clone(),
        });
        self.live_name = Some(name);
        self.live_wait_script_stop = wait_script_stop;
        self.live_stop_wait_started = None;
        let world = play.world();
        let mut runner = scenario::ScenarioRunner::with_world(scenario, world);
        runner.set_map_members(play.map_members());
        let budget = scenario::budget_s_from_env();
        if let Some(budget) = budget {
            runner.set_deadline(budget);
            self.live_soak_until = Some(Instant::now() + budget);
            self.live_announced_pass = false;
        }
        runner.set_live_names(&names);
        runner.set_obj_names(play.obj_names());
        *self.scenario.lock().unwrap() = Some(runner);
        self.scripts.inject = scenario::settings_inject_map(settings_inject);
        self.names = names.clone();
        for n in &names {
            self.load_and_login(n);
        }
        self.focus(&names[0]);
        // A scenario that names a script card selects the real `$RS2B0T`
        // catalog script on the driven slot (same as the panel): fill the
        // catalog from `$RS2B0T`, then Start on StartScript after seed.
        // A fleet launch Starts the scenario's own card on each member slot.
        if let Some(fleet) = fleet_start {
            self.fill_rs2b0t_cards_once();
            let scripts = &mut self.scripts;
            let starts = live_start::fleet_catalog_starts(
                fleet,
                &names,
                &mut scripts.js,
                &scripts.legacy,
                scripts.inject.as_ref(),
                &fixture_loadouts,
            )?;
            *self.pending_script.lock().unwrap() = starts;
        // Exact example files Load as File cards and select by identity_id.
        } else if let Some(file_name) = start_file {
            let path = script::live_example_path(file_name)
                .ok_or_else(|| format!("no in-tree example {file_name}"))?;
            let loaded = self
                .scripts
                .js
                .load(&path)
                .map_err(|e| format!("load {file_name}: {e}"))?;
            let identity = loaded.identity_id();
            let card = self
                .scripts
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
            let siblings = self.sibling_modules_for_card(&card)?;
            *self.pending_script.lock().unwrap() = live_card_starts(
                live_core,
                &names,
                &card,
                bag,
                siblings,
                fixture_loadouts,
                &pair_watch,
            )?;
        } else if let Some(card_name) = start_script {
            if let Some(id) = script::compiled_id(card_name) {
                self.script_sel = Some(script::ScriptSel::Compiled(id));
                let bag = self.scripts.compiled_bag(&self.core, &names[0], id)?;
                *self.pending_script.lock().unwrap() =
                    vec![PendingCatalogStart::compiled(names[0].clone(), id, bag)];
            } else {
                self.fill_rs2b0t_cards_once();
                self.scripts
                    .js
                    .ensure_js(script::ScriptSource::Catalog, card_name)
                    .map_err(|e| format!("transpile {card_name}: {e}"))?;
                let card = self
                    .scripts
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
                let siblings = self.sibling_modules_for_card(&card)?;
                *self.pending_script.lock().unwrap() = live_card_starts(
                    live_core,
                    &names,
                    &card,
                    bag,
                    siblings,
                    fixture_loadouts,
                    &pair_watch,
                )?;
            }
        }
        self.live_core_deadline = self.live_witnesses.as_ref().and_then(|armed| {
            live_gate::core_deadline(
                Some(&armed.catalog),
                Some(&armed.pair),
                budget,
                scenario_deadline,
                Instant::now(),
            )
        });
        Ok(())
    }

    /// The sibling modules a card's isolate resolves at Start.
    fn sibling_modules_for_card(
        &self,
        card: &script::JsCard,
    ) -> Result<Vec<(String, String)>, String> {
        live_start::card_siblings(&self.scripts.js, card)
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
        let name = app.focused_name();
        let origin = app.here.map(|h| Tile {
            x: h.x,
            z: h.z,
            level: h.level,
        });
        let options = app.nav.find_options();
        let profile = self.server_profile.clone();
        let state = self.focused_walk_state(&name);
        let request = host_play::walk_map::WalkRequest {
            slot: name.as_deref(),
            origin,
            destination: app
                .map_model
                .pending()
                .map(|s| s.target.unwrap_or(s.requested))
                .or(Some(dest)),
            options,
            map_members: state.map_members,
            members: profile
                .as_ref()
                .map_or(&host_play::WorldMembersFact::Unknown, |p| p.world_members()),
        };
        let result = request.run(|| {
            let context = self.map_context(app)?;
            let from = origin.ok_or(ActionError::NoOrigin)?;
            let command = app
                .map_model
                .confirm(ActionKind::Walk, &context, Some(from), options);
            app.clear_consumed_map_selection();
            let command = command?;
            let play = self.core.play().ok_or(ActionError::NoFocus)?;
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
            let route = play.map_walk(command, &context, &state, &bank, &self.travellers)?;
            app.walk_dest = Some(destination);
            Ok(route)
        });
        match result {
            Ok(_) => app.error = None,
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

    /// Walk every marked bot to the pending destination through the shared
    /// [`frontend_core::walk_marked`], the same command the panel's picker
    /// runs. Nothing marked keeps the map selection.
    fn map_walk_group(&mut self, app: &mut TuiApp) {
        if app.table.selection.is_empty() {
            app.error = Some("no bots selected".into());
            return;
        }
        let options = app.nav.find_options();
        let destination = app
            .map_model
            .pending()
            .map(|s| s.target.unwrap_or(s.requested));
        let prepared = self.map_context(app).and_then(|context| {
            let plan = app.map_model.confirm_walk_plan(&context, options);
            app.clear_consumed_map_selection();
            plan.map(|plan| (context, plan))
        });
        let planned = prepared.as_ref().ok().map(|(_, plan)| plan.destination());
        let report = frontend_core::walk_marked(
            &app.table.selection,
            &self.core,
            frontend_core::MarkedWalk {
                prepared,
                destination,
                options,
                arms: &self.travellers,
            },
            |name| frontend_core::WalkInputs {
                state: self.focused_walk_state(&Some(name.to_string())),
                bank: self
                    .snapshots
                    .lock()
                    .unwrap()
                    .get(name)
                    .map(|snap| {
                        snap.bank()
                            .iter()
                            .map(|it| (it.def.id, it.count))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default(),
            },
        );
        if report.done_count() > 0 {
            let mut latch = self.tick_latch.lock().unwrap();
            for name in report.done() {
                latch.remove(name);
            }
            drop(latch);
            self.walk_clear.store(false, Ordering::Relaxed);
            app.walk_dest = planned;
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
    /// the env/persisted root is already known (live boot). Errors are
    /// debug-only.
    fn fill_rs2b0t_cards_once(&mut self) {
        if self.scripts.catalog_filled() {
            return;
        }
        let root = self.catalog_root();
        self.scripts.fill_catalog_once(root.as_deref());
        self.scripts.mark_catalog_filled();
    }

    /// Opening Browse: fill from `$RS2B0T`/persisted root, or prompt for a
    /// clone root, or honour a prior defer (panel parity).
    fn on_script_browse_open(&mut self, app: &mut TuiApp) {
        if self.scripts.catalog_filled() {
            return;
        }
        if let Some(root) = self.catalog_root() {
            self.scripts.fill_catalog_once(Some(&root));
            return;
        }
        self.scripts.mark_catalog_filled();
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
            .scripts
            .js
            .register_rs2b0t(root, &script::default_rs2b0t_path_file())?;
        let _ = script::clear_rs2b0t_import_at(&script::default_rs2b0t_import_file());
        self.rs2b0t_catalog_open = false;
        app.rs2b0t_catalog_open = false;
        Ok(n)
    }

    /// Show the script coordinator's latest notice on the strip.
    fn apply_script_notice(&mut self, app: &mut TuiApp) {
        if let Some(notice) = self.scripts.take_notice() {
            notice.apply(&mut app.error);
        }
    }

    /// The catalog root a Start may fill the catalog from (first use only).
    fn start_catalog_root(&self) -> Option<PathBuf> {
        if self.scripts.catalog_filled() {
            None
        } else {
            self.catalog_root()
        }
    }

    /// Operator Start on the focused profile: `sel` becomes its pending
    /// selection and Starts with its own parameters; the assignment is
    /// saved once the Start is Ready (the same core path as the panel).
    fn script_start(&mut self, app: &mut TuiApp, sel: &script::ScriptSel) {
        let Some(name) = app.focused_name() else {
            app.error = Some("script: no focused profile".into());
            return;
        };
        self.scripts.set_pending_browse(&name, sel.clone());
        let root = self.start_catalog_root();
        let result = self
            .scripts
            .start_selected(&mut self.core, &name, Some(sel), root.as_deref());
        match result {
            Ok(()) => self.scripts.show_load_failures(),
            Err(e) => {
                self.apply_script_notice(app);
                app.error = Some(format!("script: {e}"));
                return;
            }
        }
        self.apply_script_notice(app);
    }

    /// Fold settled script work (reload validation, Start setup, parameter
    /// writes) after the core poll and show its notice. Called once per
    /// pump.
    fn poll_scripts(&mut self, app: &mut TuiApp) {
        self.scripts.poll(&mut self.core);
        self.scripts.take_start_failures();
        self.apply_script_notice(app);
        app.reload_confirm = self.scripts.reload_awaiting_confirm();
        // Follow an Apply-to-all report shown in the popup as its writes
        // settle (rewritten in place; nothing allocates while unchanged).
        if let (Some(shown), Some(report)) = (
            app.params_state.report.as_mut(),
            self.scripts.last_settings_sync(),
        ) {
            if shown != report.summary() {
                shown.clear();
                shown.push_str(report.summary());
            }
        }
    }

    fn script_start_all(&mut self, app: &mut TuiApp) {
        let root = self.start_catalog_root();
        if !app.table.selection.is_empty() {
            frontend_core::start_marked(
                &app.table.selection,
                &mut self.core,
                &mut self.scripts,
                app.script_sel.as_ref(),
                root.as_deref(),
            );
            self.apply_script_notice(app);
        } else {
            self.scripts.start_all(&mut self.core, root.as_deref());
            self.apply_script_notice(app);
        }
    }

    /// Save the selected script as the assignment of every marked bot; the
    /// one bulk report names each bot that could not take it.
    fn script_assign_marked(&mut self, app: &mut TuiApp) {
        let Some(card) = app.script_sel.clone() else {
            return;
        };
        let root = self.start_catalog_root();
        let report = frontend_core::assign_marked(
            &app.table.selection,
            &mut self.core,
            &mut self.scripts,
            &card,
            root.as_deref(),
        );
        self.apply_script_notice(app);
        app.error = Some(report.summary());
    }

    /// Assign the selected script to every marked bot and start it there
    /// through the paced Start permit; the running Start report follows.
    fn script_restart_marked(&mut self, app: &mut TuiApp) {
        let Some(card) = app.script_sel.clone() else {
            return;
        };
        let root = self.start_catalog_root();
        frontend_core::assign_and_restart_marked(
            &app.table.selection,
            &mut self.core,
            &mut self.scripts,
            &card,
            root.as_deref(),
        );
        self.apply_script_notice(app);
    }

    /// Freeze the focused bot's settings for the selected script against
    /// the marked bots (the same core command as the panel's Fleet window)
    /// and ask to confirm the frozen scope; nothing is written before that.
    fn prepare_apply_settings_marked(&mut self, app: &mut TuiApp) {
        let (Some(source), Some(card)) = (app.focused_name(), app.script_sel.clone()) else {
            app.error = Some("Apply to marked: focus a bot and select a script".into());
            return;
        };
        match frontend_core::prepare_apply_settings_marked(
            &app.table.selection,
            &self.core,
            &mut self.scripts,
            &source,
            &card,
        ) {
            Ok(scope) => app.confirm(ConfirmKind::ApplyMarked(scope.prompt().to_string())),
            Err(error) => app.error = Some(error),
        }
        self.apply_script_notice(app);
    }

    /// Log in every marked bot, loading the ones not loaded yet.
    fn login_marked(&mut self, app: &mut TuiApp) {
        let (core, mut surface) = self.core_and_surface();
        let report = frontend_core::login_marked(&app.table.selection, core, &mut surface);
        app.error = Some(report.summary());
    }

    /// Log out every marked bot that is logged in or logging in.
    fn logout_marked(&mut self, app: &mut TuiApp) {
        let report =
            frontend_core::logout_marked(&app.table.selection, &mut self.core, &mut self.scripts);
        self.apply_script_notice(app);
        app.error = Some(report.summary());
    }

    fn script_stop_all(&mut self, app: &mut TuiApp) {
        if !app.table.selection.is_empty() {
            let report =
                frontend_core::stop_marked(&app.table.selection, &mut self.core, &mut self.scripts);
            app.reload_confirm = self.scripts.reload_awaiting_confirm();
            self.apply_script_notice(app);
            app.error = Some(report.summary());
        } else {
            self.scripts.stop_all(&mut self.core);
            app.reload_confirm = self.scripts.reload_awaiting_confirm();
            self.apply_script_notice(app);
        }
    }

    /// Reload the focused heading's card, or confirm a shown warning.
    fn script_reload(&mut self, app: &mut TuiApp) {
        let focused = app.focused_name();
        let target =
            self.scripts
                .reload_target(&self.core, focused.as_deref(), app.script_sel.as_ref());
        self.scripts.begin_reload(&mut self.core, target);
        app.reload_confirm = self.scripts.reload_awaiting_confirm();
        self.apply_script_notice(app);
    }

    fn script_reload_cancel(&mut self, app: &mut TuiApp) {
        self.scripts.cancel_reload();
        app.reload_confirm = false;
        self.apply_script_notice(app);
    }

    fn loaded_params_card(&self, app: &TuiApp) -> Option<(script::ScriptSource, String, PathBuf)> {
        let script::ScriptSel::Loaded(source, lookup) = app.params_card()? else {
            return None;
        };
        let card = self.scripts.js.get(source, &lookup)?;
        Some((source, card.name.clone(), card.path.clone()))
    }

    fn params_bag(
        &mut self,
        app: &TuiApp,
        profile: &str,
    ) -> Result<serde_json::Map<String, serde_json::Value>, String> {
        if let Some(script::ScriptSel::Compiled(id)) = app.params_card() {
            return self
                .scripts
                .compiled_edit_bag(&self.core, Some(profile), id);
        }
        let (source, name, path) = self
            .loaded_params_card(app)
            .ok_or("parameters unavailable")?;
        Ok(self.scripts.merged_profile_bag(
            &mut self.core,
            profile,
            source,
            &name,
            &path,
            &app.params_schema,
        ))
    }

    /// Open the params popup over the focused profile's bag for the card.
    fn open_params(&mut self, app: &mut TuiApp) {
        let Some(profile) = app.focused_name() else {
            app.error = Some("parameters: no focused profile".into());
            return;
        };
        match self.params_bag(app, &profile) {
            Ok(bag) => app.open_script_params(bag),
            Err(error) => app.error = Some(error),
        }
        self.apply_script_notice(app);
    }

    /// One key into the open params popup: each edit is a typed parameter
    /// write on the focused profile through the coordinator.
    fn params_key(&mut self, app: &mut TuiApp, key: crossterm::event::KeyEvent) -> AppAction {
        let (Some(profile), Some(selection)) = (app.focused_name(), app.params_card()) else {
            app.params_state.open = false;
            return AppAction::None;
        };
        let data = self.template.as_ref().and_then(|t| t.game_data());
        let scripts = &mut self.scripts;
        let core = &mut self.core;
        let mut commit = |field: &str, value: serde_json::Value| match &selection {
            script::ScriptSel::Compiled(id) => scripts
                .set_compiled_setting(core, &profile, *id, field, value)
                .map(|_| ()),
            script::ScriptSel::Loaded(source, lookup) => {
                let (name, path) = scripts
                    .js
                    .get(*source, lookup)
                    .map(|card| (card.name.clone(), card.path.clone()))
                    .ok_or("parameters unavailable")?;
                scripts
                    .set_profile_setting(core, &profile, *source, &name, &path, field, value)
                    .map(|_| ())
            }
        };
        let action = app.params_on_key(&mut commit, &self.loadouts, data.as_deref(), key);
        self.apply_script_notice(app);
        action
    }

    /// Re-read the params popup's bag from the focused profile.
    fn refresh_params_bag(&mut self, app: &mut TuiApp) {
        let Some(profile) = app.focused_name() else {
            return;
        };
        match self.params_bag(app, &profile) {
            Ok(bag) => app.params_bag = bag,
            Err(error) => app.error = Some(error),
        }
    }

    /// Freeze Apply to all for the params popup's card and ask to confirm.
    fn prepare_settings_sync(&mut self, app: &mut TuiApp) {
        let Some(profile) = app.focused_name() else {
            return;
        };
        let result = if let Some(script::ScriptSel::Compiled(id)) = app.params_card() {
            self.scripts
                .prepare_compiled_settings_sync(&self.core, &profile, id, None)
        } else if let Some((source, name, path)) = self.loaded_params_card(app) {
            Ok(self
                .scripts
                .prepare_settings_sync(&self.core, &profile, source, &name, &path))
        } else {
            Err("parameters unavailable".into())
        };
        match result {
            Ok(scope) => {
                app.params_state.sync_prompt =
                    Some(format!("{} y apply · n cancel", scope.prompt()))
            }
            Err(error) => app.error = Some(error),
        }
        self.apply_script_notice(app);
    }

    fn apply_settings_sync(&mut self, app: &mut TuiApp) {
        app.params_state.sync_prompt = None;
        if let Err(error) = self.scripts.apply_settings_sync(&mut self.core) {
            app.error = Some(error);
        }
        // The popup repeats the report: the strip may be too narrow for it.
        app.params_state.report = self
            .scripts
            .last_settings_sync()
            .map(|report| report.summary().to_string());
        self.apply_script_notice(app);
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
            self.scripts.cancel_queued(&name);
            self.scripts.publish_start_places(&mut self.core);
            self.core.stop_script(&name);
        }
    }

    /// Load a local JS bot file into the library, select it for Start,
    /// and persist the store. Errors land on the strip.
    fn script_load(&mut self, app: &mut TuiApp, path: &Path) {
        match self.scripts.js.load(path) {
            Ok(card) => {
                app.script_sel = Some(script::ScriptSel::Loaded(card.source, card.name));
                app.browse_changed = true;
                app.error = if self.scripts.js.load_failures().is_empty() {
                    None
                } else {
                    Some(self.scripts.js.named_failure_output())
                };
                if let Some(parent) = path.parent() {
                    self.script_load_last_dir = Some(parent.to_path_buf());
                    app.script_load_last_dir = self.script_load_last_dir.clone();
                }
            }
            Err(e) => {
                app.error = if self.scripts.js.load_failures().len() > 1 {
                    Some(self.scripts.js.named_failure_output())
                } else {
                    Some(format!("script: {e}"))
                };
            }
        }
    }

    /// Persist the settings popup's changes onto the profile it was opened
    /// for (the operator vault; `--live`'s temp vault is ephemeral) and
    /// mirror guardian and memory settings onto a running slot's arm. A
    /// refusal shows on the message line and in the popup; `Saved <name>.`
    /// shows in the same popup once the write is durable, and a write that
    /// fails later shows its error there too, with the popup back on what is
    /// saved.
    fn persist_settings(&mut self, app: &mut TuiApp) {
        let Some(name) = app.settings_profile.clone() else {
            return;
        };
        // Field edit, not a whole-settings replacement: the popup owns the
        // guardian fields and the memory row, and each arm change lands
        // through its own write (the form tracks the latest).
        let settings = &app.settings;
        let lowmem_changed = self
            .core
            .vault()
            .and_then(|v| v.get(&name))
            .is_some_and(|p| p.settings.lowmem != settings.lowmem);
        if lowmem_changed {
            match self.core.set_memory_mode(&name, settings.lowmem) {
                Ok(op) => app.settings_save.submitted(op, name.clone()),
                Err(e) => {
                    let reason = format!("settings: {e}");
                    app.settings_save.refused(reason.clone());
                    app.error = Some(reason);
                }
            }
        }
        match self.core.set_random_settings(
            &name,
            settings.random_events,
            &settings.lamp_skill,
            settings.lamp_auto,
        ) {
            Ok(op) => app
                .settings_save
                .submitted(&mut self.core, op, Some(&name), &name),
            Err(e) => {
                let reason = format!("settings: {e}");
                app.settings_save.refused(reason.clone());
                app.error = Some(reason);
            }
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
        self.poll_scripts(app);

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
        let ids_changed = app.profile_ids.len() != members.len()
            || members.iter().enumerate().any(|(index, name)| {
                let identity = self
                    .core
                    .profile_identity(name)
                    .unwrap_or_else(|| frontend_core::ProfileIdentity::synthetic(name));
                app.profile_ids.get(index).copied() != Some(identity)
            });
        if ids_changed {
            app.profile_ids.clear();
            app.profile_ids.extend(members.iter().map(|name| {
                self.core
                    .profile_identity(name)
                    .unwrap_or_else(|| frontend_core::ProfileIdentity::synthetic(name))
            }));
            app.table.selection.retain(app.profile_ids.iter().copied());
        }
        app.focused = self
            .core
            .selected()
            .and_then(|selected| app.names.iter().position(|n| n == selected));
        self.copy_projection(app);
        // The settings popup and the status pane read the focused slot's
        // login-time vs current memory mode from here.
        app.memory = app
            .focused_name()
            .and_then(|name| self.core.memory_status(&name));
        // A Browse pick is the pending selection of the profile whose
        // heading it replaced.
        if std::mem::take(&mut app.browse_changed) {
            if let (Some(name), Some(sel)) = (self.last_focused.as_deref(), app.script_sel.as_ref())
            {
                self.scripts.set_pending_browse(name, sel.clone());
            }
        }
        // The script heading follows the focused profile: reload when the
        // focus changes (a fresh focus must not carry its script draft).
        // The settings popup stays bound to the profile it was opened for:
        // a focus change never closes it or rewrites its buffers.
        let focused = app.focused_name();
        if self.last_focused.as_deref() != focused.as_deref() {
            self.last_focused = focused.clone();
            let heading = focused
                .as_deref()
                .and_then(|n| self.scripts.heading(&self.core, n));
            if heading.as_ref().is_some_and(|sel| {
                matches!(
                    sel,
                    script::ScriptSel::Loaded(script::ScriptSource::Catalog, _)
                )
            }) {
                self.fill_rs2b0t_cards_once();
            }
            app.script_sel = heading;
            if !app.settings_state.open {
                app.settings = focused
                    .as_deref()
                    .and_then(|n| self.core.vault().and_then(|v| v.get(n)))
                    .map(|p| p.settings.clone())
                    .unwrap_or_default();
            }
        }
        if app.settings_state.open {
            if let (None, Some(name)) = (app.settings_profile.as_ref(), focused.as_deref()) {
                // Freshly opened: bind the focused profile and load its row;
                // later focus changes keep this binding and its buffers.
                app.settings = self
                    .core
                    .vault()
                    .and_then(|v| v.get(name))
                    .map(|p| p.settings.clone())
                    .unwrap_or_default();
                app.settings_title = format!("{} — {name}", crate::settings::TITLE);
                app.settings_profile = Some(name.to_string());
                app.settings_dirty = false;
            }
        }

        let mut write_failed = false;
        for failure in self.core.take_write_failures() {
            app.error = Some(failure.to_string());
            write_failed = true;
        }
        if write_failed && app.params_state.open {
            // A failed write rolled the profile back: show what is saved.
            self.refresh_params_bag(app);
        }
        self.refresh_background_notice(app, Instant::now());
        // The script pane's Browse picker lists library cards with registry fields.
        app.script_cards = self.scripts.browse_cards().map(BrowseCard::from).collect();
        let present = categories_present(&app.script_cards);
        let order = resolve_category_order(&self.script_category_order, &present);
        if order != self.script_category_order {
            self.script_category_order = order.clone();
        }
        app.script_category_order = self.script_category_order.clone();
        app.params_unavailable = None;
        match &app.script_sel {
            Some(script::ScriptSel::Loaded(source, name)) => {
                if let Some(card) = self.scripts.js.get(*source, name) {
                    app.params_schema.clone_from(&card.settings_schema);
                } else {
                    app.params_schema.clear();
                    app.params_unavailable = Some("script parameters unavailable");
                }
            }
            Some(script::ScriptSel::Compiled(id)) => {
                match self
                    .scripts
                    .compiled_schema(&self.core, app.focused_name().as_deref(), *id)
                {
                    frontend_core::scripts::SchemaView::Ready { fields, .. } => {
                        if app.params_schema.as_slice() != fields {
                            app.params_schema.clear();
                            app.params_schema.extend_from_slice(fields);
                        }
                    }
                    frontend_core::scripts::SchemaView::Unavailable(reason) => {
                        app.params_schema.clear();
                        app.params_unavailable = Some(reason);
                    }
                }
            }
            None => app.params_schema.clear(),
        }
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
            app.chat_data.script_paint = self
                .core
                .status(name)
                .and_then(|st| st.script_paint.clone());
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
            app.script_queued = self.scripts.start_queue_place(name).is_some();
        } else {
            app.script_queued = false;
            if app.map_active {
                app.map_observed.clear();
            }
        }
        if app.settings_dirty {
            self.persist_settings(app);
            app.settings_dirty = false;
        }
        // A closed popup lets go of its profile only once its last edit
        // persisted there; a persist still in flight then never shows in a
        // later popup.
        if !app.settings_state.open && app.settings_profile.take().is_some() {
            app.settings_save.form_changed();
        }
        // `Saved <name>.` or the failure shows only on the popup the persist
        // came from. A failed write put the profile back, so that popup
        // shows what is saved rather than the edit that did not land.
        for settled in app.settings_save.settle(&mut self.core) {
            let frontend_core::FormSettled::Failed(failed) = settled else {
                continue;
            };
            if !failed.same_form {
                continue;
            }
            if let Some(saved) = app
                .settings_profile
                .as_deref()
                .and_then(|name| self.core.vault().and_then(|v| v.get(name)))
            {
                app.settings = saved.settings.clone();
            }
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
        live_gate::script_self_stop_observed(
            play.script_state(name),
            play.script_lifecycle_receipt(name).as_ref(),
            needle,
        )
    }

    /// The `--live` terminal state: `Some(exit code)` when the runner
    /// passed (0) or failed (1); `None` while it runs. Proof lines are
    /// returned, not printed, so a headed loop can hold them until after
    /// alternate-screen restore. The shared live gate decides exactly as the
    /// panel watches do: the witness this run armed fails the run when it
    /// fails, and holds the PASS while Pending, before the clean-stop grace.
    fn live_status(&mut self) -> (Option<i32>, Vec<ProofLine>) {
        // The UI loop polls this every frame, interactive use included:
        // without a `--live` run there is nothing to decide, so read no
        // clock, take no lock and touch no witness.
        let Some(name) = self.live_name.as_deref() else {
            return (None, Vec::new());
        };
        // Only the witness this run armed can decide it, borrowed from the
        // run; a plain `--live` run has none and evaluates no gate.
        let (core_gate, pair_gate) = match &self.live_witnesses {
            None => (CoreGate::Disabled, CoreGate::Disabled),
            Some(armed) => {
                let now = Instant::now();
                (
                    live_gate::catalog_core_gate(
                        Some(&armed.catalog),
                        self.live_core_deadline,
                        now,
                    ),
                    live_gate::pair_core_gate(Some(&armed.pair), self.live_core_deadline, now),
                )
            }
        };
        let catalog = self.live_witnesses.as_ref().map(|armed| &armed.catalog);
        let pair = self.live_witnesses.as_ref().map(|armed| &armed.pair);
        // The witness receipts ride with the terminal line: stdout before a
        // PASS, stderr before a FAIL, labelled with the `--live` token.
        let witnesses = |ok: bool| {
            let label = format!("script_{name}");
            [
                (
                    live_gate::CATALOG_CORE_TAG,
                    live_gate::catalog_core_record(catalog, &core_gate),
                ),
                (
                    live_gate::PAIRED_CORE_TAG,
                    live_gate::pair_core_record(pair, &pair_gate),
                ),
            ]
            .into_iter()
            .filter_map(|(tag, witness)| witness.map(|witness| format!("{tag}: {label} {witness}")))
            .map(|line| {
                if ok {
                    ProofLine::Stdout(line)
                } else {
                    ProofLine::Stderr(line)
                }
            })
            .collect::<Vec<_>>()
        };
        let fail = |receipt: String, message: &str| {
            let mut lines = witnesses(false);
            lines.push(ProofLine::Stderr(receipt));
            lines.push(ProofLine::Stderr(format!("FAIL: {message}")));
            (Some(1), lines)
        };
        if let Some(message) = self.terminal_startup_failure() {
            return fail(format!("FAIL: live {name} startup: {message}"), &message);
        }
        for gate in [&core_gate, &pair_gate] {
            if let CoreGate::Failed(message) = gate {
                return fail(format!("FAIL: live {name} {message}"), message);
            }
        }
        let status = self.scenario.lock().unwrap().as_ref().map(|r| r.status());
        if let Some(scenario::RunnerStatus::Passed) = &status {
            let mut grace = self.live_stop_wait_started;
            let hold = live_gate::pass_hold(
                &[&core_gate, &pair_gate],
                self.live_wait_script_stop,
                |needle| self.script_self_stop_observed(needle),
                &mut grace,
                Instant::now(),
            );
            self.live_stop_wait_started = grace;
            match hold {
                PassHold::Core | PassHold::CleanStop => return (None, Vec::new()),
                PassHold::TimedOut(message) => {
                    return fail(format!("FAIL: live {name} {message}"), &message);
                }
                PassHold::Release { .. } => {}
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
        let (code, proof, announced) =
            live_proof(name, status, &evidence, soaking, self.live_announced_pass);
        let mut lines = match code {
            Some(1) => witnesses(false),
            _ if announced && !self.live_announced_pass => witnesses(true),
            _ => Vec::new(),
        };
        lines.extend(proof);
        self.live_announced_pass = announced;
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
        let generation = self.core.resource_generation();
        if self.notice_sig != Some((background, generation)) {
            app.background_notice = Some(background_ack_text(background, self.core.resources()));
            self.notice_sig = Some((background, generation));
        }
    }

    /// Copy the core's shared projection into the app: fleet rows and
    /// their counts, the selected slot's detail and the resource meter, each
    /// only when it moved since the last copy (and into the app's own
    /// buffers).
    fn copy_projection(&mut self, app: &mut TuiApp) {
        let view = self.core.fleet_view();
        if self.copied_rows.replace(view.rows_generation()) != Some(view.rows_generation()) {
            view.copy_rows_into(&mut app.fleet);
            app.counts = view.counts();
        }
        if self.copied_view.replace(view.generation()) != Some(view.generation()) {
            view.copy_detail_into(&mut app.detail);
        }
        let meter = self.core.resource_generation();
        if self.copied_meter.replace(meter) != Some(meter) {
            app.resources.clone_from(self.core.resources());
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
fn new_app(title: impl Into<String>) -> TuiApp {
    let mut app = TuiApp::new(title);
    app.restore_map_preferences(host_play::panel_ui_path());
    app
}

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
            .ok_or("--world requires the rs2b2t profile")?;
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
        host_play::memory::require_live_benchmark(session.profile_class())?;
        session.persist_ui = false;
        // Vault/card first; unlock constructs Play once (single load_pack).
        let run = host_play::memory::Run::prepare_unseeded(config, "tui")?;
        session.options.mainland = true;
        session.unlock_at(&run.vault, &run.pass)?;
        run.bind_seed_nav(host_play::memory::SeedNav::FromPlay {
            world: session.core.play().and_then(|p| p.world()),
            obj_names: session.core.play().map(|p| p.obj_names()),
            map_members: session.core.play().is_some_and(|p| p.map_members()),
        })?;
        session.names = run.names.clone();
        session.load_and_login_all();
        session.focus(&run.names[0]);
        let mut app = new_app(format!(
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
            session.live_catalog_core = args.catalog_core;
            session.live_pair_core = args.pair_core;
            session.live_prepare_script(scenario)?;
            let mut app = new_app(format!(
                "tui-play --live {name} · {}",
                session.profile_label()
            ));
            app.names = session.names.clone();
            app.focused = Some(0);
            run_loop(session, app)
        }
        RunMode::Interactive => {
            let vault_path = session
                .server_profile
                .as_ref()
                .expect("bound session")
                .vault_path()
                .to_path_buf();
            let vault_exists = vault_path.is_file();
            let pass = host_play::passphrase::obtain(
                "tui-play",
                args.pass_stdin,
                host_play::passphrase::Purpose::for_vault(vault_exists),
            )?;
            if let Err(e) = session.unlock_at(&vault_path, &pass) {
                return Err(format!("vault {}: {e}", vault_path.display()));
            }
            // The passphrase has done its job; do not keep it through the run.
            drop(pass);
            // A fresh vault starts empty: no seeded `test` profile (M-311).
            // Pass `--user` (or use host-play) to create the first profile.
            let focus = session.bootstrap_interactive_profiles(&args.users)?;
            session.load_and_login(&focus);
            session.focus(&focus);
            let mut app = new_app(format!(
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
    /// Profiles for an interactive boot: create missing `--user` names,
    /// then focus the first `--user` (or the vault's first profile). A
    /// fresh vault stays empty — no seeded `test` profile (M-311) — so
    /// booting one without `--user` is an honest error, not a dead end.
    fn bootstrap_interactive_profiles(&mut self, users: &[String]) -> Result<String, String> {
        // `--user` names may not exist: create them like host-play does.
        for u in users {
            if self.core.vault().is_none_or(|v| v.get(u).is_none()) {
                self.create_profile(u)?;
            }
        }
        self.names = self
            .core
            .vault()
            .map(|v| v.profiles().map(|p| p.username.clone()).collect())
            .unwrap_or_default();
        users
            .first()
            .cloned()
            .or_else(|| self.names.first().cloned())
            .ok_or_else(|| {
                "vault has no profiles (start tui-play with --user <name> to create one)".into()
            })
    }

    /// Create a missing profile (fresh game secret, uid one past the
    /// vault's max, from the 274M base).
    fn create_profile(&mut self, username: &str) -> Result<(), String> {
        let uid = self
            .core
            .vault()
            .map(|v| v.profiles().map(|p| p.uid).max().unwrap_or(274_000_000) + 1)
            .unwrap_or(274_000_001);
        let profile = Profile {
            username: username.into(),
            password: profile_password(username).into(),
            uid,
            settings: vault::ProfileSettings::default(),
        };
        // Startup only (before the terminal loop): waiting on the write
        // here keeps a failed first-run profile a startup error.
        let op = self
            .core
            .create_profile(profile, frontend_core::ArmMirror::None, "profile")?;
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
                    app.draw_params_overlay(frame, &session.loadouts, params_data.as_deref());
                })
                .map_err(|e| e.to_string())?;
        }
        if event::poll(Duration::from_millis(50)).map_err(|e| e.to_string())? {
            // One routing model: the app decides which pane, popup or
            // overlay an event reaches (params/loadouts come back as
            // actions that need the session). A resize needs nothing here:
            // the next draw lays out for the new size and re-records every
            // hit region before another event is read.
            let action = match event::read().map_err(|e| e.to_string())? {
                Event::Key(k) if k.kind == KeyEventKind::Press => app.on_key(k),
                Event::Mouse(m) => app.on_mouse(m),
                _ => AppAction::None,
            };
            dispatch(&mut session, &mut app, action);
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
/// `host_play::arm_walk_on`, chat and manual walks go through [`WireCmd`],
/// the script actions dispatch `Play::script_start_load` / pause / stop /
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
        AppAction::MemoryRelog(name) => session.memory_relog_request(app, &name),
        AppAction::MemoryRelogNow(name) => session.memory_relog_now(app, &name),
        AppAction::Remove(name) => session.remove(app, &name),
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
                    if session.scripts.js.load_failures().is_empty() {
                        None
                    } else {
                        Some(session.scripts.js.named_failure_output())
                    }
                }
                Err(e) => Some(e),
            };
        }
        AppAction::ScriptLoad(path) => session.script_load(app, &path),
        AppAction::ScriptParams => session.open_params(app),
        AppAction::ScriptStartAll => session.script_start_all(app),
        AppAction::ScriptStopAll => session.script_stop_all(app),
        AppAction::ScriptAssignMarked => session.script_assign_marked(app),
        AppAction::ScriptRestartMarked => session.script_restart_marked(app),
        AppAction::ScriptApplyMarkedPrepare => session.prepare_apply_settings_marked(app),
        AppAction::LoginMarked => session.login_marked(app),
        AppAction::LogoutMarked => session.logout_marked(app),
        AppAction::ScriptReload => session.script_reload(app),
        AppAction::ScriptReloadCancel => session.script_reload_cancel(app),
        AppAction::ScriptSyncPrepare => session.prepare_settings_sync(app),
        AppAction::ScriptSyncApply => session.apply_settings_sync(app),
        AppAction::ScriptSyncCancel => session.scripts.cancel_settings_sync(),
        AppAction::AckBackground => session.ack_background_bots(app),
        AppAction::ParamsKey(key) => {
            let action = session.params_key(app, key);
            dispatch(session, app, action);
        }
        AppAction::LoadoutsKey(key) => {
            app.loadouts_on_key(&mut session.loadouts, key);
        }
        AppAction::MouseCapture(on) => set_mouse_capture(on),
        AppAction::Batch(actions) => {
            for action in actions {
                dispatch(session, app, action);
            }
        }
        AppAction::None => {}
    }
}

/// Load every profile and log every member in (confirmed in the app).
fn multibox_key(session: &mut TuiSession, app: &mut TuiApp) {
    let loaded = session.load_and_login_all();
    app.error = session
        .error
        .take()
        .or_else(|| (loaded > 0).then(|| format!("loaded {loaded} member(s)")));
}

/// Apply the operator's mouse-capture choice to the live terminal. Off lets
/// the terminal select and copy text; keyboard keeps working either way.
fn set_mouse_capture(on: bool) {
    if !ALT_SCREEN.load(Ordering::SeqCst) {
        return;
    }
    let mut stdout = std::io::stdout();
    let _ = if on {
        crossterm::execute!(stdout, crossterm::event::EnableMouseCapture)
    } else {
        crossterm::execute!(stdout, crossterm::event::DisableMouseCapture)
    };
}

pub fn main() -> ExitCode {
    let args = parse_args();
    host_play::passphrase::warn_legacy_env("tui-play");
    if let Some(out) = &args.map_bundle {
        return map_bundle(&args.profile, out);
    }
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

/// `--map-bundle OUT` (release packaging, `tools/release/package.py`): bake
/// the shipped WalkTo map terrain for the pinned client cache into
/// `OUT/map/<revision>/` and print its description (JSON) on stdout.
/// Progress goes to stderr at most once a second.
fn map_bundle(profile: &ProfileOptions, out: &Path) -> ExitCode {
    let (Some(revision), Some(cache), Some(unpack)) = (
        profile.revision.as_deref(),
        profile.cache_dir.as_deref(),
        profile.unpack_dir.as_deref(),
    ) else {
        usage();
    };
    let revision = match host_play::parse_revision(revision) {
        Ok(revision) => revision.as_i32() as u16,
        Err(error) => {
            eprintln!("tui-play: {error}");
            return ExitCode::from(2);
        }
    };
    let mut reported: Option<(Instant, MapStage)> = None;
    let result = host_play::bake_shipped_map_images(revision, cache, unpack, out, |status| {
        let MapJobStatus::Running(progress) = status else {
            return;
        };
        if reported.is_none_or(|(at, stage)| {
            stage != progress.stage || at.elapsed() >= Duration::from_secs(1)
        }) {
            eprintln!("tui-play: map bundle: {}", progress.message);
            reported = Some((Instant::now(), progress.stage));
        }
    });
    match result
        .map_err(|error| error.to_string())
        .and_then(|shipped| serde_json::to_string(&shipped).map_err(|error| error.to_string()))
    {
        Ok(json) => {
            println!("{json}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("tui-play: map bundle: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
#[path = "bin_tests.rs"]
mod tests;
