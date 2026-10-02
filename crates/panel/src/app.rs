//! Panel shell: docking enabled, multi-viewport disabled, amber chrome,
//! running on the panel-owned window loop in `crate::window`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime};

use crate::chrome::{
    button_cells, button_cells_min, button_row_layout, equal_button_width, move_heading,
    multibox_tooltip, resolve_heading_order, BUTTON_GAP, CONFIG_HOST_ROW, CONFIG_MIN,
    CONFIG_SCRIPT_ROW, MIN_BUTTON, SCRIPT_ROW,
};
use crate::focus::{draw_for_slot, should_capture, should_draw};
use crate::game_view::GameView;
use crate::grid::grid_cells;
use crate::overlay::{draw_queue_card_for, PathOverlay};
use crate::paint::PaintOverlay;
use crate::picker;
use crate::rail::{
    draw_status_dot, light_rgb, next_os_window_size, os_window_size, rail_preview_open,
    rail_split_ratio, BASE_WINDOW_H, BASE_WINDOW_W, FOLD_GLYPH, RAIL_W, REMOVE_GLYPH, TILE_H,
    TILE_W, UNFOLD_GLYPH,
};
use crate::script_picker::{
    self, card_columns, card_desc_height, card_kind_source, card_rect_activated,
    card_transpile_label, card_width, centered_row_x, chip_frame_padding, chip_text_color,
    chip_wraps, dialog_date_color, display_category, format_mtime, move_category,
    overlay_first_pos, resolve_category_order, title_clip_width, DialogMode, BROWSE_WINDOW_TITLE,
    CARD_GAP, CARD_MIN_W, FILE_DIALOG_FIRST_H, FILE_DIALOG_FIRST_W, GLYPH_CHEVRON, GLYPH_FILE,
    GLYPH_FOLDER, SCRIPTS_FIRST_H, SCRIPTS_FIRST_W,
};
use crate::walk_map::WalkMapRenderer;
use crate::window::{self, Gpu, RedrawMode, ShotStatus, Theme};
use dear_imgui_rs::internal::RawWrapper;
use dear_imgui_rs::{
    ChildFlags, ColorDisplayMode, ComboBoxOptions, ComboBoxPreviewMode, Condition, DockBuilder,
    DockNodeFlags, DragDropTargetFlags, Id, Key, MouseButton, SplitDirection, StyleColor, StyleVar,
    TableColumnFlags, TableFlags, TreeNodeFlags, Ui, WindowClass, WindowFlags,
};
use host::debug_enabled;
use host_play::progress::{
    ProfileProgress, ProfileProgressObserver, ProfileProgressStage, ProfileProgressUnit,
};

use crate::input_capture::{capture_keys, discard_unconsumed_native_capture, stream_capture};
use crate::session::{
    debug_dest_cheats, debug_main_buttons_for, debug_maxme_cheats, script_active,
    script_pause_enabled, script_stop_enabled, ProfilePreparationCompletion, Session,
};
use crate::theme::{
    applet_offset, apply_amber, apply_amber_current, fit_applet, game_window_title, native_applet,
    panel_scale, panel_split_ratio, scale_px, scale_size, ui_scale, ACCENT, ACCENT_HOVER, BG,
    DOCKHOST_PADDING, ERROR, GREEN, PANEL_WIDTH, PANEL_WINDOW, RAIL_WINDOW, TEXT, TEXT_DIM,
};
use frontend_core::resources::{background_ack_text, format_background, format_bots};
use frontend_core::scripts::BrowseCard;
use frontend_core::views::run_state_label;
use frontend_core::{FleetRow, FormNotice, Phase, ResourceView};

#[path = "live_harness.rs"]
mod live_harness;
use live_harness::*;

/// Runner configuration: docking on, viewports off, amber CRT, 50 fps cap.
/// `auto_dockspace` is off so we own the split (game left, 330px panel right).
/// Default `RedrawMode::Poll` spins the UI thread and starves the
/// 20 ms slot; WaitUntil matches the client tick.
pub fn runner_config() -> window::PanelConfig {
    // Viewports stay off: the panel renders into the single main viewport only.
    window::PanelConfig {
        window_title: "bot".into(),
        window_size: (BASE_WINDOW_W as f64, BASE_WINDOW_H as f64),
        clear_color: BG,
        theme: Some(Theme::Dark),
        redraw: RedrawMode::WaitUntil { fps: 50.0 },
        ini_filename: None,
        restore_previous_geometry: false,
        docking: window::DockingConfig {
            enable: true,
            auto_dockspace: false,
            dockspace_flags: dock_flags(),
            ..Default::default()
        },
        io_config_flags: Some(dear_imgui_rs::ConfigFlags::DOCKING_ENABLE),
        ..Default::default()
    }
}

/// Apply the panel's single physical layout scale to ImGui's style and font.
/// Call on an unscaled theme style; monitor changes restore that baseline first.
pub fn apply_ui_scale(style: &mut dear_imgui_rs::Style, dpi: f32) {
    let scale = panel_scale(dpi);
    style.set_font_scale_dpi(scale);
    // SAFETY: `style` is the live ImGuiStyle owned by the context; ScaleAllSizes
    // mutates only that structure using the Dear ImGui API.
    unsafe {
        dear_imgui_rs::sys::ImGuiStyle_ScaleAllSizes(style.raw_mut(), scale);
    }
}

fn scaled_button_row_layout(ui: &Ui, avail: f32, count: usize) -> (f32, bool) {
    let scale = ui_scale(ui);
    let (width, stack) = button_row_layout(avail / scale, count);
    (width * scale, stack)
}

fn scaled_button_cells(ui: &Ui, avail: f32, count: usize) -> Vec<(f32, bool)> {
    let scale = ui_scale(ui);
    button_cells(avail / scale, count)
        .into_iter()
        .map(|(width, same_line)| (width * scale, same_line))
        .collect()
}

fn scaled_button_cells_min(ui: &Ui, avail: f32, count: usize, min_width: f32) -> Vec<(f32, bool)> {
    let scale = ui_scale(ui);
    button_cells_min(avail / scale, count, min_width)
        .into_iter()
        .map(|(width, same_line)| (width * scale, same_line))
        .collect()
}
/// Initialize amber chrome and base sizes before the monitor scale is applied.
fn amber_style(ctx: &mut dear_imgui_rs::Context) {
    let style = ctx.style_mut();
    style.set_scrollbar_size(crate::theme::THIN_SCROLLBAR_SIZE);
    style.set_scrollbar_rounding(crate::theme::THIN_SCROLLBAR_ROUNDING);
    apply_amber(style, &crate::theme::ChromeColors::default());
}

/// Dock layouts for [`dock_host`]: single-bot `[game | panel]` or the
/// MultiBox `[game | panel | rail]` strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DockLayout {
    Single,
    Rail,
}

/// Per-frame panel state: lazily-created game texture and the session (vault,
/// running slots, focus).
struct PanelState {
    #[cfg(feature = "memory-profile")]
    memory: Option<host_play::memory::Run>,
    game_view: Option<GameView>,
    session: Session,
    dock_inited: bool,
    /// Which dock layout the tree was last built with; `None` before the
    /// first init. A MultiBox toggle rebuilds the tree when this differs.
    dock_layout: Option<DockLayout>,
    game_dock_node: Option<Id>,
    panel_dock_node: Option<Id>,
    rail_dock_node: Option<Id>,
    docked_game_title: String,
    last_upload: Option<(String, u64)>,
    /// Focus to restore after sequenced pair terminal captures.
    pair_capture_restore: Option<String>,
    /// Cached queue-card overlay for the focused slot (see `overlay`).
    overlay: PathOverlay,
    /// Cached script-paint overlay over the Game chatbox (see `paint`).
    paint: PaintOverlay,
    /// One cached tile texture per wall member (blitted at TILE_W×TILE_H).
    views: HashMap<String, TileView>,
    /// The fleet rows the rail and the grid draw: a copy of the core's
    /// projection, refreshed only when its rows generation moves, with each
    /// row's cap strings built once per refresh (never per frame).
    fleet: Vec<FleetRow>,
    fleet_labels: Vec<CapLabels>,
    fleet_generation: Option<u64>,
    /// Last resource sample reported under `BOT_DEBUG`.
    debug_resources: u64,
    /// Headed `--live` watch (`null_raster`, `stress50`, `stress50_full`,
    /// `script_<name>`) or `--smoke`; `None` interactive.
    live: Option<LiveHarness>,
    /// Last dock-host viewport size; a width/height change rebuilds the
    /// split so the panel stays 330 and the rail 264.
    dock_size: Option<[f32; 2]>,
    /// winit window so MultiBox can grow and re-shrink the OS inner size
    /// when the rail covers or leaves the 765×503 blit.
    os_window: Option<Arc<dyn OsWindow>>,
    /// The rail need [`ensure_window_fits`] last fitted and the window has
    /// not met yet (a work-area clamp keeps it short). The fit runs again
    /// only when the need changes, so a clamped window is not re-fitted
    /// (and pinned) every frame.
    last_fit_need: Option<(f32, f32)>,
    /// Whole-window shot coordination: the scenario sink (slot thread)
    /// → the render readback (`window::ShotState`) → the shot files.
    shot_state: Arc<Mutex<crate::window::ShotState>>,
    /// Per-run shot dir (`~/.274bot/smoke/<runId>`), created when a
    /// `--live script_*` run starts or lazily on the first write (the
    /// interactive F12 capture has no run start to hook).
    shot_dir: Option<PathBuf>,
    /// One application-owned WalkTo map renderer (not per bot).
    walk_map: WalkMapRenderer,
}

/// Deferred boot work for [`run_panel`]. The unlock / live-harness flows
/// spawn slot threads, and a slot renderer is built lazily at its first
/// paint — running these before GPU init would let a slot construct its
/// own wgpu device ahead of `on_gpu_init`'s `inject_device` (the
/// shared-device seam's ordering invariant). The first `on_frame` is
/// guaranteed to run after GPU init, so the boot runs at the top of the
/// first frame.
#[derive(Debug)]
enum Boot {
    #[cfg(feature = "memory-profile")]
    Memory(host_play::memory::Config),
    /// The vault passphrase from the in-panel prompt or `--vault-pass-stdin`.
    /// Failure is non-fatal: the in-panel prompt covers typing. `Debug` never
    /// prints it.
    Unlock { pass: vault::Secret },
    /// Live harness spawns. Failure is fatal (`FAIL:` + exit).
    Live(LiveBoot),
}

struct ProfilePrepareJob {
    generation: u64,
    receiver: Receiver<Result<Arc<host_play::SharedClientTemplate>, String>>,
    progress: LatestProgress,
}

struct ProfileValidateJob {
    generation: u64,
    boot: Boot,
    receiver: Receiver<Result<host_play::ValidatedTemplate, String>>,
    progress: LatestProgress,
}

type LatestProgress = Arc<Mutex<Option<ProfileProgress>>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProgressPhase {
    Preparing,
    FinalChecks,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StartupProgressView {
    phase: ProgressPhase,
    progress: ProfileProgress,
}

fn progress_channel(initial: ProfileProgress) -> (ProfileProgressObserver, LatestProgress) {
    let latest = Arc::new(Mutex::new(Some(initial)));
    let worker_latest = Arc::clone(&latest);
    let observer = ProfileProgressObserver::new(move |progress| {
        if let Ok(mut latest) = worker_latest.try_lock() {
            *latest = Some(progress);
        }
    });
    (observer, latest)
}

struct StartupPreparation {
    prepare: Option<ProfilePrepareJob>,
    validate: Option<ProfileValidateJob>,
    pending_boot: Option<Boot>,
    failed_generation: Option<u64>,
}

impl StartupPreparation {
    fn new(pending_boot: Option<Boot>) -> Self {
        Self {
            prepare: None,
            validate: None,
            pending_boot,
            failed_generation: None,
        }
    }

    fn in_flight(&self) -> bool {
        self.prepare.is_some() || self.validate.is_some()
    }
}

fn startup_progress(startup: &StartupPreparation, generation: u64) -> Option<StartupProgressView> {
    let (phase, latest) = if let Some(job) = startup
        .validate
        .as_ref()
        .filter(|job| job.generation == generation)
    {
        (ProgressPhase::FinalChecks, &job.progress)
    } else {
        let job = startup
            .prepare
            .as_ref()
            .filter(|job| job.generation == generation)?;
        (ProgressPhase::Preparing, &job.progress)
    };
    let progress = latest.lock().ok()?.as_ref().copied()?;
    Some(StartupProgressView { phase, progress })
}

/// The deferred boot for a [`run_panel`] call, derived from the run mode and
/// the passphrase read from `--vault-pass-stdin` — pure, so the mapping is
/// testable (the boot itself runs after GPU init; nothing here spawns or
/// unlocks).
fn boot_for(mode: &RunMode, vault_pass: Option<vault::Secret>) -> Option<Boot> {
    if mode.is_smoke() {
        return Some(Boot::Live(LiveBoot::Smoke));
    }
    match mode.live_name() {
        Some("null_raster") => Some(Boot::Live(LiveBoot::NullRaster)),
        Some("stress50") => Some(Boot::Live(LiveBoot::Stress50)),
        Some("stress50_full") => Some(Boot::Live(LiveBoot::Stress50Full)),
        Some("nav_full") => Some(Boot::Live(LiveBoot::Script {
            name: "nav_full".to_string(),
        })),
        Some(name) if name.starts_with("script_") => Some(Boot::Live(LiveBoot::Script {
            name: name.to_string(),
        })),
        _ => vault_pass.map(|pass| Boot::Unlock { pass }),
    }
}

/// Execute a deferred [`Boot`] (slot spawns) after GPU init. Returns the
/// fatal `FAIL:` message for live-harness failures; the vault unlock
/// failure is non-fatal (the in-panel prompt covers typing).
fn boot_execute(state: &mut PanelState, boot: Boot) -> Result<(), String> {
    state.session.ensure_profile_bound()?;
    state.session.require_bot_operation()?;
    match boot {
        #[cfg(feature = "memory-profile")]
        Boot::Memory(config) => {
            let run = host_play::memory::Run::prepare_unseeded(config, "panel")?;
            state.session.prepare_memory(&run)?;
            state.memory = Some(run);
            Ok(())
        }
        Boot::Unlock { pass } => {
            if !state.session.unlock(&pass) {
                eprintln!(
                    "panel: vault: {}",
                    state.session.error.clone().unwrap_or_default()
                );
            }
            Ok(())
        }
        Boot::Live(live) => {
            state.live = Some(live.start(
                &mut state.session,
                Arc::clone(&state.shot_state),
                &mut state.shot_dir,
            )?);
            Ok(())
        }
    }
}

fn boot_failure_is_fatal(boot: &Boot) -> bool {
    !matches!(boot, Boot::Unlock { .. })
}

fn fail_startup(session: &mut Session, fatal: bool, error: String) {
    session.set_profile_preparing(false);
    session.error = Some(error.clone());
    if fatal {
        eprintln!("FAIL: {error}");
        std::process::exit(1);
    }
}

/// Poll the two sequential startup workers. Both resource-heavy phases run
/// away from the event loop; profile/template installation and all vault,
/// Play, GPU, JS-store and slot ownership remain on this UI thread.
fn drive_startup(state: &mut PanelState, startup: &mut StartupPreparation) {
    if let Some(pass) = state.session.take_requested_unlock() {
        if let Some(job) = startup
            .validate
            .as_mut()
            .filter(|job| matches!(&job.boot, Boot::Unlock { .. }))
        {
            job.boot = Boot::Unlock { pass };
        } else if startup
            .pending_boot
            .as_ref()
            .is_none_or(|boot| matches!(boot, Boot::Unlock { .. }))
        {
            startup.pending_boot = Some(Boot::Unlock { pass });
            startup.failed_generation = None;
        }
    }

    let generation = state.session.profile_generation();
    if startup
        .prepare
        .as_ref()
        .is_some_and(|job| job.generation != generation)
    {
        startup.prepare = None;
        startup.failed_generation = None;
        state.session.set_profile_preparing(false);
    }

    let prepared = startup
        .prepare
        .as_ref()
        .and_then(|job| match job.receiver.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                Some(Err("profile preparation worker stopped".into()))
            }
        });
    if let Some(result) = prepared {
        let job = startup.prepare.take().expect("polled preparation job");
        match state
            .session
            .finish_profile_preparation(job.generation, result)
        {
            ProfilePreparationCompletion::Installed | ProfilePreparationCompletion::Stale => {
                startup.failed_generation = None;
            }
            ProfilePreparationCompletion::Failed => {
                startup.failed_generation = Some(job.generation);
                let fatal = startup
                    .pending_boot
                    .as_ref()
                    .is_some_and(boot_failure_is_fatal);
                let error = state.session.error.clone().unwrap_or_default();
                fail_startup(&mut state.session, fatal, error);
                if !fatal {
                    startup.pending_boot = None;
                }
            }
        }
    }

    let validated = startup
        .validate
        .as_ref()
        .and_then(|job| match job.receiver.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                Some(Err("profile validation worker stopped".into()))
            }
        });
    if let Some(result) = validated {
        let job = startup.validate.take().expect("polled validation job");
        state.session.set_profile_preparing(false);
        if job.generation != state.session.profile_generation() {
            return;
        }
        let fatal = boot_failure_is_fatal(&job.boot);
        match result.and_then(|ticket| state.session.install_validated_template(ticket)) {
            Ok(()) => {
                if let Err(error) = boot_execute(state, job.boot) {
                    fail_startup(&mut state.session, fatal, error);
                }
            }
            Err(error) => fail_startup(&mut state.session, fatal, error),
        }
    }

    if !state.session.profile_bound()
        && startup.prepare.is_none()
        && startup.failed_generation != Some(generation)
    {
        match state.session.profile_preparation() {
            Ok((generation, preparation)) => {
                let (sender, receiver) = mpsc::sync_channel(1);
                let (observer, progress) = progress_channel(ProfileProgress::steps(
                    ProfileProgressStage::SelectingServerProfile,
                    0,
                    1,
                ));
                state.session.error = None;
                state.session.set_profile_preparing(true);
                std::thread::spawn(move || {
                    let _ = sender.send(preparation.run_with_progress(&observer));
                });
                startup.prepare = Some(ProfilePrepareJob {
                    generation,
                    receiver,
                    progress,
                });
            }
            Err(error) => {
                startup.failed_generation = Some(generation);
                let fatal = startup
                    .pending_boot
                    .as_ref()
                    .is_some_and(boot_failure_is_fatal);
                fail_startup(&mut state.session, fatal, error);
            }
        }
    }

    if state.session.profile_bound() && startup.validate.is_none() && startup.pending_boot.is_some()
    {
        let boot = startup.pending_boot.take().expect("pending boot");
        let fatal = boot_failure_is_fatal(&boot);
        match state.session.template_for_validation() {
            Ok(template) => {
                let (sender, receiver) = mpsc::sync_channel(1);
                let (observer, progress) = progress_channel(ProfileProgress::steps(
                    ProfileProgressStage::FinalChecks,
                    0,
                    1,
                ));
                state.session.error = None;
                state.session.set_profile_preparing(true);
                std::thread::spawn(move || {
                    let _ = sender.send(template.validate_for_play_with_progress(&observer));
                });
                startup.validate = Some(ProfileValidateJob {
                    generation,
                    boot,
                    receiver,
                    progress,
                });
            }
            Err(error) => fail_startup(&mut state.session, fatal, error),
        }
    }
}

/// One rail/grid tile's GPU texture (uploaded when the slot's `FrameBuf`
/// hands a new frame to `take`).
struct TileView {
    view: GameView,
}

/// One fleet row's rail/grid cap strings, rebuilt with the rows.
#[derive(Default)]
struct CapLabels {
    /// `name: brief###cap`, the select label (stable ID).
    title: String,
    /// The active public world number, empty for a local profile.
    world: String,
    /// `wN · brief`, the world marker's tooltip.
    world_tip: String,
}

/// Drop rail/grid `GameView`s that are not painting (left the wall, or
/// only-render-selected hid the slot). Unbind-to-owned keeps the panel's
/// 765×503 texture, which Metal pads to 8 MB — dispose frees it.
fn dispose_idle_views(
    views: &mut HashMap<String, TileView>,
    gpu: &mut Gpu,
    keep: impl Fn(&str) -> bool,
) {
    let drop_names: Vec<String> = views
        .keys()
        .filter(|name| !keep(name.as_str()))
        .cloned()
        .collect();
    for name in drop_names {
        if let Some(tv) = views.remove(&name) {
            tv.view.dispose(gpu);
        }
    }
}

impl PanelState {
    fn with_session(session: Session, shot_state: Arc<Mutex<crate::window::ShotState>>) -> Self {
        Self {
            #[cfg(feature = "memory-profile")]
            memory: None,
            game_view: None,
            session,
            dock_inited: false,
            dock_layout: None,
            game_dock_node: None,
            panel_dock_node: None,
            rail_dock_node: None,
            docked_game_title: String::new(),
            last_upload: None,
            pair_capture_restore: None,
            overlay: PathOverlay::new(),
            paint: PaintOverlay::new(),
            views: HashMap::new(),
            fleet: Vec::new(),
            fleet_labels: Vec::new(),
            fleet_generation: None,
            debug_resources: 0,
            live: None,
            dock_size: None,
            os_window: None,
            last_fit_need: None,
            shot_state,
            shot_dir: None,
            walk_map: WalkMapRenderer::new(),
        }
    }

    /// Copy the core's fleet rows for the rail and the grid when (and only
    /// when) they changed, and rebuild their cap strings; both reuse their
    /// buffers.
    fn sync_fleet(&mut self) {
        use std::fmt::Write as _;
        let view = self.session.core.fleet_view();
        if self.fleet_generation == Some(view.rows_generation()) {
            return;
        }
        self.fleet_generation = Some(view.rows_generation());
        view.copy_rows_into(&mut self.fleet);
        self.fleet_labels
            .resize_with(self.fleet.len(), CapLabels::default);
        for (row, labels) in self.fleet.iter().zip(&mut self.fleet_labels) {
            labels.title.clear();
            // `###cap`: the select keeps its ID while the brief changes.
            let _ = write!(labels.title, "{}: {}###cap", row.name, row.brief);
            labels.world.clear();
            labels.world_tip.clear();
            if let Some(number) = row.world {
                let _ = write!(labels.world, "{number}");
                let _ = write!(labels.world_tip, "w{number} · {}", row.brief);
            }
        }
    }

    /// Under `BOT_DEBUG`, report each new resource sample (1 Hz) with the
    /// game image's present counters.
    fn debug_resources(&mut self) {
        let generation = self.session.core.resource_generation();
        if !debug_enabled() || generation == self.debug_resources {
            return;
        }
        self.debug_resources = generation;
        let view = self.session.core.resources();
        eprintln!("[panel] ram={} bots={}", view.ram.text(), view.bots);
        if let Some(view) = self.game_view.as_ref() {
            let s = view.present_stats;
            eprintln!(
                "[panel] present pixmap={} tex={} bind_noop={} bind_rereg={}",
                s.pixmap, s.tex, s.bind_noop, s.bind_rereg
            );
        }
    }
}

#[cfg(test)]
impl Default for PanelState {
    fn default() -> Self {
        Self::with_session(
            Session::new(),
            Arc::new(Mutex::new(crate::window::ShotState::default())),
        )
    }
}

const LIVE_USAGE: &str =
    "usage: panel-play [--profile NAME|--rs2b2t] [--smoke] [--lowmem|--highmem] [--vault-pass-stdin] [--nav-paints on|off] [--live null_raster|stress50|stress50_full|nav_full|script_<name>] [--prepare-fixture <scenario>] [--run-prepared] [--fixture-path PATH] [--server-root PATH]\n       BUDGET_S=<seconds>  override scenario deadline (rs2b0t); PASS keeps the window until the budget ends\n       --prepare-fixture   offline server-native .sav write (no live boot); --run-prepared reuses identity with zero setup cheats\n       --vault-pass-stdin  read the vault passphrase from a piped stdin (or ask on the terminal) and unlock before the window opens; without it use the in-window prompt";

/// What `panel-play` should do this run: the normal interactive panel, a
/// `--live NAME` harness, or `--smoke` (one whole-window shot at scene 2,
/// then exit 0).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunMode {
    Interactive,
    Live(String),
    Smoke,
    /// Offline prepare only — no GPU/window; writes `.sav` + identity receipt.
    PrepareFixture(String),
}

#[derive(Debug, Clone)]
pub struct PanelArgs {
    pub mode: RunMode,
    pub profile: host_play::ProfileOptions,
    /// Dedicated catalog_watch proof bridge; ordinary panel-play stays false.
    pub catalog_core: bool,
    /// Dedicated pair_watch proof bridge; ordinary panel-play stays false.
    pub pair_core: bool,
    /// Dedicated external_watch proof bridge; ordinary panel-play stays false.
    pub external_core: bool,
    /// Absolute raw `.ts` override; absent uses the tracked host-play fixture.
    pub external_ts: Option<std::path::PathBuf>,
    /// Session-only headed live paint choice; absent preserves panel behavior.
    pub nav_paints: Option<bool>,
    /// Session-only memory choice; absent preserves the vault profile and UI gate.
    pub memory_override: Option<bool>,
    /// When set with `--live script_*`, apply run-prepared fixture mode.
    pub run_prepared: bool,
    /// Identity receipt path override (`~/.274bot/fixtures/<scenario>.json`).
    pub fixture_path: Option<std::path::PathBuf>,
    /// Server engine root for offline prepare (or `BOT_SERVER_ROOT`).
    pub server_root: Option<std::path::PathBuf>,
    /// `--vault-pass-stdin`: read the vault passphrase from a piped stdin (or
    /// a hidden terminal prompt) and unlock before the window opens. The
    /// passphrase is never an argument or an environment variable.
    pub vault_pass_stdin: bool,
}

pub fn parse_args(
    args: impl IntoIterator<Item = impl AsRef<str>>,
    env_live: Option<&str>,
) -> Result<PanelArgs, (i32, String)> {
    let (memory_override, args) = parse_memory_override(args)?;
    let (vault_pass_stdin, args) = parse_vault_pass_flags(args)?;
    let (profile, rest) =
        host_play::parse_profile_args(args).map_err(|msg| (2, format!("panel-play: {msg}")))?;
    let (nav_paints, rest) = parse_nav_paints(rest)?;
    let (external_ts, rest) = parse_external_ts(rest)?;
    let (fixture_flags, live_args) = parse_fixture_flags(rest)?;
    let mode = parse_live_args(live_args, env_live)?;
    let mode = match (mode, fixture_flags.prepare_fixture) {
        (RunMode::Interactive, Some(name)) => RunMode::PrepareFixture(name),
        (_, Some(_)) => {
            return Err((
                2,
                "panel-play: --prepare-fixture cannot combine with --live/--smoke".into(),
            ));
        }
        (other, None) => other,
    };
    if fixture_flags.run_prepared {
        match &mode {
            RunMode::Live(name) if name.starts_with("script_") => {}
            _ => {
                return Err((
                    2,
                    "panel-play: --run-prepared requires --live script_<scenario>".into(),
                ));
            }
        }
    }
    Ok(PanelArgs {
        mode,
        profile,
        catalog_core: false,
        pair_core: false,
        external_core: false,
        external_ts,
        nav_paints,
        memory_override,
        run_prepared: fixture_flags.run_prepared,
        fixture_path: fixture_flags.fixture_path,
        server_root: fixture_flags.server_root,
        vault_pass_stdin,
    })
}

/// Consumes `--vault-pass-stdin` and rejects the removed `--vault-pass`
/// (either spelling) without echoing its value.
fn parse_vault_pass_flags(args: Vec<String>) -> Result<(bool, Vec<String>), (i32, String)> {
    let mut from_stdin = false;
    let mut rest = Vec::with_capacity(args.len());
    for arg in args {
        if host_play::passphrase::is_removed_flag(&arg) {
            return Err((2, host_play::passphrase::removed_flag_error("panel-play")));
        }
        if arg == "--vault-pass-stdin" {
            from_stdin = true;
        } else {
            rest.push(arg);
        }
    }
    Ok((from_stdin, rest))
}

#[derive(Debug, Default)]
struct FixtureFlags {
    prepare_fixture: Option<String>,
    run_prepared: bool,
    fixture_path: Option<PathBuf>,
    server_root: Option<PathBuf>,
}

fn parse_fixture_flags(args: Vec<String>) -> Result<(FixtureFlags, Vec<String>), (i32, String)> {
    let mut flags = FixtureFlags::default();
    let mut rest = Vec::with_capacity(args.len());
    let mut it = args.into_iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--prepare-fixture" => {
                let Some(name) = it.next() else {
                    return Err((
                        2,
                        "panel-play: --prepare-fixture needs a scenario name".into(),
                    ));
                };
                if scenario::get(&name).is_none() {
                    return Err((
                        2,
                        format!("panel-play: unknown prepare-fixture scenario {name}"),
                    ));
                }
                flags.prepare_fixture = Some(name);
            }
            "--run-prepared" => flags.run_prepared = true,
            "--fixture-path" => {
                let Some(raw) = it.next() else {
                    return Err((2, "panel-play: --fixture-path needs a path".into()));
                };
                flags.fixture_path = Some(PathBuf::from(raw));
            }
            "--server-root" => {
                let Some(raw) = it.next() else {
                    return Err((2, "panel-play: --server-root needs a path".into()));
                };
                flags.server_root = Some(PathBuf::from(raw));
            }
            _ => rest.push(arg),
        }
    }
    if flags.prepare_fixture.is_some() && flags.run_prepared {
        return Err((
            2,
            "panel-play: --prepare-fixture and --run-prepared are mutually exclusive".into(),
        ));
    }
    Ok((flags, rest))
}

fn parse_memory_override(
    args: impl IntoIterator<Item = impl AsRef<str>>,
) -> Result<(Option<bool>, Vec<String>), (i32, String)> {
    let mut value = None;
    let mut rest = Vec::new();
    for arg in args {
        let arg = arg.as_ref();
        let next = match arg {
            "--lowmem" => Some(true),
            "--highmem" => Some(false),
            _ => None,
        };
        if let Some(next) = next {
            if value.is_some_and(|current| current != next) {
                return Err((2, "panel-play: --lowmem and --highmem conflict".into()));
            }
            value = Some(next);
        } else {
            rest.push(arg.to_string());
        }
    }
    Ok((value, rest))
}

fn parse_nav_paints(args: Vec<String>) -> Result<(Option<bool>, Vec<String>), (i32, String)> {
    let mut value = None;
    let mut rest = Vec::with_capacity(args.len());
    let mut it = args.into_iter();
    while let Some(arg) = it.next() {
        if arg != "--nav-paints" {
            rest.push(arg);
            continue;
        }
        let Some(raw) = it.next() else {
            return Err((2, "panel-play: --nav-paints needs on or off".into()));
        };
        value = Some(match raw.as_str() {
            "on" => true,
            "off" => false,
            _ => {
                return Err((
                    2,
                    format!("panel-play: --nav-paints expects on or off, got {raw}"),
                ))
            }
        });
    }
    Ok((value, rest))
}

fn parse_external_ts(args: Vec<String>) -> Result<(Option<PathBuf>, Vec<String>), (i32, String)> {
    let mut value = None;
    let mut rest = Vec::with_capacity(args.len());
    let mut it = args.into_iter();
    while let Some(arg) = it.next() {
        if arg != "--external-ts" {
            rest.push(arg);
            continue;
        }
        let Some(raw) = it.next() else {
            return Err((2, "panel-play: --external-ts needs an absolute path".into()));
        };
        let path = PathBuf::from(&raw);
        if !path.is_absolute() {
            return Err((
                2,
                format!("panel-play: --external-ts {raw} is relative; pass an absolute path"),
            ));
        }
        value = Some(path);
    }
    Ok((value, rest))
}

impl RunMode {
    /// The `--live` harness name, `None` for interactive and `--smoke`.
    fn live_name(&self) -> Option<&str> {
        match self {
            RunMode::Live(name) => Some(name),
            _ => None,
        }
    }

    fn is_smoke(&self) -> bool {
        matches!(self, RunMode::Smoke)
    }

    fn prepare_fixture_name(&self) -> Option<&str> {
        match self {
            RunMode::PrepareFixture(name) => Some(name),
            _ => None,
        }
    }
}

/// The interactive F12 capture label: `manual-<stamp>` (the 377 stamp so
/// shot files sort chronologically). Already in the 377 safe alphabet, so
/// `safe_label` is a no-op on it.
fn manual_shot_label(now: SystemTime) -> String {
    format!("manual-{}", scenario::shot::stamp_utc(now))
}

/// F12 whole-window shot: pair the pixels with the focused slot's last
/// published live snapshot, then use the same request/readback/write path
/// as scenario captures. Without a focused published snapshot, fail closed
/// instead of writing an invented empty sidecar.
fn enqueue_manual_shot(state: &mut PanelState) {
    let label = manual_shot_label(SystemTime::now());
    let focused = state.session.focus.lock().unwrap().focused.clone();
    let Some(focused) = focused else {
        eprintln!("[panel] F12: no focused slot has a live snapshot");
        return;
    };
    let json = {
        let states = state.session.nav_states.lock().unwrap();
        let Some((snapshot, _)) = states.get(&focused) else {
            eprintln!("[panel] F12: focused slot {focused} has no live snapshot");
            return;
        };
        if !snapshot.ingame() || snapshot.tile().is_none() {
            eprintln!("[panel] F12: focused slot {focused} has no live player snapshot");
            return;
        }
        match serde_json::to_string_pretty(snapshot) {
            Ok(json) => json,
            Err(error) => {
                eprintln!("[panel] F12: snapshot serialization failed: {error}");
                return;
            }
        }
    };
    state
        .shot_state
        .lock()
        .unwrap()
        .enqueue(label.clone(), json);
    println!("[panel] F12: shot {label} queued");
}

/// `--live NAME` wins over `BOT_LIVE`. Empty env is ignored. `--smoke`
/// wins over both. Unknown flags/names → `Err((2, msg))`;
/// `--help`/`-h` → `Err((0, usage))`.
pub fn parse_live_args(
    args: impl IntoIterator<Item = impl AsRef<str>>,
    env_live: Option<&str>,
) -> Result<RunMode, (i32, String)> {
    let mut live = env_live.filter(|s| !s.is_empty()).map(str::to_string);
    let mut smoke = false;
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_ref() {
            "--live" => {
                let Some(name) = it.next() else {
                    return Err((2, "panel-play: --live needs a name".into()));
                };
                live = Some(name.as_ref().to_string());
            }
            "--smoke" => smoke = true,
            "--help" | "-h" => return Err((0, LIVE_USAGE.into())),
            other => return Err((2, format!("panel-play: unknown {other}"))),
        }
    }
    if smoke {
        return Ok(RunMode::Smoke);
    }
    if let Some(name) = live.as_deref() {
        let script_ok = name.strip_prefix("script_").is_some_and(|n| {
            n == host_play::external_loader::LIVE_SCENARIO || scenario::get(n).is_some()
        });
        if name != "null_raster"
            && name != "stress50"
            && name != "stress50_full"
            && name != "nav_full"
            && !script_ok
        {
            return Err((2, LIVE_USAGE.into()));
        }
    }
    Ok(live.map(RunMode::Live).unwrap_or(RunMode::Interactive))
}

/// The selected slot's script paint, as the shared `Arc` (no copy):
/// `None` when the slot has no status row, `Some(None)` when it paints
/// nothing.
fn focused_paint(session: &Session) -> Option<Option<Arc<script::shim::ScriptPaint>>> {
    let name = session.core.selected()?;
    session
        .statuses()
        .iter()
        .find(|s| s.username == name)
        .map(|s| s.script_paint.clone())
}

fn overlay_script_paint(
    ui: &Ui,
    gpu: &mut Gpu,
    state: &mut PanelState,
    paint: Option<Option<Arc<script::shim::ScriptPaint>>>,
    min: [f32; 2],
    size: [f32; 2],
) {
    match paint {
        Some(paint) => {
            if let Some((hit, generation)) =
                state
                    .paint
                    .frame(ui, Some(gpu), paint.as_deref(), min, size)
            {
                match hit {
                    crate::paint::PaintFrameHit::Button(id) => {
                        state.session.script_paint_click(&id, generation);
                    }
                    crate::paint::PaintFrameHit::Select { key, name } => {
                        state.session.script_paint_select(&key, &name, generation);
                    }
                }
            }
        }
        None => state.paint.release_canvas(gpu),
    }
}

/// Dockspace: no splitter, no extra splits. Tab bar hides on a
/// single-window leaf. `NO_UNDOCKING` stays off here — it would lock the
/// panel against config tabs. Game/rail apply it via their window class.
fn dock_flags() -> DockNodeFlags {
    DockNodeFlags::AUTO_HIDE_TAB_BAR | DockNodeFlags::NO_RESIZE | DockNodeFlags::NO_DOCKING_SPLIT
}

/// Game leaves: locked in the split, tab bar hidden while they host a
/// single window. Scripts may still tab onto Game if the operator docks
/// them; they do not spawn there. The rail has its own class — it needs
/// a visible tab so the close X can exist.
fn game_window_class() -> WindowClass {
    WindowClass::new(Id::from(1u32))
        .dock_node_flags_override_set(dock_flags() | DockNodeFlags::NO_UNDOCKING)
        .docking_always_tab_bar(false)
}

/// MultiBox sidecar: same lock-in as Game, but the tab bar stays up so
/// the window `.opened` X is visible. Closing it turns MultiBox off;
/// [`ensure_window_fits`] then shrinks the OS window.
fn rail_window_class() -> WindowClass {
    WindowClass::new(Id::from(3u32))
        .dock_node_flags_override_set(
            DockNodeFlags::NO_RESIZE
                | DockNodeFlags::NO_DOCKING_SPLIT
                | DockNodeFlags::NO_UNDOCKING,
        )
        .docking_always_tab_bar(true)
}

/// 274bot panel: same fixed width / no splits, but a visible tab bar so
/// General/Nav config can spawn as sibling tabs.
fn panel_window_class() -> WindowClass {
    WindowClass::new(Id::from(2u32))
        .dock_node_flags_override_set(DockNodeFlags::NO_RESIZE | DockNodeFlags::NO_DOCKING_SPLIT)
        .docking_always_tab_bar(true)
        .docking_allow_unclassed(true)
}

fn size_changed(prev: Option<[f32; 2]>, size: [f32; 2]) -> bool {
    match prev {
        None => true,
        Some(p) => (p[0] - size[0]).abs() > 1.0 || (p[1] - size[1]).abs() > 1.0,
    }
}

/// 274bot panel sibling tabs. `dock_host` rebuilds the tree when the
/// MultiBox rail toggles or the OS window size changes; SetNextWindowDockID
/// with Appearing/FirstUseEver is ignored while those windows are already
/// visible, so they must be `DockBuilder::dock_window`'d onto the new leaf
/// or they float over the panel with a leftover tab bar.
fn dock_panel_tabs(ui: &Ui, panel: Id) {
    DockBuilder::dock_window(ui, PANEL_WINDOW, panel);
    for title in [
        "Profiles",
        "General config",
        "Nav config",
        "Script prefs",
        "Debug",
    ] {
        DockBuilder::dock_window(ui, title, panel);
    }
}

/// The OS window as the panel frame drives it. `winit::Window` in
/// production; tests stand in a recording frame for the rail fit.
trait OsWindow {
    fn set_title(&self, title: &str);
    fn request_redraw(&self);
    /// Fit the OS frame to a logical inner-size need, clamped into the
    /// monitor work area (M-003).
    fn fit_to_work_area(&self, need_logical: (f64, f64));
}

impl OsWindow for winit::window::Window {
    fn set_title(&self, title: &str) {
        winit::window::Window::set_title(self, title);
    }
    fn request_redraw(&self) {
        winit::window::Window::request_redraw(self);
    }
    fn fit_to_work_area(&self, need_logical: (f64, f64)) {
        window::fit_window_to_work_area(self, need_logical);
    }
}

/// Grow the OS window when the rail would cover the native blit. Shrink
/// it back when the strip leaves — MultiBox off, Grid, or the rail tab
/// X (`set_multibox(false)`). Falling edge of [`DockLayout::Rail`].
///
/// `current_physical` is ImGui `display_size` in framebuffer pixels: the
/// platform is locked to a 1× framebuffer and DPI is applied to layout.
/// Do not call `Window::inner_size()` here: that is a per-frame X11
/// `GetGeometry` round trip and panics with BadDrawable if the drawable
/// is already gone.
/// The need is clamped into the current monitor work area (M-003), so
/// opening the rail on a small screen shrinks and repositions instead of
/// pushing the frame off the right edge. The fit (and its OS queries)
/// runs once per change of the need; see [`take_fit_need`].
fn ensure_window_fits(
    state: &mut PanelState,
    rail_open: bool,
    current_physical: [f32; 2],
    scale: f32,
) {
    let Some(window) = state.os_window.as_ref() else {
        return;
    };
    let current = (current_physical[0] / scale, current_physical[1] / scale);
    let rail_was_open = state.dock_layout == Some(DockLayout::Rail);
    let need = next_os_window_size(current, rail_was_open, rail_open);
    if take_fit_need(&mut state.last_fit_need, need, current) {
        window.fit_to_work_area((f64::from(need.0), f64::from(need.1)));
    }
}

/// Whether [`ensure_window_fits`] fits for `need` this frame. A window
/// that already meets the need clears `last`. Otherwise the fit runs only
/// when the need differs from the one last fitted: a work-area clamp (or a
/// platform that applies the size without a `Resized`, as Wayland does)
/// leaves the window short of the need for good, and fitting again every
/// frame would snap back a window the operator drags.
fn take_fit_need(last: &mut Option<(f32, f32)>, need: (f32, f32), current: (f32, f32)) -> bool {
    let close = |a: (f32, f32), b: (f32, f32)| (a.0 - b.0).abs() <= 1.0 && (a.1 - b.1).abs() <= 1.0;
    if close(need, current) {
        *last = None;
        return false;
    }
    if last.is_some_and(|fitted| close(fitted, need)) {
        return false;
    }
    *last = Some(need);
    true
}

/// Fullscreen dock host: game left, 330 logical px panel at 100% scale,
/// optional 264 logical px rail. Both widths scale with monitor DPI and stay
/// fixed as the OS window grows.
fn dock_host(ui: &Ui, state: &mut PanelState, game_title: &str) {
    let rail_open = state.session.multibox && !state.session.wall.grid;
    let scale = ui_scale(ui);
    ensure_window_fits(state, rail_open, ui.io().display_size(), scale);
    let viewport = ui.main_viewport();
    let pos = viewport.pos();
    let vs = viewport.size();
    let (need_w, _) = os_window_size(rail_open);
    // Until winit applies a grow, lay out against the need so the blit
    // is not parked under the panel/rail for a frame.
    let size = [vs[0].max(need_w * scale), vs[1].max(1.0)];
    // Zero host padding: the dockspace must span the viewport, because the
    // split ratio below divides the viewport width. Default 8px padding
    // narrowed the dockspace by 16px, starving the game leaf below the
    // 765px blit (778.7px leaf, 762.7 content) so the blit clamped
    // flush-left: 16px gray left, ~6 right. Docked leaves keep their own
    // padding; only the host frame goes edge to edge.
    let _host_pad = ui.push_style_var(StyleVar::WindowPadding(DOCKHOST_PADDING));
    ui.window("##274bot-dockhost")
        .flags(
            WindowFlags::NO_TITLE_BAR
                | WindowFlags::NO_RESIZE
                | WindowFlags::NO_MOVE
                | WindowFlags::NO_COLLAPSE
                | WindowFlags::NO_BRING_TO_FRONT_ON_FOCUS
                | WindowFlags::NO_NAV_FOCUS
                | WindowFlags::NO_DOCKING,
        )
        .position([pos[0], pos[1]], Condition::Always)
        .size([size[0], size[1]], Condition::Always)
        .build(|| {
            let dock_id = ui.get_id("274bot-dockspace");
            let _ = ui.dock_space_with_class(dock_id, [0.0, 0.0], dock_flags(), None);
            let want = if state.session.multibox && !state.session.wall.grid {
                DockLayout::Rail
            } else {
                DockLayout::Single
            };
            if state.dock_layout != Some(want) || size_changed(state.dock_size, size) {
                DockBuilder::remove_node(ui, dock_id);
                state.dock_layout = Some(want);
                state.dock_size = Some(size);
                state.dock_inited = false;
            }
            if !state.dock_inited && DockBuilder::node_exists(ui, dock_id) {
                DockBuilder::set_node_size(ui, dock_id, [size[0], size[1]]);
                match want {
                    DockLayout::Single => {
                        let ratio = panel_split_ratio(size[0], scale);
                        let (right, left) =
                            DockBuilder::split_node(ui, dock_id, SplitDirection::Right, ratio);
                        dock_panel_tabs(ui, right);
                        DockBuilder::dock_window(ui, game_title, left);
                        state.game_dock_node = Some(left);
                        state.panel_dock_node = Some(right);
                        state.rail_dock_node = None;
                    }
                    DockLayout::Rail => {
                        let rail_ratio = rail_split_ratio(size[0], scale);
                        let (rail, main) =
                            DockBuilder::split_node(ui, dock_id, SplitDirection::Right, rail_ratio);
                        let panel_ratio =
                            panel_split_ratio((size[0] - RAIL_W * scale).max(1.0), scale);
                        let (panel, game) =
                            DockBuilder::split_node(ui, main, SplitDirection::Right, panel_ratio);
                        DockBuilder::dock_window(ui, RAIL_WINDOW, rail);
                        dock_panel_tabs(ui, panel);
                        DockBuilder::dock_window(ui, game_title, game);
                        state.game_dock_node = Some(game);
                        state.panel_dock_node = Some(panel);
                        state.rail_dock_node = Some(rail);
                    }
                }
                DockBuilder::finish(ui, dock_id);
                state.docked_game_title = game_title.to_string();
                state.dock_inited = true;
            } else if state.docked_game_title != game_title {
                if let Some(left) = state.game_dock_node {
                    DockBuilder::dock_window(ui, game_title, left);
                }
                state.docked_game_title = game_title.to_string();
            }
        });
}

/// The Game pane: the single focused applet, or — while MultiBox is in
/// Grid mode — one cell per wall member.
fn game_window_flags() -> WindowFlags {
    WindowFlags::NO_COLLAPSE
        | WindowFlags::NO_SCROLLBAR
        | WindowFlags::NO_SCROLL_WITH_MOUSE
        | WindowFlags::NO_RESIZE
}

fn game_window(ui: &Ui, gpu: &mut Gpu, state: &mut PanelState, title: &str) {
    let built = ui.window(title).flags(game_window_flags()).build(|| {
        let avail = ui.content_region_avail();
        if state.session.walkto_open {
            picker::draw_picker(ui, Some(gpu), &mut state.session, &mut state.walk_map);
        } else if state.session.multibox && state.session.wall.grid {
            grid_pane(ui, gpu, state, avail);
        } else {
            game_pane(ui, gpu, state, avail);
        }
    });
    state.session.set_game_pane_open(built.is_some());
}

/// Single-bot / rail Game pane: native 765×503, centred in the leaf.
/// Does not scale with the host window; grid mode fits cells to avail.
fn game_pane(ui: &Ui, gpu: &mut Gpu, state: &mut PanelState, avail: [f32; 2]) {
    let size = scale_size(ui, native_applet());
    let cursor = ui.cursor_pos();
    let off = applet_offset(avail, size);
    ui.set_cursor_pos([cursor[0] + off[0], cursor[1] + off[1]]);
    if state.game_view.is_none() {
        state.game_view = Some(GameView::init(gpu));
    }
    let (draw, capture) = {
        let focus = state.session.focus.lock().unwrap();
        (should_draw(&focus), should_capture(&focus))
    };
    if draw {
        let buf = state.session.focused_pixels();
        // One consumer per `FrameBuf`: `take` moves the stored frame out
        // (a `FrameOutput::Texture` hands its view off once, no Clone),
        // and `present` routes either variant — the `Texture` arm binds
        // the client's frame view directly (the shared-device seam, no
        // read-back), the `PixMap` arm uploads the CPU pixels. `take`
        // returning `None` is "no new frame since the last upload", the
        // same skip the old snapshot path gave.
        let name = state.session.focused_name().unwrap_or_default();
        let gen = buf.as_ref().map(|p| p.generation()).unwrap_or(0);
        let dirty = state.last_upload.as_ref() != Some(&(name.clone(), gen));
        if dirty {
            let took_frame = if let Some(frame) = buf.as_ref().and_then(|p| p.take()) {
                if let Some(view) = state.game_view.as_mut() {
                    #[cfg(feature = "render-diagnostics")]
                    {
                        let roi = buf.as_ref().and_then(|p| p.take_pixel_roi());
                        client::render::diagnostics::prepare_present_roi(
                            roi.map(|r| (r, name.as_str(), gen)),
                        );
                        match &frame {
                            client::render::backend::FrameOutput::PixMap(_) => {
                                client::render::diagnostics::arm_game_image_cpu_upload();
                            }
                            client::render::backend::FrameOutput::Texture(_) => {
                                client::render::diagnostics::arm_game_image_gpu_present();
                            }
                        }
                    }
                    view.present(gpu, frame);
                }
                true
            } else {
                false
            };
            record_presented_upload(&mut state.last_upload, name.clone(), gen, took_frame);
        }
        let view = state.game_view.as_ref().expect("game view initialized");
        ui.image(view.tex_id, size);
        let min = ui.item_rect_min();
        #[cfg(feature = "render-diagnostics")]
        {
            let fb = ui.io().display_framebuffer_scale();
            client::render::diagnostics::note_game_image_fb(
                min[0], min[1], size[0], size[1], fb[0], fb[1],
            );
            let ctx = state
                .last_upload
                .as_ref()
                .map(|(n, _)| n.as_str())
                .unwrap_or(name.as_str());
            client::render::diagnostics::note_readback_context(ctx);
        }
        // Queue-card overlay: the armed route's remaining tiles are
        // painted by the client's 3D renderer and on the pack map, so the
        // Image only carries the displayed slot's own queue card.
        let queue = state.session.queue_for(&name);
        state.overlay.frame(ui, queue, min, size);
        // Script-paint overlay: the focused slot's paint renders in an
        // ImGui window over the chatbox rect — never on the game texture.
        let paint = focused_paint(&state.session);
        overlay_script_paint(ui, gpu, state, paint, min, size);
        // Capture: only map/enqueue while on and hovered;
        // capture off skips the coord math entirely (tx is
        // also None).
        if capture && ui.is_item_hovered() {
            let mouse = ui.io().mouse_pos();
            let min = ui.item_rect_min();
            stream_capture(
                &state.session.capture_tx,
                mouse[0] - min[0],
                mouse[1] - min[1],
                size[0],
                size[1],
                ui.is_mouse_clicked(MouseButton::Left),
                ui.is_mouse_clicked(MouseButton::Right),
                ui.is_mouse_released(MouseButton::Left),
                ui.is_mouse_released(MouseButton::Right),
                &capture_keys(ui),
            );
        }
    } else {
        ui.text_disabled("renderer off");
        state.paint.release_canvas(gpu);
    }
}

/// MultiBox grid-mode Game pane: same cap as the sidecar (dot, name +
/// brief, fold, ✗) over a bigger blit. Status stays visible with the
/// 330px panel collapsed. `only_render_selected` / fold hide the blit
/// only. Capture reaches the focused cell's body.
fn grid_pane(ui: &Ui, gpu: &mut Gpu, state: &mut PanelState, avail: [f32; 2]) {
    state.sync_fleet();
    if state.fleet.is_empty() {
        ui.text_disabled("no wall members");
        state.paint.release_canvas(gpu);
        return;
    }
    let rows = std::mem::take(&mut state.fleet);
    let labels = std::mem::take(&mut state.fleet_labels);
    let cells = grid_cells(rows.len(), avail);
    let (capture, only_selected) = {
        let focus = state.session.focus.lock().unwrap();
        (should_capture(&focus), focus.only_render_selected)
    };
    {
        let focus = state.session.focus.lock().unwrap();
        dispose_idle_views(&mut state.views, gpu, |name| {
            rows.iter().any(|row| row.name == name) && draw_for_slot(&focus, name)
        });
    }
    // Only the focused cell paints the script overlay (an `Arc`, no copy).
    let mut focused_paint = focused_paint(&state.session);
    for ((row, labels), [cx, cy, cw, ch]) in rows.iter().zip(&labels).zip(cells) {
        let name = &row.name;
        let (is_focused, draw) = {
            let focus = state.session.focus.lock().unwrap();
            (
                focus.focused.as_deref() == Some(name.as_str()),
                draw_for_slot(&focus, name),
            )
        };
        ui.set_cursor_pos([cx, cy]);
        let preview = rail_preview_open(
            name,
            is_focused,
            only_selected,
            true,
            &state.session.ui.rail_preview,
        );
        let (cap_select, cap_remove, cap_fold) = rail_cap(ui, row, labels, is_focused, cw, preview);
        let mut body_clicked = false;
        if preview {
            let after = ui.cursor_pos();
            let remain_h = (cy + ch - after[1]).max(1.0);
            let size = fit_applet([cw, remain_h]);
            ui.set_cursor_pos([
                cx + ((cw - size[0]) * 0.5).max(0.0),
                after[1] + ((remain_h - size[1]) * 0.5).max(0.0),
            ]);
            body_clicked = cell_body(ui, gpu, state, name, size, draw);
            let image_min = ui.item_rect_min();
            if is_focused && capture && ui.is_item_hovered() {
                let mouse = ui.io().mouse_pos();
                let min = image_min;
                stream_capture(
                    &state.session.capture_tx,
                    mouse[0] - min[0],
                    mouse[1] - min[1],
                    size[0],
                    size[1],
                    ui.is_mouse_clicked(MouseButton::Left),
                    ui.is_mouse_clicked(MouseButton::Right),
                    ui.is_mouse_released(MouseButton::Left),
                    ui.is_mouse_released(MouseButton::Right),
                    &capture_keys(ui),
                );
            }
            draw_queue_card_for(ui, row.queue, image_min);
            if is_focused {
                overlay_script_paint(
                    ui,
                    gpu,
                    state,
                    focused_paint.take(),
                    ui.item_rect_min(),
                    size,
                );
            }
        } else if is_focused {
            state.paint.release_canvas(gpu);
        }
        if cap_fold {
            let next = !preview;
            state.session.ui.rail_preview.insert(name.clone(), next);
            crate::ui_state::save(&state.session.ui);
        } else if cap_remove {
            state.session.rail_remove(name);
        } else if cap_select || body_clicked {
            state.session.select(name);
        }
    }
    state.fleet = rows;
    state.fleet_labels = labels;
}

/// Right panel: rs2b0t chrome squished into the 330px strip. Vertical scroll
/// only — wrap/clip, never a horizontal bar.
fn panel_window(ui: &Ui, session: &mut Session, progress: Option<StartupProgressView>) {
    // The ### suffix preserves the existing docking identity across revisions.
    ui.window(format!("{}###{PANEL_WINDOW}", session.app_title()))
        .flags(WindowFlags::NO_RESIZE | WindowFlags::NO_COLLAPSE)
        .build(|| {
            let _width = ui.push_item_width(-1.0);
            let _wrap = ui.push_text_wrap_pos(0.0);
            title_row(ui, session);
            ui.text_colored(TEXT_DIM, crate::build_info::build_line());
            ui.set_item_tooltip(crate::build_info::build_tooltip());
            banner(ui, session, progress);
            login_logout_row(ui, session);
            walkto_button(ui, session);
            slot_config_row(ui, session);
            let order = resolve_heading_order(&session.ui.section_order);
            // The log fills the leftover height only when nothing follows it.
            let log_last = !crate::ui_state::panel_section_visible(&session.ui, "parameters")
                && order
                    .iter()
                    .rev()
                    .find(|id| crate::ui_state::panel_section_visible(&session.ui, id))
                    .is_some_and(|id| id == "log");
            for id in order {
                if !crate::ui_state::panel_section_visible(&session.ui, &id) {
                    continue;
                }
                match id.as_str() {
                    "status" => status_section(ui, session),
                    "resource" => {
                        if !session.multibox {
                            resource_section(ui, session);
                        }
                    }
                    "profile" => profile_section(ui, session),
                    "script" => script_section(ui, session),
                    "debug" => debug_section(ui, session),
                    "log" => log_section(ui, session, log_last),
                    _ => {}
                }
            }
            if crate::ui_state::panel_section_visible(&session.ui, "parameters") {
                parameters_section(ui, session);
            }
        });
}

fn title_row(ui: &Ui, session: &mut Session) {
    ui.text_colored(ACCENT, session.app_title());
    ui.same_line();
    let avail = ui.content_region_avail()[0];
    let (w, stack) = scaled_button_row_layout(ui, avail, 2);
    if !stack {
        let total = w * 2.0 + scale_px(ui, BUTTON_GAP);
        ui.set_cursor_pos_x(ui.cursor_pos()[0] + (avail - total).max(0.0));
    }
    if ui.button_with_size("MultiBox", [w, 0.0]) {
        session.set_multibox(!session.multibox);
    }
    ui.set_item_tooltip(multibox_tooltip(session.multibox));
    if !stack {
        gap_line(ui);
    }
    // Grid is a MultiBox submode: hide the rail, Game pane lays members.
    let _grid_disabled = if !session.multibox {
        Some(ui.begin_disabled())
    } else {
        None
    };
    if ui.button_with_size("Grid", [w, 0.0]) {
        session.set_grid(!session.wall.grid);
    }
    ui.set_item_tooltip(if session.multibox {
        "grid mode — hide rail"
    } else {
        "enable MultiBox first"
    });
}

#[derive(Debug, PartialEq, Eq)]
struct LoadingText {
    description: String,
    filled: String,
    empty: String,
    percent: u8,
    caption: String,
}

fn loading_text(phase: ProgressPhase, progress: &ProfileProgress) -> LoadingText {
    const CELLS: u64 = 20;
    let completed = progress.completed.min(progress.total);
    let percent = if progress.total == 0 {
        100
    } else {
        ((u128::from(completed) * 100) / u128::from(progress.total)) as u8
    };
    let filled = (u64::from(percent) * CELLS / 100) as usize;
    let description = match (phase, progress.stage) {
        (ProgressPhase::FinalChecks, ProfileProgressStage::FinalChecks) => {
            ProfileProgressStage::FinalChecks.description().to_string()
        }
        (ProgressPhase::FinalChecks, stage) => {
            format!("Final checks: {}", stage.description())
        }
        (ProgressPhase::Preparing, stage) => stage.description().to_string(),
    };
    let caption = match progress.unit {
        ProfileProgressUnit::Bytes => {
            format!("{} of {} bytes checked", progress.completed, progress.total)
        }
        ProfileProgressUnit::Files => {
            format!("{} of {} files checked", progress.completed, progress.total)
        }
        ProfileProgressUnit::Steps => format!(
            "{} of {} steps complete",
            progress.completed, progress.total
        ),
    };
    LoadingText {
        description,
        filled: "#".repeat(filled),
        empty: "-".repeat(CELLS as usize - filled),
        percent,
        caption,
    }
}

fn loading_banner(ui: &Ui, phase: ProgressPhase, progress: &ProfileProgress) {
    let text = loading_text(phase, progress);
    ui.text_colored(ACCENT, &text.description);
    ui.text_colored(ACCENT, format!("[{}", text.filled));
    ui.same_line_with_spacing(0.0, 0.0);
    ui.text_disabled(format!("{}] {:>3}%", text.empty, text.percent));
    ui.text_disabled(text.caption);
}

fn banner(ui: &Ui, session: &Session, progress: Option<StartupProgressView>) {
    if let Some(err) = &session.error {
        ui.text_colored(ERROR, err);
    } else if session.profile_preparing() {
        if let Some(progress) = progress {
            loading_banner(ui, progress.phase, &progress.progress);
        } else {
            loading_banner(
                ui,
                ProgressPhase::Preparing,
                &ProfileProgress::steps(ProfileProgressStage::SelectingServerProfile, 0, 1),
            );
        }
    } else if let Some(detail) = session.core.fleet_view().detail() {
        slot_banner(ui, detail);
    }
}

/// The selected bot's state line while it is on its way in (with an
/// elapsed timer when one applies), parked, or failing. Nothing while it is
/// in game or not started.
fn slot_banner(ui: &Ui, detail: &frontend_core::SlotDetail) {
    let phase = detail.row.phase;
    if matches!(phase, Phase::Ready | Phase::Offline) {
        return;
    }
    let text = sentence_case(&detail.state);
    if phase.is_error() {
        ui.text_colored(ERROR, &text);
    } else if let Some(since) = detail.since {
        let elapsed = since.elapsed().as_secs_f64();
        let text = text.trim_end_matches('…');
        ui.text_colored(ACCENT, format!("{text} — {elapsed:.1}s"));
    } else {
        ui.text_colored(ACCENT, &text);
    }
}

/// `text` with its first letter upper-cased (banner headline style).
fn sentence_case(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Collapsing section header. Open state is per focused profile in
/// `PanelUiState.collapsed`; defaults closed for script + parameters.
fn section_open(ui: &Ui, session: &mut Session, id: &str) -> bool {
    let user = session.focused_name().unwrap_or_else(|| "_".into());
    let closed = session
        .ui
        .collapsed
        .get(&user)
        .and_then(|m| m.get(id))
        .copied()
        .unwrap_or_else(|| crate::ui_state::default_section_closed(id));
    let desired = !closed;
    ui.set_next_item_open(desired);
    let open = ui.collapsing_header(id, TreeNodeFlags::NONE);
    ui.set_item_tooltip("drag to reorder");
    heading_dnd(ui, session, id);
    if open != desired {
        session
            .ui
            .collapsed
            .entry(user)
            .or_default()
            .insert(id.to_string(), !open);
        crate::ui_state::save(&session.ui);
    }
    open
}

fn heading_key(id: &str) -> [u8; 16] {
    let mut a = [0u8; 16];
    let b = id.as_bytes();
    let n = b.len().min(16);
    a[..n].copy_from_slice(&b[..n]);
    a
}

fn heading_from_key(a: [u8; 16]) -> Option<String> {
    let n = a.iter().position(|&c| c == 0).unwrap_or(16);
    std::str::from_utf8(&a[..n]).ok().map(str::to_string)
}

/// Drag a collapsing header onto another to reorder the strip. Persists.
fn heading_dnd(ui: &Ui, session: &mut Session, id: &str) {
    if let Some(_tip) = ui
        .drag_drop_source_config("274bot-heading")
        .begin_payload(heading_key(id))
    {
        ui.text(id);
    }
    if let Some(tgt) = ui.drag_drop_target() {
        if let Some(Ok(p)) =
            tgt.accept_payload::<[u8; 16], _>("274bot-heading", DragDropTargetFlags::NONE)
        {
            if p.delivery {
                if let Some(from) = heading_from_key(p.data) {
                    let mut order = resolve_heading_order(&session.ui.section_order);
                    move_heading(&mut order, &from, id);
                    session.ui.section_order = order;
                    crate::ui_state::save(&session.ui);
                }
            }
        }
    }
}

fn kv_row(ui: &Ui, key: &str, value: &str) {
    ui.text_disabled(key);
    ui.same_line();
    ui.text_wrapped(value);
}

fn draw_resource_rows(ui: &Ui, view: &ResourceView, show_background: bool) {
    kv_row(ui, "bots", &format_bots(view.bots, view.ingame));
    if show_background && view.background > 0 {
        kv_row(ui, "background", &format_background(view.background));
    }
    kv_row(ui, "cpu", view.cpu.text());
    kv_row(ui, "ram", view.ram.text());
    kv_row(ui, "traffic", view.traffic.text());
}

/// Same-line gap that matches [`equal_button_width`]'s `BUTTON_GAP`.
fn gap_line(ui: &Ui) {
    ui.same_line_with_spacing(0.0, scale_px(ui, BUTTON_GAP));
}

fn mock_button(ui: &Ui, label: &str, hint: &str, size: [f32; 2]) {
    let _disabled = ui.begin_disabled();
    ui.button_with_size(label, size);
    // SetItemTooltip: only this widget, including while disabled. `tooltip_text`
    // is SetTooltip and every mock dumps into one always-on blob.
    ui.set_item_tooltip(hint);
}

/// `hop_label_px` write path: clamp to the `NavSettings` 8..=28 field
/// invariant (the struct has no setter; the modal is the only writer).
fn clamp_hop_label_px(px: i32) -> i32 {
    px.clamp(8, 28)
}

/// Unlock / create / reset the default vault. Shared by the profile
/// heading and the Profiles window so a locked vault is not shown as empty.
fn vault_unlock_prompt(ui: &Ui, session: &mut Session) {
    let exists = match session.default_vault_exists() {
        Ok(exists) => exists,
        Err(error) => {
            session.error = Some(format!("server profile: {error}"));
            false
        }
    };
    // A new vault needs a non-empty passphrase; existing vaults open with
    // whatever passphrase decrypts them.
    let hint = if exists {
        "vault passphrase".to_string()
    } else {
        "new vault passphrase".to_string()
    };
    ui.input_text("##vault-pass", &mut session.pass_scratch)
        .password(true)
        .hint(hint)
        .build();
    let w = ui.content_region_avail()[0];
    let label = if exists {
        "Unlock vault"
    } else {
        "Create vault"
    };
    if ui.button_with_size(label, [w, 0.0]) {
        submit_vault_pass(session, exists);
    }
    if exists {
        if ui.button_with_size("Reset vault", [w, 0.0]) {
            session.vault_reset_understood = false;
            ui.open_popup(VAULT_RESET_POPUP);
        }
        ui.set_item_tooltip("delete the vault file — forgotten passphrase");
    }
    if scary_confirm_popup(
        ui,
        VAULT_RESET_POPUP,
        "This deletes the vault file on disk. Every saved profile is gone. \
         You will Create vault with a new passphrase. Wrong-passphrase unlock \
         never does this on its own.",
        "Reset",
        &mut session.vault_reset_understood,
    ) {
        session.reset_vault();
    }
}

/// Unlock with the passphrase exactly as typed: a vault made in the terminal
/// may begin or end with spaces. Create keeps the vault rule (non-empty once
/// trimmed) and stores the typed text too, so both front ends agree.
fn submit_vault_pass(session: &mut Session, exists: bool) {
    let typed = session.pass_scratch.as_str();
    let usable = if exists {
        !typed.is_empty()
    } else {
        !typed.trim().is_empty()
    };
    if usable {
        let pass = vault::Secret::from(typed);
        session.request_unlock(pass);
        session.pass_scratch.clear();
    }
}

/// profile: orange current name (like script) plus Profiles (opens the
/// picker). Unlock prompt until the vault is open. Sits under WalkTo /
/// config, above debug.
fn profile_section(ui: &Ui, session: &mut Session) {
    if !section_open(ui, session, "profile") {
        return;
    }
    if session.core.vault().is_none() {
        vault_unlock_prompt(ui, session);
        return;
    }
    profile_combo(ui, session);
    let w = ui.content_region_avail()[0];
    if ui.button_with_size("Profiles", [w, 0.0]) {
        session.wall.chooser_open = true;
    }
    ui.set_item_tooltip("pick or edit a vault profile");
}

/// Profile switcher: black field, orange current name, inverse arrow
/// (orange square, black chevron). **Profiles** still opens the picker.
fn profile_combo(ui: &Ui, session: &mut Session) {
    let names = session.profile_names();
    let preview = session
        .focused_name()
        .unwrap_or_else(|| "(none)".to_string());
    ui.set_next_item_width(-1.0);
    let black = [0.0, 0.0, 0.0, 1.0];
    let _frame = ui.push_style_color(StyleColor::FrameBg, black);
    let _frame_h = ui.push_style_color(StyleColor::FrameBgHovered, black);
    let _frame_a = ui.push_style_color(StyleColor::FrameBgActive, black);
    let _popup = ui.push_style_color(StyleColor::PopupBg, black);
    let _text = ui.push_style_color(StyleColor::Text, ACCENT);
    let opts = ComboBoxOptions::new().preview_mode(ComboBoxPreviewMode::Preview);
    if let Some(_open) = ui.begin_combo_with_flags("##profile", &preview, opts) {
        for name in &names {
            let selected = preview.as_str() == name;
            let _row = ui.push_style_color(StyleColor::Text, if selected { ACCENT } else { TEXT });
            if ui.selectable_config(name).selected(selected).build() {
                session.select(name);
            }
            if selected {
                ui.set_item_default_focus();
            }
        }
    }
    paint_inverse_combo_arrow(ui);
}

/// Cover the default combo chevron with an orange chip and a black arrow.
fn paint_inverse_combo_arrow(ui: &Ui) {
    let min = ui.item_rect_min();
    let max = ui.item_rect_max();
    let h = max[1] - min[1];
    if h <= scale_px(ui, 2.0) {
        return;
    }
    let x0 = max[0] - h;
    let dl = ui.get_window_draw_list();
    dl.add_rect([x0, min[1]], max, ACCENT).filled(true).build();
    let cx = x0 + h * 0.5;
    let cy = (min[1] + max[1]) * 0.5;
    let s = h * 0.22;
    dl.add_triangle(
        [cx - s, cy - s * 0.45],
        [cx + s, cy - s * 0.45],
        [cx, cy + s],
        [0.0, 0.0, 0.0, 1.0],
    )
    .filled(true)
    .build();
}

fn logout_enabled(
    vault_open: bool,
    focused: bool,
    connected: bool,
    queued: bool,
    guard_parked: bool,
) -> bool {
    vault_open && focused && (connected || queued || guard_parked)
}

/// Log in / Logout above WalkTo. Always drawn; disabled while the vault
/// is locked or no profile is focused. Logout needs a connected, genuinely
/// queued or repeat-guard-parked focused slot, so an unloaded profile cannot
/// be latched accidentally.
fn login_logout_row(ui: &Ui, session: &mut Session) {
    let avail = ui.content_region_avail()[0];
    let cells = scaled_button_cells(ui, avail, 2);
    let vault_open = session.core.vault().is_some();
    let focused = session.focused_name();
    let can_login = vault_open && focused.is_some();
    let focused_queued = focused
        .as_deref()
        .and_then(|name| session.queue_for(name))
        .is_some();
    let guard_parked = session.focused_guard_parked();
    let can_logout = logout_enabled(
        vault_open,
        focused.is_some(),
        session.focused_connected(),
        focused_queued,
        guard_parked,
    );
    {
        let _off = (!can_login).then(|| ui.begin_disabled());
        if ui.button_with_size("Log in", [cells[0].0, 0.0]) {
            if let Some(name) = focused.as_deref() {
                session.login(name);
            }
        }
        ui.set_item_tooltip(if !vault_open {
            "unlock the vault first"
        } else if focused.is_none() {
            "pick a profile"
        } else {
            "handshake the focused profile"
        });
    }
    if cells[1].1 {
        gap_line(ui);
    }
    {
        let _off = (!can_logout).then(|| ui.begin_disabled());
        if ui.button_with_size("Logout", [cells[1].0, 0.0]) {
            if let Some(name) = focused.as_deref() {
                session.logout(name);
            }
        }
        ui.set_item_tooltip(if !vault_open {
            "unlock the vault first"
        } else if focused.is_none() {
            "pick a profile"
        } else if focused_queued {
            "cancel the focused slot's queued login"
        } else if guard_parked {
            "end the parked session — its held script work is discarded"
        } else {
            "log out the focused slot — it stays in the picker"
        });
    }
}

/// WalkTo and Fleet share the action row. Fleet opens independently of the
/// Game/Grid wall, so the wall's rendering controls remain unchanged.
fn walkto_button(ui: &Ui, session: &mut Session) {
    let avail = ui.content_region_avail()[0];
    let cells = scaled_button_cells(ui, avail, 2);
    if ui.button_with_size("WalkTo", [cells[0].0, 0.0]) {
        session.walkto_open = !session.walkto_open;
    }
    ui.set_item_tooltip("open the tile picker — close with the window ✕");
    if cells[1].1 {
        gap_line(ui);
    }
    if ui.button_with_size("Fleet", [cells[1].0, 0.0]) {
        session.fleet_open = true;
    }
    ui.set_item_tooltip("open the marked-bot Fleet window");
}

fn debug_caption(id: &str) -> &str {
    match id {
        "DebugPanel" => "Panel",
        "Lumbridge" => "Lumb",
        s => s,
    }
}

/// Local-engine debug cheats. Omitted on rs2b2t.
fn debug_section(ui: &Ui, session: &mut Session) {
    if !session.debug_ui() {
        return;
    }
    if !section_open(ui, session, "debug") {
        return;
    }
    let show_tutskip = session.focused_tutorial_skipped() == Some(false);
    let main = debug_main_buttons_for(session.profile_class(), show_tutskip);
    let avail = ui.content_region_avail()[0];
    // Packed one row even when a scrollbar trims avail below MIN_BUTTON —
    // stacking turns DebugPanel/Lumbridge/maxme/Teles into four strip-width
    // buttons.
    let scale = ui_scale(ui);
    let w = equal_button_width(avail / scale, main.len()) * scale;
    for (i, label) in main.iter().enumerate() {
        if i > 0 {
            gap_line(ui);
        }
        let caption = debug_caption(label);
        match *label {
            "DebugPanel" => {
                if ui.button_with_size(caption, [w, 0.0]) {
                    session.debug_panel_open = true;
                }
                ui.set_item_tooltip("DebugPanel v2 — full cheat catalog");
            }
            "TutSkip" => {
                if ui.button_with_size(caption, [w, 0.0]) {
                    session.cheat_focused("setvar tutorial 1000");
                    session.mark_tutorial_skipped();
                }
                ui.set_item_tooltip("setvar tutorial 1000");
            }
            "Lumbridge" => {
                if ui.button_with_size(caption, [w, 0.0]) {
                    session.cheat_focused("~home");
                }
                ui.set_item_tooltip("~home — Lumbridge courtyard");
            }
            "maxme" => {
                if ui.button_with_size(caption, [w, 0.0]) {
                    session.debug_panel_open = true;
                    crate::debug_panel::request_bulk_send(
                        ui,
                        session,
                        "maxme".into(),
                        debug_maxme_cheats()
                            .iter()
                            .map(|cmd| (*cmd).to_string())
                            .collect(),
                        "maxme (19× setstat 99)".into(),
                        "This sets every skill to 99.".into(),
                    );
                }
                ui.set_item_tooltip("19× setstat 99 — confirmation is required");
            }
            "Teles" if ui.button_with_size(caption, [w, 0.0]) => {
                ui.open_popup("##debug-teles");
            }
            _ => {}
        }
    }
    debug_teleports_popup(ui, session);
}

fn debug_teleports_popup(ui: &Ui, session: &mut Session) {
    ui.popup("##debug-teles", || {
        let dests = debug_dest_cheats();
        let scale = ui_scale(ui);
        let avail = PANEL_WIDTH;
        let cols = (1usize..=6)
            .rev()
            .find(|&n| equal_button_width(avail, n) >= MIN_BUTTON)
            .unwrap_or(1);
        let (bw, _) = button_row_layout(avail, cols);
        let bw = bw * scale;
        for (i, dest) in dests.iter().enumerate() {
            if i > 0 && i % cols != 0 {
                gap_line(ui);
            }
            if ui.button_with_size(dest.label, [bw, 0.0]) {
                session.cheat_focused(dest.cheat);
                ui.close_current_popup();
            }
            ui.set_item_tooltip(dest.tooltip);
        }
    });
}

/// script: real Browse/Start/Pause/Stop with the rs2b0t disable rules.
/// `active` = Running|Paused|Stopping: Start and Browse are disabled while
/// a script holds the slot; Pause/Resume is enabled only while Running or
/// Paused (label switches to "Resume"); Stop is enabled while active but
/// not already Stopping. Browse lists the compiled ids then the loaded JS
/// cards, and selecting does not Start — Start is the section button.
/// Load opens a path modal; Start spawns the loaded card's isolate.
fn script_section(ui: &Ui, session: &mut Session) {
    if !section_open(ui, session, "script") {
        return;
    }
    let state = session.focused_script_state();
    let active = script_active(state);
    let paused = state == script::RunState::Paused;

    let name = session
        .script_sel
        .as_ref()
        .map(|sel| match sel {
            script::ScriptSel::Loaded(source, identity) => session
                .scripts
                .js
                .get(*source, identity)
                .map(|card| card.name.clone())
                .unwrap_or_else(|| {
                    std::path::Path::new(identity)
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| identity.clone())
                }),
            script::ScriptSel::Compiled(_) => sel.label(),
        })
        .unwrap_or_else(|| "(none)".to_string());
    ui.text_colored(ACCENT, name);
    if let Some(sel) = &session.script_sel {
        ui.set_item_tooltip(sel.label());
    }
    if session.heading_is_pending() {
        ui.same_line();
        ui.text_disabled("(pending)");
    }
    if let (Some(script::ScriptSel::Loaded(source, sel_name)), Some((front_src, front_name))) = (
        session.script_sel.clone(),
        session.transpile_front().map(|(s, n)| (s, n.to_string())),
    ) {
        if let Some(line) = card_transpile_label(
            Some((front_src, front_name.as_str())),
            source,
            &sel_name,
            session.transpile_done,
            session.transpile_total,
        ) {
            ui.same_line();
            ui.text_disabled(line);
        }
    }
    let avail = ui.content_region_avail()[0];
    let (w, stack) = scaled_button_row_layout(ui, avail, 3);
    {
        let _browse = if active {
            Some(ui.begin_disabled())
        } else {
            None
        };
        if ui.button_with_size("Browse…", [w, 0.0]) {
            session.on_script_browse_open();
            session.script_browse_open = true;
        }
        ui.set_item_tooltip("pick a compiled script or a loaded JS bot");
    }
    if !stack {
        gap_line(ui);
    }
    {
        let _load = if active {
            Some(ui.begin_disabled())
        } else {
            None
        };
        if ui.button_with_size("Load", [w, 0.0]) {
            session.fill_rs2b0t_cards_once();
            session.open_script_load_browser();
        }
        ui.set_item_tooltip("load an out-of-tree JS bot file (native tick or defineBot)");
    }
    if !stack {
        gap_line(ui);
    }
    let confirming = session.script_reload_confirmation_pending();
    let validating = session.reload_validation_pending();
    {
        let _validating = validating.then(|| ui.begin_disabled());
        let label = if validating {
            "Validating…"
        } else if confirming {
            "Confirm"
        } else {
            "Reload"
        };
        if ui.button_with_size(label, [w, 0.0]) {
            session.begin_script_reload_clicked();
        }
    }
    ui.set_item_tooltip(if validating {
        "validating candidate script off the UI thread"
    } else if confirming {
        "confirm reload: restart listed running bots and stop listed paused bots"
    } else {
        "hash source and supported siblings; unchanged skips transpile"
    });
    if (confirming || validating) && ui.button_with_size("Cancel reload", [avail, 0.0]) {
        session.cancel_reload();
    }

    let (sw, sstack) = scaled_button_row_layout(ui, ui.content_region_avail()[0], SCRIPT_ROW.len());
    {
        let _start = if active {
            Some(ui.begin_disabled())
        } else {
            None
        };
        if ui.button_with_size("Start", [sw, 0.0]) {
            session.script_start_selected();
        }
    }
    if !sstack {
        gap_line(ui);
    }
    {
        let _pause = if script_pause_enabled(state) {
            None
        } else {
            Some(ui.begin_disabled())
        };
        if ui.button_with_size(if paused { "Resume" } else { "Pause" }, [sw, 0.0]) {
            session.script_toggle_pause();
        }
    }
    if !sstack {
        gap_line(ui);
    }
    {
        let queued = session
            .core
            .selected()
            .is_some_and(|name| session.scripts.start_queue_place(name).is_some());
        let _stop = if script_stop_enabled(state) || queued {
            None
        } else {
            Some(ui.begin_disabled())
        };
        if ui.button_with_size("Stop", [sw, 0.0]) {
            session.script_stop();
        }
    }
    let status = run_state_label(state);
    match session.focused_script_last_error() {
        Some(err) => kv_row(ui, "status", &format!("{status}: {err}")),
        None => kv_row(ui, "status", status),
    }
}

fn category_key(cat: &str) -> [u8; 32] {
    let mut a = [0u8; 32];
    let b = cat.as_bytes();
    let n = b.len().min(31);
    a[..n].copy_from_slice(&b[..n]);
    a
}

fn category_from_key(a: [u8; 32]) -> Option<String> {
    let n = a.iter().position(|&c| c == 0).unwrap_or(32);
    std::str::from_utf8(&a[..n]).ok().map(str::to_string)
}

fn card_categories(cards: &[BrowseCard<'_>]) -> Vec<String> {
    let mut out = Vec::new();
    for card in cards {
        let cat = display_category((*card).category()).to_string();
        if !out.iter().any(|c| c == &cat) {
            out.push(cat);
        }
    }
    out
}

fn script_category_chips(ui: &Ui, session: &mut Session, order: &[String]) {
    let avail = ui.content_region_avail()[0];
    let style = ui.clone_style();
    let gap = style.item_spacing()[0];
    let mut pad = chip_frame_padding(style.frame_padding());
    pad[1] = pad[1].max(scale_px(ui, script_picker::CHIP_PAD_Y));
    let _pad = ui.push_style_var(StyleVar::FramePadding(pad));
    let _border_sz = ui.push_style_var(StyleVar::FrameBorderSize(scale_px(ui, 1.0)));
    let _border = ui.push_style_color(StyleColor::Border, ACCENT);
    let mut used = 0.0;
    let font_sz = ui.current_font_size();
    let all_selected = session.browse_category_filter.is_none();
    let labels: Vec<(String, Option<String>, bool)> =
        std::iter::once(("All".to_string(), None, all_selected))
            .chain(order.iter().map(|cat| {
                (
                    cat.clone(),
                    Some(cat.clone()),
                    session.browse_category_filter.as_deref() == Some(cat.as_str()),
                )
            }))
            .collect();
    for (label, cat, selected) in labels {
        let chip_w = ui
            .current_font()
            .calc_text_size(font_sz, f32::MAX, 0.0, &label)[0]
            + pad[0] * 2.0;
        if chip_wraps(used, chip_w, gap, avail) {
            used = 0.0;
        } else if used > 0.0 {
            ui.same_line_with_spacing(0.0, gap);
        }
        let _b = selected.then(|| ui.push_style_color(StyleColor::Button, ACCENT));
        let _h = selected.then(|| ui.push_style_color(StyleColor::ButtonHovered, ACCENT_HOVER));
        let _a = selected.then(|| ui.push_style_color(StyleColor::ButtonActive, ACCENT));
        let _t = ui.push_style_color(StyleColor::Text, chip_text_color(selected));
        if ui.button(&label) {
            session.browse_category_filter = cat.clone();
        }
        if let Some(cat) = cat.as_deref() {
            category_chip_dnd(ui, session, cat);
        }
        used = if used == 0.0 {
            chip_w
        } else {
            used + gap + chip_w
        };
    }
}

fn category_chip_dnd(ui: &Ui, session: &mut Session, cat: &str) {
    if let Some(_tip) = ui
        .drag_drop_source_config("274bot-script-cat")
        .begin_payload(category_key(cat))
    {
        ui.text(cat);
    }
    if let Some(tgt) = ui.drag_drop_target() {
        if let Some(Ok(p)) =
            tgt.accept_payload::<[u8; 32], _>("274bot-script-cat", DragDropTargetFlags::NONE)
        {
            if p.delivery {
                if let Some(from) = category_from_key(p.data) {
                    let cards: Vec<_> = session.scripts.browse_cards().collect();
                    let present = card_categories(&cards);
                    let mut order =
                        resolve_category_order(&session.ui.script_category_order, &present);
                    move_category(&mut order, &from, cat);
                    session.ui.script_category_order = order;
                    crate::ui_state::save(&session.ui);
                }
            }
        }
    }
}

fn browse_script_card(
    ui: &Ui,
    session: &Session,
    card: BrowseCard<'_>,
    w: f32,
) -> Option<script::ScriptSel> {
    let selection = card.selection();
    let selected = session.script_sel.as_ref() == Some(&selection);
    let id = match card {
        BrowseCard::Compiled(card) => format!("##scard-compiled-{}", card.id.0),
        BrowseCard::Loaded(card) => format!("##scard-{}", card.identity_key()),
    };
    let _border = selected.then(|| ui.push_style_color(StyleColor::Border, ACCENT));
    ui.child_window(&id)
        .size([w, 0.0])
        .border(true)
        .child_flags(ChildFlags::BORDERS | ChildFlags::AUTO_RESIZE_Y)
        .flags(WindowFlags::NO_SCROLLBAR)
        .build(ui, || {
            let inner = ui.content_region_avail()[0].max(1.0);
            let row_h = ui.text_line_height_with_spacing();
            let origin = ui.cursor_screen_pos();
            let badge = card_kind_source(card.kind(), card.source());
            let font_sz = ui.current_font_size();
            let badge_w = ui
                .current_font()
                .calc_text_size(font_sz, f32::MAX, 0.0, &badge)[0];
            let gap = ui.clone_style().item_spacing()[0];
            let title_w = title_clip_width(inner, badge_w, gap);
            let badges_below = title_w < scale_px(ui, 32.0);
            let title_color = if card.unloadable().is_some() {
                TEXT_DIM
            } else {
                ACCENT
            };
            if badges_below {
                {
                    let _clip =
                        ui.push_clip_rect(origin, [origin[0] + inner, origin[1] + row_h], true);
                    ui.text_colored(title_color, card.name());
                }
                let badge_origin = [origin[0], origin[1] + row_h];
                ui.set_cursor_screen_pos(badge_origin);
                {
                    let _clip = ui.push_clip_rect(
                        badge_origin,
                        [badge_origin[0] + inner, badge_origin[1] + row_h],
                        true,
                    );
                    ui.text_disabled(&badge);
                }
            } else {
                {
                    let _clip =
                        ui.push_clip_rect(origin, [origin[0] + title_w, origin[1] + row_h], true);
                    ui.text_colored(title_color, card.name());
                }
                let badge_origin = [origin[0] + title_w + gap, origin[1]];
                ui.set_cursor_screen_pos(badge_origin);
                {
                    let _clip = ui.push_clip_rect(
                        badge_origin,
                        [origin[0] + inner, origin[1] + row_h],
                        true,
                    );
                    ui.text_disabled(&badge);
                }
            }
            if let Some(loaded) = card.loaded() {
                if let Some(line) = card_transpile_label(
                    session.transpile_front(),
                    loaded.source,
                    &loaded.name,
                    session.transpile_done,
                    session.transpile_total,
                ) {
                    let _dim = ui.push_style_color(StyleColor::Text, TEXT_DIM);
                    ui.text_disabled(line);
                }
                if let Some(failure) = session.scripts.js.load_failure(&loaded.identity_key()) {
                    ui.text_colored(ERROR, format!("failed {}", failure.stage.as_str()));
                    if selected {
                        ui.text_wrapped(failure.named_line());
                    }
                }
            }
            if !card.description().is_empty() {
                let _wrap = ui.push_text_wrap_pos(0.0);
                let line_h = ui.text_line_height();
                let full = ui.current_font().calc_text_size(
                    ui.current_font_size(),
                    f32::MAX,
                    inner,
                    card.description(),
                )[1];
                let h = card_desc_height(line_h, selected, full).max(1.0);
                if selected || h + 0.5 >= full {
                    ui.text_wrapped(card.description());
                } else {
                    ui.child_window("##desc")
                        .size([inner, h])
                        .flags(WindowFlags::NO_SCROLLBAR | WindowFlags::NO_SCROLL_WITH_MOUSE)
                        .build(ui, || {
                            ui.text_wrapped(card.description());
                        });
                }
            }
            if let Some(tags) = card.tags().filter(|tags| !tags.is_empty()) {
                let _dim = ui.push_style_color(StyleColor::Text, TEXT_DIM);
                let _wrap = ui.push_text_wrap_pos(0.0);
                ui.text_wrapped(tags.join(", "));
            }
        });
    let min = ui.item_rect_min();
    let max = ui.item_rect_max();
    card_rect_activated(
        ui.is_mouse_hovering_rect(min, max),
        ui.is_mouse_released(MouseButton::Left),
        ui.is_mouse_dragging_with_threshold(MouseButton::Left, scale_px(ui, 5.0)),
    )
    .then_some(selection)
}

fn browse_card_grid(
    ui: &Ui,
    session: &Session,
    cards: &[&BrowseCard<'_>],
) -> Option<script::ScriptSel> {
    let avail = ui.content_region_avail()[0];
    let scale = ui_scale(ui);
    let cols = card_columns(avail / scale, CARD_MIN_W, CARD_GAP);
    let w = card_width(avail / scale, cols, CARD_GAP) * scale;
    let mut selected = None;
    for (i, card) in cards.iter().enumerate() {
        if i > 0 && i % cols != 0 {
            ui.same_line_with_spacing(0.0, scale_px(ui, CARD_GAP));
        }
        selected = browse_script_card(ui, session, **card, w).or(selected);
    }
    selected
}
fn browse_window_body(ui: &Ui, session: &mut Session) {
    let w = ui.content_region_avail()[0];
    let validating = session.reload_validation_pending();
    let label = if validating {
        "Validating catalog…"
    } else if session.scripts.catalog_refresh_confirm() {
        "Confirm catalog reload"
    } else {
        "Refresh catalog"
    };
    {
        let _validating = validating.then(|| ui.begin_disabled());
        if ui.button_with_size(label, [w, 0.0]) {
            session.begin_refresh_catalog();
        }
    }
    if (session.scripts.catalog_refresh_confirm() || validating)
        && ui.button_with_size("Cancel reload", [w, 0.0])
    {
        session.cancel_reload();
    }
    ui.spacing();
    if script::rs2b0t_import_deferred() {
        if ui.button_with_size("Import catalog…", [w, 0.0]) {
            session.rs2b0t_catalog_defer_ok = false;
            session.script_dialog_search.clear();
            session.rs2b0t_catalog_dir = script_picker::default_load_browse_dir(
                session.ui.script_catalog_last_dir.as_deref(),
            );
            session.rs2b0t_catalog_open = true;
        }
        ui.spacing();
    }
    let mut present = Vec::new();
    for card in session.scripts.browse_cards() {
        let cat = display_category(card.category()).to_string();
        if !present.iter().any(|known| known == &cat) {
            present.push(cat);
        }
    }
    let order = resolve_category_order(&session.ui.script_category_order, &present);
    if order != session.ui.script_category_order {
        session.ui.script_category_order = order.clone();
        crate::ui_state::save(&session.ui);
    }
    if !order.is_empty() {
        script_category_chips(ui, session, &order);
        ui.spacing();
    }
    let busy = !session.transpile_queue.is_empty();
    let need = session.scripts.js.cards_needing_transpile().len();
    if need > 0 || busy {
        let w = ui.content_region_avail()[0];
        let _off = busy.then(|| ui.begin_disabled());
        if ui.button_with_size("Transpile all", [w, 0.0]) {
            session.queue_transpile_all();
        }
        drop(_off);
        if busy {
            if let Some((source, name)) = session.transpile_front() {
                if let Some(line) = card_transpile_label(
                    Some((source, name)),
                    source,
                    name,
                    session.transpile_done,
                    session.transpile_total,
                ) {
                    ui.text_disabled(line);
                }
            }
        }
        ui.spacing();
    }
    let named_failures = session.scripts.js.named_failure_output();
    if !named_failures.is_empty() {
        // Collapsed by default: the list is long (dim catalog cards are
        // expected misses) and pushes the script browser off screen.
        let red = ui.push_style_color(StyleColor::Text, ERROR);
        let open = ui.collapsing_header(
            format!(
                "{} failed###script-failures",
                session.scripts.js.load_failures().len()
            ),
            TreeNodeFlags::NONE,
        );
        red.pop();
        if open {
            if ui.button("Copy failures") {
                if let Ok(mut clip) = arboard::Clipboard::new() {
                    let _ = clip.set_text(&named_failures);
                }
            }
            ui.text_wrapped(&named_failures);
        }
        ui.spacing();
    }
    let cards: Vec<BrowseCard<'_>> = session.scripts.browse_cards().collect();
    let mut picked = None;
    ui.child_window("##script-list")
        .size([0.0, ui.content_region_avail()[1].max(scale_px(ui, 80.0))])
        .build(ui, || {
            let filter = session.browse_category_filter.clone();
            let mut any = false;
            for cat in &order {
                if filter.as_deref().is_some_and(|f| f != cat.as_str()) {
                    continue;
                }
                let group: Vec<&BrowseCard<'_>> = cards
                    .iter()
                    .filter(|c| display_category((*c).category()) == cat.as_str())
                    .collect();
                if group.is_empty() {
                    continue;
                }
                any = true;
                if filter.is_none() {
                    ui.text_disabled(cat);
                }
                picked = browse_card_grid(ui, session, &group).or(picked.take());
            }
            if !any {
                ui.text_disabled("no scripts — Browse is empty");
            }
        });
    if let Some(selection) = picked {
        session.select_script_selection(selection);
    }
}

fn overlay_right_strip(ui: &Ui, session: &Session) -> f32 {
    let rail = if session.multibox && !session.wall.grid {
        RAIL_W
    } else {
        0.0
    };
    scale_px(ui, PANEL_WIDTH + rail)
}

fn overlay_spawn_pos(ui: &Ui, session: &Session, win: [f32; 2]) -> [f32; 2] {
    let vp = ui.main_viewport();
    overlay_first_pos(vp.pos(), vp.size(), overlay_right_strip(ui, session), win)
}

/// Browse picker: script cards with category tabs. Non-modal window
/// (same pattern as Nav config). FirstUseEver sits over the game pane so
/// it does not spawn as a Game tab; the operator can still dock it later.
fn browse_window(ui: &Ui, session: &mut Session) {
    if session.script_browse_open {
        let mut open = true;
        let size = scale_size(ui, [SCRIPTS_FIRST_W, SCRIPTS_FIRST_H]);
        let pos = overlay_spawn_pos(ui, session, size);
        ui.window(BROWSE_WINDOW_TITLE)
            .opened(&mut open)
            .flags(WindowFlags::NO_COLLAPSE | WindowFlags::NO_SCROLLBAR)
            .position(pos, Condition::FirstUseEver)
            .size(size, Condition::FirstUseEver)
            .build(|| browse_window_body(ui, session));
        session.script_browse_open = open;
    }
    // A newly opened picker must appear above the Browse window that requested it.
    file_dialog_windows(ui, session);
}

fn persist_dialog_cwd(session: &mut Session, mode: DialogMode) {
    match mode {
        DialogMode::File => {
            session.ui.script_load_last_dir = Some(session.script_load_dir.clone());
        }
        DialogMode::Folder => {
            session.ui.script_catalog_last_dir = Some(session.rs2b0t_catalog_dir.clone());
        }
    }
    if session.persist_ui {
        crate::ui_state::save(&session.ui);
    }
}

fn file_dialog_windows(ui: &Ui, session: &mut Session) {
    let size = scale_size(ui, [FILE_DIALOG_FIRST_W, FILE_DIALOG_FIRST_H]);
    let pos = overlay_spawn_pos(ui, session, size);
    if session.script_load_open {
        let mut open = true;
        ui.window("Load script")
            .opened(&mut open)
            .flags(WindowFlags::NO_COLLAPSE)
            .position(pos, Condition::FirstUseEver)
            .size(size, Condition::FirstUseEver)
            .build(|| file_dialog_body(ui, session, DialogMode::File));
        session.script_load_open &= open;
    }
    if session.rs2b0t_catalog_open {
        let mut open = true;
        ui.window("Import rs2b0t catalog")
            .opened(&mut open)
            .flags(WindowFlags::NO_COLLAPSE)
            .position(pos, Condition::FirstUseEver)
            .size(size, Condition::FirstUseEver)
            .build(|| file_dialog_body(ui, session, DialogMode::Folder));
        session.rs2b0t_catalog_open &= open;
    }
}

fn file_dialog_body(ui: &Ui, session: &mut Session, mode: DialogMode) {
    match mode {
        DialogMode::File => ui.text_wrapped("Choose a JavaScript or TypeScript file to load as a script."),
        DialogMode::Folder if session.rs2b0t_catalog_defer_ok => ui.text_wrapped(
            "No script catalog is configured. Choose your rs2b0t folder to add its scripts to Browse, or choose Not now.",
        ),
        DialogMode::Folder => ui.text_wrapped(
            "Choose the rs2b0t folder whose scripts you want to add to Browse.",
        ),
    }
    ui.separator();
    let cwd = match mode {
        DialogMode::File => session.script_load_dir.clone(),
        DialogMode::Folder => session.rs2b0t_catalog_dir.clone(),
    };
    let avail = ui.content_region_avail();
    let side_w = scale_px(ui, 150.0)
        .min(avail[0] * 0.28)
        .max(scale_px(ui, 120.0));
    let home = script::bot_home();
    ui.child_window("##fd-side")
        .size([
            side_w,
            (avail[1] - scale_px(ui, 8.0)).max(scale_px(ui, 80.0)),
        ])
        .border(true)
        .build(ui, || {
            for place in script_picker::sidebar_places(&home) {
                let label = format!("{}  {}", place.glyph, place.label);
                if ui.button_with_size(&label, [-1.0, 0.0]) {
                    match mode {
                        DialogMode::File => session.script_load_dir = place.path.clone(),
                        DialogMode::Folder => session.rs2b0t_catalog_dir = place.path.clone(),
                    }
                    persist_dialog_cwd(session, mode);
                }
            }
        });
    ui.same_line();
    ui.child_window("##fd-main")
        .size([
            (avail[0] - side_w - scale_px(ui, 8.0)).max(scale_px(ui, 80.0)),
            (avail[1] - scale_px(ui, 8.0)).max(scale_px(ui, 80.0)),
        ])
        .build(ui, || {
            let crumbs = script_picker::breadcrumb_prefixes(&cwd);
            for (i, (label, prefix)) in crumbs.iter().enumerate() {
                if i > 0 {
                    ui.same_line();
                    ui.text_disabled(GLYPH_CHEVRON);
                    ui.same_line();
                }
                let id = format!("{label}##crumb{i}");
                if ui.small_button(&id) {
                    match mode {
                        DialogMode::File => session.script_load_dir = prefix.clone(),
                        DialogMode::Folder => session.rs2b0t_catalog_dir = prefix.clone(),
                    }
                    persist_dialog_cwd(session, mode);
                }
            }
            ui.input_text("##fd-search", &mut session.script_dialog_search)
                .hint("Search")
                .build();
            if ui.small_button("Name") {
                if session.script_dialog_sort == script_picker::DialogSort::Name {
                    session.script_dialog_sort_desc = !session.script_dialog_sort_desc;
                } else {
                    session.script_dialog_sort = script_picker::DialogSort::Name;
                    session.script_dialog_sort_desc = false;
                }
            }
            ui.same_line();
            if ui.small_button("Date") {
                if session.script_dialog_sort == script_picker::DialogSort::Date {
                    session.script_dialog_sort_desc = !session.script_dialog_sort_desc;
                } else {
                    session.script_dialog_sort = script_picker::DialogSort::Date;
                    session.script_dialog_sort_desc = false;
                }
            }
            let mut rows = script_picker::dialog_rows(&cwd, mode, &session.script_dialog_search);
            script_picker::sort_dialog_rows(
                &mut rows,
                session.script_dialog_sort,
                session.script_dialog_sort_desc,
            );
            if session.script_load_sel >= rows.len() {
                session.script_load_sel = rows.len().saturating_sub(1);
            }
            let table_h =
                (ui.content_region_avail()[1] - scale_px(ui, 48.0)).max(scale_px(ui, 80.0));
            if let Some(_t) = ui.begin_table_with_sizing(
                "##fd-table",
                2,
                TableFlags::ROW_BG
                    | TableFlags::BORDERS_INNER_H
                    | TableFlags::SCROLL_Y
                    | TableFlags::RESIZABLE
                    | TableFlags::NO_SAVED_SETTINGS,
                [0.0, table_h],
                0.0,
            ) {
                ui.table_setup_column("Name", TableColumnFlags::NONE, None, None);
                ui.table_setup_column("Date", TableColumnFlags::NONE, None, None);
                ui.table_headers_row();
                for (i, row) in rows.iter().enumerate() {
                    ui.table_next_row();
                    ui.table_next_column();
                    let glyph = if row.is_dir { GLYPH_FOLDER } else { GLYPH_FILE };
                    let label = format!("{glyph}  {}##fdrow{i}", row.name);
                    let selected = i == session.script_load_sel;
                    if ui
                        .selectable_config(&label)
                        .selected(selected)
                        .span_all_columns(true)
                        .build()
                    {
                        session.script_load_sel = i;
                    }
                    if ui.is_item_hovered() && ui.is_mouse_double_clicked(MouseButton::Left) {
                        if row.is_dir {
                            match mode {
                                DialogMode::File => session.script_load_dir.push(&row.name),
                                DialogMode::Folder => session.rs2b0t_catalog_dir.push(&row.name),
                            }
                            persist_dialog_cwd(session, mode);
                        } else if mode == DialogMode::File {
                            session.load_js(&cwd.join(&row.name));
                        }
                    }
                    ui.table_next_column();
                    ui.text_colored(
                        dialog_date_color(selected, i % 2 == 1),
                        format_mtime(row.mtime),
                    );
                }
            }
            ui.spacing();
            let fw = ui.content_region_avail()[0];
            let btn_w = scale_px(ui, 96.0);
            match mode {
                DialogMode::File => {
                    let name = rows
                        .get(session.script_load_sel)
                        .filter(|r| !r.is_dir)
                        .map(|r| r.name.as_str())
                        .unwrap_or("");
                    ui.text_disabled(name);
                    let x = centered_row_x(fw, 2, btn_w, scale_px(ui, BUTTON_GAP));
                    ui.set_cursor_pos_x(ui.cursor_pos_x() + x);
                    if ui.button_with_size("Load", [btn_w, 0.0]) {
                        if let Some(row) = rows.get(session.script_load_sel).filter(|r| !r.is_dir) {
                            session.load_js(&cwd.join(&row.name));
                        }
                    }
                    ui.same_line();
                    if ui.button_with_size("Cancel", [btn_w, 0.0]) {
                        session.script_load_open = false;
                    }
                }
                DialogMode::Folder => {
                    let has_index = script_picker::rs2b0t_root_has_index(&cwd);
                    if has_index {
                        ui.text_colored(ACCENT, "catalog index found");
                    } else {
                        ui.text_disabled("no src/bot/scripts/index.ts here");
                    }
                    let n =
                        1 + usize::from(has_index) + usize::from(session.rs2b0t_catalog_defer_ok);
                    let folder_btn = scale_px(ui, 140.0);
                    let x = centered_row_x(fw, n, folder_btn, scale_px(ui, BUTTON_GAP));
                    ui.set_cursor_pos_x(ui.cursor_pos_x() + x);
                    if has_index {
                        if ui.button_with_size("Use this folder", [folder_btn, 0.0]) {
                            if let Err(e) = session.import_rs2b0t_catalog(&cwd) {
                                session.error = Some(e);
                            } else {
                                session.error = None;
                            }
                        }
                        ui.same_line();
                    }
                    if session.rs2b0t_catalog_defer_ok {
                        if ui.button_with_size("Not now", [folder_btn, 0.0]) {
                            session.defer_rs2b0t_catalog();
                        }
                        ui.same_line();
                    }
                    if ui.button_with_size("Cancel", [folder_btn, 0.0]) {
                        session.rs2b0t_catalog_open = false;
                    }
                }
            }
        });
}

/// Nav config window: Routing, Display, Path paint (only while the path
/// is shown), and Debug groups. Preference edits write `session.ui.nav`
/// through the checked shared-preferences writer; the pause toggle also
/// updates the host policy immediately. FirstUseEver docks as a 274bot
/// panel tab; undock to float.
fn nav_settings_window(ui: &Ui, session: &mut Session, panel_dock: Option<Id>) {
    if !session.nav_settings_open {
        return;
    }
    let mut open = true;
    let panel_class = panel_window_class();
    ui.set_next_window_class(&panel_class);
    if let Some(id) = panel_dock {
        ui.set_next_window_dock_id_with_cond(id, Condition::FirstUseEver);
    }
    ui.window("Nav config")
        .opened(&mut open)
        .flags(WindowFlags::NO_COLLAPSE)
        .size(scale_size(ui, [360.0, 480.0]), Condition::FirstUseEver)
        .build(|| {
            let mut nav = session.ui.nav.clone();
            let previous_pause_script_on_manual_walk_abort = nav.pause_script_on_manual_walk_abort;
            let mut changed = false;
            ui.text_colored(ACCENT, "Routing");
            if ui.checkbox("allow teleports", &mut nav.allow_teleports) {
                changed = true;
            }
            if ui.checkbox("allow wilderness", &mut nav.allow_wilderness) {
                changed = true;
            }
            if ui.checkbox("allow bank fetch", &mut nav.allow_bank_fetch) {
                changed = true;
            }

            if ui.checkbox(
                "Pause script on manual movement",
                &mut nav.pause_script_on_manual_walk_abort,
            ) {
                changed = true;
            }

            ui.spacing();
            ui.text_colored(ACCENT, "Display");
            if ui.checkbox("show nav path", &mut nav.show_nav_path) {
                changed = true;
            }

            if nav.show_nav_path {
                ui.spacing();
                ui.text_colored(ACCENT, "Path paint");
                changed |= nav_color_field(ui, "path", &mut nav.color_path);
                changed |= nav_color_field(ui, "transport", &mut nav.color_transport);
                changed |= nav_color_field(ui, "click", &mut nav.color_click);
                changed |= nav_color_field(ui, "text", &mut nav.color_text);
                if ui.checkbox("hop labels", &mut nav.hop_labels) {
                    changed = true;
                }
                if ui.input_int("hop label px", &mut nav.hop_label_px) {
                    nav.hop_label_px = clamp_hop_label_px(nav.hop_label_px);
                    changed = true;
                }
                if ui.checkbox("camera follow", &mut nav.camera_follow) {
                    changed = true;
                }
            }

            ui.spacing();
            ui.text_colored(ACCENT, "Debug");
            if ui.checkbox("collision fill", &mut nav.collision_fill) {
                changed = true;
            }
            changed |= nav_color_field(ui, "collision", &mut nav.color_collision);
            if ui.checkbox("NSEW labels", &mut nav.nsew_labels) {
                changed = true;
            }
            if ui.checkbox("client trail", &mut nav.client_trail) {
                changed = true;
            }
            changed |= nav_color_field(ui, "client", &mut nav.color_client);
            changed |= nav_color_field(ui, "client run-alt", &mut nav.color_client_run_alt);
            if ui.checkbox("component flood", &mut nav.component_flood) {
                changed = true;
            }

            ui.spacing();
            ui.text_colored(ACCENT, "WalkTo map");
            let mut ask = session.map_bake.choice() == frontend_core::MapBakeChoice::Ask;
            if ui.checkbox("ask before baking terrain", &mut ask) {
                session.set_map_bake_choice(if ask {
                    frontend_core::MapBakeChoice::Ask
                } else {
                    frontend_core::MapBakeChoice::Always
                });
            }

            if changed {
                session.ui.nav = nav;
                if previous_pause_script_on_manual_walk_abort
                    != session.ui.nav.pause_script_on_manual_walk_abort
                {
                    session.core.set_pause_script_on_manual_walk_abort(
                        session.ui.nav.pause_script_on_manual_walk_abort,
                    );
                }
                match crate::ui_state::save_checked(&session.ui) {
                    Ok(()) => {
                        if session
                            .error
                            .as_deref()
                            .is_some_and(|error| error.starts_with("Nav config: "))
                        {
                            session.error = None;
                        }
                    }
                    Err(error) => {
                        session.error = Some(format!("Nav config: {error}"));
                    }
                }
            }
        });
    session.nav_settings_open = open;
}

/// Nav paint colour: imgui color-edit (preview square opens the picker)
/// plus hex display. The stored string stays `#RRGGBB`.
fn nav_color_field(ui: &Ui, label: &str, color: &mut String) -> bool {
    let [r, g, b] = crate::nav_settings::parse_html_color(color, [0xff, 0xff, 0xff]);
    let mut rgb = [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0];
    ui.set_next_item_width(scale_px(ui, 148.0));
    let edited = ui
        .color_edit3_config(format!("{label}##nav-color-{label}"), &mut rgb)
        .display_mode(ColorDisplayMode::Hex)
        .build();
    if edited {
        *color = format!(
            "#{:02X}{:02X}{:02X}",
            (rgb[0] * 255.0).round().clamp(0.0, 255.0) as u8,
            (rgb[1] * 255.0).round().clamp(0.0, 255.0) as u8,
            (rgb[2] * 255.0).round().clamp(0.0, 255.0) as u8
        );
    }
    edited
}

/// parameters: read-only preview of the selected card's merged settings rows.
/// Typed editors live in [`script_prefs_window`]; optional via
/// `PanelUiState.show_parameters_rail`.
fn parameters_section(ui: &Ui, session: &mut Session) {
    if !section_open(ui, session, "parameters") {
        return;
    }
    if let Some(placeholder) = parameters_rail_placeholder(session) {
        ui.text_disabled(placeholder);
        return;
    }
    match &session.script_sel {
        Some(script::ScriptSel::Loaded(source, name)) => {
            if let Some(card) = session.scripts.js.get(*source, name) {
                let bag = session.merged_settings_bag(*source, name, &card.settings_schema);
                for (label, value) in script::parameter_rows(&card.settings_schema, &bag) {
                    kv_row(ui, &label, &value);
                }
            }
        }
        Some(script::ScriptSel::Compiled(id)) => {
            if let frontend_core::scripts::SchemaView::Ready { fields, .. } = session
                .scripts
                .compiled_schema(&session.core, session.focused_name().as_deref(), *id)
            {
                match session.scripts.compiled_edit_bag(
                    &session.core,
                    session.focused_name().as_deref(),
                    *id,
                ) {
                    Ok(bag) => {
                        for (label, value) in script::parameter_rows(fields, &bag) {
                            kv_row(ui, &label, &value);
                        }
                    }
                    Err(error) => ui.text_disabled(error),
                }
            }
        }
        None => {}
    }
}

/// All parameter controls share the same account-scoped persistence path.
fn persist_profile_setting(
    session: &mut Session,
    selection: &script::ScriptSel,
    id: &str,
    value: serde_json::Value,
) {
    match selection {
        script::ScriptSel::Compiled(card) => {
            let Some(profile) = session.focused_name() else {
                session.error = Some("parameters: select a profile first".into());
                return;
            };
            if let Err(error) =
                session
                    .scripts
                    .set_compiled_setting(&mut session.core, &profile, *card, id, value)
            {
                session.error = Some(error);
            }
        }
        script::ScriptSel::Loaded(source, lookup) => {
            let Some(card) = session.scripts.js.get(*source, lookup).cloned() else {
                return;
            };
            if let Some(profile) = session.focused_name() {
                session.set_profile_setting(&profile, *source, &card.name, &card.path, id, value);
            } else {
                session
                    .scripts
                    .legacy
                    .set_value(*source, &card.name, id, value);
                let _ = session.scripts.legacy.save();
            }
        }
    }
}

fn script_parameter_editors(ui: &Ui, session: &mut Session) {
    let Some(selection) = session.script_sel.clone() else {
        ui.text_wrapped("select a script with a parameter schema");
        return;
    };
    let (schema, mut bag): (std::borrow::Cow<'_, [script::SettingDef]>, _) = match &selection {
        script::ScriptSel::Loaded(source, name) => {
            let Some(card) = session.scripts.js.get(*source, name).cloned() else {
                ui.text_disabled("(parameters unavailable)");
                return;
            };
            let bag = if let Some(profile) = session.focused_name() {
                session.merged_profile_bag(
                    &profile,
                    *source,
                    &card.name,
                    &card.path,
                    &card.settings_schema,
                )
            } else {
                session.merged_settings_bag(*source, name, &card.settings_schema)
            };
            (std::borrow::Cow::Owned(card.settings_schema), bag)
        }
        script::ScriptSel::Compiled(id) => {
            let profile = session.focused_name();
            let fields =
                match session
                    .scripts
                    .compiled_schema(&session.core, profile.as_deref(), *id)
                {
                    frontend_core::scripts::SchemaView::Ready { fields, .. } => fields,
                    frontend_core::scripts::SchemaView::Unavailable(reason) => {
                        ui.text_disabled(reason);
                        return;
                    }
                };
            let bag =
                match session
                    .scripts
                    .compiled_edit_bag(&session.core, profile.as_deref(), *id)
                {
                    Ok(bag) => bag,
                    Err(error) => {
                        ui.text_disabled(error);
                        return;
                    }
                };
            (std::borrow::Cow::Borrowed(fields), bag)
        }
    };
    if schema.is_empty() {
        ui.text_disabled("(no parameters)");
        return;
    }
    let game_data = session.selected_game_data();
    let game_data_ref = game_data.as_deref();
    let mut last_group: Option<String> = None;
    for def in schema.iter() {
        if !script::setting_visible(def.show_if.as_deref(), &bag) {
            continue;
        }
        if def.group.as_deref().map(String::from) != last_group {
            last_group = def.group.clone();
            if let Some(ref g) = last_group {
                ui.separator();
                ui.text(g);
            }
        }
        let label = def.label.as_deref().unwrap_or(&def.id).to_string();
        let resolved =
            script::resolve_setting_options_with_labels(def, &session.loadouts, game_data_ref);
        match def.ty.as_str() {
            "boolean" => {
                let mut value = bag
                    .get(&def.id)
                    .and_then(|v| v.as_bool())
                    .unwrap_or_else(|| def.default.as_deref() == Some("true"));
                if ui.checkbox(&label, &mut value) {
                    persist_profile_setting(session, &selection, &def.id, serde_json::json!(value));
                    bag.insert(def.id.clone(), serde_json::json!(value));
                }
            }
            "number" => {
                let mut value = bag
                    .get(&def.id)
                    .and_then(|v| v.as_f64())
                    .or_else(|| def.default.as_deref().and_then(|s| s.parse::<f64>().ok()))
                    .unwrap_or(0.0) as i32;
                if ui.input_int(&label, &mut value) {
                    persist_profile_setting(session, &selection, &def.id, serde_json::json!(value));
                    bag.insert(def.id.clone(), serde_json::json!(value));
                }
            }
            "string" if !resolved.is_empty() => {
                ui.text(&label);
                let opts = &resolved.values;
                let current = bag
                    .get(&def.id)
                    .and_then(|v| v.as_str())
                    .or(def.default.as_deref())
                    .unwrap_or("")
                    .to_string();
                ui.set_next_item_width(-1.0);
                let combo_opts = ComboBoxOptions::new().preview_mode(ComboBoxPreviewMode::Preview);
                let preview = resolved.label_for(&current);
                if let Some(_popup) = ui.begin_combo_with_flags(
                    format!("##param-{id}", id = def.id),
                    preview,
                    combo_opts,
                ) {
                    for opt in opts {
                        let selected = opt == &current;
                        let shown = resolved.label_for(opt);
                        if ui.selectable_config(shown).selected(selected).build() {
                            persist_profile_setting(
                                session,
                                &selection,
                                &def.id,
                                serde_json::json!(opt),
                            );
                            bag.insert(def.id.clone(), serde_json::json!(opt));
                        }
                    }
                }
            }
            "string[]" if !resolved.is_empty() => {
                ui.text(&label);
                let opts = &resolved.values;
                let mut selected: Vec<String> = bag
                    .get(&def.id)
                    .and_then(|v| v.as_array())
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|x| x.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                let mut changed = false;
                for opt in opts {
                    let mut on = selected.iter().any(|s| s == opt);
                    let shown = resolved.label_for(opt);
                    if ui.checkbox(format!("{shown}##param-{id}-{opt}", id = def.id), &mut on) {
                        changed = true;
                        if on {
                            if !selected.iter().any(|s| s == opt) {
                                selected.push(opt.clone());
                            }
                        } else {
                            selected.retain(|s| s != opt);
                        }
                    }
                }
                if changed {
                    let coerced =
                        script::coerce_setting_value(&def.ty, &serde_json::json!(selected));
                    persist_profile_setting(session, &selection, &def.id, coerced.clone());
                    bag.insert(def.id.clone(), coerced);
                }
            }
            "string" | "tile" | "list" | "string[]" => {
                let mut text = bag
                    .get(&def.id)
                    .map(script::format_setting_value)
                    .or(def.default.as_deref().map(String::from))
                    .unwrap_or_default();
                if ui.input_text(&label, &mut text).build() {
                    let coerced = script::coerce_setting_value(&def.ty, &serde_json::json!(text));
                    persist_profile_setting(session, &selection, &def.id, coerced.clone());
                    bag.insert(def.id.clone(), coerced);
                }
            }
            _ => {
                ui.text_disabled(format!("{label} (unsupported type {})", def.ty));
            }
        }
        if let Some(help) = def.help.as_deref() {
            ui.set_item_tooltip(help);
        }
    }
    if session.focused_name().is_some() {
        apply_to_all_section(ui, session);
    }
}

/// Apply to all: copy the focused profile's parameters for this card to
/// every wall member assigned the same card, after a confirmation naming
/// the frozen scope. The last result stays below it.
fn apply_to_all_section(ui: &Ui, session: &mut Session) {
    ui.separator();
    if let Some(scope) = session.scripts.prepared_settings_sync() {
        ui.text_wrapped(scope.prompt());
        if ui.button("Apply") {
            session.apply_settings_sync();
        }
        ui.same_line();
        if ui.button("Cancel##apply-to-all") {
            session.cancel_settings_sync();
        }
    } else {
        if ui.button("Apply to same-card members…") {
            session.prepare_settings_sync();
        }
        ui.set_item_tooltip(
            "copy these parameters to every wall member assigned this card; members on another card are skipped",
        );
    }
    if let Some(report) = session.scripts.last_settings_sync() {
        ui.text_wrapped(report.summary());
    }
}

/// Script prefs window: typed parameter editors + optional rail preview toggle.
/// FirstUseEver docks as a 274bot panel tab; undock to float.
fn script_prefs_window(ui: &Ui, session: &mut Session, panel_dock: Option<Id>) {
    if !session.script_prefs_open {
        return;
    }
    let mut open = true;
    let panel_class = panel_window_class();
    ui.set_next_window_class(&panel_class);
    if let Some(id) = panel_dock {
        ui.set_next_window_dock_id_with_cond(id, Condition::FirstUseEver);
    }
    ui.window("Script prefs")
        .opened(&mut open)
        .flags(WindowFlags::NO_COLLAPSE)
        .size(scale_size(ui, [360.0, 480.0]), Condition::FirstUseEver)
        .build(|| {
            if ui.checkbox(
                "Show parameters in rail",
                &mut session.ui.show_parameters_rail,
            ) {
                crate::ui_state::save(&session.ui);
            }
            ui.spacing();
            ui.text_wrapped(
                "Edits are validated and saved before live delivery. Native cards report applied, pending boundary, or restart required; Load scripts receive the complete bag.",
            );
            ui.spacing();
            if edit_parameters_enabled() {
                script_parameter_editors(ui, session);
            } else {
                ui.text_disabled("parameter editors not available");
            }
        });
    session.script_prefs_open = open;
}

/// Why a selected Loaded card is not in the library: its catalog has not been
/// read yet (or could not be), versus a card that is simply missing. Shared by
/// the Script prefs hint and the parameters rail so neither calls a card that
/// was never loaded "no parameters".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MissingCard {
    CatalogUnavailable,
    CardUnavailable,
}

impl MissingCard {
    fn of(session: &Session, source: script::ScriptSource) -> Self {
        if source == script::ScriptSource::Catalog && !session.scripts.catalog_filled() {
            Self::CatalogUnavailable
        } else {
            Self::CardUnavailable
        }
    }

    fn prefs_hint(self) -> &'static str {
        match self {
            Self::CatalogUnavailable => "script catalog unavailable",
            Self::CardUnavailable => "selected script parameters unavailable",
        }
    }

    fn rail_text(self) -> &'static str {
        match self {
            Self::CatalogUnavailable => "(script catalog unavailable)",
            Self::CardUnavailable => "(parameters unavailable)",
        }
    }
}

/// The parameters rail's placeholder when there are no rows to list, else
/// `None` (the rail then lists the selected card's merged settings).
fn parameters_rail_placeholder(session: &Session) -> Option<&'static str> {
    match &session.script_sel {
        None => Some("(no script selected)"),
        Some(script::ScriptSel::Loaded(source, name)) => {
            match session.scripts.js.get(*source, name) {
                None => Some(MissingCard::of(session, *source).rail_text()),
                Some(card) if card.settings_schema.is_empty() => Some("(no parameters)"),
                Some(_) => None,
            }
        }
        Some(script::ScriptSel::Compiled(id)) => match session.scripts.compiled_schema(
            &session.core,
            session.focused_name().as_deref(),
            *id,
        ) {
            frontend_core::scripts::SchemaView::Unavailable(_) => Some("(parameters unavailable)"),
            frontend_core::scripts::SchemaView::Ready { fields: [], .. } => Some("(no parameters)"),
            frontend_core::scripts::SchemaView::Ready { .. } => None,
        },
    }
}

/// Tooltip when Script prefs is disabled (empty schema, unavailable card, or no selection).
fn script_prefs_disabled_hint(session: &Session) -> Option<&'static str> {
    match &session.script_sel {
        None => Some("select a script first"),
        Some(script::ScriptSel::Compiled(id)) => match session.scripts.compiled_schema(
            &session.core,
            session.focused_name().as_deref(),
            *id,
        ) {
            frontend_core::scripts::SchemaView::Unavailable(reason) => Some(reason),
            frontend_core::scripts::SchemaView::Ready { fields: [], .. } => {
                Some("selected script has no parameters")
            }
            frontend_core::scripts::SchemaView::Ready { .. } => None,
        },
        Some(script::ScriptSel::Loaded(source, name)) => {
            let Some(card) = session.scripts.js.get(*source, name) else {
                return Some(MissingCard::of(session, *source).prefs_hint());
            };
            if card.settings_schema.is_empty() {
                Some("selected script has no parameters")
            } else {
                None
            }
        }
    }
}

/// Under WalkTo, above profile: two rows — General/Nav then Loadouts/Script prefs.
fn slot_config_row(ui: &Ui, session: &mut Session) {
    let avail = ui.content_region_avail()[0];
    for row in [CONFIG_HOST_ROW, CONFIG_SCRIPT_ROW] {
        let cells = scaled_button_cells_min(ui, avail, row.len(), CONFIG_MIN);
        for (i, &(w, same_line)) in cells.iter().enumerate() {
            if same_line {
                gap_line(ui);
            }
            match row[i] {
                "General config" => {
                    if ui.button_with_size("General config", [w, 0.0]) {
                        session.global_settings_open = true;
                    }
                    ui.set_item_tooltip("slot render + global cadence / capture");
                }
                "Nav config" => {
                    if ui.button_with_size("Nav config", [w, 0.0]) {
                        session.nav_settings_open = true;
                    }
                    ui.set_item_tooltip("nav debug paints and labels");
                }
                "Loadouts" => {
                    if ui.button_with_size("Loadouts", [w, 0.0]) {
                        session.loadouts_open = true;
                        session.loadouts_sel = 0;
                        crate::loadouts::sync_draft(session);
                    }
                    ui.set_item_tooltip("equipment and inventory presets");
                }
                "Script prefs" => {
                    if let Some(hint) = script_prefs_disabled_hint(session) {
                        mock_button(ui, "Script prefs", hint, [w, 0.0]);
                    } else {
                        if ui.button_with_size("Script prefs", [w, 0.0]) {
                            session.script_prefs_open = true;
                        }
                        ui.set_item_tooltip("script parameter editors");
                    }
                }
                _ => {}
            }
        }
    }
}

/// Parameter editing is enabled for compat cards with a settings schema.
fn edit_parameters_enabled() -> bool {
    true
}

/// status: the selected bot's rows from the shared detail projection
/// (state, player, world, tile, meaningful walk/queue/modal values, welcome,
/// random, the last login error while retrying, the newest operation, mem),
/// wrapped.
fn status_section(ui: &Ui, session: &mut Session) {
    if !section_open(ui, session, "status") {
        return;
    }
    let walk = session.walk_status_text();
    let mem = Session::mem_notice_text(session.focused_lowmem(), session.focused_memory_notice());
    let Some(d) = session.core.fleet_view().detail() else {
        kv_row(ui, "state", "no bot selected");
        status_kv_row(ui, "walk", &walk);
        kv_row(ui, "mem", &mem);
        return;
    };
    status_detail_rows(ui, d, &walk, &mem);
}

fn status_detail_rows(ui: &Ui, d: &frontend_core::SlotDetail, walk: &str, mem: &str) {
    kv_row(ui, "state", &d.state);
    kv_row(
        ui,
        "player",
        if d.player.is_empty() { "?" } else { &d.player },
    );
    if let Some(world) = d.row.world {
        kv_row(ui, "world", &format!("w{world}"));
    }
    kv_row(ui, "tile", &format!("{} {}", d.tile.0, d.tile.1));
    status_kv_row(ui, "walk", walk);
    let queue = d
        .row
        .queue
        .map_or_else(|| "—".to_string(), |q| q.to_string());
    status_kv_row(ui, "queue", &queue);
    if matches!(d.row.phase, Phase::Ready) {
        status_kv_row(ui, "modals", &d.modal.to_string());
    }
    if let Some(welcome) = d.welcome.as_deref() {
        kv_row(ui, "welcome", welcome);
    }
    if let Some(random) = d.random.as_deref() {
        kv_row(ui, "random", random);
    }
    if !d.row.phase.is_error() {
        if let Some(error) = d.row.error.as_deref() {
            kv_row(ui, "last error", error);
        }
    }
    if let Some(op) = d.row.last_op.as_ref() {
        kv_row(ui, "operation", &op.to_string());
    }
    kv_row(ui, "mem", mem);
}

fn status_kv_row(ui: &Ui, label: &str, value: &str) {
    if status_value_visible(label, value) {
        kv_row(ui, label, value);
    }
}

fn resource_section(ui: &Ui, session: &mut Session) {
    if !section_open(ui, session, "resource") {
        return;
    }
    draw_resource_rows(ui, session.core.resources(), true);
}

/// log: the shared structured log (see [`crate::log_pane`]).
fn log_section(ui: &Ui, session: &mut Session, last: bool) {
    if !section_open(ui, session, "log") {
        return;
    }
    if session.ui.log_detached {
        ui.text_disabled("log is floating in a separate window");
        if ui.button("Attach log") {
            session.ui.log_detached = false;
            crate::ui_state::save(&session.ui);
        }
    } else {
        crate::log_pane::log_body(ui, session, last);
    }
}

/// Draw the shared log in a separate in-app window.
fn floating_log_window(ui: &Ui, session: &mut Session) {
    if !session.ui.log_detached {
        return;
    }
    let viewport = ui.main_viewport();
    let work_pos = viewport.work_pos();
    let work_size = viewport.work_size();
    let scale = ui_scale(ui);
    let size = scale_size(
        ui,
        [
            PANEL_WIDTH,
            (work_size[1] / scale - 80.0).clamp(240.0, 560.0),
        ],
    );
    let pos = [
        work_pos[0] + ((work_size[0] - size[0]) * 0.5).max(0.0),
        work_pos[1] + ((work_size[1] - size[1]) * 0.5).max(0.0),
    ];
    let mut open = true;
    ui.window("Log")
        .opened(&mut open)
        .flags(WindowFlags::NO_COLLAPSE | WindowFlags::NO_DOCKING)
        .position(pos, Condition::FirstUseEver)
        .size(size, Condition::FirstUseEver)
        .size_constraints(
            scale_size(ui, [280.0, 180.0]),
            [f32::MAX, scale_px(ui, 720.0)],
        )
        .build(|| crate::log_pane::log_body(ui, session, true));
    if !open {
        session.ui.log_detached = false;
        crate::ui_state::save(&session.ui);
    }
}

/// Selected picker button: amber fill, dark text (illuminated invert).
fn inverted_button(ui: &Ui, label: &str, selected: bool, size: [f32; 2]) -> bool {
    let _b = selected.then(|| ui.push_style_color(StyleColor::Button, ACCENT));
    let _h = selected.then(|| ui.push_style_color(StyleColor::ButtonHovered, ACCENT_HOVER));
    let _a = selected.then(|| ui.push_style_color(StyleColor::ButtonActive, ACCENT));
    let _t = selected.then(|| ui.push_style_color(StyleColor::Text, BG));
    ui.button_with_size(label, size)
}

const MEM_POPUP: &str = "mem-pick";

/// Sticky highmem/lowmem popup (click-away to close), opened by the mem
/// button the same way Teles opens dests.
fn mem_popup(ui: &Ui, session: &mut Session) {
    ui.popup(MEM_POPUP, || {
        ui.text_disabled("mem");
        let low = session.focused_lowmem();
        if inverted_button(ui, "highmem", !low, [0.0, 0.0]) {
            session.request_focused_lowmem(false);
        }
        gap_line(ui);
        if inverted_button(ui, "lowmem", low, [0.0, 0.0]) {
            session.request_focused_lowmem(true);
        }
        if session
            .focused_memory_notice()
            .is_some_and(|notice| notice.differs())
        {
            ui.text_wrapped("server tabs + sound follow at the next login");
            let notice = session.focused_memory_notice().expect("checked above");
            if notice.relog_pending {
                ui.text_disabled("relog queued…");
            } else if session.mem_relog_armed() || !session.focused_memory_relog_warning() {
                if ui.button_with_size("Relog now", [0.0, 0.0]) {
                    session.request_focused_memory_relog();
                }
            } else if ui.button_with_size(
                "Relog now (interrupts script work — click again)",
                [0.0, 0.0],
            ) {
                session.request_focused_memory_relog();
            }
        }
    });
}

/// none / GPU / CPU row. Click the mem button (current highmem/lowmem) for
/// the sticky picker, like Teles.
fn raster_picker(ui: &Ui, session: &mut Session) {
    let cur = session.focused_raster();
    let avail = ui.content_region_avail()[0];
    let cells = scaled_button_cells(ui, avail, 3);
    if inverted_button(ui, "none", cur == vault::RasterMode::Off, [cells[0].0, 0.0]) {
        session.request_focused_raster(vault::RasterMode::Off);
    }
    if cells[1].1 {
        gap_line(ui);
    }
    if inverted_button(ui, "GPU", cur == vault::RasterMode::Gpu, [cells[1].0, 0.0]) {
        session.request_focused_raster(vault::RasterMode::Gpu);
    }
    if cells[2].1 {
        gap_line(ui);
    }
    if inverted_button(ui, "CPU", cur == vault::RasterMode::Cpu, [cells[2].0, 0.0]) {
        session.request_focused_raster(vault::RasterMode::Cpu);
    }
    let mem = if session.focused_lowmem() {
        "lowmem"
    } else {
        "highmem"
    };
    let w = ui.content_region_avail()[0];
    if ui.button_with_size(mem, [w, 0.0]) {
        ui.open_popup(MEM_POPUP);
    }
    ui.set_item_tooltip(
        "Game pane highmem / lowmem — the live client flips at once, server tabs + sound follow at the next login (Relog now in this picker)",
    );
    mem_popup(ui, session);
}

/// General config collapsible row: orange title, black strip, orange border
/// and expand arrow (not the profile combo chip).
fn config_section(ui: &Ui, session: &mut Session, id: &str, body: impl FnOnce(&Ui, &mut Session)) {
    let closed = session
        .ui
        .config_collapsed
        .get(id)
        .copied()
        .unwrap_or(false);
    let desired = !closed;
    ui.set_next_item_open(desired);
    let accent = session.ui.chrome.accent_rgba();
    let bg_deep = session.ui.chrome.bg_deep_rgba();
    let open = {
        let _border_sz = ui.push_style_var(StyleVar::FrameBorderSize(scale_px(ui, 1.0)));
        let _text = ui.push_style_color(StyleColor::Text, accent);
        let _header = ui.push_style_color(StyleColor::Header, bg_deep);
        let _header_h = ui.push_style_color(StyleColor::HeaderHovered, bg_deep);
        let _header_a = ui.push_style_color(StyleColor::HeaderActive, bg_deep);
        let _border = ui.push_style_color(StyleColor::Border, accent);
        ui.collapsing_header(id, TreeNodeFlags::FRAME_PADDING)
    };
    if open != desired {
        session.ui.config_collapsed.insert(id.to_string(), !open);
        crate::ui_state::save(&session.ui);
    }
    if open {
        body(ui, session);
    }
}

fn global_capture_section(ui: &Ui, session: &mut Session) {
    let on = session.focus.lock().unwrap().capture;
    let mut cur = on;
    if ui.checkbox("capture input", &mut cur) {
        session.set_capture(cur);
    }
    ui.text_wrapped(if on {
        "the focused grid/Game member; at most one keyboard. Does not raise fps."
    } else {
        "watch-only; no input work"
    });
}

fn panel_heading_toggles(ui: &Ui, session: &mut Session) {
    ui.text_colored(session.ui.chrome.accent_rgba(), "Panel");
    for id in crate::ui_state::PANEL_SECTION_IDS {
        let mut visible = crate::ui_state::panel_section_visible(&session.ui, id);
        if ui.checkbox(*id, &mut visible) {
            crate::ui_state::set_panel_section_visible(&mut session.ui, id, visible);
            crate::ui_state::save(&session.ui);
        }
    }
    ui.spacing();
    let mut chrome = session.ui.chrome.clone();
    let mut changed = false;
    changed |= chrome_color_field(ui, "accent", &mut chrome.accent);
    changed |= chrome_color_field(ui, "accent hover", &mut chrome.accent_hover);
    changed |= chrome_color_field(ui, "bg", &mut chrome.bg);
    changed |= chrome_color_field(ui, "bg deep", &mut chrome.bg_deep);
    changed |= chrome_color_field(ui, "text", &mut chrome.text);
    changed |= chrome_color_field(ui, "text dim", &mut chrome.text_dim);
    changed |= chrome_color_field(ui, "frame", &mut chrome.frame);
    changed |= chrome_color_field(ui, "hover fill", &mut chrome.hover_fill);
    changed |= chrome_color_field(ui, "active fill", &mut chrome.active_fill);
    changed |= chrome_color_field(ui, "border", &mut chrome.border);
    changed |= chrome_color_field(ui, "warn", &mut chrome.warn);
    changed |= chrome_color_field(ui, "error", &mut chrome.error);
    changed |= chrome_color_field(ui, "green", &mut chrome.green);
    if changed {
        session.ui.chrome = chrome;
        crate::ui_state::save(&session.ui);
    }
}

fn chrome_color_field(ui: &Ui, label: &str, color: &mut String) -> bool {
    let [r, g, b] = crate::nav_settings::parse_html_color(color, [0xff, 0xb0, 0x00]);
    let mut rgb = [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0];
    ui.set_next_item_width(scale_px(ui, 148.0));
    let edited = ui
        .color_edit3_config(format!("{label}##chrome-color-{label}"), &mut rgb)
        .display_mode(ColorDisplayMode::Hex)
        .build();
    if edited {
        *color = format!(
            "#{:02X}{:02X}{:02X}",
            (rgb[0] * 255.0).round().clamp(0.0, 255.0) as u8,
            (rgb[1] * 255.0).round().clamp(0.0, 255.0) as u8,
            (rgb[2] * 255.0).round().clamp(0.0, 255.0) as u8
        );
    }
    edited
}

fn global_config_section(ui: &Ui, session: &mut Session) {
    ui.text_colored([1.0, 1.0, 1.0, 1.0], "Server:");
    {
        let _color = ui.push_style_color(StyleColor::Text, session.ui.chrome.accent_rgba());
        ui.text_wrapped(session.server_label());
    }
    if session.profile_bound() {
        // `server_label` already carries the active revision (for example
        // "local-289 · revision 289"); don't repeat a second bare 289.
        ui.text_disabled("revision fixed for this session");
    } else {
        let revision_preview = session
            .effective_revision_label()
            .unwrap_or_else(|error| format!("invalid: {error}"));
        ui.text("Revision before session");
        ui.set_next_item_width(-1.0);
        if let Some(_open) = ui.begin_combo("##session_revision", &revision_preview) {
            for revision in [274_u16, 289] {
                let selected = revision_preview == revision.to_string();
                if ui
                    .selectable_config(revision.to_string())
                    .selected(selected)
                    .build()
                {
                    session.error = session.set_server_revision(revision).err();
                }
                if selected {
                    ui.set_item_default_focus();
                }
            }
        }
        ui.text_wrapped(
            "Restart to apply a revision change; explicit CLI/environment selection still wins.",
        );
    }
    ui.spacing();
    ui.text_colored(session.ui.chrome.accent_rgba(), "Session");
    ui.text_colored([1.0, 1.0, 1.0, 1.0], "Slot:");
    ui.same_line();
    let slot = session.focused_name().unwrap_or_else(|| "—".into());
    ui.text_colored(session.ui.chrome.accent_rgba(), slot);
    global_capture_section(ui, session);
    crate::log_pane::session_log_row(ui, session);
    let mut sidecar = session.focus.lock().unwrap().sidecar_50;
    if ui.checkbox("sidecar 50 fps", &mut sidecar) {
        session.set_sidecar_50(sidecar);
    }
    ui.text_wrapped("all rail/grid members; all-or-nothing");
    let current = session.focus.lock().unwrap().only_render_selected;
    let mut only = current;
    if ui.checkbox("only render selected", &mut only) {
        let (next, open_warn) = apply_only_render_selected(current, only);
        session.focus.lock().unwrap().only_render_selected = next;
        if open_warn {
            session.wall.render_all_warn_open = true;
        }
        session.wake_all_slots();
    }
    let live = session.scenario.lock().unwrap().is_some();
    if live {
        let mut full = session.focus.lock().unwrap().live_full_rate;
        if ui.checkbox("full rate (this run)", &mut full) {
            session.set_live_full_rate(full);
        }
        ui.text_wrapped("every drawing slot at 50 fps; not sidecar, not capture, not saved");
    }
    let mut focused_50 = session.focus.lock().unwrap().focused_50;
    if ui.checkbox("focused 50 fps", &mut focused_50) {
        session.set_focused_50(focused_50);
    }
    ui.text_wrapped("Game pane only; the focused client on the rail uses the rail cadence.");
    ui.spacing();
    panel_heading_toggles(ui, session);
}

/// rendering: none/GPU/CPU picker; `set_draw` is applied by the slot
/// threads from the shared focus on every frame.
fn slot_render_section(ui: &Ui, session: &mut Session) {
    ui.text_wrapped("Game pane only. Rail members stay GPU / lowmem at 1 fps (CPU/none as fallback). Click lowmem/highmem for the sticky picker. Switching GPU↔CPU or mem drops + reattaches the renderer — the client stays logged in.");
    raster_picker(ui, session);
}

fn slot_capture_section(ui: &Ui, session: &mut Session) {
    let mut auto_cur = session.cred_settings.auto_login;
    if ui.checkbox("auto-login on title", &mut auto_cur) {
        session.cred_settings.auto_login = auto_cur;
        if let Some(name) = session.chooser_edit.as_deref().filter(|n| !n.is_empty()) {
            let name = name.to_string();
            session.set_auto_login(&name, auto_cur);
        }
        session.note_chooser_edited();
    }
    let mut world_changed = false;
    if let Some(worlds) = session
        .server_profile
        .as_ref()
        .and_then(|p| p.public_worlds())
    {
        let preview = session
            .cred_settings
            .world
            .map_or_else(|| "auto".to_string(), |number| format!("w{number}"));
        ui.text_disabled("world (next slot start)");
        ui.set_next_item_width(-1.0);
        if let Some(_open) = ui.begin_combo("##account-world", &preview) {
            if ui
                .selectable_config("auto")
                .selected(session.cred_settings.world.is_none())
                .build()
            {
                session.cred_settings.world = None;
                world_changed = true;
            }
            for world in &worlds.worlds {
                let label = format!("w{}", world.number);
                if ui
                    .selectable_config(&label)
                    .selected(session.cred_settings.world == Some(world.number))
                    .build()
                {
                    session.cred_settings.world = Some(world.number);
                    world_changed = true;
                }
            }
        }
    }
    if world_changed {
        session.note_chooser_edited();
    }
    ui.text_wrapped("this profile; handshake on spawn unless latched out");
}

/// Sidecar rail window: only while MultiBox is on and Grid is off. Bulk
/// Login all / Logout all, the only-render-selected checkbox (unchecking
/// needs the render-all confirm), one tile per wall member (cap only while
/// only-render-selected, else cap + 1 fps body or renderer-off
/// placeholder), `+ add bot`, and the 1 Hz resource card.
fn rail_window(ui: &Ui, gpu: &mut Gpu, state: &mut PanelState) {
    let mut open = true;
    ui.window(format!(
        "{}-rail###{RAIL_WINDOW}",
        state.session.app_title()
    ))
    .opened(&mut open)
    .flags(WindowFlags::NO_COLLAPSE | WindowFlags::NO_RESIZE)
    .build(|| {
        rail_bulk_row(ui, state);
        rail_tiles(ui, gpu, state);
        add_bot_button(ui, state);
        resource_card(ui, state.session.core.resources());
    });
    if !open {
        state.session.set_multibox(false);
    }
}

/// Apply an "only render selected" checkbox change: re-checking on is
/// immediate; unchecking keeps the safe default and opens the scary
/// confirm instead. Returns `(new_only_render_selected, open_warn)`.
pub fn apply_only_render_selected(current: bool, checked: bool) -> (bool, bool) {
    if checked && !current {
        // user just checked "only render selected" — apply immediately
        return (true, false);
    }
    if !checked && current {
        // user just unchecked — do not apply; open warning
        return (true, true);
    }
    (current, false)
}

/// Sticky bulk row: Login all / Logout all, then the only-render-selected
/// checkbox. Unchecking does not write `false` until the render-all
/// warning is accepted; re-checking on is immediate (slot threads apply
/// `set_draw` from it every frame).
fn rail_bulk_row(ui: &Ui, state: &mut PanelState) {
    let avail = ui.content_region_avail()[0];
    let (w, stack) = scaled_button_row_layout(ui, avail, 2);
    if ui.button_with_size("Login all", [w, 0.0]) {
        state.session.login_all();
    }
    if !stack {
        ui.same_line();
    }
    if ui.button_with_size("Logout all", [w, 0.0]) {
        state.session.logout_all();
    }
    let (w, stack) = scaled_button_row_layout(ui, ui.content_region_avail()[0], 2);
    if ui.button_with_size("Start all", [w, 0.0]) {
        state.session.script_start_all();
    }
    if !stack {
        ui.same_line();
    }
    if ui.button_with_size("Stop all", [w, 0.0]) {
        state.session.script_stop_all();
    }
    let current = state.session.focus.lock().unwrap().only_render_selected;
    let mut only = current;
    if ui.checkbox("only render selected", &mut only) {
        let (next, open_warn) = apply_only_render_selected(current, only);
        state.session.focus.lock().unwrap().only_render_selected = next;
        if open_warn {
            state.session.wall.render_all_warn_open = true;
        }
        // The policy flips every wall member's draw state; kick the slots
        // so parked threads apply the change within a frame.
        state.session.wake_all_slots();
    }
}

/// Fitted dialog width: wrap + auto-resize so first open is not a sliver.
const DIALOG_W: f32 = 400.0;

/// Popup body text wrapped at the fitted dialog width's content edge.
/// `Ui::text_wrapped` always wraps at the window edge; in an auto-sizing
/// popup that edge is the popup's own content width, so the popup
/// collapses to a sliver. Fixed-width `DIALOG_W` windows keep
/// `text_wrapped`: their edge is fixed and accounts for a scrollbar.
fn popup_text(ui: &Ui, text: &str) {
    let edge = scale_px(ui, DIALOG_W) - ui.clone_style().window_padding()[0];
    let _wrap = ui.push_text_wrap_pos(edge);
    ui.text(text);
}

const VAULT_RESET_POPUP: &str = "Reset vault?";
const PROFILE_DELETE_POPUP: &str = "Delete profile?";
/// Unsaved-changes prompt when an explicit edit-target switch lands on a
/// dirty form: Discard drops the draft, Keep editing stays on the target.
const PROFILE_EDIT_SWITCH_POPUP: &str = "Unsaved profile edits?";

/// Teles-style confirm: click-out / Escape / any of these keys / Cancel
/// dismisses. Only I understand then the confirm button commits.
fn confirm_dismiss_key(ui: &Ui) -> bool {
    use Key::*;
    [
        Escape, Enter, Tab, Backspace, Delete, Space, LeftArrow, RightArrow, UpArrow, DownArrow,
    ]
    .into_iter()
    .any(|k| ui.is_key_pressed(k))
}

/// Returns true when the operator checked I understand and pressed confirm.
fn scary_confirm_popup(
    ui: &Ui,
    id: &str,
    body: &str,
    confirm: &str,
    understood: &mut bool,
) -> bool {
    let mut did = false;
    ui.popup(id, || {
        if confirm_dismiss_key(ui) {
            *understood = false;
            ui.close_current_popup();
            return;
        }
        popup_text(ui, body);
        ui.spacing();
        ui.checkbox("I understand", understood);
        ui.spacing();
        let avail = ui.content_region_avail()[0];
        let (w, stack) = scaled_button_row_layout(ui, avail, 2);
        let ok = {
            let _off = ui.begin_disabled_with_cond(!*understood);
            ui.button_with_size(confirm, [w, 0.0])
        };
        if ok && *understood {
            did = true;
            *understood = false;
            ui.close_current_popup();
        }
        if !stack {
            gap_line(ui);
        }
        if ui.button_with_size("Cancel", [w, 0.0]) {
            *understood = false;
            ui.close_current_popup();
        }
    });
    did
}

/// Status values that carry no information are omitted instead of showing
/// placeholder punctuation. Other rows keep their existing text unchanged.
pub fn status_value_visible(label: &str, value: &str) -> bool {
    !matches!(label, "walk" | "queue" | "modals") || !matches!(value.trim(), "" | "—" | "-1")
}

/// Scary confirm before "only render selected" can be unchecked: OK stays
/// disabled until "I understand" is ticked, then writes
/// `only_render_selected = false`. Cancel keeps the safe default; the
/// box that triggered this stays checked either way.
fn render_all_warn_window(ui: &Ui, session: &mut Session) {
    if !session.wall.render_all_warn_open {
        return;
    }
    let mut open = true;
    ui.window("Render all wall members?")
        .opened(&mut open)
        .flags(WindowFlags::NO_COLLAPSE | WindowFlags::ALWAYS_AUTO_RESIZE)
        .size_constraints(
            scale_size(ui, [DIALOG_W, 80.0]),
            [scale_px(ui, DIALOG_W), scale_px(ui, 720.0)],
        )
        .build(|| {
            let _wrap = ui.push_text_wrap_pos(scale_px(ui, DIALOG_W - 16.0));
            ui.text_wrapped(
                "This runs a GPU renderer for every client. Much lighter than the old CPU path, \
                 but a full wall still drives real GPU load on this machine.",
            );
            ui.spacing();
            let mut understood = session.wall.render_all_understood;
            if ui.checkbox("I understand", &mut understood) {
                session.wall.render_all_understood = understood;
            }
            ui.spacing();
            let avail = ui.content_region_avail()[0];
            let (w, stack) = scaled_button_row_layout(ui, avail, 2);
            let ok_clicked = {
                let _disabled = ui.begin_disabled_with_cond(!understood);
                ui.button_with_size("OK", [w, 0.0])
            };
            if ok_clicked && understood {
                session.focus.lock().unwrap().only_render_selected = false;
                session.wake_all_slots();
                session.wall.render_all_warn_open = false;
            }
            if !stack {
                ui.same_line();
            }
            if ui.button_with_size("Cancel", [w, 0.0]) {
                session.wall.render_all_warn_open = false;
            }
        });
    if !open {
        session.wall.render_all_warn_open = false;
    }
    if !session.wall.render_all_warn_open {
        session.wall.render_all_understood = false;
    }
}

fn background_ack_window(ui: &Ui, session: &mut Session) {
    if session.background_ack_open && session.background_bot_count() == 0 {
        session.dismiss_background_ack();
    }
    if !session.background_ack_open {
        return;
    }
    let mut open = true;
    let others = session.background_bot_count();
    let body = background_ack_text(others, session.core.resources());
    ui.window("Other profiles keep running")
        .opened(&mut open)
        .flags(WindowFlags::NO_COLLAPSE | WindowFlags::ALWAYS_AUTO_RESIZE)
        .size_constraints(
            scale_size(ui, [DIALOG_W, 80.0]),
            [scale_px(ui, DIALOG_W), scale_px(ui, 720.0)],
        )
        .build(|| {
            let _wrap = ui.push_text_wrap_pos(scale_px(ui, DIALOG_W - 16.0));
            ui.text_wrapped(&body);
            ui.spacing();
            let w = ui.content_region_avail()[0];
            if ui.button_with_size("Got it, don't show again", [w, 0.0]) {
                session.ack_background_bots();
            }
        });
    if !open {
        session.dismiss_background_ack();
    }
}

/// One tile per wall member, in wall order: cap (world number or local
/// traffic-light dot, name + brief, ✗) then a `TILE_W`×`TILE_H` body. The
/// body blits the slot's `FrameBuf` when `draw_for_slot` says this member
/// paints; otherwise it shows the renderer-off placeholder. While
/// `only_render_selected` is on (the safe default) the strip is collapsed:
/// cap only, no body. Clicking the name or body focuses the member; the ✗
/// (a sibling button, never part of the name click) removes it.
fn rail_tiles(ui: &Ui, gpu: &mut Gpu, state: &mut PanelState) {
    ui.spacing();
    state.sync_fleet();
    let rows = std::mem::take(&mut state.fleet);
    let labels = std::mem::take(&mut state.fleet_labels);
    let only_selected = state.session.focus.lock().unwrap().only_render_selected;
    {
        let focus = state.session.focus.lock().unwrap();
        dispose_idle_views(&mut state.views, gpu, |name| {
            rows.iter().any(|row| row.name == name) && draw_for_slot(&focus, name)
        });
    }
    for (row, labels) in rows.iter().zip(&labels) {
        let name = &row.name;
        let (is_focused, draw) = {
            let focus = state.session.focus.lock().unwrap();
            (
                focus.focused.as_deref() == Some(name.as_str()),
                draw_for_slot(&focus, name),
            )
        };
        let avail = ui.content_region_avail()[0];
        let preview = rail_preview_open(
            name,
            is_focused,
            only_selected,
            false,
            &state.session.ui.rail_preview,
        );
        let (cap_select, cap_remove, cap_fold) =
            rail_cap(ui, row, labels, is_focused, avail, preview);
        let body_clicked = if preview {
            rail_body(ui, gpu, state, name, draw)
        } else {
            false
        };
        if cap_fold {
            let next = !preview;
            state.session.ui.rail_preview.insert(name.clone(), next);
            crate::ui_state::save(&state.session.ui);
        } else if cap_remove {
            state.session.rail_remove(name);
        } else if cap_select || body_clicked {
            state.session.select(name);
        }
        ui.spacing();
    }
    state.fleet = rows;
    state.fleet_labels = labels;
}

/// Cap row: the active public-world number (or a status dot for local
/// profiles) in the row's light, the member's name plus its brief status
/// (click selects), and a small red ✗ (rail remove: logout arm then
/// `stop_slot`, never `vault`). `width` is the strip the row must fit (rail
/// avail or grid cell width). Every string is the row's cached label: a
/// frame formats nothing.
fn rail_cap(
    ui: &Ui,
    row: &FleetRow,
    labels: &CapLabels,
    is_focused: bool,
    width: f32,
    preview: bool,
) -> (bool, bool, bool) {
    const BTN: f32 = 28.0;
    const DOT_W: f32 = 18.0;
    let _id = ui.push_id(row.name.as_str());
    let [screen_x, screen_y] = ui.cursor_screen_pos();
    // Fractional scaled spacing can leave the next rail row between pixels.
    ui.set_cursor_screen_pos([screen_x, screen_y.round()]);
    let light = row.light();
    let colour = light_rgb(light);
    let btn = scale_px(ui, BTN);
    let dot_w = scale_px(ui, DOT_W);
    let marker_x = ui.cursor_pos_x();
    if labels.world.is_empty() {
        draw_status_dot(ui, light, dot_w);
    } else {
        world_marker(ui, &labels.world, colour, dot_w);
        ui.set_item_tooltip(&labels.world_tip);
    }
    #[cfg(test)]
    record_test_rail_item(ui);
    ui.same_line_with_pos(marker_x + dot_w + scale_px(ui, BUTTON_GAP));
    let name_w =
        (width - btn * 2.0 - dot_w - scale_px(ui, BUTTON_GAP * 3.0)).max(scale_px(ui, 10.0));
    let clicked = ui
        .selectable_config(&labels.title)
        .selected(is_focused)
        .size([name_w, 0.0])
        .build();
    #[cfg(test)]
    record_test_rail_item(ui);
    gap_line(ui);
    let fold_g = if preview { FOLD_GLYPH } else { UNFOLD_GLYPH };
    let folded = ui.button_with_size(fold_g, [btn, 0.0]);
    #[cfg(test)]
    record_test_rail_item(ui);
    ui.set_item_tooltip(if preview {
        "fold preview"
    } else {
        "show preview"
    });
    gap_line(ui);
    let red = ui.push_style_color(StyleColor::Text, ERROR);
    let removed = ui.button_with_size(REMOVE_GLYPH, [btn, 0.0]);
    #[cfg(test)]
    record_test_rail_item(ui);
    red.pop();
    ui.set_item_tooltip("drop from the wall — does not delete the vault profile");
    (clicked, removed, folded)
}

#[cfg(test)]
std::thread_local! {
    static TEST_RAIL_ITEM_BOUNDS: std::cell::RefCell<Vec<[f32; 4]>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(test)]
fn record_test_rail_item(ui: &Ui) {
    let min = ui.item_rect_min();
    let max = ui.item_rect_max();
    TEST_RAIL_ITEM_BOUNDS.with_borrow_mut(|bounds| bounds.push([min[0], min[1], max[0], max[1]]));
}

#[cfg(test)]
pub(crate) fn draw_test_rail_cap(ui: &Ui, width: f32) -> (Vec<[f32; 4]>, [f32; 4]) {
    let row = FleetRow {
        name: "pixel-layout".into(),
        ..FleetRow::default()
    };
    let labels = CapLabels {
        title: "pixel-layout".into(),
        ..CapLabels::default()
    };
    let colour = light_rgb(row.light());
    TEST_RAIL_ITEM_BOUNDS.with_borrow_mut(Vec::clear);
    let _ = rail_cap(ui, &row, &labels, true, width, false);
    let bounds = TEST_RAIL_ITEM_BOUNDS.with_borrow_mut(std::mem::take);
    (bounds, colour)
}

/// Public world marker: a disc in the status colour, centred in the
/// `width`-wide status cell and on the text line, with the world number
/// knocked out in the background colour. Drawn as geometry so the digit is
/// the rail's own font and the disc size does not depend on glyph metrics.
/// Occupies one text line, like the local-profile status marker.
fn world_marker(ui: &Ui, number: &str, colour: [f32; 4], width: f32) {
    let line_h = ui.text_line_height();
    let [x, y] = ui.cursor_screen_pos();
    let center = [x + width * 0.5, y + line_h * 0.5];
    let radius = (line_h * 0.5 + scale_px(ui, 1.5)).min(width * 0.5);
    let [text_w, text_h] =
        ui.current_font()
            .calc_text_size(ui.current_font_size(), f32::MAX, 0.0, number);
    let dl = ui.get_window_draw_list();
    dl.add_circle(center, radius, colour).filled(true).build();
    dl.add_text(
        [
            (center[0] - text_w * 0.5).round(),
            (center[1] - text_h * 0.5).round(),
        ],
        crate::theme::BG,
        number,
    );
    drop(dl);
    ui.dummy([width, line_h]);
}

/// Tile body: the member's `FrameBuf` blitted into a `size` box via a
/// cached [`GameView`] per name (uploaded when the mailbox hands a new
/// frame to `take`), or the renderer-off placeholder. Returns whether the
/// box was clicked (the grid cell / rail tile select path).
fn cell_body(
    ui: &Ui,
    gpu: &mut Gpu,
    state: &mut PanelState,
    name: &str,
    size: [f32; 2],
    draw: bool,
) -> bool {
    if !draw {
        if let Some(tv) = state.views.remove(name) {
            tv.view.dispose(gpu);
        }
        let _id = ui.push_id(name);
        return ui.selectable_config("renderer off").size(size).build();
    }
    if !state.views.contains_key(name) {
        state.views.insert(
            name.to_string(),
            TileView {
                view: GameView::init(gpu),
            },
        );
    }
    let tv = state.views.get_mut(name).expect("inserted above");
    // One consumer per `FrameBuf`: in rail mode the Game pane draws the
    // focused slot (or the first spawned slot when nothing is focused), so
    // that member's tile must not take the same frame too (grid mode has
    // no Game pane — the focused cell owns its frame).
    let game_pane_owns = if state.session.wall.grid {
        false
    } else {
        match state.session.focused_pixels() {
            Some(buf) => state
                .session
                .core
                .slot_io(name)
                .map(|s| Arc::ptr_eq(&s.pixels, &buf))
                .unwrap_or(false),
            None => false,
        }
    };
    if !game_pane_owns {
        // `take` moves the stored frame out; `present` routes it: the
        // `PixMap` (CPU) arm uploads into the tile's owned texture, the
        // `Texture` (GPU) arm binds the client's frame view directly.
        if let Some(frame) = state
            .session
            .core
            .slot_io(name)
            .and_then(|s| s.pixels.take())
        {
            tv.view.present(gpu, frame);
        }
    }
    ui.image(tv.view.tex_id, size);
    ui.is_item_clicked_with_button(MouseButton::Left)
}

/// Rail tile body: the fixed `TILE_W`×`TILE_H` case of [`cell_body`].
fn rail_body(ui: &Ui, gpu: &mut Gpu, state: &mut PanelState, name: &str, draw: bool) -> bool {
    cell_body(ui, gpu, state, name, scale_size(ui, [TILE_W, TILE_H]), draw)
}

/// `+ add bot`: opens the profile picker (same window as Profiles).
fn add_bot_button(ui: &Ui, state: &mut PanelState) {
    ui.spacing();
    let w = ui.content_region_avail()[0];
    if ui.button_with_size("+ add bot", [w, 0.0]) {
        state.session.wall.chooser_open = true;
    }
}

/// Resource card at the rail bottom: the operator session's 1 Hz meter
/// (bots, CPU, process RAM, traffic). A rate still measuring, a value the
/// platform cannot measure and a failed sample each say so.
fn resource_card(ui: &Ui, view: &ResourceView) {
    ui.spacing();
    ui.text_disabled("resource");
    ui.separator();
    draw_resource_rows(ui, view, false);
}

/// Rising-edge helper: `(open_popup, new_prev)`. `open_popup` is true only
/// on the `want` false→true edge, so `+ add bot` reopens after a close;
/// `new_prev` tracks `want` on **both** values, or a closed chooser would
/// keep a stale `true` and the next reopen would never fire.
pub fn chooser_should_open_popup(want: bool, prev: bool) -> (bool, bool) {
    (want && !prev, want)
}

/// Profiles always dock to the 274bot panel node, never the MultiBox rail
/// or Game split — a floating dialog with no dock id lands on Game and
/// imgui.ini keeps it.
fn chooser_dock_id(panel: Option<Id>) -> Option<Id> {
    panel
}

/// The right edge Profiles controls may reach, in screen space: the docked
/// window runs past the viewport's right edge on a narrow display
/// (1024×768), so its controls end that edge less the window's own left
/// padding. Call at the top of the window.
fn chooser_right_edge(ui: &Ui) -> f32 {
    let viewport = ui.main_viewport();
    let padding = ui.cursor_screen_pos()[0] - ui.window_pos()[0];
    viewport.work_pos()[0] + viewport.work_size()[0] - padding
}

/// The width from the cursor that is both inside the current window and on
/// screen, up to `right` from [`chooser_right_edge`].
fn on_screen_avail(ui: &Ui, right: f32) -> f32 {
    ui.content_region_avail()[0]
        .min(right - ui.cursor_screen_pos()[0])
        .max(0.0)
}

/// Unsaved-changes prompt for a staged leave of the edit form (switch,
/// close, delete of its profile, MultiBox off): Discard drops the draft and
/// leaves, Keep editing (or Escape) stays on the current form. Rendered from
/// the Profiles window while `pending_edit_switch` is set. The prompt goes
/// when the session retires it (the save it waited on settled), so it never
/// shows over the result it warned about.
fn edit_switch_popup(ui: &Ui, session: &mut Session) {
    let open = ui.is_popup_open(PROFILE_EDIT_SWITCH_POPUP);
    if session.pending_edit_switch.is_none() && !open {
        return;
    }
    if !open {
        ui.open_popup(PROFILE_EDIT_SWITCH_POPUP);
    }
    ui.popup(PROFILE_EDIT_SWITCH_POPUP, || {
        let Some(switch) = session.pending_edit_switch.as_ref() else {
            ui.close_current_popup();
            return;
        };
        if ui.is_key_pressed(Key::Escape) {
            session.cancel_pending_edit_switch();
            ui.close_current_popup();
            return;
        }
        popup_text(ui, &switch.prompt);
        ui.spacing();
        let avail = ui.content_region_avail()[0];
        let (w, stack) = scaled_button_row_layout(ui, avail, 2);
        if ui.button_with_size("Discard", [w, 0.0]) {
            session.confirm_pending_edit_switch();
            ui.close_current_popup();
        }
        if !stack {
            gap_line(ui);
        }
        if ui.button_with_size("Keep editing", [w, 0.0]) {
            session.cancel_pending_edit_switch();
            ui.close_current_popup();
        }
    });
}

/// Profile picker (single-bot and MultiBox). Click a row to focus it
/// (and load onto the wall while MultiBox is on); Edit opens user/pass;
/// ✕ deletes the vault row only. Load all is MultiBox-only.
fn chooser_window(ui: &Ui, session: &mut Session, panel_dock: Option<Id>) {
    if !session.wall.chooser_open {
        return;
    }
    // A leave prompt was staged from elsewhere (the tab's ✕, MultiBox): its
    // popup draws inside this window, so bring the window's tab forward.
    if std::mem::take(&mut session.focus_profiles) {
        ui.set_window_focus(Some("Profiles"));
    }
    let mut open = true;
    ui.set_next_window_class(&panel_window_class());
    if let Some(id) = chooser_dock_id(panel_dock) {
        ui.set_next_window_dock_id_with_cond(id, Condition::Appearing);
    }
    ui.window("Profiles")
        .opened(&mut open)
        .flags(WindowFlags::NO_COLLAPSE)
        .size(scale_size(ui, [PANEL_WIDTH, 560.0]), Condition::FirstUseEver)
        .build(|| {
            let right = chooser_right_edge(ui);
            // Wrap text where the on-screen part of the window ends.
            let wrap =
                ui.cursor_screen_pos()[0] + on_screen_avail(ui, right) - ui.window_pos()[0];
            let _wrap = ui.push_text_wrap_pos(wrap);
            if session.core.vault().is_none() {
                vault_unlock_prompt(ui, session);
            } else {
            let names: Vec<String> = session.core.vault()
                .map(|v| v.profiles().map(|p| p.username.clone()).collect())
                .unwrap_or_default();
            let w = on_screen_avail(ui, right);
            let focused = session.focused_name();
            let members = session.core.members().to_vec();
            let multibox = session.multibox;
            if multibox && ui.button_with_size("Load all", [w, 0.0]) {
                session.load_all();
            }
            if ui.button_with_size("New profile", [w, 0.0]) {
                session.begin_edit_profile(None);
            }
            ui.spacing();
            ui.text_colored(ACCENT, "Vault profiles");
            let mut picked: Option<String> = None;
            let mut removed: Option<String> = None;
            let mut edit: Option<String> = None;
            if names.is_empty() {
                ui.text_disabled("vault is empty — New profile then Save");
            } else {
                const ROW_H: f32 = 24.0;
                let viewport_h = ui.main_viewport().work_size()[1] / ui_scale(ui);
                // Leave room for the edit form when it is open; the list has
                // its own scroll region, so a long vault remains usable.
                let max_list = if session.chooser_edit.is_some() {
                    220.0
                } else {
                    (viewport_h - 280.0).clamp(120.0, 360.0)
                };
                let need = (names.len() as f32) * ROW_H + 8.0;
                let list_h = scale_px(ui, need.min(max_list));
                ui.child_window("##profiles-list")
                    .size([0.0, list_h])
                    .build(ui, || {
                        for name in &names {
                            let selected = if multibox {
                                members.iter().any(|m| m == name)
                            } else {
                                focused.as_deref() == Some(name.as_str())
                            };
                            // A profile whose first save is still being
                            // written is not selectable, editable or
                            // deletable until the write settles.
                            let saving = session.core.profile_saving(name);
                            let _saving = saving.then(|| ui.begin_disabled());
                            let (p, r, e) = chooser_row(ui, name, selected, right);
                            if saving {
                                continue;
                            }
                            if e {
                                edit = Some(name.clone());
                            } else if r {
                                removed = Some(name.clone());
                            } else if p {
                                picked = Some(name.clone());
                            }
                        }
                    });
            }
            if let Some(name) = edit {
                session.begin_edit_profile(Some(&name));
            } else if let Some(name) = removed {
                session.delete_understood = false;
                session.pending_profile_delete = Some(name);
                ui.open_popup(PROFILE_DELETE_POPUP);
            } else if let Some(name) = picked {
                // Row pick is pure focus (a wall load in MultiBox): the
                // picker and any open edit form stay exactly as they are.
                session.pick_profile(&name);
            }
            if session.chooser_edit.is_some() {
                ui.spacing();
                ui.separator();
                {
                    let edit = session.chooser_edit.as_deref().unwrap_or("");
                    if edit.is_empty() {
                        ui.text_colored(ACCENT, "New profile");
                    } else {
                        ui.text_colored(ACCENT, "Editing");
                        ui.same_line();
                        ui.text_colored(ACCENT, edit);
                    }
                }
                // One id scope per opened target: a text field still active
                // from the previous target cannot write its edit buffer into
                // this one's fields.
                let _form = ui.push_id(session.chooser_form);
                ui.text_disabled("user");
                if ui
                    .input_text("##cred-user", &mut session.cred_user)
                    .hint("username")
                    .build()
                {
                    session.note_chooser_edited();
                }
                ui.text_disabled("pass");
                if ui
                    .input_text("##cred-pass", &mut session.cred_pass)
                    .password(true)
                    .hint("password")
                    .build()
                {
                    session.note_chooser_edited();
                }
                slot_capture_section(ui, session);
                slot_random_section(ui, session);
                slot_clue_section(ui, session);
                // Save's outcome, next to Save: a refusal or a write that
                // failed after it was queued (nothing was written; the
                // banner is hidden while Profiles is open) or, once the
                // write is durable, `Saved <name>.`.
                match session.chooser_save.notice() {
                    Some(FormNotice::Saved(saved)) => ui.text_colored(GREEN, saved),
                    Some(notice) => {
                        if let Some(reason) = notice.error() {
                            ui.text_colored(ERROR, reason);
                            ui.text_disabled(frontend_core::NOTHING_SAVED);
                        }
                    }
                    None => {}
                }
                let avail = on_screen_avail(ui, right);
                let (bw, stack) = scaled_button_row_layout(ui, avail, 2);
                // One Save at a time: the form follows a rename or a new
                // profile only once its write is durable.
                let saving = session.form_saving().then(|| ui.begin_disabled());
                if ui.button_with_size("Save", [bw, 0.0]) {
                    session.save_credentials();
                }
                drop(saving);
                if !stack {
                    gap_line(ui);
                }
                if ui.button_with_size("Cancel", [bw, 0.0]) {
                    session.request_cancel_edit();
                }
            }
            edit_switch_popup(ui, session);
            }
            ui.spacing();
            if let Some(record) = session.core.saves_in_flight().last() {
                let name = record.destination.as_str();
                ui.text_disabled("saving ");
                ui.same_line_with_spacing(0.0, 0.0);
                ui.text_disabled(name);
                ui.same_line_with_spacing(0.0, 0.0);
                ui.text_disabled("…");
            }
            let w = on_screen_avail(ui, right);
            if ui.button_with_size("Close", [w, 0.0]) {
                session.request_close_profiles();
            }
            let pending = session.pending_profile_delete.clone();
            let body = match pending.as_deref() {
                Some(n) => format!(
                    "Remove {n} from the vault? A running wall member stays up. This cannot be undone."
                ),
                None => "Remove this profile from the vault?".into(),
            };
            if scary_confirm_popup(
                ui,
                PROFILE_DELETE_POPUP,
                &body,
                "Delete",
                &mut session.delete_understood,
            ) {
                if let Some(name) = session.pending_profile_delete.take() {
                    session.delete_profile(&name);
                }
            } else if !ui.is_popup_open(PROFILE_DELETE_POPUP) {
                session.pending_profile_delete = None;
            }
        });
    // The ✕ closes Profiles like Close does: while the form's save is
    // still being written Discard / Keep editing asks first.
    if !open {
        open = !session.request_close_profiles();
    }
    session.wall.chooser_open = session.wall.chooser_open && open;
}

fn settings_window(ui: &Ui, session: &mut Session, panel_dock: Option<Id>) {
    if !session.global_settings_open {
        return;
    }
    let mut open = true;
    let panel_class = panel_window_class();
    ui.set_next_window_class(&panel_class);
    if let Some(id) = panel_dock {
        ui.set_next_window_dock_id_with_cond(id, Condition::FirstUseEver);
    }
    ui.window("General config")
        .opened(&mut open)
        .flags(WindowFlags::NO_COLLAPSE)
        .size(
            scale_size(ui, [PANEL_WIDTH, 480.0]),
            Condition::FirstUseEver,
        )
        .size_constraints(
            scale_size(ui, [280.0, 80.0]),
            [f32::MAX, scale_px(ui, 720.0)],
        )
        .build(|| {
            config_section(ui, session, "Global", |ui, session| {
                global_config_section(ui, session);
            });
            ui.spacing();
            config_section(ui, session, "render", |ui, session| {
                slot_render_section(ui, session);
            });
        });
    session.global_settings_open = open;
}

fn debug_panel_window(ui: &Ui, session: &mut Session, panel_dock: Option<Id>) {
    if !session.debug_panel_open || !session.debug_ui() {
        session.release_debug_catalog();
        return;
    }
    let mut open = true;
    ui.set_next_window_class(&panel_window_class());
    if let Some(id) = panel_dock {
        ui.set_next_window_dock_id_with_cond(id, Condition::FirstUseEver);
    }
    ui.window("Debug")
        .opened(&mut open)
        .flags(WindowFlags::NO_COLLAPSE)
        .size(
            scale_size(ui, [PANEL_WIDTH, 480.0]),
            Condition::FirstUseEver,
        )
        .build(|| {
            crate::debug_panel::draw(ui, session);
            if std::mem::take(&mut session.debug_open_teleports) {
                ui.open_popup("##debug-teles");
            }
            debug_teleports_popup(ui, session);
        });
    session.debug_panel_open = open;
    if !open {
        session.release_debug_catalog();
    }
}

/// Profile-global clue traversal partner. Unlike card parameters this follows
/// the account across every script assignment and is persisted by Save.
fn slot_clue_section(ui: &Ui, session: &mut Session) {
    ui.text_disabled("clue duel partner");
    ui.set_next_item_width(-1.0);
    if ui
        .input_text(
            "##clue-duel-partner",
            &mut session.cred_settings.clue_duel_partner,
        )
        .hint("other account name")
        .build()
    {
        session.note_chooser_edited();
    }
    ui.text_wrapped(
        "run this account in Duel Arena Clue helper mode when clue 3554 needs the crossing",
    );
}

/// random: the guardian's per-profile toggles + lamp reward settings
/// (guardian spec `ProfileSettings`). Checkboxes and the skill combo write
/// straight through the vault upsert (`Session::set_random_settings`); the
/// lamp auto rubs through the guardian when enabled (`lamp_auto`).
fn slot_random_section(ui: &Ui, session: &mut Session) {
    let mut events = session.cred_settings.random_events;
    if ui.checkbox("random events", &mut events) {
        session.cred_settings.random_events = events;
        if let Some(name) = session.chooser_edit.as_deref().filter(|n| !n.is_empty()) {
            let name = name.to_string();
            session.set_random_settings(
                &name,
                events,
                &session.cred_settings.lamp_skill.clone(),
                session.cred_settings.lamp_auto,
            );
        }
        session.note_chooser_edited();
    }
    ui.text_wrapped("talk random events through on this profile. Off: never talk, never hold — still detects and shows the status row.");
    let mut auto = session.cred_settings.lamp_auto;
    if ui.checkbox("lamp auto", &mut auto) {
        session.cred_settings.lamp_auto = auto;
        if let Some(name) = session.chooser_edit.as_deref().filter(|n| !n.is_empty()) {
            let name = name.to_string();
            session.set_random_settings(
                &name,
                session.cred_settings.random_events,
                &session.cred_settings.lamp_skill.clone(),
                auto,
            );
        }
        session.note_chooser_edited();
    }
    ui.text_wrapped(
        "this profile; claim a lamp reward without a confirmation click (guardian rubs when lamp auto is on).",
    );
    let mut skill = session.cred_settings.lamp_skill.clone();
    if lamp_skill_combo(ui, &mut skill) {
        session.cred_settings.lamp_skill = skill.clone();
        if let Some(name) = session.chooser_edit.as_deref().filter(|n| !n.is_empty()) {
            let name = name.to_string();
            session.set_random_settings(
                &name,
                session.cred_settings.random_events,
                &skill,
                session.cred_settings.lamp_auto,
            );
        }
        session.note_chooser_edited();
    }
}

/// lamp skill combo: the client skill table (the lamp dialogue's own
/// names), previewing the current value even when it is not in the list.
/// Returns true when the pick changes `skill`.
fn lamp_skill_combo(ui: &Ui, skill: &mut String) -> bool {
    let presets = lamp_skill_presets();
    let preview = skill.clone();
    ui.set_next_item_width(-1.0);
    let opts = ComboBoxOptions::new().preview_mode(ComboBoxPreviewMode::Preview);
    let mut changed = false;
    if let Some(_open) = ui.begin_combo_with_flags("##lamp-skill", &preview, opts) {
        for preset in presets {
            let selected = preset == skill;
            let _row = ui.push_style_color(StyleColor::Text, if selected { ACCENT } else { TEXT });
            if ui.selectable_config(preset).selected(selected).build() {
                *skill = preset.to_string();
                changed = true;
            }
            if selected {
                ui.set_item_default_focus();
            }
        }
    }
    changed
}

/// The lamp dialogue's skill names from the client skill table, minus the
/// unused slots.
fn lamp_skill_presets() -> Vec<&'static str> {
    client::client::Skill::names
        .iter()
        .copied()
        .filter(|n| !n.starts_with('-'))
        .collect()
}

/// One picker row: name (click focuses / loads), Edit, then red ✕ (vault
/// delete, confirm). Sibling buttons so an Edit/✕ click never also picks.
/// The row ends at `right` (see [`chooser_right_edge`]), so Edit and ✕ stay
/// on screen when the docked window runs past the display.
fn chooser_row(ui: &Ui, name: &str, selected: bool, right: f32) -> (bool, bool, bool) {
    const EDIT_W: f32 = 44.0;
    const X_W: f32 = 28.0;
    let avail = on_screen_avail(ui, right);
    let edit_w = scale_px(ui, EDIT_W);
    let x_w = scale_px(ui, X_W);
    let name_w = (avail - edit_w - x_w - scale_px(ui, BUTTON_GAP * 2.0)).max(scale_px(ui, 10.0));
    let loaded = ui
        .selectable_config(name)
        .selected(selected)
        .close_popups(false)
        .size([name_w, 0.0])
        .build();
    gap_line(ui);
    let edit = ui.button_with_size(format!("Edit##edit-{name}"), [edit_w, 0.0]);
    gap_line(ui);
    let _red = ui.push_style_color(StyleColor::Text, ERROR);
    let removed = ui.button_with_size(format!("✕##{name}"), [x_w, 0.0]);
    (loaded, removed, edit)
}

/// Offline prepare: mint isolated identity, write server-native `.sav` via
/// `tools/harness`, write durable fixture identity — no live engine/window.
fn run_offline_prepare_fixture(args: &PanelArgs, scenario: &str) -> Result<(), String> {
    let sc = scenario::get(scenario).ok_or_else(|| format!("unknown scenario {scenario}"))?;
    let profile_count = sc.seed.profiles.len();
    if profile_count == 0 {
        return Err(format!("scenario {scenario} has zero seed profiles"));
    }
    let fixture_preset = scenario::fixture_preset_for(scenario)?;
    let server_root = args
        .server_root
        .clone()
        .or_else(|| std::env::var_os("BOT_SERVER_ROOT").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("/Users/acfrazier/experiments/Server/engine"));
    if !server_root.join("data/pack/server/obj.dat").is_file() {
        return Err(format!(
            "server root missing pack data: {} (pass --server-root or BOT_SERVER_ROOT)",
            server_root.display()
        ));
    }
    let identity_path = args
        .fixture_path
        .clone()
        .unwrap_or_else(|| scenario::default_fixture_path(scenario));
    let sav_dir = identity_path
        .parent()
        .map(|p| p.join(scenario))
        .unwrap_or_else(|| scenario::default_fixture_sav_dir(scenario));
    let names = host_play::mint_live_names(profile_count);
    let entries = host_play::mint_live_entries(&names);
    let pass = host_play::live_vault_passphrase();
    let passwords: Vec<String> = entries.iter().map(|(_, p)| p.clone()).collect();
    let identity = scenario::prepare_offline_fixture(scenario::OfflinePrepareOpts {
        scenario: scenario.to_string(),
        fixture_preset: fixture_preset.to_string(),
        profile: "main".into(),
        server_root,
        identity_path: identity_path.clone(),
        sav_dir,
        usernames: names,
        passwords,
        vault_passphrase: pass,
        overwrite: true,
    })?;
    println!(
        "[panel] offline fixture prepared: scenario={} identity={} accounts={}",
        identity.scenario,
        identity_path.display(),
        identity.accounts.len()
    );
    for a in &identity.accounts {
        println!(
            "[panel]   {} sav={} sha256={} bytes={}",
            a.username, a.sav_path, a.sav_sha256, a.sav_bytes
        );
    }
    println!("PASS: offline prepare fixture {scenario}");
    Ok(())
}

fn init_panel_running(
    args: &PanelArgs,
    shot_state: Arc<Mutex<crate::window::ShotState>>,
    permit: host_play::InstancePermit,
) -> Result<(PanelState, StartupPreparation), window::PanelError> {
    let mut state = PanelState::with_session(Session::with_instance(permit), shot_state);
    state.session.set_memory_override(args.memory_override);
    state.session.set_nav_paints_override(args.nav_paints);
    state.session.set_catalog_core_enabled(args.catalog_core);
    state.session.set_pair_core_enabled(args.pair_core);
    state.session.set_external_core_enabled(args.external_core);
    state.session.set_external_ts(args.external_ts.clone());
    crate::log_pane::apply_session_log_pref(&mut state.session);
    let fixture_mode = if args.run_prepared {
        scenario::FixtureMode::RunPrepared
    } else {
        scenario::FixtureMode::Default
    };
    state
        .session
        .set_fixture_boot(fixture_mode, args.fixture_path.clone());
    state
        .session
        .configure_profile(args.profile.clone())
        .map_err(window::PanelError::ServerProfile)?;
    let vault_pass = if args.vault_pass_stdin && matches!(args.mode, RunMode::Interactive) {
        let exists = state
            .session
            .default_vault_exists()
            .map_err(window::PanelError::ServerProfile)?;
        match host_play::passphrase::obtain(
            "panel-play",
            true,
            host_play::passphrase::Purpose::for_vault(exists),
        ) {
            Ok(pass) => Some(pass),
            Err(msg) => {
                eprintln!("FAIL: {msg}");
                std::process::exit(1);
            }
        }
    } else {
        None
    };
    let boot = boot_for(&args.mode, vault_pass);
    #[cfg(feature = "memory-profile")]
    let boot = match host_play::memory::Config::from_env() {
        Ok(Some(config)) => {
            client::profiling::enable();
            // The profile is not bound yet (an unbound session reads as
            // Remote), so check the class of the profile it will bind.
            let class = state
                .session
                .resolve_profile()
                .map(|profile| profile.profile_class());
            if let Err(error) = class.and_then(host_play::memory::require_live_benchmark) {
                eprintln!("FAIL: {error}");
                std::process::exit(1);
            }
            Some(Boot::Memory(config))
        }
        Ok(None) => boot,
        Err(error) => {
            eprintln!("FAIL: {error}");
            std::process::exit(1);
        }
    };
    Ok((state, StartupPreparation::new(boot)))
}

/// `Some(true)` continue, `Some(false)` exit, `None` still open.
fn instance_conflict_choice(ui: &Ui, holder: &host_play::InstanceHolder) -> Option<bool> {
    let mut open = true;
    let mut choice = None;
    ui.window("Another 274bot is running")
        .opened(&mut open)
        .flags(WindowFlags::NO_COLLAPSE | WindowFlags::ALWAYS_AUTO_RESIZE)
        .size_constraints(
            scale_size(ui, [DIALOG_W, 80.0]),
            [scale_px(ui, DIALOG_W), scale_px(ui, 720.0)],
        )
        .build(|| {
            let _wrap = ui.push_text_wrap_pos(scale_px(ui, DIALOG_W - 16.0));
            ui.text_wrapped(host_play::instance_conflict_message(holder));
            ui.spacing();
            let avail = ui.content_region_avail()[0];
            let (w, stack) = scaled_button_row_layout(ui, avail, 2);
            if ui.button_with_size("Exit", [w, 0.0]) {
                choice = Some(false);
            }
            if !stack {
                ui.same_line();
            }
            if ui.button_with_size("Continue anyway", [w, 0.0]) {
                choice = Some(true);
            }
        });
    if !open {
        Some(false)
    } else {
        choice
    }
}

/// Resolve a normal event-loop return against any failure latched by `--live`.
/// A live failure remains fatal if the window closes before the frame exits.
fn finish_panel_run(
    result: Result<(), window::PanelError>,
    live_failure: Option<String>,
) -> Result<(), window::PanelError> {
    match (result, live_failure) {
        (Err(error), _) => Err(error),
        (Ok(()), Some(message)) => Err(window::PanelError::LiveHarnessFailure(message)),
        (Ok(()), None) => Ok(()),
    }
}

/// Exit status for a live terminal decision; failure takes precedence.
fn live_exit_code(live: &LiveHarness, failure: Option<&str>) -> Option<i32> {
    if failure.is_some() || live.failure().is_some() {
        Some(1)
    } else if live.exit_pass() {
        Some(0)
    } else {
        None
    }
}

/// Open the 274bot panel window. Call after the vault has been started.
/// `args.mode` selects the normal interactive panel, a `--live NAME` harness,
/// or `--smoke` (temp `test` vault, one whole-window shot at scene 2,
/// exit 0). `--prepare-fixture` is offline-only and never opens a window.
pub fn run_panel(args: PanelArgs) -> Result<(), window::PanelError> {
    if let Some(scenario) = args.mode.prepare_fixture_name() {
        return run_offline_prepare_fixture(&args, scenario).map_err(|e| {
            eprintln!("FAIL: {e}");
            std::process::exit(1);
        });
    }
    let skip_lock = !matches!(args.mode, RunMode::Interactive);
    #[cfg(feature = "memory-profile")]
    let skip_lock = skip_lock
        || host_play::memory::Config::from_env()
            .ok()
            .flatten()
            .is_some();
    let shot_state = Arc::new(Mutex::new(crate::window::ShotState::default()));
    let mut instance_prompt = None;
    let mut running = None;
    match host_play::resolve_instance_permit(host_play::InstanceKind::Panel, skip_lock) {
        Ok(host_play::InstancePermitOutcome::Ready(permit)) => {
            running = Some(init_panel_running(&args, Arc::clone(&shot_state), permit)?);
        }
        Ok(host_play::InstancePermitOutcome::NeedsConfirm(holder)) => {
            instance_prompt = Some(holder);
        }
        Err(e) => {
            eprintln!("panel: instance lock: {e}");
            std::process::exit(1);
        }
    }
    let mut presented = false;

    let mut cfg = runner_config();
    let mut window_title = running
        .as_ref()
        .map(|(state, _)| state.session.app_title())
        .unwrap_or_else(|| "274bot".into());
    cfg.window_title.clone_from(&window_title);
    let os_window: Arc<Mutex<Option<Arc<winit::window::Window>>>> = Arc::new(Mutex::new(None));
    let os_window_init = Arc::clone(&os_window);
    let live_failure = Arc::new(Mutex::new(None));
    let live_failure_frame = Arc::clone(&live_failure);
    let result = window::run(
        cfg,
        amber_style,
        move |window, device, queue, _| {
            client::render::backend::inject_device(device.clone(), queue.clone());
            *os_window_init.lock().unwrap() = Some(Arc::clone(window));
        },
        Arc::clone(&shot_state),
        move |ui, gpu| {
            let _profile_draw = client::profiling::UI_DRAW.start();
            if let Some(holder) = instance_prompt.as_ref() {
                match instance_conflict_choice(ui, holder) {
                    None => return false,
                    Some(false) => return true,
                    Some(true) => {
                        instance_prompt = None;
                        match init_panel_running(
                            &args,
                            Arc::clone(&shot_state),
                            host_play::InstancePermit::skip(),
                        ) {
                            Ok(ready) => running = Some(ready),
                            Err(e) => {
                                eprintln!("panel: {e}");
                                std::process::exit(1);
                            }
                        }
                    }
                }
            }
            let Some((state, startup)) = running.as_mut() else {
                return false;
            };
            if presented {
                drive_startup(state, startup);
            }
            presented = true;
            if state.os_window.is_none() {
                if let Some(window) = os_window.lock().unwrap().clone() {
                    state.os_window = Some(window);
                    state.last_fit_need = None;
                }
            }
            let title = state.session.app_title();
            if title != window_title {
                if let Some(window) = state.os_window.as_ref() {
                    window.set_title(&title);
                    window_title = title;
                }
            }
            if startup.in_flight() {
                if let Some(w) = state.os_window.as_ref() {
                    w.request_redraw();
                }
            }
            let progress = startup_progress(startup, state.session.profile_generation());
            ui_frame(ui, gpu, state, progress, &live_failure_frame)
        },
    );
    let failure = live_failure.lock().unwrap().take();
    finish_panel_run(result, failure)
}

/// Whole-window shots (the 377 harness pattern): write completed captures
/// to the per-run dir, then hand the scenario sink's new requests to the
/// next render pass's readback. The per-run dir is created lazily on the
/// first write (the interactive F12 capture has no run start to hook).
/// Returns how many shots were written this frame — the `--smoke` watch
/// exits 0 once its single scene2 shot lands.
fn pump_shots(state: &mut PanelState) -> usize {
    let done = {
        let mut shots = state.shot_state.lock().unwrap();
        std::mem::take(&mut shots.done)
    };
    let mut written = 0;
    for cap in done {
        if state.shot_dir.is_none() {
            match scenario::shot::create_run_dir() {
                Ok(dir) => state.shot_dir = Some(dir),
                Err(error) => {
                    let message = format!("shot directory unavailable: {error}");
                    eprintln!("[panel] shot {}: {message}", cap.label);
                    state
                        .shot_state
                        .lock()
                        .unwrap()
                        .mark_failed(&cap.label, &message);
                    continue;
                }
            }
        }
        if let Some(dir) = state.shot_dir.as_deref() {
            match scenario::shot::write_shot(
                dir,
                &cap.label,
                &cap.rgba,
                cap.width,
                cap.height,
                &cap.snapshot_json,
            ) {
                Ok(path) => {
                    println!("[panel] shot {} -> {}", cap.label, path.display());
                    #[cfg(feature = "render-diagnostics")]
                    client::render::diagnostics::dump_png_slot(
                        &cap.rgba,
                        cap.width as i32,
                        cap.height as i32,
                        &cap.label,
                        true,
                        cap.pixel_roi.as_ref(),
                    );
                    state.shot_state.lock().unwrap().mark_written(&cap.label);
                    written += 1;
                }
                Err(error) => {
                    eprintln!("[panel] shot {}: {error}", cap.label);
                    state
                        .shot_state
                        .lock()
                        .unwrap()
                        .mark_failed(&cap.label, &error.to_string());
                }
            }
        }
    }
    // `--smoke` render-settle gate: hold the scene2 request until the
    // focused slot has held scene 2 for [`SMOKE_SETTLE`] and is still
    // ingame — the slot's 1 fps renderer needs wall-clock time to
    // rasterize the world, and an early readback captures the title
    // screen. `script_*` and interactive shots drain as before.
    let hold = state
        .live
        .as_ref()
        .is_some_and(|h| h.holds_shot_promotion(&state.session, Instant::now()));
    if !hold {
        drive_pair_capture_focus(state);
        let focused = state.session.focused_name();
        // Rebuild current scene2 JSON only for the pending actor-tagged job.
        // Ordinary focused frames and untagged promote must not serialize.
        let pending_actor_is_focused = focused.as_deref().is_some_and(|actor| {
            state
                .shot_state
                .lock()
                .unwrap()
                .pending_actor_request()
                .and_then(|request| request.actor.as_deref())
                == Some(actor)
        });
        let ready_json = pending_actor_is_focused
            .then_some(focused.as_deref())
            .flatten()
            .and_then(|actor| pair_actor_capture_ready(state, actor));
        let presented = ready_json.is_some().then(|| focused.clone()).flatten();
        let mut shots = state.shot_state.lock().unwrap();
        if std::env::var("BOT_DEBUG").as_deref() == Ok("1") && !shots.requests.is_empty() {
            let labels = shots
                .requests
                .iter()
                .map(|request| request.label.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            eprintln!("[panel] capture queued for readback: {labels}");
        }
        if let (Some(actor), Some(json)) = (presented.as_deref(), ready_json) {
            shots.refresh_actor_sidecar(actor, json);
        }
        shots.promote_ready(focused.as_deref(), presented.as_deref());
    }
    written
}

/// Focus the next pending pair actor only after the previous capture has
/// left the GPU readback/write path. Restore the prior focus when no
/// actor-tagged job remains.
fn drive_pair_capture_focus(state: &mut PanelState) {
    let (next, in_flight) = {
        let shots = state.shot_state.lock().unwrap();
        (
            shots
                .pending_actor_request()
                .and_then(|request| request.actor.clone()),
            shots.capture_in_flight(),
        )
    };
    if in_flight {
        return;
    }
    if let Some(actor) = next {
        if state.pair_capture_restore.is_none() {
            state.pair_capture_restore = state.session.focused_name();
        }
        if let Err(error) = state.session.focus_existing(&actor) {
            let labels = {
                let shots = state.shot_state.lock().unwrap();
                shots
                    .requests
                    .iter()
                    .filter(|request| request.actor.as_deref() == Some(actor.as_str()))
                    .map(|request| request.label.clone())
                    .collect::<Vec<_>>()
            };
            state
                .shot_state
                .lock()
                .unwrap()
                .fail_labels(&labels, &error);
        }
        return;
    }
    if let Some(restore) = state.pair_capture_restore.take() {
        let _ = state.session.focus_existing(&restore);
    }
}

fn presented_capture_actor(state: &PanelState) -> Option<String> {
    let focused = state.session.focused_name()?;
    match state.last_upload.as_ref() {
        Some((name, gen)) if *name == focused && *gen > 0 => Some(name.clone()),
        _ => None,
    }
}

/// Current scene2 sidecar for an actor whose selected frame is already in
/// the game view. A previously presented matching frame plus a stale
/// enqueue-time sidecar does not prove a now-offworld actor.
fn pair_actor_capture_ready(state: &PanelState, actor: &str) -> Option<String> {
    let presented = presented_capture_actor(state)?;
    if presented != actor {
        return None;
    }
    current_pair_actor_sidecar(&state.session, actor)
}

fn current_pair_actor_sidecar(session: &Session, actor: &str) -> Option<String> {
    let states = session.nav_states.lock().unwrap();
    let (snapshot, _) = states.get(actor)?;
    if !snapshot.ingame() || snapshot.scene_state() != 2 {
        return None;
    }
    actor_snapshot_json(actor, snapshot).ok()
}

fn record_presented_upload(
    last_upload: &mut Option<(String, u64)>,
    name: String,
    gen: u64,
    took_frame: bool,
) {
    if took_frame {
        *last_upload = Some((name, gen));
    }
}

/// The per-frame UI body: session pump, live harness ticks, dock host,
/// chrome, game pane, rail.
fn ui_frame(
    ui: &Ui,
    gpu: &mut Gpu,
    state: &mut PanelState,
    progress: Option<StartupProgressView>,
    live_failure: &Mutex<Option<String>>,
) -> bool {
    apply_amber_current(&state.session.ui.chrome);
    let wrote_shots = pump_shots(state);
    state.session.pump_status();
    #[cfg(feature = "memory-profile")]
    if let Some(run) = state.memory.as_mut() {
        state.session.memory_focus(run);
        if let Some(play) = state.session.core.play() {
            match run.poll(play) {
                Ok(true) => {
                    eprintln!("PASS: memory panel observation complete");
                    std::process::exit(0);
                }
                Ok(false) => {}
                Err(error) => {
                    eprintln!("FAIL: memory panel: {error}");
                    std::process::exit(1);
                }
            }
        }
    }
    state.session.pump_script_transpile();
    if let Some(live) = state.live.as_mut() {
        // Harness runs only: an interactive frame copies no status rows.
        let statuses = state.session.statuses().to_vec();
        let terminal_shot_status = {
            let label = state
                .session
                .scenario
                .lock()
                .unwrap()
                .as_ref()
                .and_then(|runner| runner.terminal_shot());
            label
                .map(|label| state.shot_state.lock().unwrap().status(label))
                .unwrap_or(ShotStatus::Missing)
        };
        let failure = live.tick(
            &mut state.session,
            &statuses,
            &terminal_shot_status,
            Some(&state.shot_state),
            wrote_shots,
        );
        if let Some(code) = live_exit_code(live, failure.as_deref()) {
            if code == 1 {
                if let Some(msg) = failure.as_deref().or_else(|| live.failure()) {
                    eprintln!("FAIL: {msg}");
                    *live_failure.lock().unwrap() = Some(msg.to_owned());
                }
            }
            return true;
        }
    }
    // Interactive whole-window capture: F12 enqueues one shot per press
    // (a rising edge, no repeat) with no scenario involved. Harness runs
    // keep their own keys — a manual shot during `--smoke` would trip the
    // "any written shot passes" watch before the scene2 shot lands.
    if state.live.is_none() && ui.is_key_pressed_with_repeat(Key::F12, false) {
        enqueue_manual_shot(state);
    }
    let title = game_window_title(state.session.focused_name().as_deref());
    dock_host(ui, state, &title);
    state.debug_resources();
    let game_class = game_window_class();
    let panel_class = panel_window_class();
    ui.set_next_window_class(&panel_class);
    panel_window(ui, &mut state.session, progress);
    floating_log_window(ui, &mut state.session);
    crate::fleet::window(ui, &mut state.session);
    ui.set_next_window_class(&game_class);
    // Frame owner: identity replacement and close-release happen outside
    // the Game window build closure so a rebind cannot keep stale buffers.
    let nav = state.session.map_nav_digest();
    let geom = picker::pack().map(|world| {
        (
            world.collision.origin.x,
            world.collision.origin.z,
            world.collision.width as u32,
            world.collision.height as u32,
        )
    });
    state.walk_map.sync_identity(nav, geom, Some(gpu));
    if !state.session.walkto_open {
        picker::note_closed();
        state.session.release_walk_map();
        if state.walk_map.is_open() {
            state.walk_map.release(Some(gpu));
        }
    }
    game_window(ui, gpu, state, &title);
    if state.session.multibox && !state.session.wall.grid {
        ui.set_next_window_class(&rail_window_class());
        rail_window(ui, gpu, state);
    }
    // Every frame, not only while open: the prev latches must track
    // the close so the next open is a fresh rising edge.
    chooser_window(ui, &mut state.session, state.panel_dock_node);
    settings_window(ui, &mut state.session, state.panel_dock_node);
    debug_panel_window(ui, &mut state.session, state.panel_dock_node);
    browse_window(ui, &mut state.session);
    nav_settings_window(ui, &mut state.session, state.panel_dock_node);
    script_prefs_window(ui, &mut state.session, state.panel_dock_node);
    crate::loadouts::window(ui, &mut state.session);
    render_all_warn_window(ui, &mut state.session);
    background_ack_window(ui, &mut state.session);
    discard_unconsumed_native_capture();
    false
}

#[cfg(test)]
#[path = "app_tests.rs"]
mod tests;
