//! `tui-play`: the headless operator panel binary. Same flag spirit as
//! `host-play` / `panel-play` (`--vault`, `--vault-pass-stdin`,
//! `--host`, `--port`, `--cache`, `--user`, `--live script_<name>`).
//!
//! The operator lifecycle (vault, the play session, fleet membership,
//! selection, Load/Log in/Log out/Remove, script Start/Stop settlement and
//! status polling) lives in the shared [`frontend_core::OperatorSession`];
//! the core publishes each slot's snapshot and steps its walk arm, the
//! binary's per-frame hook adds mainland seeding and the `--live` runner,
//! and the UI loop polls the core, refreshes [`TuiApp`], routes
//! keys/clicks, and dispatches the returned [`AppAction`] — map
//! Walk-confirm consumes a revision-bound map command, chat
//! Continue/Answer and WASD walks go through the core's slot accessors,
//! and the settings popup writes `ProfileSettings` on the focused profile.
//!
//! **Raster Off:** every profile is spawned with `RasterMode::Off` and the
//! TUI never attaches a `Renderer` (no `panel` / imgui / wgpu anywhere).
//! `--live script_<name>` mints per-run usernames into an ephemeral vault
//! (like the panel) and PASS/FAIL comes from the scenario runner, not a
//! screenshot.

use std::collections::HashSet;
use std::env;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use client::client::Client;
use crossterm::event::{self, Event, KeyEventKind};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use nav_planner::tile::Tile;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use frontend_core::resources::background_ack_text;
#[cfg(feature = "js")]
use frontend_core::scripts::{CardSource, CatalogActionKind, OwnedSchema};
use frontend_core::scripts::{CompiledCardId, LoadoutForm, PaintFrame, ScriptChoice};
use frontend_core::selection::ProfileIdentity;
#[cfg(test)]
use frontend_core::test_support::VaultFixture;
use frontend_core::walk_map::{
    peek_walk_catalogue, WalkMapDemand, WalkMapJobState, WalkMapStage, WalkReadyCatalogue,
};
use frontend_core::UiPreferences;
use frontend_core::{
    load_map_bake_choice, GuardianEdit, MapDesk, OperatorCommand, OperatorSession, Setting,
    SubmitError, SurvivalEdit, Targets,
};
#[cfg(test)]
use host_play::{PlayOptions, SharedClientTemplate};

use crate::app::{AppAction, ChatData, ErrorNeed, MapCatalogueStatus, SettingFeedback, TuiApp};
use crate::chat::ChatAction;
use crate::commands::Unavailable;
use crate::loadouts::LoadoutsIntent;
use crate::overlay::{ConfirmKind, Modal};
#[cfg(feature = "js")]
use crate::profile_import::ImportAction;
#[cfg(feature = "js")]
use crate::script_shape::rs2b0t_root_has_index;
use crate::script_shape::{categories_present, resolve_category_order, BrowseCard};
#[cfg(feature = "js")]
use frontend_core::rs2b0t_import::Stage;

/// What `tui-play` should do this run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunMode {
    /// Interactive operator panel.
    Interactive,
    /// `--live script_<name>`: PASS/FAIL from the scenario runner.
    Live(String),
}

/// Parsed `tui-play` flags.
#[derive(Debug)]
pub struct Args {
    /// `--vault-pass-stdin`: read the vault passphrase from a piped stdin. It
    /// is never an argument or an environment variable; on a terminal it is
    /// asked for at a hidden prompt.
    pub pass_stdin: bool,
    pub users: Vec<String>,
    pub live: Option<String>,
    pub world: Option<u16>,
    pub profile: frontend_core::launch::ProfileArgs,
    /// `--catalog-core`: qualify the `--live` run under the shared catalog
    /// core witness (the panel `catalog_watch` mode).
    pub catalog_core: bool,
    /// `--pair-core`: qualify a paired `--live` run under the shared pair
    /// witness (the panel `pair_watch` mode).
    pub pair_core: bool,
    /// `--map-bundle OUT`: release packaging only; bake the shipped WalkTo
    /// map bundle (catalogue and terrain) for `--revision`, `--cache` and
    /// `--unpack` under `OUT` and exit.
    pub map_bundle: Option<PathBuf>,
}

const USAGE: &str = "usage: tui-play [--profile NAME|--rs2b2t] [--revision 274|289] \
         [--host HOST] [--port PORT] [--asset-host HOST] [--http-port PORT] \
         [--engine DIR] [--cache DIR] [--unpack DIR] [--nav-pack PATH] [--nav-flags PATH] \
         [--content DIR] [--vault PATH] [--catalog DIR] [--cache-manifest PATH] [--vault-pass-stdin] \
         [--world N] [--live script_<name> [--catalog-core | --pair-core]] [--user USER]... \
         (default user: first profile in saved order)\n\
         release packaging: tui-play --map-bundle OUT --revision 274|289 --cache JAG_DIR \
         --unpack SNAPSHOT_ROOT (bakes the shipped WalkTo map bundle into OUT/map/<revision>)";

fn exit_after_work_shutdown(code: i32) -> ! {
    let code = match frontend_core::process::shutdown_background_work() {
        Ok(()) => code,
        Err(error) => {
            api::host_log!(stderr; api::hostlog::Category::Lifecycle, api::hostlog::Level::Error, "background work shutdown: {error}");
            1
        }
    };
    std::process::exit(code)
}

/// Usage on stderr for a bad invocation (exit 2, the CLI family's convention).
fn usage() -> ! {
    eprintln!("{USAGE}");
    exit_after_work_shutdown(2);
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
            exit_after_work_shutdown(0);
        }
        Err(msg) => {
            eprintln!("{msg}");
            usage();
        }
    }
}

/// Testable CLI parse.
pub fn parse_args_from(args: impl IntoIterator<Item = impl AsRef<str>>) -> Result<Args, String> {
    let (profile, rest) =
        frontend_core::launch::parse_profile_args(args).map_err(|e| format!("tui-play: {e}"))?;
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
        if frontend_core::launch::is_removed_passphrase_flag(arg.as_ref()) {
            return Err(frontend_core::launch::removed_passphrase_flag_error(
                "tui-play",
            ));
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
        && (!parsed.profile.has_revision()
            || !parsed.profile.has_cache_dir()
            || !parsed.profile.has_unpack_dir()
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
pub fn live_scenario(name: &str) -> Result<frontend_core::live::Scenario, String> {
    let script = name
        .strip_prefix("script_")
        .ok_or_else(|| format!("tui-play: --live {name}: only script_<name> is supported"))?;
    frontend_core::live::Scenario::get(script)
        .ok_or_else(|| format!("tui-play: --live {name}: unknown scenario"))
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

/// The TUI's per-frame client work on every slot. The core already
/// publishes the frame and runs the shared live harness; this adds TUI-owned
/// mainland seeding.
struct TuiFrameHook {
    /// Hop a fresh slot to the mainland once in a loaded scene.
    mainland: bool,
    mainland_sent: Mutex<HashSet<String>>,
}

impl frontend_core::frames::FrameHook for TuiFrameHook {
    fn frame(
        &self,
        client: &mut dyn std::any::Any,
        name: &str,
        frame: &frontend_core::frames::FrameContext<'_>,
    ) {
        let c = client
            .downcast_mut::<Client>()
            .expect("tui slots run the client this TUI links");
        let Self {
            mainland,
            mainland_sent,
        } = self;
        if frame.session_boundary() {
            frame.mark_walk_clear();
        }
        if seed_mainland_on_ready(
            c,
            mainland_sent,
            name,
            *mainland,
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
    }
}

/// The headless slot surface: every spawn raster-off with no per-slot
/// devices. A lifetime reset that ended an armed walk flags the picked walk
/// destination for clearing.
struct TuiSurface<'a> {
    walk_cleared: &'a mut bool,
    map_desk: &'a mut MapDesk,
    models: frontend_core::Models<'a>,
    /// The TUI's per-frame client work, lent only to the vault rows that
    /// start the play.
    frame_hook: Option<Arc<dyn frontend_core::frames::FrameHook>>,
}

impl frontend_core::SlotSurface for TuiSurface<'_> {
    type Io = ();

    fn attach(
        &mut self,
        _name: &str,
        profile: &mut frontend_core::SpawnProfile<'_>,
        _retained: Option<()>,
    ) -> frontend_core::SlotAttach<()> {
        profile.set_raster(frontend_core::command::RasterMode::Off);
        frontend_core::SlotAttach::new((), None, None)
    }

    fn lifetime_reset(&mut self, _name: &str, walk_was_armed: bool) {
        if walk_was_armed {
            self.map_desk.clear_walk_dest();
            *self.walk_cleared = true;
        }
    }

    fn released(&mut self, _name: &str) {}

    fn models(&mut self) -> frontend_core::Models<'_> {
        self.models.reborrow().with_map(&mut *self.map_desk)
    }

    /// The TUI draws no game image and captures no game input.
    fn draw_changed(&mut self, _switch: frontend_core::command::DrawSwitch, _on: bool) {}

    fn frame_hook(&mut self) -> Option<Arc<dyn frontend_core::frames::FrameHook>> {
        self.frame_hook.take()
    }
}

/// The stage of the session file the log pane's status line follows.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum SessionFileKey {
    /// Nothing to say: no file was ever on, or a failure has left.
    #[default]
    Quiet,
    /// The saved choice is being written.
    Saving,
    /// The core holds a failure at the setting (the choice did not save, or
    /// it saved and the file could not be opened or written).
    Failure,
    /// The file is being opened.
    Opening,
    /// Lines are written to the file.
    Writing,
    /// The file is closed.
    Off,
}

/// What the log pane's status line last said about the session file: the
/// stage, the text put on the line for it, and whether a file was open since
/// the line last said it was off. A frame compares the stage and the
/// core's failure text with these and builds new text only when one moved.
#[derive(Debug, Default)]
struct SessionFileShown {
    key: SessionFileKey,
    text: Option<String>,
    was_open: bool,
}

/// The TUI session: the running play, the vault and the per-username walk
/// arms (the core owns the per-slot snapshot publication).
pub struct TuiSession {
    #[cfg(feature = "memory-profile")]
    memory: Option<frontend_core::bench::BenchRun>,
    /// Shared operator lifecycle (vault, play, fleet, selection, removals,
    /// status polling and operation results). Headless: no per-slot IO.
    core: OperatorSession<()>,
    auto_world: Option<u16>,
    pub error: Option<String>,
    /// The banner message's expiry clock ([`Self::expire_error`]).
    error_expiry: frontend_core::ErrorExpiry<ErrorNeed>,
    /// All profile names (for the strip's slot list), in vault order.
    names: Vec<String>,
    /// Hop a fresh slot to the mainland once in a loaded scene (the TUI's own
    /// seed; the host never sees it).
    mainland: bool,
    /// The server-profile binding: its prepared template, connection
    /// settings and the launch the next play session uses.
    launcher: frontend_core::launch::ProfileLauncher,
    /// Core-owned map demand, bake consent and accepted walk destination.
    map_desk: MapDesk,
    map_catalogue_named: bool,
    /// The username the settings popup currently edits; reload
    /// `ProfileSettings` into the app when it changes.
    last_focused: Option<String>,
    /// A slot lifetime reset ended an armed walk: the picked walk
    /// destination clears on the next pump.
    walk_reset: bool,

    /// The `--live` run's scenario name, for the PASS/FAIL label.
    live_name: Option<String>,
    /// `--catalog-core` / `--pair-core`: the shared witness the next
    /// `--live` prepare arms.
    live_catalog_core: bool,
    live_pair_core: bool,
    /// The armed witness's own ceiling (`None` without a core gate).
    live_core_deadline: Option<Instant>,
    /// `BUDGET_S` soak: keep pumping after proof PASS until this instant.
    live_soak_until: Option<Instant>,
    live_announced_pass: bool,
    live_stop_wait_started: Option<Instant>,
    /// The Browse-selected card (catalog Start after seed, or operator Start).
    script_sel: Option<ScriptChoice>,
    /// Shared script coordination (card library and catalog, assignment,
    /// per-profile parameters, Start/Stop fleet, reload, Apply to all): the
    /// same owner the panel uses.
    scripts: frontend_core::Scripts,
    /// First-run rs2b0t clone-root folder browser.
    #[cfg(feature = "js")]
    rs2b0t_catalog_open: bool,
    #[cfg(feature = "js")]
    rs2b0t_catalog_dir: PathBuf,
    /// The rs2b0t profile import shown by [`Modal::ProfileImport`].
    #[cfg(feature = "js")]
    profile_import: frontend_core::rs2b0t_import::ProfileImport,
    /// Preview waiting for the first asynchronous catalog scan.
    #[cfg(feature = "js")]
    profile_import_preview_pending: bool,
    /// The shared profile form behind [`Modal::NewProfile`]: the same
    /// editor the panel's Profiles window binds, opened only on its
    /// blank new-profile row. The settings popup keeps its own
    /// form-save model, so this editor is lent only to the New profile
    /// rows (open, draft edits, create, close).
    profile_editor: frontend_core::ProfileEditor,
    /// The username a New profile Save is still writing, while its
    /// overlay waits for the write to settle.
    new_profile_pending: Option<String>,
    /// Browse category order keys (in-memory; panel persists to panel-ui.json).
    script_category_order: Vec<String>,
    /// The process local-state service's shared operator preferences
    /// (`panel-ui.json` through the service): every preference read and
    /// write in this process goes through this clone.
    preferences: UiPreferences,
    /// Process-wide loadout presets: the form's editor over the process
    /// local-state service (slots read the same file through it).
    loadouts: LoadoutForm,
    /// Last directory visited in the out-of-tree Load file browser.
    #[cfg(feature = "js")]
    script_load_last_dir: Option<PathBuf>,
    /// Core projection generations the app last copied (fleet rows, the
    /// whole view, the resource meter, the capacity warning): a pump copies
    /// only what moved.
    copied_rows: Option<u64>,
    copied_view: Option<u64>,
    copied_meter: Option<u64>,
    copied_capacity: Option<u64>,
    /// The fleet result last shown on the message line: its command and its
    /// text, so a result is copied there only when it changed.
    shown_result: Option<(Option<api::identity::RequestId>, String)>,
    persist_ui: bool,
    /// The retained core error last projected, so expiry does not show it again.
    shown_preference_error: Option<String>,
    /// The Load-order failure last put on the message line.
    shown_order_failure: Option<String>,
    /// What the log pane's status line last said about the session file
    /// ([`TuiSession::refresh_session_log_status`]).
    session_file_shown: SessionFileShown,
    /// Unique profiles queued for startup Load + Login, keyed by vault UID.
    startup_profiles: Vec<ProfileIdentity>,
    /// The last queued profile that should receive automatic startup focus.
    startup_focus: Option<ProfileIdentity>,
    /// Defer the default boot choice until shared startup preparation orders profiles.
    startup_default_pending: bool,
    /// The operator superseded only startup focus. Queued loads remain.
    startup_focus_cancelled: bool,
    background_bots_acked: bool,
    ack_checked_at: Option<Instant>,
    /// Background count and resource generation the notice was built for.
    notice_sig: Option<(usize, u64)>,
    /// The acknowledgement state the notice text was built with.
    notice_feedback: Option<SettingFeedback>,
    /// Keep test sessions from replacing the process-wide Quester folder
    /// registry while another test is using its snapshot.
    #[cfg(test)]
    _quester_paths_test_guard: seam::QuesterPathsTestGuard,
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

/// A real command seam for app-level tests: the actions a key returns are
/// applied through the dispatcher, over a session on isolated stores and
/// the models the app lends, as the event loop does.
#[cfg(test)]
pub(crate) mod seam {
    use super::*;
    use crossterm::event::KeyEvent;

    static QUESTER_PATHS_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    std::thread_local! {
        static QUESTER_PATHS_TEST_LOCK_DEPTH: std::cell::Cell<usize> =
            const { std::cell::Cell::new(0) };
        static QUESTER_PATHS_TEST_MUTEX_GUARD: std::cell::RefCell<
            Option<std::sync::MutexGuard<'static, ()>>,
        > = const { std::cell::RefCell::new(None) };
    }

    pub(crate) struct QuesterPathsTestGuard {
        _not_send: std::marker::PhantomData<std::rc::Rc<()>>,
    }

    impl Drop for QuesterPathsTestGuard {
        fn drop(&mut self) {
            let last_guard = QUESTER_PATHS_TEST_LOCK_DEPTH.with(|depth| {
                let next = depth
                    .get()
                    .checked_sub(1)
                    .expect("the Quester test lock depth is positive");
                depth.set(next);
                next == 0
            });
            if last_guard {
                QUESTER_PATHS_TEST_MUTEX_GUARD.with(|slot| {
                    let guard = slot.borrow_mut().take();
                    debug_assert!(guard.is_some());
                    drop(guard);
                });
            }
        }
    }

    /// Isolated stores do not isolate the process-global Quester registry, so
    /// test sessions hold this lock for their lifetime. Helpers may acquire it
    /// while a session owns it, so the per-thread guard is reentrant.
    pub(crate) fn lock_quester_paths_test_state() -> QuesterPathsTestGuard {
        let nested = QUESTER_PATHS_TEST_LOCK_DEPTH.with(|depth| depth.get() != 0);
        if !nested {
            let guard = QUESTER_PATHS_TEST_LOCK
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            QUESTER_PATHS_TEST_MUTEX_GUARD.with(|slot| {
                assert!(slot.borrow().is_none());
                *slot.borrow_mut() = Some(guard);
            });
        }
        QUESTER_PATHS_TEST_LOCK_DEPTH.with(|depth| {
            depth.set(
                depth
                    .get()
                    .checked_add(1)
                    .expect("the Quester test lock depth fits in usize"),
            );
        });
        QuesterPathsTestGuard {
            _not_send: std::marker::PhantomData,
        }
    }

    pub(crate) struct Seam {
        session: TuiSession,
    }

    impl Seam {
        /// A session on isolated stores: no play and no vault.
        pub(crate) fn new() -> Self {
            let mut launcher = frontend_core::launch::ProfileLauncher::legacy_local();
            launcher.set_play_host("127.0.0.1");
            let session =
                TuiSession::with_launcher(launcher, frontend_core::launch::InstancePermit::skip());
            Self { session }
        }

        /// Restore preferences while serializing access to the process-wide
        /// Quester source registry shared by tests.
        pub(crate) fn restore_preferences(
            app: &mut TuiApp,
            preferences: frontend_core::UiPreferences,
        ) {
            let _guard = lock_quester_paths_test_state();
            app.restore_preferences(preferences);
        }

        /// A session over a vault holding `profiles` (name, uid) and an
        /// empty play: a command that names a profile resolves it to its
        /// vault identity.
        pub(crate) fn with_profiles(profiles: &[(&str, i32)]) -> Self {
            let mut seam = Self::new();
            seam.session.core.set_spawn_workers(false);
            let path = frontend_core::scripts::paths::bot_home().join("seam-vault");
            let mut vault = VaultFixture::create(&path, "test-passphrase-01");
            for (name, uid) in profiles {
                vault.put(frontend_core::test_support::FixtureProfile::new(name, "pw").uid(*uid));
            }
            let play = frontend_core::test_support::empty_play(&PlayOptions {
                host: "127.0.0.1".into(),
                transport: client::Transport::Tcp,
                port: 43594,
                cache_dir: "/tmp".into(),
                lowmem: true,
                mainland: false,
            });
            vault.start_session(&mut seam.session.core, play);
            seam
        }

        /// Apply `action` as the event loop does.
        pub(crate) fn apply(&mut self, app: &mut TuiApp, action: AppAction) {
            dispatch(&mut self.session, app, action);
        }

        /// Poll the session as the event loop does.
        pub(crate) fn pump(&mut self, app: &mut TuiApp) {
            self.session.pump(app);
        }

        /// Send `key` to `app`, check that it returns `expected`, then
        /// apply that action.
        pub(crate) fn press(&mut self, app: &mut TuiApp, key: KeyEvent, expected: AppAction) {
            let action = app.on_key(key);
            assert_eq!(action, expected);
            self.apply(app, action);
        }

        /// Send `key` to `app` and apply whatever action it returns.
        pub(crate) fn send(&mut self, app: &mut TuiApp, key: KeyEvent) {
            let action = app.on_key(key);
            self.apply(app, action);
        }

        /// Commit `text` as the fleet filter of a bare fleet model
        /// (`SetFleetFilter`).
        pub(crate) fn filter(&mut self, model: &mut frontend_core::FleetModel, text: &str) {
            let staged = model.staged_filter_mut();
            staged.clear();
            staged.push_str(text);
            let mut walk_cleared = false;
            let mut desk = MapDesk::default();
            let mut surface = TuiSurface {
                walk_cleared: &mut walk_cleared,
                map_desk: &mut desk,
                models: frontend_core::Models::none().with_fleet(model),
                frame_hook: None,
            };
            self.session
                .core
                .submit(
                    OperatorCommand::SetFleetFilter,
                    Targets::Application,
                    &mut surface,
                )
                .expect("SetFleetFilter opens");
        }

        /// Make the session's preferences refuse writes (a file past the
        /// size bound), as a damaged store does.
        pub(crate) fn refuse_preference_writes(&mut self) {
            let path = self.session.preferences.path().to_path_buf();
            crate::test_support::write_private_fixture(
                &path,
                vec![b' '; host_play::local_state::MAX_PREFERENCES_BYTES as usize + 1],
            )
            .expect("write the oversized preferences file");
            assert!(self.session.preferences.reload().is_err());
        }

        /// Undo [`Self::refuse_preference_writes`]: a valid empty document.
        pub(crate) fn allow_preference_writes(&mut self) {
            let path = self.session.preferences.path().to_path_buf();
            crate::test_support::write_private_fixture(&path, b"{}")
                .expect("write an empty preferences document");
            self.session
                .preferences
                .reload()
                .expect("the empty document loads");
        }
    }
}

impl TuiSession {
    /// Empty session over the legacy local connection (`options.host`).
    #[cfg(test)]
    fn new(options: PlayOptions) -> Self {
        let mut launcher = frontend_core::launch::ProfileLauncher::legacy_local();
        launcher.set_play_host(&options.host);
        Self::with_launcher(launcher, frontend_core::launch::InstancePermit::skip())
    }

    /// A session bound to the prepared host `template`.
    #[cfg(test)]
    fn new_bound(template: Arc<host_play::SharedClientTemplate>) -> Self {
        let mut launcher = frontend_core::launch::ProfileLauncher::legacy_local();
        launcher.install_template(frontend_core::launch::ProfileTemplate::from_host(template));
        Self::with_launcher(launcher, frontend_core::launch::InstancePermit::skip())
    }

    fn with_launcher(
        launcher: frontend_core::launch::ProfileLauncher,
        permit: frontend_core::launch::InstancePermit,
    ) -> Self {
        #[cfg(test)]
        let _quester_paths_test_guard = seam::lock_quester_paths_test_state();
        #[cfg(test)]
        frontend_core::test_support::IsolatedStores::ensure_thread();
        // A missing store is a first run; a refused or corrupt one is said out
        // loud (and nothing is saved over a refused one).
        let (scripts, restore_error) = frontend_core::Scripts::open_default();
        if let Some(error) = restore_error {
            eprintln!("tui-play: {error}");
        }
        // The composition root opens the process local-state service once;
        // every consumer clones these preferences and the loadouts form
        // writes through the same service.
        let mut core = OperatorSession::open(permit);
        core.open_process_state();
        let loadouts = LoadoutForm::operator_of(&core);
        let preferences = core
            .preferences()
            .expect("the session's local state was bound just now");
        // The bound operator preferences (the durable walk globals and
        // their store, the manual-movement pause) become the session's;
        // the app reads its walk-permission view itself when it binds them.
        core.adopt_operator_preferences();
        // This front end draws no game image: the capacity warning says so.
        core.declare_headless_render();
        let background_ack = frontend_core::resources::background_bots_acked(&preferences);
        Self {
            #[cfg(feature = "memory-profile")]
            memory: None,
            core,
            auto_world: None,
            error: None,
            error_expiry: frontend_core::ErrorExpiry::default(),
            names: Vec::new(),
            mainland: false,
            launcher,
            map_desk: MapDesk::new(load_map_bake_choice(&preferences)),
            map_catalogue_named: false,
            last_focused: None,
            walk_reset: false,
            live_name: None,
            live_catalog_core: false,
            live_pair_core: false,
            live_core_deadline: None,
            live_soak_until: None,
            live_announced_pass: false,
            live_stop_wait_started: None,
            script_sel: None,
            scripts,
            #[cfg(feature = "js")]
            rs2b0t_catalog_open: false,
            #[cfg(feature = "js")]
            rs2b0t_catalog_dir: Self::default_catalog_browse_dir(),
            #[cfg(feature = "js")]
            profile_import: frontend_core::rs2b0t_import::ProfileImport::new(),
            #[cfg(feature = "js")]
            profile_import_preview_pending: false,
            profile_editor: frontend_core::ProfileEditor::new(),
            new_profile_pending: None,
            script_category_order: Vec::new(),
            preferences: preferences.share(),
            loadouts,
            #[cfg(feature = "js")]
            script_load_last_dir: None,
            copied_rows: None,
            copied_view: None,
            copied_meter: None,
            shown_result: None,
            copied_capacity: None,
            persist_ui: true,
            shown_preference_error: None,
            shown_order_failure: None,
            session_file_shown: SessionFileShown::default(),
            startup_profiles: Vec::new(),
            startup_focus: None,
            startup_default_pending: false,
            startup_focus_cancelled: false,
            background_bots_acked: background_ack,
            ack_checked_at: Some(Instant::now()),
            notice_sig: None,
            notice_feedback: None,
            #[cfg(test)]
            _quester_paths_test_guard,
        }
    }

    /// The server profile this session is bound to, once bound.
    fn bound(&self) -> Option<&frontend_core::launch::BoundProfile> {
        self.launcher.bound()
    }

    #[cfg(feature = "memory-profile")]
    fn profile_class(&self) -> frontend_core::launch::ProfileClass {
        self.bound()
            .map_or(frontend_core::launch::ProfileClass::Remote, |profile| {
                profile.class()
            })
    }

    fn app_title(&self) -> String {
        let revision = self.bound().map_or(274, |profile| profile.revision());
        format!("{revision}bot")
    }

    fn profile_label(&self) -> String {
        self.bound().map_or_else(
            || "legacy local-274 · revision 274".into(),
            |profile| profile.label(),
        )
    }

    #[cfg(feature = "js")]
    fn catalog_root(&self) -> Option<PathBuf> {
        match self.bound() {
            Some(profile) => profile.catalog_root().map(Path::to_path_buf),
            None => frontend_core::scripts::paths::rs2b0t_root(),
        }
    }

    /// Live harness bag: schema defaults + legacy overrides + inject.
    /// Empty schema still keeps inject keys (Thiever `target: Guard`).
    /// `None` only when the merged bag is empty.
    #[cfg(feature = "js")]
    fn pending_settings_bag(
        &self,
        source: CardSource,
        name: &str,
    ) -> Option<serde_json::Map<String, serde_json::Value>> {
        let none = OwnedSchema::default();
        let schema = self
            .scripts
            .card(source, name)
            .map_or_else(|| none.as_ref(), |card| card.schema());
        let merged = self.scripts.legacy_bag(source, name, schema);
        (!merged.is_empty()).then_some(merged)
    }

    #[cfg(feature = "js")]
    fn default_catalog_browse_dir() -> PathBuf {
        let home = frontend_core::scripts::paths::bot_home();
        if home.as_os_str() == "." {
            PathBuf::from("/")
        } else {
            home
        }
    }

    /// Unlock the vault at `path` (`UnlockVault`), or create it on a first
    /// run, when there is no vault file yet (`CreateVault`), and start the
    /// play.
    fn unlock_at(&mut self, path: &Path, pass: &str) -> Result<(), String> {
        let (launch, hook) = self.play_launch();
        let command = if path.is_file() {
            self.core
                .stage_vault_unlock(path.to_path_buf(), pass, launch);
            OperatorCommand::UnlockVault
        } else {
            self.core
                .stage_vault_create(path.to_path_buf(), pass, launch);
            OperatorCommand::CreateVault
        };
        let (core, mut surface) = self.core_and_surface();
        surface.frame_hook = Some(hook);
        let submitted = core
            .submit(command, Targets::Application, &mut surface)
            .map_err(|error| error.to_string())?;
        if let Some(reason) = self.core.command_failure(submitted.request()) {
            return Err(reason.to_string());
        }
        self.play_started();
        Ok(())
    }

    /// Start the play over an already-written fixture `vault` (tests).
    #[cfg(test)]
    fn start_play(&mut self, vault: VaultFixture) -> Result<(), String> {
        let (launch, hook) = self.play_launch();
        frontend_core::test_support::tui::start_play_with_vault(
            &mut self.core,
            vault,
            launch,
            hook,
        )?;
        self.play_started();
        Ok(())
    }

    /// What the next `Play` launches from, and the per-frame hook every
    /// slot runs. The play is empty (shared cache + FIFO); the boot then
    /// loads and logs in the focused profile only and `m` loads and logs in
    /// the rest.
    fn play_launch(&mut self) -> (frontend_core::launch::PlayLaunch, Arc<TuiFrameHook>) {
        let hook = Arc::new(TuiFrameHook {
            mainland: self.mainland,
            mainland_sent: Mutex::new(HashSet::new()),
        });
        let mut launch = self.launcher.take_launch();
        if let Some(number) = self.auto_world {
            launch = launch.with_auto_world(number);
        }
        (launch, hook)
    }

    /// Log the started play's debug trace when requested.
    fn play_started(&mut self) {
        if !self.core.has_play() {
            return;
        }
        if std::env::var_os("BOT_DEBUG").is_some() {
            let game_data = self.core.game_data();
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
    }

    /// Load `name` into the fleet and log it in (the TUI boot and `--live`
    /// intent). `RasterMode::Off` always (the headless surface never
    /// attaches a `Renderer`); after a DC the slot re-handshakes only when
    /// the profile's `auto_login` is on.
    fn load_and_login(&mut self, name: &str) -> bool {
        let failure = self
            .submit(OperatorCommand::Load, Targets::Profile(name))
            .err()
            .or_else(|| {
                self.submit(OperatorCommand::Login, Targets::Profile(name))
                    .err()
            });
        match failure {
            Some(error) => {
                self.error = Some(error);
                false
            }
            None => true,
        }
    }

    /// Issue one operator command through the core's command seam (the
    /// entry point the panel and the headless driver use). `Err` carries a
    /// refused command or the failure of the operation it opened.
    fn submit(
        &mut self,
        command: OperatorCommand,
        targets: Targets<'_>,
    ) -> Result<frontend_core::Submitted, String> {
        let submitted = self
            .submit_raw(command, targets)
            .map_err(|error| error.to_string())?;
        match submitted.operation().and_then(|op| self.core.failure(op)) {
            Some(failure) => Err(failure),
            None => Ok(submitted),
        }
    }

    /// [`Self::submit`] without reading the operation's failure.
    fn submit_raw(
        &mut self,
        command: OperatorCommand,
        targets: Targets<'_>,
    ) -> Result<frontend_core::Submitted, SubmitError> {
        let (core, mut surface) = self.core_and_surface();
        core.submit(command, targets, &mut surface)
    }

    /// Split borrow: the core plus the headless surface, whose lifetime
    /// reset flags the picked walk destination when it ended an armed walk.
    fn core_and_surface(&mut self) -> (&mut OperatorSession<()>, TuiSurface<'_>) {
        (
            &mut self.core,
            TuiSurface {
                walk_cleared: &mut self.walk_reset,
                map_desk: &mut self.map_desk,
                models: {
                    let models = frontend_core::Models::none().with_scripts(&mut self.scripts);
                    #[cfg(feature = "js")]
                    let models = models.with_import(&mut self.profile_import);
                    models.with_loadouts(&mut self.loadouts)
                },
                frame_hook: None,
            },
        )
    }

    /// [`Self::core_and_surface`] that also lends the models the app owns:
    /// the fleet's marks and filter, the Quester Paths controller, the
    /// walk-permission view and the settings form-save model. Handlers that
    /// submit rows over them build their surface here.
    fn core_and_surface_with<'a>(
        &'a mut self,
        app: &'a mut TuiApp,
    ) -> (&'a mut OperatorSession<()>, TuiSurface<'a>) {
        (
            &mut self.core,
            TuiSurface {
                walk_cleared: &mut self.walk_reset,
                map_desk: &mut self.map_desk,
                models: {
                    let models = frontend_core::Models::none().with_scripts(&mut self.scripts);
                    #[cfg(feature = "js")]
                    let models = models.with_import(&mut self.profile_import);
                    models
                        .with_loadouts(&mut self.loadouts)
                        .with_fleet(&mut app.fleet_model)
                        .with_quester(&mut app.quester_paths_controller)
                        .with_walk_globals(&mut app.walk_permissions)
                        .with_form_save(&mut app.settings_save)
                },
                frame_hook: None,
            },
        )
    }

    /// Submit `command` for `targets` over the app's lent models. `Err` is
    /// a refused command, the owner's refusal or failure text of the first
    /// failed member, or the failure of the operation it opened.
    fn submit_with(
        &mut self,
        app: &mut TuiApp,
        command: OperatorCommand,
        targets: Targets<'_>,
    ) -> Result<frontend_core::Submitted, String> {
        let (core, mut surface) = self.core_and_surface_with(app);
        let submitted =
            core.submit(command, targets, &mut surface)
                .map_err(|error| match error {
                    frontend_core::SubmitError::NoTarget(OperatorCommand::GroupWalk) => {
                        "no bots selected".to_string()
                    }
                    error => error.to_string(),
                })?;

        if let Some(reason) = core.command_failure(submitted.request()) {
            return Err(reason.to_string());
        }
        match submitted.operation().and_then(|op| core.failure(op)) {
            Some(failure) => Err(failure),
            None => Ok(submitted),
        }
    }
    /// Submit with app-owned models without collapsing a bulk report into
    /// its first failed member.
    fn submit_raw_with(
        &mut self,
        app: &mut TuiApp,
        command: OperatorCommand,
        targets: Targets<'_>,
    ) -> Result<frontend_core::Submitted, String> {
        let (core, mut surface) = self.core_and_surface_with(app);
        core.submit(command, targets, &mut surface)
            .map_err(|error| error.to_string())
    }

    /// [`Self::core_and_surface_with`] lending the shared profile
    /// editor instead of the settings popup's form-save model. Only
    /// the New profile rows submit over it, so the editor is open
    /// exactly while its overlay is.
    fn core_and_surface_with_editor<'a>(
        &'a mut self,
        app: &'a mut TuiApp,
    ) -> (&'a mut OperatorSession<()>, TuiSurface<'a>) {
        (
            &mut self.core,
            TuiSurface {
                walk_cleared: &mut self.walk_reset,
                map_desk: &mut self.map_desk,
                models: {
                    let models = frontend_core::Models::none().with_scripts(&mut self.scripts);
                    #[cfg(feature = "js")]
                    let models = models.with_import(&mut self.profile_import);
                    models
                        .with_loadouts(&mut self.loadouts)
                        .with_fleet(&mut app.fleet_model)
                        .with_quester(&mut app.quester_paths_controller)
                        .with_walk_globals(&mut app.walk_permissions)
                        .with_editor(&mut self.profile_editor)
                },
                frame_hook: None,
            },
        )
    }

    /// Submit one New profile row over the editor surface, without
    /// collapsing its result: the form's own notice (a refusal, or
    /// `Saved <name>.` once durable) is what the overlay shows.
    fn submit_raw_editor(
        &mut self,
        app: &mut TuiApp,
        command: OperatorCommand,
        targets: Targets<'_>,
    ) -> Result<frontend_core::Submitted, String> {
        let (core, mut surface) = self.core_and_surface_with_editor(app);
        core.submit(command, targets, &mut surface)
            .map_err(|error| error.to_string())
    }

    /// Log in the focused member (explicit handshake; recreates a terminal
    /// worker).
    fn login(&mut self, app: &mut TuiApp) {
        let Some(name) = app.focused_name() else {
            return;
        };
        app.error = self
            .submit(OperatorCommand::Login, Targets::Profile(&name))
            .err();
    }

    /// Log out the focused member; the slot stays loaded and latched.
    fn logout(&mut self, app: &mut TuiApp) {
        if let Some(name) = app.focused_name() {
            self.scripts.stage_cancel_queued("logged out");
            if let Err(error) = self.submit_with(
                app,
                OperatorCommand::CancelQueuedStart,
                Targets::Profile(&name),
            ) {
                app.error = Some(error);
            }
            self.apply_script_notice(app);
            if let Err(error) = self.submit(OperatorCommand::Logout, Targets::Profile(&name)) {
                app.error = Some(error);
            }
        }
    }

    /// Settings-popup `r`: Relog-now for a memory-mode switch. Confirms
    /// first when running script work would be interrupted or a queued Start
    /// would be cancelled; otherwise relogs directly.
    fn memory_relog_request(&mut self, app: &mut TuiApp, name: &str) {
        let script_work = self.core.memory_relog_warning(name) || self.scripts.start_waiting(name);
        if script_work {
            app.confirm(ConfirmKind::MemoryRelog(name.to_string()));
        } else {
            self.memory_relog_now(app, name);
        }
    }

    /// Relog through the login FIFO to apply the entire queued memory mode.
    fn memory_relog_now(&mut self, app: &mut TuiApp, name: &str) {
        self.scripts.stage_cancel_queued("logged out");
        if let Err(error) = self.submit_with(
            app,
            OperatorCommand::CancelQueuedStart,
            Targets::Profile(name),
        ) {
            app.error = Some(error);
        }
        self.apply_script_notice(app);
        app.error = self
            .submit(OperatorCommand::RelogNow, Targets::Profile(name))
            .err();
    }

    /// Remove `name` (frozen when the operator confirmed): clean logout,
    /// then its worker stops. The neighbour becomes selected when it was.
    fn remove(&mut self, app: &mut TuiApp, name: &str) {
        self.scripts.stage_cancel_queued("removed");
        if let Err(error) = self.submit_with(
            app,
            OperatorCommand::CancelQueuedStart,
            Targets::Profile(name),
        ) {
            app.error = Some(error);
        }
        self.apply_script_notice(app);
        let startup_identity = self.core.profile_identity(name);
        let submitted = match self.submit(OperatorCommand::Remove, Targets::Profile(name)) {
            Ok(submitted) => submitted,
            Err(error) => {
                app.error = Some(error);
                return;
            }
        };
        if let Some(removal) = submitted.removal() {
            // An explicit removal cancels an automatic default not yet chosen.
            self.startup_default_pending = false;
            let cancelled_startup_target =
                startup_identity.is_some_and(|identity| self.cancel_startup_target(identity));
            if !cancelled_startup_target
                && (removal.reselected().is_some() || removal.selection_cleared())
            {
                self.cancel_startup_focus();
            }
            if let Some(next) = removal.reselected() {
                app.focused = app.names.iter().position(|n| n == next);
            } else if removal.selection_cleared() {
                app.focused = None;
            }
        }
        // The fleet table drops the member on the next pump.
    }

    /// Load every vault profile and log in every member (Load+login all). A
    /// loaded, logged-out member is re-armed and a terminal worker
    /// recreated. Returns how many were newly loaded.
    #[cfg(feature = "memory-profile")]
    fn load_and_login_all(&mut self) -> usize {
        let last = self.last_focused.clone();
        self.core.set_focus_hint(last.as_deref());
        // An empty "all" (nothing left to load, nobody to log in) is not an
        // error, as before.
        let mut added = 0;
        let failure = match self.submit_raw(OperatorCommand::Load, Targets::Everyone) {
            Ok(load) => {
                added = load.added();
                load.operation().and_then(|op| self.core.failure(op))
            }
            Err(SubmitError::NoTarget(_)) => None,
            Err(error) => Some(error.to_string()),
        };
        let failure =
            failure.or_else(
                || match self.submit_raw(OperatorCommand::Login, Targets::Everyone) {
                    Ok(login) => login.operation().and_then(|op| self.core.failure(op)),
                    Err(SubmitError::NoTarget(_)) => None,
                    Err(error) => Some(error.to_string()),
                },
            );
        if failure.is_some() {
            self.error = failure;
        }
        added
    }

    fn startup_profile_name(&self, identity: ProfileIdentity) -> Option<&str> {
        self.core
            .profiles()
            .find(|profile| ProfileIdentity::uid(profile.uid()) == identity)
            .map(|profile| profile.username())
    }

    /// Remove deleted profiles without cancelling other startup targets.
    fn prune_startup_targets(&mut self) {
        let core = &self.core;
        self.startup_profiles.retain(|identity| {
            core.profiles()
                .any(|profile| ProfileIdentity::uid(profile.uid()) == *identity)
        });
        if self
            .startup_focus
            .is_some_and(|identity| !self.startup_profiles.contains(&identity))
        {
            self.startup_focus = self.startup_profiles.last().copied();
        }
        if self.startup_profiles.is_empty() && !self.startup_default_pending {
            self.startup_focus = None;
            self.startup_focus_cancelled = false;
        }
    }

    /// Cancel only `identity` from the deferred startup work.
    fn cancel_startup_target(&mut self, identity: ProfileIdentity) -> bool {
        let before = self.startup_profiles.len();
        self.startup_profiles.retain(|queued| *queued != identity);
        let cancelled = self.startup_profiles.len() != before;
        if cancelled && self.startup_focus == Some(identity) {
            self.startup_focus = self.startup_profiles.last().copied();
        }
        if self.startup_profiles.is_empty() && !self.startup_default_pending {
            self.startup_focus = None;
            self.startup_focus_cancelled = false;
        }
        cancelled
    }

    /// Supersede only focus; the deferred startup loads and logins remain.
    fn cancel_startup_focus(&mut self) {
        if self.startup_default_pending
            || !self.startup_profiles.is_empty()
            || self.startup_focus.is_some()
        {
            self.startup_focus_cancelled = true;
        }
    }

    fn queue_startup_profile(&mut self, identity: ProfileIdentity) {
        if !self.startup_profiles.contains(&identity) {
            self.startup_profiles.push(identity);
        }
    }

    fn queue_startup_focus(&mut self, name: &str) {
        let Some(identity) = self.core.profile_identity(name) else {
            return;
        };
        self.queue_startup_profile(identity);
        self.startup_focus = Some(identity);
        self.startup_focus_cancelled = false;
    }

    /// Choose the default only after shared preparation reconciles profile order.
    fn queue_default_startup(&mut self) {
        self.startup_default_pending = false;
        let Some(name) = self.core.profile_order().names().first() else {
            return;
        };
        let Some(identity) = self.core.profile_identity(name) else {
            return;
        };
        self.queue_startup_profile(identity);
        if self.startup_focus.is_none() && !self.startup_focus_cancelled {
            self.startup_focus = Some(identity);
        }
    }

    /// Load and log in each unique startup target in the reconciled saved order.
    fn apply_startup_requests(&mut self) {
        self.prune_startup_targets();
        let core = &self.core;
        self.startup_profiles.sort_unstable_by_key(|identity| {
            core.profiles()
                .find(|profile| ProfileIdentity::uid(profile.uid()) == *identity)
                .and_then(|profile| core.profile_order().position(profile.username()))
                .unwrap_or(usize::MAX)
        });
        let queued = std::mem::take(&mut self.startup_profiles);
        let focus = self.startup_focus.take();
        let cancel_focus = std::mem::take(&mut self.startup_focus_cancelled);
        self.startup_default_pending = false;
        for identity in queued {
            if let Some(name) = self.startup_profile_name(identity).map(str::to_owned) {
                self.load_and_login(&name);
            }
        }
        if !cancel_focus {
            if let Some(name) =
                focus.and_then(|identity| self.startup_profile_name(identity).map(str::to_owned))
            {
                self.focus(&name);
            }
        }
    }

    /// Select `name` (pure bookkeeping: never a spawn or login).
    fn focus(&mut self, name: &str) {
        // Fleet, keyboard, and saved-profile selection share this boundary.
        // An operator choice supersedes startup focus without dropping its
        // readiness-gated load and login.
        self.cancel_startup_focus();
        if let Err(error) = self.submit(OperatorCommand::Select, Targets::Profile(name)) {
            self.error = Some(error);
        }
    }

    /// `--live script_*` boot: minted ephemeral vault + spawn + runner. With
    /// `--catalog-core` / `--pair-core` the Play-owned witness is armed
    /// before either slot publishes, exactly like the panel watches.
    fn live_prepare_script(
        &mut self,
        scenario: frontend_core::live::Scenario,
    ) -> Result<(), String> {
        self.persist_ui = false;
        let view = scenario.view();
        let name = view.name().to_owned();
        let profile_count = view.profile_count();
        let scenario_deadline = view.deadline();
        let start_script = view.start_script().map(str::to_owned);
        let start_file = view.start_file().map(str::to_owned);
        let fleet_start = view.fleet_start();
        #[cfg(feature = "js")]
        let inject_companion_as = view.inject_companion_as().map(str::to_owned);
        let settings_inject = scenario.settings_inject();
        #[cfg(feature = "js")]
        let mut settings_inject = settings_inject;
        #[cfg(feature = "js")]
        let fixture_loadouts = scenario.fixture_loadouts();
        #[cfg(not(feature = "js"))]
        if fleet_start.is_some() || start_file.is_some() {
            return Err("live file/fleet starts require JavaScript support".into());
        }
        #[cfg(not(feature = "js"))]
        if let Some(card_name) = start_script.as_deref() {
            if CompiledCardId::lookup(card_name).is_none() {
                return Err(format!(
                    "live script {card_name} requires JavaScript support"
                ));
            }
        }
        #[cfg(feature = "js")]
        let live_core = self.core.resolve_live_core(
            &name,
            profile_count,
            self.live_catalog_core,
            self.live_pair_core,
        )?;
        #[cfg(not(feature = "js"))]
        {
            self.core.resolve_live_core(
                &name,
                profile_count,
                self.live_catalog_core,
                self.live_pair_core,
            )?;
        }
        let profiles = frontend_core::live::LiveProfiles::mint(profile_count);
        let names = profiles.names();
        // The driven slot's bag names its companion, exactly as the panel
        // injects it; the companion's own bag is built by the Start plan.
        // A pair-core run builds both bags itself.
        #[cfg(feature = "js")]
        if live_core != frontend_core::live::LiveCoreMode::Pair {
            if let Some(key) = inject_companion_as.as_deref() {
                if names.len() > 1 {
                    settings_inject
                        .get_or_insert_with(serde_json::Map::new)
                        .insert(
                            key.to_string(),
                            serde_json::Value::String(frontend_core::live::screen_name(&names[1])),
                        );
                }
            }
        }
        #[cfg(feature = "js")]
        let fleet_cards = scenario.fleet_cards(names)?;
        let path = profiles.tui_vault();
        self.unlock_at(&path, profiles.passphrase())?;
        self.core.configure_live_core(names)?;
        self.live_name = Some(name);
        self.live_stop_wait_started = None;
        let play_config = self.launcher.play_config();
        let budget = frontend_core::live::budget_s_from_env();
        if let Some(budget) = budget {
            self.live_soak_until = Some(Instant::now() + budget);
            self.live_announced_pass = false;
        }
        let runner = self.core.live_scenario_runner(
            scenario,
            names,
            play_config.host(),
            play_config.port(),
            budget,
        );
        self.core.live_runner().install(runner);
        self.scripts.set_inject(settings_inject);
        self.names = names.to_vec();
        for name in names {
            self.load_and_login(name);
        }
        self.focus(&names[0]);
        #[cfg(feature = "js")]
        {
            if let Some(fleet) = fleet_start {
                self.fill_rs2b0t_cards_for_live_start()?;
                for card in fleet_cards {
                    self.scripts
                        .transpile_blocking(CardSource::Catalog, card)
                        .map_err(|error| error.to_string())?;
                }
                let plan = frontend_core::live::StartPlan::fleet_catalog(
                    &mut self.scripts,
                    fleet,
                    names,
                    &fixture_loadouts,
                )?;
                self.core.live_runner().stage(plan);
            } else if let Some(file_name) = start_file {
                let identity = self.scripts.load_live_example_identity(&file_name)?;
                let choice = self
                    .scripts
                    .card(CardSource::File, &identity)
                    .ok_or_else(|| format!("file example {file_name} missing after identity load"))?
                    .choice();
                self.script_sel = Some(choice);
                let bag = self.pending_settings_bag(CardSource::File, &identity);
                let selected = self
                    .script_sel
                    .as_ref()
                    .expect("live loaded-card selection is set");
                self.core.live_runner().stage_loaded_card(
                    &mut self.scripts,
                    names,
                    selected,
                    bag,
                    inject_companion_as.as_deref(),
                    fixture_loadouts,
                )?;
            } else if let Some(card_name) = start_script {
                if let Some(id) = CompiledCardId::lookup(&card_name) {
                    self.script_sel = Some(ScriptChoice::compiled(id));
                    let bag = self.scripts.compiled_bag(&self.core, &names[0], id)?;
                    self.core
                        .live_runner()
                        .stage(frontend_core::live::StartPlan::compiled(
                            names[0].clone(),
                            id,
                            bag,
                        ));
                } else if let Some((fixture, stem)) =
                    frontend_core::live::live_file_fixture(&card_name)
                {
                    // The checked-in live file fixture (TradeBot): the panel
                    // loads it as a File card; the catalog has no such card,
                    // so the TUI must take the same branch instead of
                    // failing with `no card (Catalog, TradeBot)`.
                    self.scripts.stage_load_file(fixture);
                    let submitted = self
                        .submit_raw(OperatorCommand::LoadScriptFile, Targets::Application)
                        .map_err(|error| format!("load {card_name} fixture: {error}"))?;
                    if let Some(error) = self.core.command_failure(submitted.request()) {
                        return Err(format!("load {card_name} fixture: {error}"));
                    }
                    if self.scripts.card(CardSource::File, &stem).is_none() {
                        return Err(format!("file fixture {card_name} missing after load"));
                    }
                    self.script_sel = Some(ScriptChoice::loaded(CardSource::File, &stem));
                    let bag = self.pending_settings_bag(CardSource::File, &stem);
                    let selected = self
                        .script_sel
                        .as_ref()
                        .expect("live loaded-card selection is set");
                    self.core.live_runner().stage_loaded_card(
                        &mut self.scripts,
                        names,
                        selected,
                        bag,
                        inject_companion_as.as_deref(),
                        fixture_loadouts,
                    )?;
                } else {
                    self.fill_rs2b0t_cards_for_live_start()?;
                    self.scripts
                        .transpile_blocking(CardSource::Catalog, &card_name)
                        .map_err(|error| error.to_string())?;
                    self.script_sel = Some(ScriptChoice::loaded(CardSource::Catalog, &card_name));
                    let bag = self.pending_settings_bag(CardSource::Catalog, &card_name);
                    let selected = self
                        .script_sel
                        .as_ref()
                        .expect("live loaded-card selection is set");
                    self.core.live_runner().stage_loaded_card(
                        &mut self.scripts,
                        names,
                        selected,
                        bag,
                        inject_companion_as.as_deref(),
                        fixture_loadouts,
                    )?;
                }
            }
        }
        #[cfg(not(feature = "js"))]
        if let Some(card_name) = start_script {
            let id = CompiledCardId::lookup(&card_name)
                .expect("no-JS live script was checked before starting");
            self.script_sel = Some(ScriptChoice::compiled(id));
            let bag = self.scripts.compiled_bag(&self.core, &names[0], id)?;
            self.core
                .live_runner()
                .stage(frontend_core::live::StartPlan::compiled(
                    names[0].clone(),
                    id,
                    bag,
                ));
        }
        let live_started = Instant::now();
        self.live_core_deadline =
            self.core
                .live_runner()
                .core_deadline(budget, scenario_deadline, live_started);
        self.core
            .live_runner()
            .configure_pair_setup_timing(live_started, self.live_core_deadline);
        Ok(())
    }

    fn map_context(&self, app: &TuiApp) -> Option<frontend_core::walk_map::WalkMapContext> {
        let name = app.focused_name()?;
        let nav = self.bound()?.nav_identity()?;
        Some(self.core.walk_map_context(
            Some(&name),
            nav,
            app.map_catalogue.as_ref().map(|catalogue| catalogue.key()),
            1,
        ))
    }

    fn bind_map_context(&self, app: &mut TuiApp) {
        if !app.map_active {
            app.map_model.close();
            return;
        }
        if let Some(context) = self.map_context(app) {
            app.map_model.bind(context);
        } else {
            app.map_model.close();
            app.error = Some("map: focused slot or navigation identity unavailable".into());
        }
    }

    fn open_map_catalogue(&mut self, app: &mut TuiApp) {
        let Some(profile) = self.bound().map(|profile| profile.share()) else {
            return;
        };
        if self.map_desk.map_demand().is_none() {
            self.map_desk
                .stage_open_demand(profile, WalkMapDemand::CatalogueOnly);
            if let Err(error) = self.submit(OperatorCommand::OpenMapDemand, Targets::Application) {
                app.set_map_unavailable(format!(
                    "coverage: catalogue unavailable: {error}; terrain imagery unavailable"
                ));
                return;
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
        self.core.poll_map_demand(&mut self.map_desk);
        let Some(profile) = self.bound().map(|profile| profile.share()) else {
            return;
        };
        let Some(status) = self.map_desk.map_demand().map(|handle| handle.status()) else {
            return;
        };
        match status.state() {
            WalkMapJobState::Ready => {
                if let Some(ready) = self
                    .map_desk
                    .map_demand()
                    .and_then(|handle| handle.ready_catalogue().ok())
                {
                    self.bind_ready_catalogue(app, ready);
                }
            }
            WalkMapJobState::Queued => {
                app.map_catalogue_status = MapCatalogueStatus::ReadingCache;
                app.map_coverage = "coverage: queued".into();
                if let Some(ready) = peek_walk_catalogue(&profile) {
                    self.bind_ready_catalogue(app, ready);
                }
            }
            WalkMapJobState::Running => {
                let Some(progress) = status.progress() else {
                    return;
                };
                app.map_catalogue_status = match progress.stage() {
                    WalkMapStage::ReadingCache => MapCatalogueStatus::ReadingCache,
                    _ => MapCatalogueStatus::DerivingPois,
                };
                let stage = match progress.stage() {
                    WalkMapStage::ReadingCache => "reading cache",
                    WalkMapStage::DerivingPois => "deriving POIs",
                    WalkMapStage::BakingPlane => "baking plane",
                    WalkMapStage::BuildingZoomLevels => "building zoom levels",
                    WalkMapStage::Publishing => "publishing",
                };
                app.map_coverage = format!(
                    "coverage: {stage} ({}/{}) {}",
                    progress.completed(),
                    progress.total(),
                    progress.message()
                );
                if let Some(ready) = peek_walk_catalogue(&profile) {
                    self.bind_ready_catalogue(app, ready);
                }
            }
            WalkMapJobState::Failed => {
                let error = status.failure().unwrap_or("unknown map cache error");
                app.set_map_unavailable(format!(
                    "coverage: catalogue unavailable: {error}; terrain imagery unavailable"
                ));
            }
            WalkMapJobState::Paused | WalkMapJobState::Cancelled => {
                app.set_map_unavailable(
                    "coverage: catalogue unavailable: demand paused; terrain imagery unavailable",
                );
            }
        }
    }

    fn bind_ready_catalogue(&mut self, app: &mut TuiApp, ready: Arc<WalkReadyCatalogue>) {
        let Some(world) = app.world.clone().or_else(|| self.core.nav_world()) else {
            return;
        };
        let Some(profile) = self.bound().map(|profile| profile.share()) else {
            return;
        };
        let identity = ready.identity();
        let Some(nav) = profile.nav_identity() else {
            return;
        };
        let data = profile.game_data().or_else(|| self.core.game_data());
        let same = app.map_catalogue.as_ref().is_some_and(|catalogue| {
            catalogue.identity() == identity && catalogue.nav_identity() == nav
        });
        if same && (self.map_catalogue_named || data.is_none()) {
            return;
        }
        let Ok((mut catalogue, named)) =
            self.core.walk_catalogue_from_ready(&profile, world, ready)
        else {
            return;
        };
        self.map_catalogue_named = named;
        if !named {
            if let Some(data) = data {
                match catalogue.with_game_data(data) {
                    Ok(named_catalogue) => {
                        self.map_catalogue_named = true;
                        catalogue = named_catalogue;
                    }
                    Err(_) => return,
                }
            }
        }
        app.bind_walk_catalogue(Arc::new(catalogue));
    }

    fn release_map_catalogue(&mut self) {
        if self.map_desk.needs_release() {
            if let Err(error) = self.submit(OperatorCommand::ReleaseMapDemand, Targets::Application)
            {
                self.error = Some(error);
            }
        }
        self.map_catalogue_named = false;
    }

    /// Confirm the pending map Walk through the shared operator session.
    fn arm_walk_on(&mut self, app: &mut TuiApp, _dest: Tile) {
        let name = app.focused_name();
        if let (Some(name), Some(context)) = (name.as_deref(), self.map_context(app)) {
            let origin = self.core.walk_slot_status(name).origin();
            let options = app.map_find_options();
            self.map_desk
                .stage_walk_to(&mut app.map_model, context, origin, options);
        }
        let targets = name.as_deref().map_or(Targets::Selected, Targets::Profile);
        let result = self.submit(OperatorCommand::WalkTo, targets);
        app.clear_consumed_map_selection();
        match result {
            Ok(submitted) => {
                let failure = self.core.command_failure(submitted.request());
                let report = self.core.commands().report(submitted.request());
                let applied = report.is_some_and(|report| {
                    report.members().iter().any(|member| {
                        matches!(
                            member.terminal(),
                            Some(api::identity::Settlement::Completed(
                                frontend_core::command::Completion::Done
                            ))
                        )
                    })
                });
                let not_logged_in = report.is_some_and(|report| {
                    report.members().iter().any(|member| {
                        matches!(
                            member.terminal(),
                            Some(api::identity::Settlement::Completed(
                                frontend_core::command::Completion::Skipped(
                                    frontend_core::command::SkipReason::NotLoggedIn
                                )
                            ))
                        )
                    })
                });

                if let Some(error) = failure {
                    app.error = Some(error.to_string());
                } else if applied {
                    app.walk_dest = self.map_desk.walk_dest();
                    app.error = None;
                } else if not_logged_in {
                    app.error = Some("map: no observed player".into());
                } else {
                    app.error = Some("map: WalkTo was not applied".into());
                }
            }
            Err(error) => app.error = Some(error),
        }
    }

    fn map_teleport(&mut self, app: &mut TuiApp, _dest: Tile) {
        let name = app.focused_name();
        if let (Some(name), Some(context)) = (name.as_deref(), self.map_context(app)) {
            let origin = self.core.walk_slot_status(name).origin();
            let options = app.map_find_options();
            self.map_desk
                .stage_teleport(&mut app.map_model, context, origin, options);
        }
        let targets = name.as_deref().map_or(Targets::Selected, Targets::Profile);
        let result = self.submit(OperatorCommand::Teleport, targets);
        app.clear_consumed_map_selection();
        app.error = match result {
            Err(error) => Some(error),
            Ok(submitted) => self
                .core
                .command_failure(submitted.request())
                .map(|error| error.to_string()),
        };
    }

    /// Walk every marked bot through the shared operator-session operation.
    fn map_walk_group(&mut self, app: &mut TuiApp) {
        if let Some(context) = self.map_context(app) {
            let options = app.map_find_options();
            self.map_desk
                .stage_group_walk(&mut app.map_model, context, options);
        }
        app.clear_consumed_map_selection();
        let result = self.submit_with(app, OperatorCommand::GroupWalk, Targets::Marked);
        match result {
            Err(error) => app.error = Some(error),
            Ok(mut submitted) => match submitted.take_bulk() {
                Some(report) => {
                    if report.done_count() > 0 {
                        app.walk_dest = self.map_desk.walk_dest();
                    }
                    app.error = Some(report.summary());
                }
                None => app.error = Some("map: GroupWalk produced no result".into()),
            },
        }
    }

    /// WASD one-tile walk: the client pathfinds this adjacent step.
    fn wasd_walk(&mut self, app: &mut TuiApp, dest: Tile) {
        let Some(name) = app.focused_name() else {
            return;
        };
        self.submit_wire(
            app,
            &name,
            frontend_core::GameInputEvent::walk(dest.x, dest.z, dest.level),
        );
    }

    fn submit_wire(&mut self, app: &mut TuiApp, name: &str, event: frontend_core::GameInputEvent) {
        self.map_desk.begin_game_input_frame();
        if !self.map_desk.stage_game_input(event) {
            app.error = Some("game input frame is full".into());
            return;
        }
        app.error = match self.submit(OperatorCommand::GameInput, Targets::Profile(name)) {
            Err(error) => Some(error),
            Ok(submitted) => self
                .core
                .command_failure(submitted.request())
                .map(|error| error.to_string()),
        };
    }

    /// Chat modal advance: queue `continue_dialog` / `answer_choice` on
    /// the focused slot (the guardian's hold still lets chat through).
    fn chat_send(&mut self, app: &mut TuiApp, action: ChatAction) {
        let Some(name) = app.focused_name() else {
            return;
        };
        match action {
            ChatAction::Continue => {
                self.submit_wire(
                    app,
                    &name,
                    frontend_core::GameInputEvent::continue_dialogue(),
                );
            }
            ChatAction::Answer(option) => {
                self.submit_wire(
                    app,
                    &name,
                    frontend_core::GameInputEvent::answer(option as i32),
                );
            }
            ChatAction::PaintButton(index) => {
                // The core decides what the button does: a script click, or
                // a host control bound to the profile and run the frame was
                // read for (refused, not retargeted, if focus moved).
                let input = app
                    .chat_data
                    .script_paint
                    .as_ref()
                    .and_then(|paint| paint.view().activate(index));
                if let Some(input) = input {
                    self.scripts.stage_paint_input(input);
                    app.error = self
                        .submit_with(
                            app,
                            OperatorCommand::ScriptPaintInput,
                            Targets::Profile(&name),
                        )
                        .err();
                }
            }
            ChatAction::PaintChrome(index) => {
                let selection = app
                    .chat_data
                    .script_paint
                    .as_ref()
                    .map(PaintFrame::view)
                    .and_then(|paint| {
                        super::chat::paint_chrome_rows(paint).get(index).map(
                            |(key, select_name, _)| {
                                (key.to_owned(), select_name.to_owned(), paint.generation())
                            },
                        )
                    });
                if let Some((key, select_name, generation)) = selection {
                    self.scripts
                        .stage_paint_input(frontend_core::scripts::PaintInput::select(
                            key,
                            select_name,
                            generation,
                        ));
                    app.error = self
                        .submit_with(
                            app,
                            OperatorCommand::ScriptPaintInput,
                            Targets::Profile(&name),
                        )
                        .err();
                }
            }
            ChatAction::None => {}
        }
    }

    #[cfg(feature = "js")]
    fn fill_rs2b0t_cards_for_live_start(&mut self) -> Result<(), String> {
        if self.scripts.catalog_filled() {
            return Ok(());
        }
        let Some(root) = self.catalog_root() else {
            self.scripts.stage_catalog_action(
                CatalogActionKind::Fill,
                None,
                frontend_core::scripts::paths::rs2b0t_path_file(),
            );
            let submitted = self
                .submit_raw(OperatorCommand::ImportCatalog, Targets::Application)
                .map_err(|error| error.to_string())?;
            if let Some(error) = self.core.command_failure(submitted.request()) {
                return Err(error.to_string());
            }
            return Ok(());
        };
        self.scripts
            .fill_catalog_blocking(&root, &frontend_core::scripts::paths::rs2b0t_path_file())
            .map_err(|error| error.to_string())
    }

    /// Fill the JS library's cards from the known catalog root once.
    #[cfg(feature = "js")]
    fn fill_rs2b0t_cards_once(&mut self, app: &mut TuiApp) {
        if self.scripts.catalog_filled() {
            return;
        }
        self.scripts.stage_catalog_action(
            CatalogActionKind::Fill,
            self.catalog_root(),
            frontend_core::scripts::paths::rs2b0t_path_file(),
        );
        if let Err(error) =
            self.submit_with(app, OperatorCommand::ImportCatalog, Targets::Application)
        {
            app.error = Some(error);
        }
        self.apply_script_notice(app);
    }

    /// Opening Browse: fill from `$RS2B0T`/persisted root, or prompt for a
    /// clone root, or honour a prior defer (panel parity).
    #[cfg(feature = "js")]
    fn on_script_browse_open(&mut self, app: &mut TuiApp) {
        if self.scripts.catalog_filled() {
            return;
        }
        let root = self.catalog_root();
        self.scripts.stage_catalog_action(
            CatalogActionKind::Fill,
            root.clone(),
            frontend_core::scripts::paths::rs2b0t_path_file(),
        );
        if let Err(error) =
            self.submit_with(app, OperatorCommand::ImportCatalog, Targets::Application)
        {
            app.error = Some(error);
            return;
        }
        self.apply_script_notice(app);
        if root.is_some() {
            return;
        }
        if frontend_core::scripts::paths::rs2b0t_import_deferred_at(
            &frontend_core::scripts::paths::rs2b0t_import_file(),
        ) {
            return;
        }
        self.rs2b0t_catalog_open = true;
        self.rs2b0t_catalog_dir = Self::default_catalog_browse_dir();
        app.rs2b0t_catalog_open = true;
        app.rs2b0t_catalog_dir = self.rs2b0t_catalog_dir.clone();
        app.catalog_sel = 0;
    }

    #[cfg(feature = "js")]
    fn defer_rs2b0t_catalog(&mut self, app: &mut TuiApp) {
        self.scripts
            .stage_defer_catalog(frontend_core::scripts::paths::rs2b0t_import_file());
        if let Err(error) = self.submit_with(
            app,
            OperatorCommand::DeferCatalogImport,
            Targets::Application,
        ) {
            app.error = Some(error);
        }
        self.rs2b0t_catalog_open = false;
        app.rs2b0t_catalog_open = false;
    }

    /// Run one step of the rs2b0t profile import on the shared flow, then
    /// show the result in the overlay.
    #[cfg(feature = "js")]
    fn profile_import(&mut self, app: &mut TuiApp, action: ImportAction) {
        // Script names map through the catalog. If its first scan is still
        // running, hold this preview until the worker publishes the catalog.
        if let ImportAction::Preview(path) = &action {
            *self.profile_import.path_mut() = path.clone();
            if let Err(error) = self.submit_with(
                app,
                OperatorCommand::EditProfileImportPath,
                Targets::Application,
            ) {
                app.error = Some(error);
                self.show_profile_import(app);
                return;
            }
            if !self.scripts.catalog_filled() {
                if let Some(root) = self.catalog_root() {
                    self.scripts.stage_catalog_action(
                        frontend_core::scripts::CatalogActionKind::Fill,
                        Some(root),
                        frontend_core::scripts::paths::rs2b0t_path_file(),
                    );
                    if let Err(error) =
                        self.submit_with(app, OperatorCommand::ImportCatalog, Targets::Application)
                    {
                        app.error = Some(error);
                    }
                    if self.scripts.catalog_work_pending() {
                        self.profile_import_preview_pending = true;
                        self.show_profile_import(app);
                        return;
                    }
                }
            }
            self.profile_import_preview_pending = false;
        }
        if self.profile_import_preview_pending && !matches!(&action, ImportAction::Close) {
            self.show_profile_import(app);
            return;
        }
        match action {
            ImportAction::Preview(_) => {
                if let Err(error) = self.submit_with(
                    app,
                    OperatorCommand::PreviewProfileImport,
                    Targets::Application,
                ) {
                    app.error = Some(error);
                }
            }
            ImportAction::Policy(policy) => {
                if let Err(error) = self.submit_with(
                    app,
                    OperatorCommand::SetProfileImportPolicy(policy),
                    Targets::Application,
                ) {
                    app.error = Some(error);
                }
            }
            ImportAction::IncludeUnusable(include) => {
                if let Err(error) = self.submit_with(
                    app,
                    OperatorCommand::SetProfileImportIncludeUnusable(include),
                    Targets::Application,
                ) {
                    app.error = Some(error);
                }
            }
            ImportAction::Confirm => {
                match self
                    .profile_import
                    .prepare_confirmation(&self.core, &self.scripts)
                {
                    Ok(()) => {
                        let planned = self
                            .profile_import
                            .planned_writes()
                            .map(|(index, name, command)| (index, name.to_owned(), command))
                            .collect::<Vec<_>>();
                        for (index, name, planned_command) in planned {
                            let staged = match planned_command {
                                OperatorCommand::CreateProfile => {
                                    self.profile_import.stage_create_account(index)
                                }
                                OperatorCommand::SaveProfile => {
                                    self.profile_import.stage_save_account(index)
                                }
                                _ => {
                                    self.refuse_command(
                                        app,
                                        planned_command,
                                        &[&name],
                                        None,
                                        &format!(
                                            "profile import account {name} has an unsupported writer"
                                        ),
                                    );
                                    break;
                                }
                            };
                            if let Err(error) = staged {
                                self.refuse_command(
                                    app,
                                    planned_command,
                                    &[&name],
                                    None,
                                    &format!("profile import: {error}"),
                                );
                                break;
                            }
                            let targets = if planned_command == OperatorCommand::CreateProfile {
                                Targets::Application
                            } else {
                                Targets::Profile(&name)
                            };
                            if let Err(error) = self.submit(planned_command, targets) {
                                app.error = Some(format!("profile import: {error}"));
                            }
                            self.profile_import.cancel_staged_account(index);
                        }
                    }
                    // Import replans the preview against the current vault
                    // before any account row exists; a refused replan is
                    // refused as the preview it repeats.
                    Err(error) => self.refuse_command(
                        app,
                        OperatorCommand::PreviewProfileImport,
                        &[],
                        None,
                        &format!("profile import: {error}"),
                    ),
                }
            }
            ImportAction::Back => {
                if let Err(error) = self.submit_with(
                    app,
                    OperatorCommand::ReturnToProfileImportPath,
                    Targets::Application,
                ) {
                    app.error = Some(error);
                }
            }
            ImportAction::AskDelete => {
                if let Err(error) = self.submit_with(
                    app,
                    OperatorCommand::SetProfileImportDeletePrompt(true),
                    Targets::Application,
                ) {
                    app.error = Some(error);
                }
            }
            ImportAction::CancelDelete => {
                if let Err(error) = self.submit_with(
                    app,
                    OperatorCommand::SetProfileImportDeletePrompt(false),
                    Targets::Application,
                ) {
                    app.error = Some(error);
                }
            }
            ImportAction::ConfirmDelete => {
                if let Err(error) = self.submit_with(
                    app,
                    OperatorCommand::DeleteProfileImportExport,
                    Targets::Application,
                ) {
                    app.error = Some(error);
                }
            }
            ImportAction::Close => {
                self.profile_import_preview_pending = false;
                self.profile_import = frontend_core::rs2b0t_import::ProfileImport::new();
                return;
            }
        }
        self.show_profile_import(app);
    }

    /// Copy the import flow into its open overlay. A flow whose overlay was
    /// closed is dropped (with any parsed file and its passwords); a report
    /// follows its writes until they settle.
    #[cfg(feature = "js")]
    fn project_profile_import(&mut self, app: &mut TuiApp) {
        match app.modal.as_ref() {
            Some(Modal::ProfileImport(view)) => {
                if view.stage == Stage::Report && !view.settled {
                    self.show_profile_import(app);
                }
            }
            _ if !self.profile_import.is_fresh() => {
                self.profile_import_preview_pending = false;
                self.profile_import = frontend_core::rs2b0t_import::ProfileImport::new();
            }
            _ => {}
        }
    }

    #[cfg(feature = "js")]
    fn show_profile_import(&self, app: &mut TuiApp) {
        let Some(Modal::ProfileImport(view)) = app.modal.as_mut() else {
            return;
        };
        let flow = &self.profile_import;
        let stage = flow.stage();
        if view.stage != stage {
            view.scroll = 0;
        }
        view.stage = stage;
        view.path.clear();
        view.path.push_str(flow.path());
        view.lines = match stage {
            Stage::Path => Vec::new(),
            Stage::Preview => flow.preview_lines(),
            Stage::Report => flow.report_lines(&self.core),
        };
        view.error = flow.error().map(str::to_owned);
        view.policy = flow.policy();
        view.unusable = flow.plan().map_or(0, |plan| plan.unusable());
        view.include_unusable = flow.include_unusable();
        view.settled = flow.settled(&self.core);
        view.can_delete = flow.can_delete(&self.core);
        view.delete_asked = flow.delete_asked();
    }

    /// Open the shared profile form on its blank new-profile row
    /// (`OpenNewProfileForm`, the panel's New profile button). The
    /// overlay is already open; this opens the editor behind it and
    /// shows its fresh draft.
    fn open_new_profile(&mut self, app: &mut TuiApp) {
        if let Err(error) = self.submit_raw_editor(
            app,
            OperatorCommand::OpenNewProfileForm,
            Targets::Application,
        ) {
            app.error = Some(error);
        }
        self.show_new_profile(app);
    }

    /// One key in the New profile overlay. Typing edits the editor's
    /// draft (staged through `EditProfileDraft`), Enter on the
    /// password field saves it (`CreateProfile`), and Esc closes the
    /// form (`CloseProfileForm`) unless its save is still being
    /// written, when the overlay stays until the write settles.
    fn new_profile_key(&mut self, app: &mut TuiApp, key: crossterm::event::KeyEvent) {
        use crate::new_profile::NewProfileField;
        use crossterm::event::{KeyCode, KeyModifiers};
        use frontend_core::command::ProfileDraftField;

        let Some(Modal::NewProfile(view)) = app.modal.as_ref() else {
            return;
        };
        let field = view.field;
        let text = !key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER);
        // While the form's save is being written, keys change nothing:
        // the write already carries its draft, and an edit now would
        // be lost when the saved profile closes the overlay.
        if key.code != KeyCode::Esc && self.profile_editor.saving(&self.core) {
            self.show_new_profile(app);
            return;
        }
        match key.code {
            KeyCode::Esc => {
                if self.profile_editor.saving(&self.core) {
                    self.show_new_profile(app);
                    return;
                }
                if let Err(error) = self.submit_raw_editor(
                    app,
                    OperatorCommand::CloseProfileForm(frontend_core::command::FormClose::Form),
                    Targets::Application,
                ) {
                    app.error = Some(error);
                }
                self.new_profile_pending = None;
                app.modal = None;
                return;
            }
            KeyCode::Tab | KeyCode::BackTab | KeyCode::Up | KeyCode::Down => {
                if let Some(Modal::NewProfile(view)) = app.modal.as_mut() {
                    view.field = field.other();
                }
            }
            KeyCode::Enter if field == NewProfileField::Username => {
                if let Some(Modal::NewProfile(view)) = app.modal.as_mut() {
                    view.field = NewProfileField::Password;
                }
            }
            KeyCode::Enter => {
                self.new_profile_save(app);
                return;
            }
            KeyCode::Backspace => {
                let (buffer, draft_field) = match field {
                    NewProfileField::Username => (
                        self.profile_editor.username_mut(),
                        ProfileDraftField::Username,
                    ),
                    NewProfileField::Password => (
                        self.profile_editor.password_mut(),
                        ProfileDraftField::Password,
                    ),
                };
                if buffer.pop().is_some() {
                    if let Err(error) = self.submit_raw_editor(
                        app,
                        OperatorCommand::EditProfileDraft(draft_field),
                        Targets::Application,
                    ) {
                        app.error = Some(error);
                    }
                }
            }
            KeyCode::Char(c) if text => {
                let (buffer, draft_field) = match field {
                    NewProfileField::Username => (
                        self.profile_editor.username_mut(),
                        ProfileDraftField::Username,
                    ),
                    NewProfileField::Password => (
                        self.profile_editor.password_mut(),
                        ProfileDraftField::Password,
                    ),
                };
                buffer.push(c);
                if let Err(error) = self.submit_raw_editor(
                    app,
                    OperatorCommand::EditProfileDraft(draft_field),
                    Targets::Application,
                ) {
                    app.error = Some(error);
                }
            }
            _ => {}
        }
        self.show_new_profile(app);
    }

    /// Save the New profile form (`CreateProfile` over the editor's
    /// draft, with the password the operator typed). A refusal shows
    /// in the form; an accepted write settles in the pump, which
    /// closes the overlay on the saved profile.
    fn new_profile_save(&mut self, app: &mut TuiApp) {
        let username = self.profile_editor.draft().username().trim().to_string();
        match self.submit_raw_editor(app, OperatorCommand::CreateProfile, Targets::Application) {
            Ok(submitted) => {
                if submitted.operation().is_some() {
                    self.new_profile_pending = Some(username);
                }
            }
            Err(error) => app.error = Some(error),
        }
        self.show_new_profile(app);
    }

    /// Copy the editor's draft and save notice into its open overlay.
    /// The password itself is never copied: only its length, for the
    /// mask. The field that takes typing is the overlay's own and is
    /// kept as it is.
    fn show_new_profile(&self, app: &mut TuiApp) {
        let Some(Modal::NewProfile(view)) = app.modal.as_mut() else {
            return;
        };
        let draft = self.profile_editor.draft();
        view.username.clear();
        view.username.push_str(draft.username());
        view.password_len = draft.password_len();
        view.error = self
            .profile_editor
            .save_state()
            .notice()
            .and_then(|notice| notice.error())
            .map(str::to_owned);
        view.saving = self.profile_editor.saving(&self.core);
    }

    /// Settle a New profile save the writer has finished: a durable
    /// one closes the overlay and queues its startup focus. The pump
    /// loads and logs in through the preference-readiness gate. A
    /// failed one keeps the form and its draft, with the failure in
    /// the form's notice. Runs from the pump in place of the settings
    /// popup's settle while the overlay is open: the two forms are
    /// never open at once, and a settings write that settles late
    /// still reaches the message line through `take_write_failures`.
    fn settle_new_profile(&mut self, app: &mut TuiApp) {
        let saved = self.profile_editor.settle(&mut self.core);
        self.show_new_profile(app);
        let Some(pending) = self.new_profile_pending.take() else {
            return;
        };
        if !saved.iter().any(|name| name == &pending) {
            if self.profile_editor.saving(&self.core) {
                // Still being written: keep waiting for it.
                self.new_profile_pending = Some(pending);
            }
            return;
        }
        if let Err(error) = self.submit_raw_editor(
            app,
            OperatorCommand::CloseProfileForm(frontend_core::command::FormClose::Form),
            Targets::Application,
        ) {
            app.error = Some(error);
        }
        if matches!(app.modal, Some(Modal::NewProfile(_))) {
            app.modal = None;
        }
        self.names = self
            .core
            .profiles()
            .map(|p| p.username().to_string())
            .collect();
        self.queue_startup_focus(&pending);
    }

    #[cfg(feature = "js")]
    fn import_rs2b0t_catalog(&mut self, app: &mut TuiApp, root: &Path) -> Result<(), String> {
        if !rs2b0t_root_has_index(root) {
            return Err(format!(
                "no catalog at {}",
                frontend_core::scripts::paths::registry_index_path(root).display()
            ));
        }
        self.scripts.stage_catalog_action(
            CatalogActionKind::Import,
            Some(root.to_path_buf()),
            frontend_core::scripts::paths::rs2b0t_path_file(),
        );
        self.submit_with(app, OperatorCommand::ImportCatalog, Targets::Application)?;
        self.apply_script_notice(app);
        Ok(())
    }

    /// Show the script coordinator's latest notice on the strip.
    fn apply_script_notice(&mut self, app: &mut TuiApp) {
        if let Some(notice) = self.scripts.take_notice() {
            notice.apply(&mut app.error);
        }
    }

    /// The catalog root a Start may fill the catalog from (first use only).
    #[cfg(feature = "js")]
    fn start_catalog_root(&self) -> Option<PathBuf> {
        if self.scripts.catalog_filled() {
            None
        } else {
            self.catalog_root()
        }
    }

    /// Operator Start on the focused profile: stage its selected card, then
    /// submit selection and Start as separate command rows.
    fn script_start(&mut self, app: &mut TuiApp, sel: &ScriptChoice) {
        let Some(name) = app.focused_name() else {
            self.refuse_command(
                app,
                OperatorCommand::StartScript,
                &[],
                Unavailable::NoFocusedBot.need(),
                "script: no focused profile",
            );
            return;
        };
        self.scripts.stage_browse_pick(sel.duplicate());
        if let Err(error) =
            self.submit_with(app, OperatorCommand::PickBrowseCard, Targets::Application)
        {
            app.error = Some(error);
            return;
        }
        if let Err(error) = self.submit_with(
            app,
            OperatorCommand::SelectScriptCard,
            Targets::Profile(&name),
        ) {
            app.error = Some(error);
            return;
        }
        #[cfg(feature = "js")]
        self.scripts
            .stage_start_catalog_root(self.start_catalog_root());
        if let Err(error) =
            self.submit_with(app, OperatorCommand::StartScript, Targets::Profile(&name))
        {
            app.error = Some(format!("script: {error}"));
            self.apply_script_notice(app);
            return;
        }
        #[cfg(feature = "js")]
        self.scripts.show_load_failures();

        self.apply_script_notice(app);
    }

    /// Fold settled script work (reload validation, Start setup, parameter
    /// writes) after the core poll and show its notice. Called once per
    /// pump.
    fn poll_scripts(&mut self, app: &mut TuiApp) {
        self.scripts.poll(&mut self.core);
        #[cfg(feature = "js")]
        if let Some((root, result)) = self.scripts.take_catalog_import_update() {
            match result {
                Ok(_) => {
                    let _ = frontend_core::scripts::paths::clear_rs2b0t_import_at(
                        &frontend_core::scripts::paths::rs2b0t_import_file(),
                    );
                    self.rs2b0t_catalog_dir = root.clone();
                    app.rs2b0t_catalog_dir = root;
                    self.rs2b0t_catalog_open = false;
                    app.rs2b0t_catalog_open = false;
                    app.error = None;
                }
                Err(error) => app.error = Some(error),
            }
        }
        #[cfg(feature = "js")]
        if self.profile_import_preview_pending && !self.scripts.catalog_work_pending() {
            self.profile_import_preview_pending = false;
            if let Err(error) = self.submit_with(
                app,
                OperatorCommand::PreviewProfileImport,
                Targets::Application,
            ) {
                app.error = Some(error);
            }
            self.show_profile_import(app);
        }
        self.scripts.take_start_failures();
        self.apply_script_notice(app);
        #[cfg(feature = "js")]
        {
            app.reload_confirm = self.scripts.reload_awaiting_confirm();
        }
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
        app.params_state.restart_badge = app.focused_name().and_then(|profile| {
            app.params_card()
                .filter(|selection| self.scripts.restart_required_for(&profile, selection))
                .map(|_| "Restart required to apply saved settings.".to_string())
        });
        let visible = app.modal.as_ref().and_then(|modal| match modal {
            Modal::Confirm(confirm) => match &confirm.kind {
                ConfirmKind::RestartSettings {
                    op, target_count, ..
                } => Some((*op, *target_count)),
                _ => None,
            },
            _ => None,
        });
        let pending = self
            .scripts
            .pending_restart_prompt()
            .map(|prompt| (prompt.operation(), prompt.profiles().len()));
        let replace = match (visible, pending) {
            (Some((current, n)), Some((next, next_n))) if current == next && n == next_n => false,
            (Some(_), Some(_)) => true,
            (Some(_), None) => {
                app.modal = None;
                false
            }
            (None, Some(_)) if app.modal.is_none() => true,
            _ => false,
        };
        if replace {
            if let Some(prompt) = self.scripts.pending_restart_prompt() {
                app.confirm(ConfirmKind::RestartSettings {
                    op: prompt.operation(),
                    target_count: prompt.profiles().len(),
                    summary: prompt.summary(),
                });
            }
        }
    }

    /// A fleet action: core freezes it (`PlanFleetAction`: `target` alone
    /// when one profile is named, else the marked profiles, hidden ones
    /// too, else the loaded shown rows; the card; the skips). A plan that
    /// needs no confirmation runs at once; otherwise the confirm overlay
    /// shows core's prompt. A refusal shows its reason on the message
    /// line.
    fn plan_fleet_action(
        &mut self,
        app: &mut TuiApp,
        action: frontend_core::FleetAction,
        target: Option<&str>,
    ) {
        if let Some(name) = target {
            self.core.stage_plan_target(name);
        }
        #[cfg(feature = "js")]
        if matches!(
            action,
            frontend_core::FleetAction::Start
                | frontend_core::FleetAction::Assign
                | frontend_core::FleetAction::AssignRestart
        ) {
            // The plan freezes the catalog root for the row that runs it.
            let root = self.start_catalog_root();
            self.scripts.stage_start_catalog_root(root);
        }
        if let Err(error) = self.submit_with(
            app,
            OperatorCommand::PlanFleetAction(action),
            Targets::Application,
        ) {
            app.error = Some(error);
            return;
        }
        self.apply_script_notice(app);
        let Some(plan) = self.core.fleet_plan() else {
            app.error = Some(format!("{}: no plan was made", action.verb()));
            return;
        };
        if plan.needs_confirm() {
            let prompt = plan.prompt().to_owned();
            app.confirm(ConfirmKind::FleetPlan { action, prompt });
        } else {
            self.run_fleet_plan(app, action);
        }
    }

    /// Open the world picker over the bound server's public worlds, as
    /// the core cached them when the play started (nothing is fetched or
    /// read here). `profile` is the bot it moves (`W` over the fleet row
    /// under the cursor), or `None` for the marked rows. A server with
    /// one world has no switching: the message line says so, as core's
    /// plan refusal would.
    fn pick_world(&mut self, app: &mut TuiApp, profile: Option<String>) {
        let Some(worlds) = self
            .core
            .switch_world_choices()
            .filter(|worlds| worlds.len() > 0)
        else {
            app.error = Some("Switch world: this server has one world".into());
            return;
        };
        let current = profile
            .as_deref()
            .and_then(|name| app.fleet_model.rows().iter().find(|row| row.name() == name))
            .and_then(|row| row.world());
        app.open_modal(Modal::WorldPicker(crate::world_picker::WorldPicker::open(
            profile, current, worlds,
        )));
    }

    /// Run the waiting fleet plan (`Targets::Plan`). The plan's frozen
    /// targets and card decide everything: marks made or changed since
    /// the plan do not join it. The one result view reports the outcome.
    /// An Assign changes the focused bot's heading when it was a target, so
    /// the Script pane re-reads it (a pending Browse pick stays). With no
    /// plan waiting, the confirmed `action` is refused at the command
    /// boundary.
    fn run_fleet_plan(&mut self, app: &mut TuiApp, action: frontend_core::FleetAction) {
        let Some(plan) = self.core.fleet_plan() else {
            self.refuse_command(
                app,
                OperatorCommand::PlanFleetAction(action),
                &[],
                None,
                "fleet: no plan is waiting",
            );
            return;
        };
        let command = plan.command();
        let selects =
            plan.action() == frontend_core::FleetAction::Login && self.core.selected().is_none();
        let assigns = matches!(
            plan.action(),
            frontend_core::FleetAction::Assign | frontend_core::FleetAction::AssignRestart
        );
        match self.submit_raw_with(app, command, Targets::Plan) {
            Ok(_) => {
                // The core may select the first member during Fleet Log in.
                if selects && self.core.selected().is_some() {
                    self.cancel_startup_focus();
                }
                // A new result replaces the last one, even with the same
                // text and no command record (two emptied plans).
                self.shown_result = None;
                self.project_fleet_result(app);
                if assigns {
                    app.script_sel = app
                        .focused_name()
                        .and_then(|name| self.scripts.heading(&self.core, &name));
                }
            }
            Err(error) => app.error = Some(error),
        }
        #[cfg(feature = "js")]
        {
            app.reload_confirm = self.scripts.reload_awaiting_confirm();
        }
        self.apply_script_notice(app);
    }

    /// Drop the waiting fleet plan without running it (`CancelFleetPlan`).
    fn cancel_fleet_plan(&mut self, app: &mut TuiApp) {
        if let Err(error) =
            self.submit_with(app, OperatorCommand::CancelFleetPlan, Targets::Application)
        {
            app.error = Some(error);
        }
    }

    /// Reload the focused heading's card, or confirm a shown warning.
    #[cfg(feature = "js")]
    fn script_reload(&mut self, app: &mut TuiApp) {
        let focused = app.focused_name();
        let target =
            self.scripts
                .reload_target(&self.core, focused.as_deref(), app.script_sel.as_ref());
        let command = if self.scripts.reload_confirmation_pending(target.as_ref()) {
            OperatorCommand::ConfirmReload
        } else {
            OperatorCommand::ReloadScripts
        };
        self.scripts.stage_reload_target(target);
        if let Err(error) = self.submit_with(app, command, Targets::Application) {
            app.error = Some(error);
        }
        app.reload_confirm = self.scripts.reload_awaiting_confirm();
        self.apply_script_notice(app);
    }

    #[cfg(feature = "js")]
    fn script_reload_cancel(&mut self, app: &mut TuiApp) {
        if let Err(error) =
            self.submit_with(app, OperatorCommand::CancelReload, Targets::Application)
        {
            app.error = Some(error);
        }
        app.reload_confirm = false;
        self.apply_script_notice(app);
    }

    #[cfg(feature = "js")]
    fn loaded_params_card(&self, app: &TuiApp) -> Option<(CardSource, String, PathBuf)> {
        let choice = app.params_card()?;
        let (source, lookup) = choice.loaded_card()?;
        let card = self.scripts.card(source, lookup)?;
        Some((source, card.name().to_owned(), card.path().to_path_buf()))
    }

    fn params_bag(
        &mut self,
        app: &TuiApp,
        profile: &str,
    ) -> Result<serde_json::Map<String, serde_json::Value>, String> {
        if let Some(id) = app.params_card().and_then(|choice| choice.compiled_id()) {
            return self
                .scripts
                .compiled_edit_bag(&self.core, Some(profile), id);
        }
        #[cfg(feature = "js")]
        {
            let (source, name, path) = self
                .loaded_params_card(app)
                .ok_or("parameters unavailable")?;
            Ok(self.scripts.merged_profile_bag(
                &mut self.core,
                profile,
                source,
                &name,
                &path,
                app.params_schema.as_ref(),
            ))
        }
        #[cfg(not(feature = "js"))]
        {
            Err("parameters unavailable".into())
        }
    }
    /// Open the params popup over the focused profile's bag for the card.
    fn open_params(&mut self, app: &mut TuiApp) {
        let Some(profile) = app.focused_name() else {
            self.refuse_command(
                app,
                OperatorCommand::SaveScriptSettings(frontend_core::command::CardKind::Native),
                &[],
                Unavailable::NoFocusedBot.need(),
                "parameters: no focused profile",
            );
            return;
        };
        match self.params_bag(app, &profile) {
            Ok(bag) => app.open_script_params(bag),
            Err(error) => app.error = Some(error),
        }
        self.apply_script_notice(app);
    }

    /// One key into the params popup: edits become typed settings rows on
    /// the focused profile.
    fn params_key(&mut self, app: &mut TuiApp, key: crossterm::event::KeyEvent) -> AppAction {
        let (Some(profile), Some(selection)) = (app.focused_name(), app.params_card()) else {
            app.params_state.open = false;
            return AppAction::None;
        };
        let was_open = app.params_state.open;
        let was_editing = app.params_state.editing;
        let old_params_state = app.params_state.clone();
        let mut pending_setting = None;
        let action = {
            let data = self.bound().and_then(|profile| profile.game_data());
            let mut commit =
                |field: &str, value: serde_json::Value, current: Option<serde_json::Value>| {
                    pending_setting = Some((field.to_owned(), value, current));
                    Ok(())
                };
            app.params_on_key(&mut commit, self.loadouts.store(), data.as_deref(), key)
        };
        if let Some((field, value, current)) = pending_setting {
            let kind = if selection.compiled_id().is_some() {
                frontend_core::command::CardKind::Native
            } else {
                frontend_core::command::CardKind::Script
            };
            self.scripts
                .stage_setting(Some(&profile), &selection, &field, value);
            if let Err(error) = self.submit_with(
                app,
                OperatorCommand::SaveScriptSettings(kind),
                Targets::Profile(&profile),
            ) {
                app.params_state = old_params_state;
                match current {
                    Some(value) => {
                        app.params_bag.insert(field, value);
                    }
                    None => {
                        app.params_bag.remove(&field);
                    }
                }
                app.params_state.error = Some(format!("Save failed: {error}"));
            }
        }
        if key.code == crossterm::event::KeyCode::Esc
            && was_open
            && (!app.params_state.open || (was_editing && !app.params_state.editing))
        {
            let edit_key = app
                .params_schema
                .as_ref()
                .iter()
                .filter(|def| def.visible(&app.params_bag))
                .nth(app.params_state.cursor)
                .map(|field| {
                    frontend_core::scripts::ParameterEditKey::new(
                        Some(&profile),
                        &selection,
                        field.id(),
                    )
                });
            if let Some(edit_key) = edit_key {
                self.scripts.stage_parameter_discard(edit_key);
                if let Err(error) = self.submit_with(
                    app,
                    OperatorCommand::DiscardParameterText,
                    Targets::Application,
                ) {
                    app.error = Some(error);
                }
            }
        }
        self.apply_script_notice(app);
        action
    }

    /// A key for the loadouts popup, when it is open: the popup's cursor and
    /// text rows take the key, and what it asks of the loadout form is
    /// staged and submitted as that row. `false` when the popup is closed.
    fn loadouts_key(&mut self, app: &mut TuiApp, key: crossterm::event::KeyEvent) -> bool {
        if !app.loadouts_state.open {
            return false;
        }
        self.loadouts_opened(app);
        if app.loadouts_state.name_scratch.is_empty() && !self.loadouts.loadouts().is_empty() {
            app.loadouts_state.fill(&self.loadouts);
        }
        let generation = self.loadouts.generation();
        let count = self.loadouts.loadouts().len();
        match app.loadouts_state.on_key(key, count) {
            LoadoutsIntent::None => {}
            LoadoutsIntent::SelectNext => {
                self.loadouts
                    .stage_selection((self.loadouts.selected() + 1) % count);
                self.submit_loadout(app, OperatorCommand::SelectLoadout);
            }
            LoadoutsIntent::Save => {
                self.loadouts.stage_draft(app.loadouts_state.text_loadout());
                self.submit_loadout(app, OperatorCommand::SaveLoadout);
            }
            LoadoutsIntent::Create => self.submit_loadout(app, OperatorCommand::CreateLoadout),
            LoadoutsIntent::Duplicate => {
                self.submit_loadout(app, OperatorCommand::DuplicateLoadout);
            }
            LoadoutsIntent::Delete => self.submit_loadout(app, OperatorCommand::DeleteLoadout),
        }
        if self.loadouts.generation() != generation {
            app.loadouts_state.fill(&self.loadouts);
        }
        true
    }

    /// The loadouts popup was opened since the last look: select its first
    /// loadout and fill the text rows from it.
    fn loadouts_opened(&mut self, app: &mut TuiApp) {
        if !std::mem::take(&mut app.loadouts_state.reopened) {
            return;
        }
        self.loadouts.stage_selection(0);
        self.submit_loadout(app, OperatorCommand::SelectLoadout);
        app.loadouts_state.fill(&self.loadouts);
    }

    /// Submit `command` for the loadout form. What the row did is the
    /// form's status line; a refused command shows on the message line.
    fn submit_loadout(&mut self, app: &mut TuiApp, command: OperatorCommand) {
        if let Err(error) = self.submit_raw(command, Targets::Application) {
            app.error = Some(error.to_string());
        }
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

    /// Keep terminal write errors beside the field that submitted them.
    fn refresh_params_errors(&mut self, app: &mut TuiApp) {
        if !app.params_state.open {
            return;
        }
        let (Some(profile), Some(selection)) = (app.focused_name(), app.params_card()) else {
            return;
        };
        app.params_state.save_errors.retain(|(field, _)| {
            self.scripts
                .parameter_text_error_for(Some(&profile), &selection, field)
                .is_some()
        });
        let mut changed = false;
        for def in app.params_schema.as_ref().iter() {
            let Some(error) =
                self.scripts
                    .parameter_text_error_for(Some(&profile), &selection, def.id())
            else {
                continue;
            };
            if let Some((_, kept)) = app
                .params_state
                .save_errors
                .iter_mut()
                .find(|(field, _)| field == def.id())
            {
                if kept != error {
                    error.clone_into(kept);
                    changed = true;
                }
            } else {
                app.params_state
                    .save_errors
                    .push((def.id().to_owned(), error.to_owned()));
                changed = true;
            }
        }
        if changed {
            // Cancellation has no writer failure event, but still restores
            // the durable bag just as a failed write does.
            self.refresh_params_bag(app);
        }
    }

    /// Freeze Apply to all for the params popup's card and ask to confirm.
    fn prepare_settings_sync(&mut self, app: &mut TuiApp) {
        let Some(profile) = app.focused_name() else {
            return;
        };
        let Some(selection) = app.params_card() else {
            app.error = Some("parameters unavailable".into());
            return;
        };
        let kind = if selection.compiled_id().is_some() {
            frontend_core::command::CardKind::Native
        } else {
            frontend_core::command::CardKind::Script
        };
        self.scripts
            .stage_settings_sync_prepare(&profile, &selection);
        match self.submit_with(
            app,
            OperatorCommand::ApplySettingsToMarked(kind),
            Targets::Profile(&profile),
        ) {
            Ok(_) => match self.scripts.prepared_settings_sync() {
                Some(scope) => {
                    app.params_state.sync_prompt =
                        Some(format!("{} y apply · n cancel", scope.prompt()));
                }
                None => app.error = Some("Apply to all: nothing prepared".into()),
            },
            Err(error) => app.error = Some(error),
        }
        self.apply_script_notice(app);
    }

    fn apply_settings_sync(&mut self, app: &mut TuiApp) {
        app.params_state.sync_prompt = None;
        if let Some(kind) = self.scripts.stage_settings_sync_apply() {
            if let Err(error) = self.submit_with(
                app,
                OperatorCommand::ApplySettingsToMarked(kind),
                Targets::Everyone,
            ) {
                app.error = Some(error);
            }
        } else {
            app.error = Some("Apply to all: nothing prepared".into());
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
            let row = if self.core.script_paused(&name) {
                OperatorCommand::ResumeScript
            } else {
                OperatorCommand::PauseScript
            };
            if let Err(error) = self.submit(row, Targets::Profile(&name)) {
                app.error = Some(error);
            }
        }
    }

    #[cfg(test)]
    /// Request a skip for the focused Quester through the shared command
    /// boundary; its refusal text is shown on the expiring message line.
    fn script_skip_quest(&mut self, app: &mut TuiApp, target: Option<api::selected::RunKey>) {
        let Some(name) = app.focused_name() else {
            self.refuse_command(
                app,
                OperatorCommand::SkipQuest { target },
                &[],
                Unavailable::NoFocusedBot.need(),
                "script: no focused profile",
            );
            return;
        };
        let result = self.submit_with(
            app,
            OperatorCommand::SkipQuest { target },
            Targets::Profile(&name),
        );
        app.error = result.err();
    }

    /// Stop the focused slot's script.
    fn script_stop(&mut self, app: &mut TuiApp) {
        if let Some(name) = app.focused_name() {
            self.scripts.stage_cancel_queued("cancelled");
            if let Err(error) = self.submit_with(
                app,
                OperatorCommand::CancelQueuedStart,
                Targets::Profile(&name),
            ) {
                app.error = Some(error);
            }
            self.apply_script_notice(app);
            if let Err(error) = self.submit(OperatorCommand::StopScript, Targets::Profile(&name)) {
                app.error = Some(error);
            }
        }
    }

    /// Load a local JS bot file into the library, select it for Start,
    /// and persist the store. Errors land on the strip.
    #[cfg(feature = "js")]
    fn script_load(&mut self, app: &mut TuiApp, path: &Path) {
        self.scripts.stage_load_file(path.to_path_buf());
        match self.submit_with(app, OperatorCommand::LoadScriptFile, Targets::Application) {
            Ok(_) => {
                app.script_sel = self.scripts.browse_selection().map(ScriptChoice::duplicate);
                app.browse_changed = true;
                app.error = if self.scripts.load_failure_count() == 0 {
                    None
                } else {
                    Some(self.scripts.named_failure_output())
                };
                if let Some(parent) = path.parent() {
                    self.script_load_last_dir = Some(parent.to_path_buf());
                    app.script_load_last_dir = self.script_load_last_dir.clone();
                }
            }
            Err(error) => {
                app.error = if self.scripts.load_failure_count() > 1 {
                    Some(self.scripts.named_failure_output())
                } else {
                    Some(format!("script: {error}"))
                };
            }
        }
        self.apply_script_notice(app);
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
            // The popup opened with no bot focused, so it has no profile to
            // save the edit on; the next pump binds the bot focused later.
            let missing = Unavailable::NoFocusedBot;
            self.refuse_command(
                app,
                OperatorCommand::SetGuardian,
                &[],
                missing.need(),
                &format!("settings: {}", missing.text()),
            );
            return;
        };
        let changed_policies = self
            .core
            .profile(&name)
            .map(|profile| {
                frontend_core::GUARDIAN_EVENTS
                    .iter()
                    .filter_map(|(event, _)| {
                        let policy = app.settings.random_event_policy(event);
                        (profile.random_event_policy(event) != policy).then_some((*event, policy))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        // Field edit, not a whole-settings replacement: the popup owns the
        // guardian fields, the memory row and the cure poison row, and
        // each row lands through its own write (the form tracks the latest).
        let lowmem = app.settings.lowmem();
        let random_events = app.settings.random_events();
        let lamp_skill = app.settings.lamp_skill().to_string();
        let lamp_auto = app.settings.lamp_auto();
        let lowmem_changed = self
            .core
            .profile(&name)
            .is_some_and(|p| p.lowmem() != lowmem);
        let cure_poison = app.settings.survival().cure_poison();
        let cure_poison_changed = self
            .core
            .profile(&name)
            .is_some_and(|p| p.survival().cure_poison() != cure_poison);
        if lowmem_changed {
            let mode = if lowmem {
                api::lifecycle::MemoryMode::Low
            } else {
                api::lifecycle::MemoryMode::High
            };
            if let Err(error) = self.submit_with(
                app,
                OperatorCommand::SetMemoryMode(mode),
                Targets::Profile(&name),
            ) {
                app.error = Some(format!("settings: {error}"));
            }
        }
        self.core
            .stage_guardian(GuardianEdit::fields(random_events, &lamp_skill, lamp_auto));
        if let Err(error) =
            self.submit_with(app, OperatorCommand::SetGuardian, Targets::Profile(&name))
        {
            app.error = Some(format!("settings: {error}"));
        }
        for (event, policy) in changed_policies {
            self.core
                .stage_guardian(GuardianEdit::policy(event, policy));
            if let Err(error) =
                self.submit_with(app, OperatorCommand::SetGuardian, Targets::Profile(&name))
            {
                app.error = Some(format!("settings: {error}"));
            }
        }
        if cure_poison_changed {
            self.core
                .stage_survival(SurvivalEdit::cure_poison(cure_poison));
            if let Err(error) =
                self.submit_with(app, OperatorCommand::SetSurvival, Targets::Profile(&name))
            {
                app.error = Some(format!("settings: {error}"));
            }
        }
    }

    /// Submit what the settings popup changed in the walk permissions
    /// (`SetNavPreference`, `SetDangerLevel`), then adopt the durable
    /// projection (the core hands it to the play session too) on every pump.
    fn project_walk_globals(&mut self, app: &mut TuiApp) {
        let dirty = std::mem::take(&mut app.nav_preferences_dirty);
        if !dirty.is_empty() && self.persist_ui {
            self.persist_walk_globals(app, &dirty);
        }
        self.project_setting_feedback(app);
        app.refresh_walk_permissions();
        self.core.refresh_walk_globals();
    }

    /// One row per walk permission the operator changed since the last
    /// durable read: the buffer (`app.nav`, the acknowledgements) against
    /// the projection (`app.walk_permissions`).
    fn persist_walk_globals(&mut self, app: &mut TuiApp, dirty: &[frontend_core::NavPreference]) {
        use frontend_core::NavPreference;
        let after = frontend_core::WalkGlobalsView::new(
            app.nav,
            app.script_scope_notice_ack,
            app.survivable_routing_notice_ack,
        );
        let now = after.globals();
        let mut danger_sent = false;
        let mut failure = None;
        for preference in dirty {
            let command = match preference {
                NavPreference::AllowTeleports => {
                    OperatorCommand::SetNavPreference(*preference, now.allow_teleports())
                }
                NavPreference::AllowWilderness => {
                    OperatorCommand::SetNavPreference(*preference, now.allow_wilderness())
                }
                NavPreference::AllowBankFetch => {
                    OperatorCommand::SetNavPreference(*preference, now.allow_bank_fetch())
                }
                NavPreference::AllowDangerZones | NavPreference::SurvivableRouting => {
                    if danger_sent {
                        continue;
                    }
                    danger_sent = true;
                    OperatorCommand::SetDangerLevel(now.danger_level())
                }
                NavPreference::ScriptScopeNoticeAck => {
                    OperatorCommand::SetNavPreference(*preference, after.script_scope_notice_ack())
                }
                NavPreference::SurvivableRoutingNoticeAck => OperatorCommand::SetNavPreference(
                    *preference,
                    after.survivable_routing_notice_ack(),
                ),
                NavPreference::PauseScriptOnManualWalkAbort => OperatorCommand::SetNavPreference(
                    *preference,
                    app.pause_script_on_manual_walk_abort,
                ),
                NavPreference::ShowSpecialAreas => {
                    OperatorCommand::SetNavPreference(*preference, app.map.layers.wilderness)
                }
            };
            if let Err(error) = self.submit_with(app, command, Targets::Application) {
                failure = Some(error);
            }
        }
        match failure {
            None => {
                if app
                    .error
                    .as_deref()
                    .is_some_and(|error| error.starts_with("settings: walk permissions:"))
                {
                    app.error = None;
                }
            }
            Some(error) => app.error = Some(format!("settings: walk permissions: {error}")),
        }
    }

    /// Submit the shared manual-walk pause preference
    /// (`SetNavPreference`): the core applies it to the current and future
    /// slots once it is saved. A refused save shows its reason and the
    /// popup shows the stored value again.
    fn project_manual_walk_pause(&mut self, app: &mut TuiApp) {
        if !std::mem::take(&mut app.pause_script_on_manual_walk_abort_dirty) || !self.persist_ui {
            return;
        }
        let pause = app.pause_script_on_manual_walk_abort;
        let command = OperatorCommand::SetNavPreference(
            frontend_core::NavPreference::PauseScriptOnManualWalkAbort,
            pause,
        );
        match self.submit_with(app, command, Targets::Application) {
            Ok(_) => {
                if app.error.as_deref().is_some_and(|error| {
                    error.starts_with("settings: pause script on manual movement:")
                }) {
                    app.error = None;
                }
            }
            Err(error) => {
                app.error = Some(format!(
                    "settings: pause script on manual movement: {error}"
                ));
                app.pause_script_on_manual_walk_abort = self
                    .core
                    .nav_preference(frontend_core::NavPreference::PauseScriptOnManualWalkAbort)
                    .unwrap_or(true);
            }
        }
        self.project_setting_feedback(app);
    }

    /// Set the global pause policy before binding or arming a TUI live run
    /// (`SetNavPreference`).
    #[cfg(all(test, feature = "js"))]
    fn set_pause_script_on_manual_walk_abort(&mut self, enabled: bool) -> Result<(), String> {
        let command = OperatorCommand::SetNavPreference(
            frontend_core::NavPreference::PauseScriptOnManualWalkAbort,
            enabled,
        );
        self.submit(command, Targets::Application).map(|_| ())
    }

    /// The remembered terrain-bake choice, shared with the panel through
    /// the process service's preferences (`SetMapBakeChoice`).
    fn persist_map_bake(&mut self, app: &mut TuiApp) {
        if !self.persist_ui {
            return;
        }
        let choice = app.map_bake;
        if let Err(error) = self.submit_with(
            app,
            OperatorCommand::SetMapBakeChoice(choice),
            Targets::Application,
        ) {
            app.error = Some(format!("settings: {error}"));
        }
        self.project_setting_feedback(app);
    }

    /// Persist changed source settings and service the shared Path controller.
    fn project_quester_paths(&mut self, app: &mut TuiApp) {
        let selected = self.bound().and_then(|profile| profile.game_data());
        self.project_quester_paths_with_game_data(app, selected);
    }

    fn project_quester_paths_with_game_data(
        &mut self,
        app: &mut TuiApp,
        selected: Option<Arc<api::game_data::SelectedGameData>>,
    ) {
        if std::mem::take(&mut app.quester_paths_dirty) {
            self.core.stage_quester_paths(
                frontend_core::quester_paths::QuesterPathsView::new(
                    app.quester_paths.enabled(),
                    app.quester_paths.folder().to_path_buf(),
                ),
                self.persist_ui,
                selected.clone(),
            );
            let failed = self
                .submit_with(app, OperatorCommand::SetQuesterPaths, Targets::Application)
                .is_err();
            if failed {
                let enabled = app.quester_paths_controller.settings().enabled();
                let folder = app
                    .quester_paths_controller
                    .settings()
                    .folder()
                    .to_path_buf();
                app.quester_paths.set_enabled(enabled);
                app.quester_paths.set_folder(folder);
            }
            self.project_setting_feedback(app);
        }
        self.core
            .advance_quester_paths(&mut app.quester_paths_controller, selected);
    }

    /// Reload the Quester Paths (`ReloadQuesterPaths`): the controller's
    /// notice, shown in the settings popup, says why a reload did not
    /// start.
    fn start_quester_paths_reload(&mut self, app: &mut TuiApp) {
        let selected = self.bound().and_then(|profile| profile.game_data());
        self.core.stage_quester_reload(selected);
        match self.submit_with(
            app,
            OperatorCommand::ReloadQuesterPaths,
            Targets::Application,
        ) {
            Ok(_) => {}
            Err(_) if app.quester_paths_controller.notice_is_error() => {}
            Err(error) => app.error = Some(error),
        }
    }

    /// Clear the banner message once it has stood for
    /// [`frontend_core::ERROR_MESSAGE_TIMEOUT`] with nothing resolving or
    /// replacing it (the same rule as the panel's red line).
    fn expire_error(&mut self, app: &mut TuiApp, now: Instant) {
        self.error_expiry
            .observe(&mut app.error, self.core.refusals(), now);
    }

    /// Refuse `command` before the app can submit it (nothing focused,
    /// nothing picked). The refusal logs at the shared command boundary like
    /// every other refused command, then shows like [`Self::show_refusal`].
    fn refuse_command(
        &mut self,
        app: &mut TuiApp,
        command: OperatorCommand,
        bots: &[&str],
        need: Option<ErrorNeed>,
        reason: &str,
    ) {
        self.core.refuse_unsubmitted(command, bots, reason);
        self.show_refusal(app, need, reason);
    }

    /// Show `reason` on the message line; the line clears when the action
    /// that supplies `need` happens ([`Self::clear_resolved_error`]).
    fn show_refusal(&mut self, app: &mut TuiApp, need: Option<ErrorNeed>, reason: &str) {
        app.error = Some(reason.to_owned());
        if let Some(need) = need {
            self.error_expiry.waits_for(need, reason);
        }
    }

    /// Clear the message line when the need it waits for now holds: the
    /// action that resolves a message clears it at once, whichever action it
    /// was. Called after a focus change and once per tick.
    fn clear_resolved_error(&mut self, app: &mut TuiApp) {
        let Some(need) = self.error_expiry.waiting_need() else {
            return;
        };
        let met = need.met(app);
        self.error_expiry.resolve(&mut app.error, |_| met);
    }

    /// Project each retained preference failure once, and clear it on recovery.
    fn project_preference_error(&mut self, app: &mut TuiApp) {
        let current = self.core.preference_error();
        if self.shown_preference_error.as_deref() == current {
            return;
        }
        if self.shown_preference_error.is_some() && app.error == self.shown_preference_error {
            app.error = None;
        }
        self.shown_preference_error = current.map(str::to_owned);
        if let Some(error) = &self.shown_preference_error {
            app.error = Some(error.clone());
        }
    }

    /// The one preference reconciliation of a pump: every accepted write
    /// settles by its request, and every model (the bake gate, the Quester
    /// source, the walk grants, the session file) follows the latest durable
    /// snapshot, the first readable one included ([`OperatorSession::poll_preferences`]).
    /// The popup and the notices then show each setting's pending and
    /// failed state beside it.
    fn poll_preferences(&mut self, app: &mut TuiApp) {
        {
            let (core, mut surface) = self.core_and_surface_for_preferences(app);
            core.poll_preferences(&mut surface);
        }
        self.project_setting_feedback(app);
    }

    /// The surface [`Self::poll_preferences`] lends: the map desk and the
    /// Quester controller, the models the durable snapshot changes here.
    /// The walk grants reach the app through its own refresh, which keeps
    /// the operator's unsettled drafts.
    fn core_and_surface_for_preferences<'a>(
        &'a mut self,
        app: &'a mut TuiApp,
    ) -> (&'a mut OperatorSession<()>, TuiSurface<'a>) {
        (
            &mut self.core,
            TuiSurface {
                walk_cleared: &mut self.walk_reset,
                map_desk: &mut self.map_desk,
                models: frontend_core::Models::none()
                    .with_quester(&mut app.quester_paths_controller),
                frame_hook: None,
            },
        )
    }

    /// Copy the core's state of every saved setting the TUI shows into the
    /// app: what to show beside it (pending, or the retained failure) and
    /// which drafts are still the operator's own change.
    fn project_setting_feedback(&mut self, app: &mut TuiApp) {
        use frontend_core::{NavPreference as N, Setting as S, SettingStatus};
        const SHOWN: [frontend_core::Setting; 12] = [
            S::Nav(N::AllowTeleports),
            S::Nav(N::AllowWilderness),
            S::Nav(N::AllowBankFetch),
            S::DangerRouting,
            S::Nav(N::PauseScriptOnManualWalkAbort),
            S::Nav(N::ScriptScopeNoticeAck),
            S::Nav(N::SurvivableRoutingNoticeAck),
            S::Nav(N::ShowSpecialAreas),
            S::MapBake,
            S::QuesterPaths,
            S::Notice(frontend_core::OperatorNotice::BackgroundBots),
            S::SessionLog,
        ];
        for setting in SHOWN {
            app.show_setting_status(setting, self.core.setting_status(setting));
        }
        // The Load order has no row of its own in the popup: its failure
        // shows once on the message line (which clears it by the shared
        // rule), and again when a later write fails.
        let order_failure = match self.core.setting_status(S::ProfileOrder) {
            SettingStatus::Failed(text) => Some(text),
            SettingStatus::Saved | SettingStatus::Pending => None,
        };
        if order_failure != self.shown_order_failure.as_deref() {
            self.shown_order_failure = order_failure.map(str::to_owned);
            if let Some(text) = order_failure {
                app.error = Some(text.to_owned());
            }
        }
        let pending = |setting| matches!(self.core.setting_status(setting), SettingStatus::Pending);
        app.set_pending_settings(crate::app::PendingSettings {
            all: !self.persist_ui,
            teleports: pending(S::Nav(N::AllowTeleports)),
            wilderness: pending(S::Nav(N::AllowWilderness)),
            bank_fetch: pending(S::Nav(N::AllowBankFetch)),
            danger: pending(S::DangerRouting),
            scope_ack: pending(S::Nav(N::ScriptScopeNoticeAck)),
            survivable_ack: pending(S::Nav(N::SurvivableRoutingNoticeAck)),
            pause: pending(S::Nav(N::PauseScriptOnManualWalkAbort)),
            special_areas: pending(S::Nav(N::ShowSpecialAreas)),
            map_bake: pending(S::MapBake),
            quester: pending(S::QuesterPaths),
            load_order: pending(S::ProfileOrder),
        });
    }

    /// Copy the focused slot's views into the app and poll the runner.
    fn pump(&mut self, app: &mut TuiApp) {
        self.expire_error(app, Instant::now());
        self.clear_resolved_error(app);
        self.prune_startup_targets();
        self.poll_preferences(app);
        let startup_pending = self.startup_default_pending || !self.startup_profiles.is_empty();
        if startup_pending && self.core.prepare_startup() {
            self.names = self.core.profile_names();
            if self.startup_default_pending {
                self.queue_default_startup();
            }
            self.apply_startup_requests();
        }
        self.project_preference_error(app);
        self.project_walk_globals(app);
        self.project_manual_walk_pause(app);
        self.project_quester_paths(app);
        // The benchmark's focus follows its run: select the member it
        // moved to.
        #[cfg(feature = "memory-profile")]
        if let Some(index) = self.memory.as_ref().map(|run| run.focus_index()) {
            app.focused = Some(index);
            let moved = self
                .memory
                .as_ref()
                .map(|run| run.names()[index].as_str())
                .filter(|name| self.core.selected() != Some(*name))
                .map(str::to_owned);
            if let Some(name) = moved {
                self.focus(&name);
            }
        }
        #[cfg(feature = "memory-profile")]
        if let Some(run) = self.memory.as_mut() {
            match run.poll(&self.core) {
                Ok(true) => {
                    restore_terminal();
                    eprintln!("PASS: memory tui observation complete");
                    exit_after_work_shutdown(0);
                }
                Ok(false) => {}
                Err(error) => {
                    restore_terminal();
                    eprintln!("FAIL: memory tui: {error}");
                    exit_after_work_shutdown(1);
                }
            }
        }

        // Start/Stop return before the isolate is up or reaped. A slot that
        // is offline or queued for login has no observe of its own, so the
        // core resolves every slot (and advances removals), then the TUI
        // commits the Starts that settled.
        self.core.poll();
        self.refresh_session_log_status(app);
        app.settings_state.preferences_pending = self.core.preferences_pending();
        self.poll_scripts(app);
        #[cfg(feature = "js")]
        self.project_profile_import(app);

        // The strip is the fleet: a removed member leaves it at once (its
        // worker may still be logging out). Both copies reuse the app's
        // buffers, so a steady pump allocates nothing here.
        let members = self.core.members();
        let names_changed = app.names.as_slice() != members;
        if names_changed {
            app.names.truncate(members.len());
            let kept = app.names.len();
            app.names.clone_from_slice(&members[..kept]);
            app.names.extend_from_slice(&members[kept..]);
        }
        let ids_changed = app.profile_ids.len() != members.len()
            || members.iter().enumerate().any(|(index, name)| {
                let identity = self.core.profile_identity_or_synthetic(name);
                app.profile_ids.get(index).copied() != Some(identity)
            });
        if ids_changed {
            app.profile_ids.clear();
            app.profile_ids.extend(
                members
                    .iter()
                    .map(|name| self.core.profile_identity_or_synthetic(name)),
            );
        }
        let restart_generation = self.scripts.restart_generation();
        if restart_generation != app.restart_badge_generation || names_changed || ids_changed {
            app.restart_badges.clear();
            for (name, identity) in app.names.iter().zip(app.profile_ids.iter().copied()) {
                if self.scripts.has_restart_badge(name) {
                    app.restart_badges.insert(identity);
                }
            }
            app.restart_badge_generation = restart_generation;
        }
        app.focused = self
            .core
            .selected()
            .and_then(|selected| app.names.iter().position(|n| n == selected));
        self.copy_projection(app);
        // The status pane follows the focused slot. The settings popup has
        // a separate value below because it stays bound across focus changes.
        app.memory = app
            .focused_name()
            .and_then(|name| self.core.memory_status(&name));
        // A Browse pick is the pending selection of the profile whose
        // heading it replaced.
        if std::mem::take(&mut app.browse_changed) {
            let focused = self.last_focused.clone();
            let selection = app.script_sel.as_ref().map(ScriptChoice::duplicate);
            if let (Some(name), Some(selection)) = (focused, selection) {
                self.scripts.stage_browse_pick(selection);
                if let Err(error) =
                    self.submit_with(app, OperatorCommand::PickBrowseCard, Targets::Application)
                {
                    app.error = Some(error);
                } else if let Err(error) = self.submit_with(
                    app,
                    OperatorCommand::SelectScriptCard,
                    Targets::Profile(&name),
                ) {
                    app.error = Some(error);
                }
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
            #[cfg(feature = "js")]
            if heading.as_ref().is_some_and(|sel| {
                sel.loaded_card()
                    .is_some_and(|(source, _)| source == CardSource::Catalog)
            }) {
                self.fill_rs2b0t_cards_once(app);
            }
            app.script_sel = heading;
            if !app.settings_state.open {
                app.settings = focused
                    .as_deref()
                    .and_then(|n| self.core.profile(n))
                    .map(|p| p.settings_draft())
                    .unwrap_or_default();
            }
        }
        let profile_to_bind = if app.settings_state.open && app.settings_profile.is_none() {
            focused.as_deref().map(str::to_owned)
        } else {
            None
        };
        if let Some(name) = profile_to_bind {
            // Freshly opened: bind the focused profile and load its row;
            // later focus changes keep this binding and its buffers.
            app.settings = self
                .core
                .profile(&name)
                .map(|p| p.settings_draft())
                .unwrap_or_default();
            app.settings_title = format!("{} — {name}", crate::settings::TITLE);
            app.settings_profile = Some(name.clone());
            app.settings_dirty = false;
            if let Err(error) = self.submit_with(
                app,
                OperatorCommand::OpenProfileForm,
                Targets::Profile(&name),
            ) {
                app.error = Some(error);
            }
        }
        app.settings_memory = app
            .settings_profile
            .as_deref()
            .and_then(|name| self.core.memory_status(name));

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
        app.script_card_choices = self
            .scripts
            .browse_cards()
            .map(frontend_core::scripts::BrowseCard::selection)
            .collect();
        let present = categories_present(&app.script_cards);
        let order = resolve_category_order(&self.script_category_order, &present);
        if order != self.script_category_order {
            self.script_category_order = order.clone();
        }
        app.script_category_order = self.script_category_order.clone();
        app.params_unavailable = None;
        match app.script_sel.as_ref() {
            Some(sel) => {
                #[cfg(feature = "js")]
                let loaded = if let Some((source, name)) = sel.loaded_card() {
                    if let Some(card) = self.scripts.card(source, name) {
                        app.params_schema.copy_from(card.schema());
                    } else {
                        app.params_schema.clear();
                        app.params_unavailable = Some("script parameters unavailable");
                    }
                    true
                } else {
                    false
                };
                #[cfg(not(feature = "js"))]
                let loaded = false;
                if !loaded {
                    if let Some(id) = sel.compiled_id() {
                        match self.scripts.compiled_schema(
                            &self.core,
                            app.focused_name().as_deref(),
                            id,
                        ) {
                            frontend_core::scripts::SchemaView::Ready { fields, .. } => {
                                app.params_schema.copy_from(fields);
                            }
                            frontend_core::scripts::SchemaView::Unavailable(reason) => {
                                app.params_schema.clear();
                                app.params_unavailable = Some(reason);
                            }
                        }
                    }
                }
            }
            None => app.params_schema.clear(),
        }
        if app.params_state.open && app.params_schema.as_ref().is_empty() {
            app.params_state.open = false;
        }
        self.refresh_params_errors(app);
        #[cfg(feature = "js")]
        {
            app.rs2b0t_catalog_open = self.rs2b0t_catalog_open;
            app.script_load_last_dir = self.script_load_last_dir.clone();
            if !app.rs2b0t_catalog_open {
                app.rs2b0t_catalog_dir = self.rs2b0t_catalog_dir.clone();
            } else {
                self.rs2b0t_catalog_dir.clone_from(&app.rs2b0t_catalog_dir);
            }
        }
        // The map paints `Play`'s packed collision, not the live loc
        // snapshot. Copy the session world each pump (an `Arc` clone)
        // so a loaded pack is not stuck behind the empty-state title.
        app.world = self.core.nav_world();
        // This is the UI loop, not a client's snapshot/tick thread. The first
        // explicit reach-layer request decodes the already verified sidecar.
        app.map_reach = if app.map_active && app.map.layers.reach {
            self.bound().and_then(|profile| {
                let world = app.world.as_ref()?;
                let bound = profile.world()?;
                if !Arc::ptr_eq(world, &bound) {
                    return None;
                }
                profile.walk_reach_source()?.get()
            })
        } else {
            None
        };
        app.refresh();
        self.bind_map_context(app);
        if app.map_active {
            self.poll_map_demand(app);
            app.refresh_walk_send(|name| self.core.walk_slot_status(name));
        }

        if let Some(name) = &focused {
            // The paint-as-chat toggle is operator state: rebuild the
            // pane data without losing it across pumps.
            let show_game_chat = app.chat_data.show_game_chat;
            // Borrow the focused slot's snapshot under the lock and copy
            // the small views the panes need (GameSnapshot is not Clone).
            let read = self.core.with_slot_snapshot(name, |s| {
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
                    if let Some(context) = self.map_context(app) {
                        match frontend_core::walk_map::observed_walk_services(s, context) {
                            Ok(records) => app.map_observed = records,
                            Err(_) => app.map_observed.clear(),
                        }
                    } else {
                        app.map_observed.clear();
                    }
                }
            });
            if read.is_none() {
                // A slot with no published snapshot must not show the
                // previous focused slot's chat / inventory.
                app.chat_data = ChatData::default();
                app.chat_data.show_game_chat = show_game_chat;
                app.inv_items.clear();
                app.stats_rows.clear();
                app.locs_near.clear();
                app.map_observed.clear();
            }
            // The script's paint frame rides the status row (copied from
            // the isolate each observe); the chat pane shows it in place
            // of the game chat while it is non-empty.
            app.chat_data.script_paint = self.core.script_paint(name);
            // Stop drops the isolate (and its paint with it); reset the
            // operator's game-chat toggle when no paint is showing so a
            // fresh Start shows the new paint by default instead of
            // hiding it until `p` again.
            if app.chat_data.script_paint.is_none() {
                app.chat_data.show_game_chat = false;
            }
            app.route = self.core.manual_walk_route(name);
            let walk_cleared = self.core.take_walk_clear();
            if std::mem::take(&mut self.walk_reset) || walk_cleared {
                app.walk_dest = None;
            }
            if self.core.has_play() {
                app.script_state = self.core.script_state(name);
            }
            app.script_queued = self.scripts.start_waiting(name);
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
            app.settings_memory = None;
            if let Err(error) = self.submit_with(
                app,
                OperatorCommand::CloseProfileForm(frontend_core::command::FormClose::Form),
                Targets::Application,
            ) {
                app.error = Some(error);
            }
        }
        // `Saved <name>.` or the failure shows only on the popup the persist
        // came from. A failed write put the profile back, so that popup
        // shows what is saved rather than the edit that did not land. The
        // New profile overlay settles its own form instead while it is
        // open, or while its save is still in flight behind another
        // overlay (the two forms are never open at once).
        if matches!(app.modal, Some(Modal::NewProfile(_))) || self.new_profile_pending.is_some() {
            self.settle_new_profile(app);
        } else {
            for settled in app.settings_save.settle(&mut self.core) {
                let Some(failed) = settled.as_failed() else {
                    continue;
                };
                if !failed.same_form() {
                    continue;
                }
                if let Some(saved) = app
                    .settings_profile
                    .as_deref()
                    .and_then(|name| self.core.profile(name))
                {
                    app.settings = saved.settings_draft();
                }
            }
        }
        if app.map_bake_dirty {
            self.persist_map_bake(app);
            app.map_bake_dirty = false;
        }
    }

    fn script_self_stop_observed(&self, needle: &str) -> bool {
        self.names
            .first()
            .is_some_and(|name| self.core.live_script_self_stop_observed(name, needle))
    }

    /// The `--live` terminal state: `Some(exit code)` when the runner
    /// passed (0) or failed (1); `None` while it runs. Proof lines are
    /// returned, not printed, so a headed loop can hold them until after
    /// alternate-screen restore. The shared live gate decides exactly as the
    /// panel watches do: the witness this run armed fails the run when it
    /// fails, and holds the PASS while Pending, before the clean-stop grace.
    fn live_status(&mut self) -> (Option<i32>, Vec<ProofLine>) {
        let Some(name) = self.live_name.as_deref() else {
            return (None, Vec::new());
        };
        let now = Instant::now();
        let runner = self.core.live_runner();
        let timing = runner.timing_view(now);
        if runner.pair_setup_expired(now) {
            let mut lines = Vec::new();
            if let Some(timing) = timing {
                lines.push(ProofLine::Stderr(live_timing_record(name, timing)));
            }
            lines.push(ProofLine::Stderr(format!(
                "FAIL: live {name} setup: paired fixture did not reach successful relog and scene readiness before its bounded pre-Start deadline"
            )));
            return (Some(1), lines);
        }
        let decision = runner.watch_decision(self.live_core_deadline, now);
        let witnesses = |ok: bool| {
            let mut lines = Vec::new();
            decision.for_each_record(|tag, witness| {
                let line = format!("{tag}: script_{name} {witness}");
                lines.push(if ok {
                    ProofLine::Stdout(line)
                } else {
                    ProofLine::Stderr(line)
                });
            });
            lines
        };
        let fail = |receipt: String, message: &str| {
            let mut lines = witnesses(false);
            if let Some(timing) = timing {
                lines.push(ProofLine::Stderr(live_timing_record(name, timing)));
            }
            lines.push(ProofLine::Stderr(receipt));
            lines.push(ProofLine::Stderr(format!("FAIL: {message}")));
            (Some(1), lines)
        };
        if let Some(message) = self.terminal_startup_failure() {
            return fail(format!("FAIL: live {name} startup: {message}"), &message);
        }
        for gate in [decision.catalog(), decision.pair()] {
            if let Some(message) = gate.failure_message() {
                return fail(format!("FAIL: live {name} {message}"), message);
            }
        }
        let status = runner.status();
        if status == Some(frontend_core::live::RunnerStatus::Passed) {
            let mut grace = self.live_stop_wait_started;
            let hold = decision.pass_hold(
                runner.wait_script_stop(),
                |needle| self.script_self_stop_observed(needle),
                &mut grace,
                now,
            );
            self.live_stop_wait_started = grace;
            match hold.kind() {
                frontend_core::live::PassHoldKind::Core
                | frontend_core::live::PassHoldKind::CleanStop => return (None, Vec::new()),
                frontend_core::live::PassHoldKind::TimedOut => {
                    let message = hold
                        .failure_message()
                        .expect("timed-out clean-stop holds carry their failure reason");
                    return fail(format!("FAIL: live {name} {message}"), message);
                }
                frontend_core::live::PassHoldKind::Release => {}
            }
        }
        let evidence = runner.evidence_json().unwrap_or_default();
        let failure = runner.failure_message();
        let soaking = self.live_soak_until.is_some_and(|t| now < t);
        let was_announced = self.live_announced_pass;
        let (code, proof, announced) = live_proof(
            name,
            status,
            &evidence,
            failure.as_deref(),
            soaking,
            self.live_announced_pass,
        );
        let timing_due = code.is_some() || (announced && !was_announced);
        let mut lines = match code {
            Some(1) => witnesses(false),
            _ if announced && !was_announced => witnesses(true),
            _ => Vec::new(),
        };
        if timing_due {
            if let Some(timing) = timing {
                let receipt = live_timing_record(name, timing);
                lines.push(if code == Some(1) {
                    ProofLine::Stderr(receipt)
                } else {
                    ProofLine::Stdout(receipt)
                });
            }
        }
        lines.extend(proof);
        self.live_announced_pass = announced;
        if code == Some(1) && std::env::var_os("BOT_DEBUG").is_some() {
            if let Some(slot) = self.names.first() {
                if let Some(details) = self.core.live_script_lifecycle_debug(slot) {
                    lines.push(ProofLine::Stderr(format!(
                        "[tui-play] script lifecycle slot={slot:?} {details}"
                    )));
                }
            }
        }
        (code, lines)
    }

    /// Return a producer-marked terminal asset-init failure for a slot owned
    /// by this scenario. Retryable login errors intentionally remain pending.
    fn terminal_startup_failure(&self) -> Option<String> {
        self.core.live_terminal_startup_failure()
    }

    /// "Got it": the acknowledgement is durable-only. It hides the notice
    /// when the saved value holds it ([`Self::refresh_background_notice`]);
    /// a pending or failed write leaves the notice up with its state beside
    /// it.
    fn ack_background_bots(&mut self, app: &mut TuiApp) {
        if self.persist_ui {
            self.core
                .stage_notice(frontend_core::OperatorNotice::BackgroundBots);
            if let Err(error) =
                self.submit_raw(OperatorCommand::AcknowledgeNotice, Targets::Application)
            {
                app.error = Some(error.to_string());
            }
            frontend_core::resources::clear_background_bots_ack_error(&mut app.error);
            self.ack_checked_at = None;
            self.project_setting_feedback(app);
            self.notice_sig = None;
            self.refresh_background_notice(app, Instant::now());
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
            self.background_bots_acked =
                frontend_core::resources::background_bots_acked(&self.preferences);
            self.ack_checked_at = Some(now);
        }
    }

    fn refresh_background_notice(&mut self, app: &mut TuiApp, now: Instant) {
        if !self.persist_ui {
            self.notice_sig = None;
            app.background_notice = None;
            return;
        }
        if self.notice_feedback.as_ref()
            != app.setting_feedback(Setting::Notice(
                frontend_core::OperatorNotice::BackgroundBots,
            ))
        {
            // The write settled: read the durable acknowledgement now.
            self.ack_checked_at = None;
        }
        self.refresh_ack_cache(now);
        let background = self.core.background_bot_count();
        if self.background_bots_acked || background == 0 {
            self.notice_sig = None;
            self.notice_feedback = None;
            app.background_notice = None;
            return;
        }
        // The notice leaves only when the durable acknowledgement is held:
        // while the write is pending, and after it failed, the notice stays
        // and says so beside "Got it".
        let generation = self.core.resource_generation();
        let setting = Setting::Notice(frontend_core::OperatorNotice::BackgroundBots);
        if self.notice_sig != Some((background, generation))
            || self.notice_feedback.as_ref() != app.setting_feedback(setting)
        {
            let feedback = app.setting_feedback(setting).cloned();
            let mut text = background_ack_text(background, self.core.resources());
            match &feedback {
                Some(SettingFeedback::Pending) => text.push_str(" — saving…"),
                Some(SettingFeedback::Failed(failure)) => {
                    text.push_str(" — ");
                    text.push_str(failure);
                }
                None => {}
            }
            app.background_notice = Some(text);
            self.notice_sig = Some((background, generation));
            self.notice_feedback = feedback;
        }
    }

    /// Follow the core's shared projection: refresh the fleet model (one
    /// cheap check per loop; it rebuilds only when the rows or the vault's
    /// profiles moved), then copy the fleet counts, the selected slot's
    /// detail and the resource meter, each only when it moved since the
    /// last copy (and into the app's own buffers).
    fn copy_projection(&mut self, app: &mut TuiApp) {
        app.fleet_model.refresh(&self.core);
        app.fleet_model.sync_card_label(&self.scripts, &self.core);
        self.project_fleet_result(app);
        let view = self.core.fleet_view();
        if self.copied_rows.replace(view.rows_generation()) != Some(view.rows_generation()) {
            app.counts = view.counts();
        }
        if self.copied_view.replace(view.generation()) != Some(view.generation()) {
            view.copy_detail_into(&mut app.detail);
        }
        let meter = self.core.resource_generation();
        if self.copied_meter.replace(meter) != Some(meter) {
            app.resources.copy_from(self.core.resources());
        }
        let capacity = self.core.capacity_generation();
        if self.copied_capacity.replace(capacity) != Some(capacity) {
            match self.core.capacity_notice() {
                Some(notice) => {
                    let line = app.capacity.get_or_insert_with(String::new);
                    line.clear();
                    line.push_str(notice.notice_text());
                }
                None => app.capacity = None,
            }
        }
    }
}

/// The handlers of the fleet marks and filter, the log, the map overlay
/// preference, the Quester Paths notice and Quit: each is a plain submit
/// over the models the app lends.
impl TuiSession {
    /// Mark or unmark `name`'s fleet row (`SetMark`).
    fn set_mark(&mut self, app: &mut TuiApp, name: &str, on: bool) {
        if let Err(error) =
            self.submit_with(app, OperatorCommand::SetMark(on), Targets::Profile(name))
        {
            app.error = Some(error);
        }
    }

    /// Mark these members' fleet rows (`MarkAll`).
    fn mark_all(&mut self, app: &mut TuiApp, identities: Vec<frontend_core::ProfileIdentity>) {
        self.core.stage_mark_all(identities);
        if let Err(error) = self.submit_with(app, OperatorCommand::MarkAll, Targets::Application) {
            app.error = Some(error);
        }
    }

    /// Clear every row mark (`ClearMarks`).
    fn clear_marks(&mut self, app: &mut TuiApp) {
        if let Err(error) = self.submit_with(app, OperatorCommand::ClearMarks, Targets::Application)
        {
            app.error = Some(error);
        }
    }

    /// Mark, unmark or invert the shown rows, or mark the shown rows in a
    /// status (`MarkFleet`). Core decides which rows; hidden marks stay.
    fn mark_fleet(&mut self, app: &mut TuiApp, op: frontend_core::MarkOp) {
        if let Err(error) =
            self.submit_with(app, OperatorCommand::MarkFleet(op), Targets::Application)
        {
            app.error = Some(error);
        }
    }

    /// Make `card` the card the fleet actions use (`SetFleetCard`). The
    /// core changes nothing else: no Browse pick, form, assignment or run.
    fn pick_fleet_card(&mut self, app: &mut TuiApp, card: frontend_core::FleetCard) {
        app.fleet_model.stage_card(card);
        if let Err(error) =
            self.submit_with(app, OperatorCommand::SetFleetCard, Targets::Application)
        {
            app.error = Some(error);
        }
        // Show the new card now, not on the next pump.
        app.fleet_model.sync_card_label(&self.scripts, &self.core);
    }

    /// Show the latest fleet result on the message line, once: the text is
    /// copied only when core changed it (a settling action rewrites it in
    /// place as its members finish).
    fn project_fleet_result(&mut self, app: &mut TuiApp) {
        let Some(result) = self.core.fleet_result() else {
            return;
        };
        let (request, text) = (result.request(), result.text());
        if text.is_empty()
            || self
                .shown_result
                .as_ref()
                .is_some_and(|(shown, shown_text)| *shown == request && shown_text == text)
        {
            return;
        }
        match &mut self.shown_result {
            Some((shown, shown_text)) => {
                *shown = request;
                shown_text.clear();
                shown_text.push_str(text);
            }
            None => self.shown_result = Some((request, text.to_owned())),
        }
        app.error = Some(text.to_owned());
    }

    /// Reorder the saved Load order (`ReorderProfiles`): stage the op, then
    /// submit it with the fleet model lent. The core refuses it unless the
    /// model shows Load order. The cursor follows its profile by identity.
    fn reorder_profiles(&mut self, app: &mut TuiApp, op: frontend_core::ReorderOp) {
        self.core.stage_reorder(op);
        if let Err(error) =
            self.submit_with(app, OperatorCommand::ReorderProfiles, Targets::Application)
        {
            app.error = Some(error);
        }
        // Follow the new order now, so a repeated key sees it.
        app.fleet_model.refresh(&self.core);
        app.table.sync(&app.fleet_model);
    }

    /// Show `order` (`SetFleetSort`). The shown rows follow it and the
    /// cursor stays on its profile.
    fn set_fleet_sort(&mut self, app: &mut TuiApp, order: frontend_core::FleetOrder) {
        if let Err(error) = self.submit_with(
            app,
            OperatorCommand::SetFleetSort(order),
            Targets::Application,
        ) {
            app.error = Some(error);
        }
        app.table.sync(&app.fleet_model);
    }

    /// Edit the fleet filter text and commit it (`SetFleetFilter`); the
    /// shown rows follow the text.
    fn set_fleet_filter(&mut self, app: &mut TuiApp, edit: crate::fleet::FilterEdit) {
        let staged = app.fleet_model.staged_filter_mut();
        match edit {
            crate::fleet::FilterEdit::Push(c) => staged.push(c),
            crate::fleet::FilterEdit::Pop => {
                staged.pop();
            }
            crate::fleet::FilterEdit::Clear => staged.clear(),
        }
        if let Err(error) =
            self.submit_with(app, OperatorCommand::SetFleetFilter, Targets::Application)
        {
            app.error = Some(error);
        }
        app.table.sync(&app.fleet_model);
    }

    /// Save the rows the log view shows (`SaveLog`). The write runs off the
    /// UI thread; the log pane's status line follows it.
    fn save_log(&mut self, app: &mut TuiApp) {
        let focused = app.focused_name();
        let (path, text) = app.log.save_request(focused.as_deref());
        self.core.stage_log_save(path, text);
        match self.submit_with(app, OperatorCommand::SaveLog, Targets::Application) {
            Ok(submitted) => match self.core.log_save(submitted.request()) {
                Some(ticket) => app.log.saving(ticket),
                None => app.log.status = Some("save log: the write did not start".into()),
            },
            Err(error) => app.log.status = Some(error),
        }
    }

    /// Turn the session log file on or off (`SetSessionLog`). The saved
    /// choice and the file are two states, and the log pane's status line
    /// follows both whoever changed them ([`Self::refresh_session_log_status`]).
    fn set_session_log(&mut self, app: &mut TuiApp, on: bool) {
        match self.submit_with(
            app,
            OperatorCommand::SetSessionLog(on),
            Targets::Application,
        ) {
            Ok(_) => {
                self.project_setting_feedback(app);
                self.refresh_session_log_status(app);
            }
            Err(error) => app.log.status = Some(error),
        }
    }

    /// The log pane's status line follows the session file every frame,
    /// whether the operator toggled it, it was on at startup, or another
    /// process changed it: "saving…" while the choice is written, the
    /// core's retained failure at the setting (a choice that did not save,
    /// or one that saved but whose file could not be opened or written,
    /// also after it had opened) for the shared error timeout, then
    /// nothing, and the file the process holds open from the sink's own
    /// state. A frame compares the stage and the failure text with what it
    /// showed and builds text only when they moved. The line is cleared
    /// only when it still holds the text this put there, so a save-log
    /// status is never taken away.
    fn refresh_session_log_status(&mut self, app: &mut TuiApp) {
        use frontend_core::log_file::SessionFilePhase;
        let (key, failure) = match app.setting_feedback(Setting::SessionLog) {
            Some(SettingFeedback::Pending) => (SessionFileKey::Saving, None),
            Some(SettingFeedback::Failed(failure)) => {
                (SessionFileKey::Failure, Some(failure.as_str()))
            }
            None => match frontend_core::log_file::session_file_phase() {
                SessionFilePhase::Off => (SessionFileKey::Off, None),
                SessionFilePhase::Opening => (SessionFileKey::Opening, None),
                SessionFilePhase::Writing => (SessionFileKey::Writing, None),
                // The failure left with its timeout: nothing more to say.
                SessionFilePhase::Failed => (SessionFileKey::Quiet, None),
            },
        };
        let shown = &self.session_file_shown;
        let unchanged = key == shown.key
            && failure.is_none_or(|failure| shown.text.as_deref() == Some(failure));
        if unchanged {
            return;
        }
        let was_open = match key {
            SessionFileKey::Opening | SessionFileKey::Writing => true,
            SessionFileKey::Off => false,
            _ => shown.was_open,
        };
        let text = match key {
            SessionFileKey::Saving => Some("session file: saving…".to_owned()),
            SessionFileKey::Failure => failure.map(str::to_owned),
            SessionFileKey::Opening | SessionFileKey::Writing => {
                let status = frontend_core::log_file::session_file_status();
                let path = status
                    .path()
                    .map_or_else(String::new, |path| path.display().to_string());
                Some(if key == SessionFileKey::Opening {
                    format!("session file opening {path}")
                } else {
                    format!("session file {path}")
                })
            }
            SessionFileKey::Off => shown.was_open.then(|| "session file off".to_owned()),
            SessionFileKey::Quiet => None,
        };
        match &text {
            Some(text) => app.log.status = Some(text.clone()),
            None => {
                if app.log.status.is_some() && app.log.status == self.session_file_shown.text {
                    app.log.status = None;
                }
            }
        }
        self.session_file_shown = SessionFileShown {
            key,
            text,
            was_open,
        };
    }

    /// Save the map's special-areas overlay choice (`SetNavPreference`),
    /// which the panel shares; a refused save shows its reason.
    fn set_special_areas(&mut self, app: &mut TuiApp, on: bool) {
        let command =
            OperatorCommand::SetNavPreference(frontend_core::NavPreference::ShowSpecialAreas, on);
        match self.submit_with(app, command, Targets::Application) {
            Ok(_) => {
                if app
                    .error
                    .as_deref()
                    .is_some_and(|error| error.starts_with("map wilderness:"))
                {
                    app.error = None;
                }
            }
            Err(error) => app.error = Some(format!("map wilderness: {error}")),
        }
        self.project_setting_feedback(app);
    }

    /// Dismiss the Quester Paths status line (`AcknowledgeNotice`).
    fn dismiss_paths_notice(&mut self, app: &mut TuiApp) {
        self.core
            .stage_notice(frontend_core::OperatorNotice::QuesterPaths);
        if let Err(error) = self.submit_with(
            app,
            OperatorCommand::AcknowledgeNotice,
            Targets::Application,
        ) {
            app.error = Some(error);
        }
    }

    /// Quit (`Quit`): every member's slot shuts down and the play closes
    /// before the terminal is torn down. Nothing loaded is not an error.
    fn quit(&mut self, app: &mut TuiApp) {
        match self.submit_raw(OperatorCommand::Quit, Targets::Everyone) {
            Ok(_) | Err(SubmitError::NoTarget(_)) => {}
            Err(error) => app.error = Some(error.to_string()),
        }
        app.quit = true;
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

fn live_timing_record(name: &str, timing: frontend_core::live::LiveTimingView) -> String {
    let millis = |duration: Duration| duration.as_millis().min(u128::from(u64::MAX)) as u64;
    let receipt = serde_json::json!({
        "setup_elapsed_ms": millis(timing.setup_elapsed()),
        "script_elapsed_ms": timing.script_elapsed().map(millis),
    });
    format!("LIVE_TIMING: script_{name} {receipt}")
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
    status: Option<frontend_core::live::RunnerStatus>,
    evidence: &str,
    failure: Option<&str>,
    soaking: bool,
    announced_pass: bool,
) -> (Option<i32>, Vec<ProofLine>, bool) {
    match status {
        Some(frontend_core::live::RunnerStatus::Passed) => {
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
        Some(frontend_core::live::RunnerStatus::Failed) => {
            let message = failure.expect("failed runner exposes its failure reason");
            (
                Some(1),
                vec![
                    ProofLine::Stderr(format!("FAIL: live {name} {evidence}")),
                    ProofLine::Stderr(format!("FAIL: {message}")),
                ],
                announced_pass,
            )
        }
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

fn prompt_instance_conflict(conflict: &frontend_core::launch::InstanceConflict) -> bool {
    eprintln!("{}", conflict.message());
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
fn new_app(title: impl Into<String>, preferences: &UiPreferences) -> TuiApp {
    let mut app = TuiApp::new(title);
    app.restore_preferences(preferences.share());
    app
}

/// Build the interactive landing that `run` opens after it unlocks the vault.
fn interactive_landing(session: &mut TuiSession, args: &Args) -> Result<TuiApp, String> {
    let focus = session.bootstrap_interactive_profiles(&args.users)?;
    if let Some(focus) = &focus {
        session.focus(focus);
        // Retain the explicit profile's UID before any later rename.
        session.queue_startup_focus(focus);
    } else if !session.names.is_empty() {
        // The shared preparation step chooses the default from saved order.
        session.startup_default_pending = true;
    }
    let mut app = new_app(
        format!(
            "{} headless · {}",
            session.app_title(),
            session.profile_label()
        ),
        &session.preferences,
    );
    app.names = session.names.clone();
    app.focused = focus
        .as_ref()
        .and_then(|focus| session.names.iter().position(|name| name == focus));
    if session.names.is_empty() {
        app.open_modal(Modal::NewProfile(
            crate::new_profile::NewProfileView::default(),
        ));
        session.open_new_profile(&mut app);
    }
    Ok(app)
}

fn unknown_profile_error(name: &str) -> String {
    format!(
        "no profile named {name} in the vault: --user only selects existing \
         profiles and creates none (start tui-play without --user and add \
         it with New profile: press N in the Fleet pane, then type its \
         username and password)"
    )
}

fn run(args: &Args, mode: RunMode) -> Result<i32, String> {
    let memory = {
        #[cfg(feature = "memory-profile")]
        {
            frontend_core::bench::BenchConfig::from_env()
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
    let permit = match frontend_core::launch::acquire_instance(
        frontend_core::launch::InstanceKind::Tui,
        skip_lock,
    ) {
        Ok(Ok(permit)) => permit,
        Ok(Err(conflict)) => {
            if !prompt_instance_conflict(&conflict) {
                return Ok(0);
            }
            frontend_core::launch::InstancePermit::skip()
        }
        Err(e) => return Err(format!("instance lock: {e}")),
    };
    let mut launcher = frontend_core::launch::ProfileLauncher::legacy_local();
    launcher
        .configure(
            args.profile.duplicate(),
            frontend_core::launch::ProfileEnv::capture(),
        )
        .map_err(|error| error.to_string())?;
    // A missing vault cannot contain the requested profile. Refuse it before
    // binding, which reads the engine key and prepares the game cache, and so
    // before any passphrase prompt; the interactive branch checks again before
    // opening the vault.
    if !memory && matches!(&mode, RunMode::Interactive) {
        if let Some(name) = args.users.first() {
            let vault_path = launcher
                .unbound_vault_path(None)
                .map_err(|error| error.to_string())?;
            if !vault_path.is_file() {
                return Err(unknown_profile_error(name));
            }
        }
    }
    launcher
        .bind_with(None)
        .map_err(|error| error.to_string())?;
    if let Some(number) = args.world {
        let mut worlds = launcher
            .bound()
            .and_then(|profile| profile.public_world_numbers())
            .ok_or("--world requires the rs2b2t profile")?;
        if !worlds.any(|world| world == number) {
            return Err(format!(
                "world {number} is not in the configured public worlds"
            ));
        }
    }
    let mut session = TuiSession::with_launcher(launcher, permit);
    session.auto_world = args.world;
    session
        .bound()
        .expect("bound session")
        .require_bot_operation()
        .map_err(|error| error.to_string())?;

    #[cfg(feature = "memory-profile")]
    if let Some(config) =
        frontend_core::bench::BenchConfig::from_env().map_err(|error| error.to_string())?
    {
        frontend_core::bench::require_live_benchmark(session.profile_class())
            .map_err(|error| error.to_string())?;
        session.persist_ui = false;
        // Vault/card first; unlock constructs Play once (single load_pack).
        let run = frontend_core::bench::BenchRun::prepare(config, "tui")
            .map_err(|error| error.to_string())?;
        session.mainland = true;
        session.unlock_at(run.vault(), run.passphrase())?;
        run.bind_seed_nav(&session.core)
            .map_err(|error| error.to_string())?;
        session.names = run.names().to_vec();
        session.load_and_login_all();
        session.focus(&run.names()[0]);
        let mut app = new_app(
            format!(
                "{} memory benchmark · {}",
                session.app_title(),
                session.profile_label()
            ),
            &session.preferences,
        );
        app.names = run.names().to_vec();
        app.focused = Some(0);
        session.memory = Some(run);
        return run_loop(session, app);
    }

    match mode {
        RunMode::Live(name) => {
            let scenario = live_scenario(&name)?;
            session.mainland = scenario.view().mainland();
            session.live_catalog_core = args.catalog_core;
            session.live_pair_core = args.pair_core;
            session.live_prepare_script(scenario)?;
            let mut app = new_app(
                format!("tui-play --live {name} · {}", session.profile_label()),
                &session.preferences,
            );
            app.names = session.names.clone();
            app.focused = Some(0);
            run_loop(session, app)
        }
        RunMode::Interactive => {
            let vault_path = session
                .bound()
                .expect("bound session")
                .vault_path()
                .to_path_buf();
            let vault_exists = vault_path.is_file();
            if !vault_exists {
                if let Some(name) = args.users.first() {
                    return Err(unknown_profile_error(name));
                }
            }
            let pass =
                frontend_core::launch::obtain_passphrase("tui-play", args.pass_stdin, vault_exists)
                    .map_err(|error| error.to_string())?;
            if let Err(e) = session.unlock_at(&vault_path, &pass) {
                return Err(format!("vault {}: {e}", vault_path.display()));
            }
            // The passphrase has done its job; do not keep it through the run.
            drop(pass);
            // A fresh vault starts empty: no seeded `test` profile (M-311),
            // and `--user` creates none. An empty vault opens on the New
            // profile form, where the operator types the first account's
            // own username and password.
            let app = interactive_landing(&mut session, args)?;
            run_loop(session, app)
        }
    }
}

impl TuiSession {
    /// Validate explicit `--user` profiles before the event loop. The default
    /// startup target waits for shared preparation to reconcile saved order.
    /// An empty vault opens the New profile form; no profile is seeded.
    fn bootstrap_interactive_profiles(
        &mut self,
        users: &[String],
    ) -> Result<Option<String>, String> {
        self.names = self
            .core
            .profiles()
            .map(|p| p.username().to_string())
            .collect();
        for user in users {
            if !self.names.contains(user) {
                return Err(unknown_profile_error(user));
            }
        }
        Ok(users.first().cloned())
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

/// One pass of the headless loop: pump the session, print the proof lines,
/// and return the live run's exit code once it decided.
fn headless_step(session: &mut TuiSession, app: &mut TuiApp) -> Option<i32> {
    session.pump(app);
    let (code, lines) = session.live_status();
    write_proof_lines(&lines);
    code
}

/// One pass of the headed loop before it waits for input: pump the session,
/// hold the proof lines until the terminal is restored, and draw. Returns
/// the live run's exit code, without drawing, once it decided.
fn headed_frame<B: ratatui::backend::Backend>(
    session: &mut TuiSession,
    app: &mut TuiApp,
    terminal: &mut Terminal<B>,
    deferred: &mut Vec<ProofLine>,
) -> Result<Option<i32>, String> {
    session.pump(app);
    let (code, lines) = session.live_status();
    deferred.extend(lines);
    if code.is_some() {
        return Ok(code);
    }
    let _profile_frame = client::profiling::UI_FRAME.start();
    let params_data = session.bound().and_then(|profile| profile.game_data());
    session.loadouts_opened(app);
    terminal
        .draw(|frame| {
            let _profile_draw = client::profiling::UI_DRAW.start();
            app.draw(frame);
            app.draw_loadouts_overlay(frame, &session.loadouts);
            app.draw_params_overlay(frame, session.loadouts.store(), params_data.as_deref());
        })
        .map_err(|e| e.to_string())?;
    Ok(None)
}

/// The crossterm event loop. `--live` runs headed when a controlling
/// terminal is available (the operator watches the panes) and degrades to
/// a headless pump loop otherwise, so the PASS/FAIL still lands in CI.
/// Headless prints proof immediately. Headed holds it until after restore
/// so the PASS JSON line cannot paint into Ratatui rows.
fn run_loop(mut session: TuiSession, mut app: TuiApp) -> Result<i32, String> {
    app.map_bake = session.map_desk.bake_gate().choice();
    if enable_raw_mode().is_err() {
        // No controlling terminal: pump the runner without drawing.
        loop {
            if let Some(code) = headless_step(&mut session, &mut app) {
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
        if let Some(code) = headed_frame(&mut session, &mut app, &mut terminal, &mut deferred)? {
            return Ok(code);
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

/// Route one [`AppAction`] onto the session: map walks arm through the
/// core's walk arms, chat and manual walks go through the core's slot
/// accessors, the script actions dispatch the shared script coordinator
/// (Start / pause / stop / the JS library), and the settings popup persists
/// on the next pump.
fn dispatch(session: &mut TuiSession, app: &mut TuiApp, action: AppAction) {
    match action {
        AppAction::Quit => session.quit(app),
        AppAction::Refused(refusal) => {
            let bot = refusal.bot.as_deref();
            match refusal.command {
                Some(command) => session.refuse_command(
                    app,
                    command,
                    bot.as_slice(),
                    refusal.need,
                    &refusal.reason,
                ),
                None => session.show_refusal(app, refusal.need, &refusal.reason),
            }
        }
        AppAction::Focus(name) => {
            session.focus(&name);
            session.clear_resolved_error(app);
        }
        AppAction::MapOpen => {
            session.bind_map_context(app);
            session.open_map_catalogue(app);
        }
        AppAction::MapClose => {
            session.release_map_catalogue();
            app.map_reach = None;
            app.map_model.close();
        }
        AppAction::ArmWalk(tile) => session.arm_walk_on(app, tile),
        AppAction::MapWalkGroup => session.map_walk_group(app),
        AppAction::WalkTile(tile) => session.wasd_walk(app, tile),
        AppAction::MapTeleport(tile) => session.map_teleport(app, tile),
        AppAction::ReloadPaths => session.start_quester_paths_reload(app),
        AppAction::DismissPathsNotice => session.dismiss_paths_notice(app),
        AppAction::SetMark(name, on) => session.set_mark(app, &name, on),
        AppAction::MarkAll(identities) => session.mark_all(app, identities),
        AppAction::ClearMarks => session.clear_marks(app),
        AppAction::MarkFleet(op) => session.mark_fleet(app, op),
        AppAction::PickFleetCard(card) => session.pick_fleet_card(app, card),
        AppAction::ReorderProfiles(op) => session.reorder_profiles(app, op),
        AppAction::FleetFilter(edit) => session.set_fleet_filter(app, edit),
        AppAction::SetFleetSort(order) => session.set_fleet_sort(app, order),
        AppAction::SaveLog => session.save_log(app),
        AppAction::SetSessionLog(on) => session.set_session_log(app, on),
        AppAction::MapWilderness(on) => session.set_special_areas(app, on),
        AppAction::Chat(action) => session.chat_send(app, action),
        AppAction::Login => session.login(app),
        AppAction::Logout => session.logout(app),
        AppAction::MemoryRelog(name) => session.memory_relog_request(app, &name),
        AppAction::MemoryRelogNow(name) => session.memory_relog_now(app, &name),
        AppAction::Remove(name) => session.remove(app, &name),
        AppAction::ScriptStart(sel) => session.script_start(app, &sel),
        AppAction::ScriptPause => session.script_toggle_pause(app),
        #[cfg(test)]
        AppAction::ScriptSkipQuest(target) => session.script_skip_quest(app, target),
        AppAction::ScriptStop => session.script_stop(app),
        AppAction::ScriptBrowse =>
        {
            #[cfg(feature = "js")]
            if app.script_browse_open {
                session.on_script_browse_open(app);
            }
        }
        #[cfg(feature = "js")]
        AppAction::ScriptImportCatalog => {
            session.rs2b0t_catalog_open = true;
            session.rs2b0t_catalog_dir = app.rs2b0t_catalog_dir.clone();
            app.rs2b0t_catalog_open = true;
            app.catalog_sel = 0;
        }
        #[cfg(feature = "js")]
        AppAction::ScriptDeferCatalog => session.defer_rs2b0t_catalog(app),
        #[cfg(feature = "js")]
        AppAction::ScriptUseCatalog => {
            let root = app.rs2b0t_catalog_dir.clone();
            session.rs2b0t_catalog_dir = root.clone();
            app.error = match session.import_rs2b0t_catalog(app, &root) {
                Ok(_) => {
                    if session.scripts.load_failure_count() == 0 {
                        None
                    } else {
                        Some(session.scripts.named_failure_output())
                    }
                }
                Err(error) => Some(error),
            };
        }
        #[cfg(feature = "js")]
        AppAction::ScriptLoad(path) => session.script_load(app, &path),
        AppAction::ScriptParams => session.open_params(app),
        AppAction::PlanFleet(action) => session.plan_fleet_action(app, action, None),
        AppAction::PickWorld(profile) => session.pick_world(app, profile),
        AppAction::PlanSwitchWorld(profile, world) => session.plan_fleet_action(
            app,
            frontend_core::FleetAction::SwitchWorld(world),
            profile.as_deref(),
        ),
        AppAction::RunFleetPlan(action) => session.run_fleet_plan(app, action),
        AppAction::CancelFleetPlan => session.cancel_fleet_plan(app),
        #[cfg(feature = "js")]
        AppAction::ScriptReload => session.script_reload(app),
        #[cfg(feature = "js")]
        AppAction::ScriptReloadCancel => session.script_reload_cancel(app),
        AppAction::ScriptSyncPrepare => session.prepare_settings_sync(app),
        AppAction::ScriptSyncApply => session.apply_settings_sync(app),
        AppAction::ScriptSyncCancel => {
            if let Err(error) = session.submit_with(
                app,
                OperatorCommand::CancelSettingsSync,
                Targets::Application,
            ) {
                app.error = Some(error);
            }
        }
        AppAction::ScriptRestartSettings => {
            #[cfg(feature = "js")]
            {
                let root = session.start_catalog_root();
                session.scripts.stage_start_catalog_root(root);
            }
            if let Err(error) = session.submit_with(
                app,
                OperatorCommand::RestartForSettings,
                Targets::Application,
            ) {
                app.error = Some(error);
            }
            session.apply_script_notice(app);
        }
        AppAction::ScriptRestartLater => {
            if let Err(error) = session.submit_with(
                app,
                OperatorCommand::DeferSettingsRestart,
                Targets::Application,
            ) {
                app.error = Some(error);
            }
        }
        AppAction::AckBackground => session.ack_background_bots(app),
        AppAction::SettingsDraftEdited(edit) => {
            match edit {
                crate::settings::SettingsEdit::RandomEvents(value) => {
                    *app.settings.random_events_mut() = value;
                }
                crate::settings::SettingsEdit::LampSkill(skill) => {
                    *app.settings.lamp_skill_mut() = skill.into();
                }
                crate::settings::SettingsEdit::LampAuto(value) => {
                    *app.settings.lamp_auto_mut() = value;
                }
                crate::settings::SettingsEdit::RandomEventPolicy { event, policy } => {
                    app.settings.set_random_event_policy(event, policy);
                }
                crate::settings::SettingsEdit::Lowmem(value) => {
                    *app.settings.lowmem_mut() = value;
                }
                crate::settings::SettingsEdit::CurePoison(value) => {
                    app.settings.survival_mut().set_cure_poison(value);
                }
            }
            if let Err(error) = session.submit_with(
                app,
                OperatorCommand::EditProfileDraft(
                    frontend_core::command::ProfileDraftField::Settings,
                ),
                Targets::Application,
            ) {
                app.error = Some(error);
            }
        }
        #[cfg(feature = "js")]
        AppAction::ProfileImport(action) => session.profile_import(app, action),
        AppAction::OpenNewProfile => session.open_new_profile(app),
        AppAction::NewProfileKey(key) => session.new_profile_key(app, key),
        AppAction::ParamsKey(key) => {
            let action = session.params_key(app, key);
            dispatch(session, app, action);
        }
        AppAction::LoadoutsKey(key) => {
            session.loadouts_key(app, key);
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
    frontend_core::launch::warn_legacy_passphrase_env("tui-play");
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
/// the shipped WalkTo map bundle (catalogue and terrain) for the pinned
/// client cache into `OUT/map/<revision>/` and print its descriptions
/// (JSON) on stdout.
/// Progress goes to stderr at most once a second.
fn map_bundle(profile: &frontend_core::launch::ProfileArgs, out: &Path) -> ExitCode {
    let revision = match profile.revision_number() {
        Ok(Some(revision)) => revision,
        Ok(None) => usage(),
        Err(error) => {
            eprintln!("tui-play: {error}");
            return ExitCode::from(2);
        }
    };
    let (Some(cache), Some(unpack)) = (profile.cache_dir(), profile.unpack_dir()) else {
        usage();
    };
    let mut reported: Option<(Instant, WalkMapStage)> = None;
    let result = frontend_core::map_bake::bake_shipped_map_bundle(
        revision,
        cache,
        unpack,
        out,
        |stage, message| {
            if reported.is_none_or(|(at, previous)| {
                previous != stage || at.elapsed() >= Duration::from_secs(1)
            }) {
                eprintln!("tui-play: map bundle: {message}");
                reported = Some((Instant::now(), stage));
            }
        },
    );
    match result {
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

#[cfg(test)]
#[path = "manual_click_live_tests.rs"]
pub(crate) mod manual_click_live_tests;

#[cfg(test)]
#[path = "bin_loop_tests.rs"]
mod loop_tests;
