//! Panel shell: docking enabled, multi-viewport disabled, amber chrome,
//! running on the panel-owned window loop in `crate::window`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
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
use crate::headed_record::{HeadedRecord, RecordFrame, StepInfo};
use crate::overlay::{PathOverlay, QueueCards};
use crate::paint::PaintOverlay;
use crate::picker;
use crate::rail::{
    draw_status_dot, light_rgb, next_os_window_size, os_window_size, rail_preview_open,
    rail_split_ratio, BASE_WINDOW_H, BASE_WINDOW_W, FOLD_GLYPH, RAIL_W, REMOVE_GLYPH, TILE_H,
    TILE_W, UNFOLD_GLYPH,
};
use crate::script_picker::{
    self, card_columns, card_desc_height, card_kind_source, card_rect_activated, card_width,
    chip_frame_padding, chip_text_color, chip_wraps, display_category, move_category,
    overlay_first_pos, resolve_category_order, title_clip_width, CARD_GAP, CARD_MIN_W,
    SCRIPTS_FIRST_H, SCRIPTS_FIRST_W,
};
#[cfg(feature = "js")]
use crate::script_picker::{
    card_transpile_label, centered_row_x, dialog_date_color, format_mtime, DialogMode,
    FILE_DIALOG_FIRST_H, FILE_DIALOG_FIRST_W, GLYPH_CHEVRON, GLYPH_FILE, GLYPH_FOLDER,
};
use crate::walk_map::WalkMapRenderer;
use crate::window::{self, Gpu, RedrawMode, ShotStatus, Theme};
use dear_imgui_rs::internal::RawWrapper;
use dear_imgui_rs::{
    ChildFlags, ColorDisplayMode, ComboBoxOptions, ComboBoxPreviewMode, Condition, DockBuilder,
    DockNodeFlags, DragDropTargetFlags, Id, Key, MouseButton, SplitDirection, StyleColor, StyleVar,
    TreeNodeFlags, Ui, WindowClass, WindowFlags,
};
#[cfg(feature = "js")]
use dear_imgui_rs::{TableColumnFlags, TableFlags};
use frontend_core::launch::{
    InstanceConflict, InstanceKind, InstancePermit, Passphrase, PreparationCompletion,
    PreparationJob, ProfilePreparation, ProgressStage, ProgressUnit, ProgressView, ValidationJob,
    ValidationPending,
};
use frontend_core::process::debug_enabled;
use frontend_core::surface::map_image_to_applet;

use crate::input_capture::{
    discard_unconsumed_native_capture, stream_capture, KeyboardOwner, MouseButtons, MouseFrame,
    MouseTarget, PaintMouseGate,
};
use crate::session::{
    debug_dest_cheats, debug_main_buttons_for, debug_maxme_cheats, script_active,
    script_pause_enabled, script_stop_enabled, PanelWindow, Session,
};
use crate::theme::{
    applet_offset, apply_amber, apply_amber_current, fit_applet, game_window_title, native_applet,
    panel_scale, panel_split_ratio, scale_px, scale_size, ui_scale, ACCENT, ACCENT_HOVER, BG,
    DOCKHOST_PADDING, ERROR, GREEN, PANEL_WIDTH, PANEL_WINDOW, RAIL_WINDOW, TEXT, TEXT_DIM, WARN,
};
use frontend_core::resources::{background_ack_text, format_background, format_bots};
#[cfg(feature = "js")]
use frontend_core::scripts::CardSource;
use frontend_core::scripts::{
    format_setting_value, BrowseCard, CompiledCardId, PaintFrame, SchemaRef, ScriptChoice,
    ScriptRunState, SettingView,
};
use frontend_core::{
    FleetOrder, FleetRow, LogoutEligibility, OperatorCommand, Phase, ProfileIdentity, ReorderOp,
    ResourceView, Targets,
};

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
    memory: Option<frontend_core::bench::BenchRun>,
    game_view: Option<GameView>,
    session: Session,
    keyboard: KeyboardOwner,
    /// Press-origin routing for captured mouse buttons at the paint
    /// boundary, shared by the single-bot pane and the grid pane.
    mouse_gate: PaintMouseGate,
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
    /// Cached queue cards over the Grid's Images, one per queued bot.
    queue_cards: QueueCards,
    /// Cached script-paint overlay over the Game chatbox (see `paint`).
    paint: PaintOverlay,
    /// One cached tile texture per wall member (blitted at TILE_W×TILE_H).
    views: HashMap<String, TileView>,
    /// Each fleet-model row's cap strings for the rail and the grid,
    /// parallel to `session.fleet_model.rows()` and rebuilt only when the
    /// model's rows generation moves (never per frame). The rows themselves
    /// are the model's: the panel keeps no copy.
    fleet_labels: Vec<CapLabels>,
    fleet_generation: Option<u64>,
    /// `Not loaded (N)###not-loaded`, rebuilt when the model's generation
    /// moves.
    not_loaded_label: String,
    not_loaded_generation: Option<u64>,
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
    /// Opt-in headed recording (`HEADED_RECORD=1`): periodic whole-window
    /// frames into `<shot_dir>/record.mp4` plus a step/tick sidecar.
    /// Finalized on PASS/FAIL (see `ui_frame`) and on window close via
    /// `Drop`, so the mp4 is always playable.
    record: HeadedRecord,
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
    Memory(frontend_core::bench::BenchConfig),
    /// The vault passphrase from the in-panel prompt or `--vault-pass-stdin`.
    /// Failure is non-fatal: the in-panel prompt covers typing. `Debug` never
    /// prints it.
    Unlock { pass: Passphrase },
    /// Live harness spawns. Failure is fatal (`FAIL:` + exit).
    Live(LiveBoot),
}

struct ProfileValidateJob {
    boot: Boot,
    job: ValidationJob,
}

struct ProfileValidatePending {
    boot: Boot,
    pending: ValidationPending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProgressPhase {
    Preparing,
    FinalChecks,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StartupProgressView {
    phase: ProgressPhase,
    progress: ProgressView,
}

struct StartupPreparation {
    prepare: Option<PreparationJob>,
    prepare_pending: Option<ProfilePreparation>,
    validate: Option<ProfileValidateJob>,
    validate_pending: Option<ProfileValidatePending>,
    pending_boot: Option<Boot>,
    failed_generation: Option<u64>,
}

impl StartupPreparation {
    fn new(pending_boot: Option<Boot>) -> Self {
        Self {
            prepare: None,
            prepare_pending: None,
            validate: None,
            validate_pending: None,
            pending_boot,
            failed_generation: None,
        }
    }

    fn in_flight(&self) -> bool {
        self.prepare.is_some()
            || self.prepare_pending.is_some()
            || self.validate.is_some()
            || self.validate_pending.is_some()
    }
}

fn startup_progress(startup: &StartupPreparation, generation: u64) -> Option<StartupProgressView> {
    let (phase, progress) = if let Some(job) = startup
        .validate
        .as_ref()
        .filter(|job| job.job.generation() == generation)
    {
        (ProgressPhase::FinalChecks, job.job.progress())
    } else if let Some(pending) = startup
        .validate_pending
        .as_ref()
        .filter(|pending| pending.pending.generation() == generation)
    {
        (ProgressPhase::FinalChecks, pending.pending.progress())
    } else if let Some(job) = startup
        .prepare
        .as_ref()
        .filter(|job| job.generation() == generation)
    {
        (ProgressPhase::Preparing, job.progress())
    } else {
        let pending = startup
            .prepare_pending
            .as_ref()
            .filter(|pending| pending.generation() == generation)?;
        (ProgressPhase::Preparing, pending.progress())
    };
    Some(StartupProgressView {
        phase,
        progress: progress?,
    })
}

/// The deferred boot for a [`run_panel`] call, derived from the run mode and
/// the passphrase read from `--vault-pass-stdin` — pure, so the mapping is
/// testable (the boot itself runs after GPU init; nothing here spawns or
/// unlocks).
fn boot_for(mode: &RunMode, vault_pass: Option<Passphrase>) -> Option<Boot> {
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
    state
        .session
        .ensure_profile_bound()
        .map_err(|error| error.to_string())?;
    state
        .session
        .require_bot_operation()
        .map_err(|error| error.to_string())?;
    match boot {
        #[cfg(feature = "memory-profile")]
        Boot::Memory(config) => {
            let run = frontend_core::bench::BenchRun::prepare(config, "panel")
                .map_err(|error| error.to_string())?;
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

fn fail_startup(
    session: &mut Session,
    fatal: bool,
    error: String,
) -> Option<window::PanelError> {
    session.set_profile_preparing(false);
    session.error = Some(error.clone());
    if fatal {
        session.retire_play();
        eprintln!("FAIL: {error}");
        Some(window::PanelError::StartupFailure(error))
    } else {
        None
    }
}

/// Poll the two sequential WorkAdmission preparations. Resource-heavy phases
/// run away from the event loop; profile/template installation and all vault,
/// Play, GPU, JS-store and slot ownership remain on this UI thread.
fn drive_startup(
    state: &mut PanelState,
    startup: &mut StartupPreparation,
) -> Option<window::PanelError> {
    if let Some(pass) = state.session.take_requested_unlock() {
        if let Some(job) = startup
            .validate
            .as_mut()
            .filter(|job| matches!(&job.boot, Boot::Unlock { .. }))
        {
            job.boot = Boot::Unlock { pass };
        } else if let Some(pending) = startup
            .validate_pending
            .as_mut()
            .filter(|pending| matches!(&pending.boot, Boot::Unlock { .. }))
        {
            pending.boot = Boot::Unlock { pass };
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
        .is_some_and(|job| job.generation() != generation)
        || startup
            .prepare_pending
            .as_ref()
            .is_some_and(|pending| pending.generation() != generation)
    {
        startup.prepare = None;
        startup.prepare_pending = None;
        startup.failed_generation = None;
        state.session.set_profile_preparing(false);
    }

    let prepared = startup
        .prepare
        .as_ref()
        .is_some_and(PreparationJob::is_finished);
    if prepared {
        let job = startup.prepare.take().expect("polled preparation job");
        let job_generation = job.generation();
        let result = job.join();
        match state
            .session
            .finish_profile_preparation(job_generation, result)
        {
            PreparationCompletion::Installed | PreparationCompletion::Stale => {
                startup.failed_generation = None;
            }
            PreparationCompletion::Failed => {
                startup.failed_generation = Some(job_generation);
                let fatal = startup
                    .pending_boot
                    .as_ref()
                    .is_some_and(boot_failure_is_fatal);
                let error = state.session.error.clone().unwrap_or_default();
                if let Some(error) = fail_startup(&mut state.session, fatal, error) {
                    return Some(error);
                }
                if !fatal {
                    startup.pending_boot = None;
                }
            }
        }
    }

    let validated = startup
        .validate
        .as_ref()
        .is_some_and(|job| job.job.is_finished());
    if validated {
        let job = startup.validate.take().expect("polled validation job");
        let job_generation = job.job.generation();
        let result = job.job.join();
        state.session.set_profile_preparing(false);
        if job_generation != state.session.profile_generation() {
            return None;
        }
        let fatal = boot_failure_is_fatal(&job.boot);
        match result.and_then(|ticket| state.session.install_validated_template(ticket)) {
            Ok(()) => {
                if let Err(error) = boot_execute(state, job.boot) {
                    if let Some(error) = fail_startup(&mut state.session, fatal, error) {
                        return Some(error);
                    }
                }
            }
            Err(error) => {
                if let Some(error) = fail_startup(&mut state.session, fatal, error.to_string()) {
                    return Some(error);
                }
            }
        }
    }

    if !state.session.profile_bound()
        && startup.prepare.is_none()
        && startup.failed_generation != Some(generation)
    {
        if startup.prepare_pending.is_none() {
            match state.session.profile_preparation() {
                Ok(preparation) => {
                    state.session.error = None;
                    state.session.set_profile_preparing(true);
                    startup.prepare_pending = Some(preparation);
                }
                Err(error) => {
                    startup.failed_generation = Some(generation);
                    let fatal = startup
                        .pending_boot
                        .as_ref()
                        .is_some_and(boot_failure_is_fatal);
                    if let Some(error) = fail_startup(&mut state.session, fatal, error.to_string()) {
                        return Some(error);
                    }
                }
            }
        }

        if let Some(pending) = startup.prepare_pending.as_ref() {
            match pending.submit() {
                Ok(job) => {
                    startup.prepare_pending = None;
                    startup.prepare = Some(job);
                }
                Err(error) if error.is_full() => {}
                Err(error) => {
                    startup.prepare_pending = None;
                    startup.failed_generation = Some(generation);
                    let fatal = startup
                        .pending_boot
                        .as_ref()
                        .is_some_and(boot_failure_is_fatal);
                    if let Some(error) = fail_startup(&mut state.session, fatal, error.to_string()) {
                        return Some(error);
                    }
                }
            }
        }
    }

    if state.session.profile_bound()
        && startup.validate.is_none()
        && startup.validate_pending.is_none()
        && startup.pending_boot.is_some()
    {
        let boot = startup.pending_boot.take().expect("pending boot");
        let fatal = boot_failure_is_fatal(&boot);
        match state.session.template_for_validation() {
            Ok(template) => {
                state.session.error = None;
                state.session.set_profile_preparing(true);
                startup.validate_pending = Some(ProfileValidatePending {
                    boot,
                    pending: ValidationPending::new(generation, template),
                });
            }
            Err(error) => {
                if let Some(error) = fail_startup(&mut state.session, fatal, error.to_string()) {
                    return Some(error);
                }
            }
        }
    }

    if let Some(pending) = startup.validate_pending.as_ref() {
        match pending.pending.submit() {
            Ok(job) => {
                let pending = startup
                    .validate_pending
                    .take()
                    .expect("validation demand retained");
                startup.validate = Some(ProfileValidateJob {
                    boot: pending.boot,
                    job,
                });
            }
            Err(error) if error.is_full() => {}
            Err(error) => {
                let pending = startup
                    .validate_pending
                    .take()
                    .expect("validation demand retained");
                let fatal = boot_failure_is_fatal(&pending.boot);
                if let Some(error) = fail_startup(&mut state.session, fatal, error.to_string()) {
                    return Some(error);
                }
            }
        }
    }
    None
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
            keyboard: KeyboardOwner::default(),
            mouse_gate: PaintMouseGate::default(),
            dock_inited: false,
            dock_layout: None,
            game_dock_node: None,
            panel_dock_node: None,
            rail_dock_node: None,
            docked_game_title: String::new(),
            last_upload: None,
            pair_capture_restore: None,
            overlay: PathOverlay::new(),
            queue_cards: QueueCards::default(),
            paint: PaintOverlay::new(),
            views: HashMap::new(),
            fleet_labels: Vec::new(),
            fleet_generation: None,
            not_loaded_label: String::new(),
            not_loaded_generation: None,
            debug_resources: 0,
            live: None,
            dock_size: None,
            os_window: None,
            last_fit_need: None,
            shot_state,
            shot_dir: None,
            record: HeadedRecord::from_env(),
            walk_map: WalkMapRenderer::new(),
        }
    }

    /// Rebuild the rail and grid cap strings of the fleet model's rows when
    /// (and only when) the rows changed; the strings reuse their buffers.
    fn sync_fleet(&mut self) {
        use std::fmt::Write as _;
        let model = &self.session.fleet_model;
        if self.fleet_generation != Some(model.rows_generation()) {
            self.fleet_generation = Some(model.rows_generation());
            self.fleet_labels
                .resize_with(model.rows().len(), CapLabels::default);
            for (row, labels) in model.rows().iter().zip(&mut self.fleet_labels) {
                labels.title.clear();
                // `###cap`: the select keeps its ID while the brief changes.
                let _ = write!(labels.title, "{}: {}###cap", row.name(), row.brief());
                labels.world.clear();
                labels.world_tip.clear();
                if let Some(number) = row.world() {
                    let _ = write!(labels.world, "{number}");
                    let _ = write!(labels.world_tip, "w{number} · {}", row.brief());
                }
            }
        }
        if self.not_loaded_generation != Some(model.generation()) {
            self.not_loaded_generation = Some(model.generation());
            self.not_loaded_label.clear();
            let _ = write!(
                self.not_loaded_label,
                "Not loaded ({})###not-loaded",
                model.counts().shown_unloaded()
            );
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
        eprintln!("[panel] ram={} bots={}", view.ram().text(), view.bots());
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

#[derive(Debug)]
pub struct PanelArgs {
    pub mode: RunMode,
    pub profile: frontend_core::launch::ProfileArgs,
    /// Dedicated catalog_watch proof bridge; ordinary panel-play stays false.
    pub catalog_core: bool,
    /// Dedicated pair_watch proof bridge; ordinary panel-play stays false.
    pub pair_core: bool,
    #[cfg(feature = "js")]
    /// Dedicated external_watch proof bridge; ordinary panel-play stays false.
    pub external_core: bool,
    #[cfg(feature = "js")]
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
    let (profile, rest) = frontend_core::launch::parse_profile_args(args)
        .map_err(|msg| (2, format!("panel-play: {msg}")))?;
    let (nav_paints, rest) = parse_nav_paints(rest)?;
    #[cfg(feature = "js")]
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
        #[cfg(feature = "js")]
        external_core: false,
        #[cfg(feature = "js")]
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
        if frontend_core::launch::is_removed_passphrase_flag(&arg) {
            return Err((
                2,
                frontend_core::launch::removed_passphrase_flag_error("panel-play"),
            ));
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
                if frontend_core::live::Scenario::get(&name).is_none() {
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

#[cfg(feature = "js")]
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
    format!("manual-{}", frontend_core::live::shot_stamp_utc(now))
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
    let json = state.session.core.with_slot_snapshot(&focused, |snapshot| {
        if !snapshot.ingame() || snapshot.tile().is_none() {
            return Err(format!(
                "focused slot {focused} has no live player snapshot"
            ));
        }
        serde_json::to_string_pretty(snapshot)
            .map_err(|error| format!("snapshot serialization failed: {error}"))
    });
    let json = match json {
        None => {
            eprintln!("[panel] F12: focused slot {focused} has no live snapshot");
            return;
        }
        Some(Err(error)) => {
            eprintln!("[panel] F12: {error}");
            return;
        }
        Some(Ok(json)) => json,
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
            #[cfg(feature = "js")]
            let external_loader = n == frontend_core::live::external_loader_scenario_name();
            #[cfg(not(feature = "js"))]
            let external_loader = false;
            external_loader || frontend_core::live::Scenario::is_registered(n)
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
fn focused_paint(session: &Session) -> Option<Option<PaintFrame>> {
    let name = session.core.selected()?;
    if !session.core.has_status(name) {
        return None;
    }
    Some(session.core.script_paint(name))
}

/// Draw the focused slot's script paint over the Game Image and apply
/// its hit, if any. Returns whether the pointer is inside the drawn
/// paint panel this frame. The panes give that to the mouse gate,
/// which decides where a press that starts now belongs: a press on
/// the paint never also clicks the game under it. A release follows
/// the press that started it, not this frame's pointer place.
fn overlay_script_paint(
    ui: &Ui,
    gpu: &mut Gpu,
    state: &mut PanelState,
    paint: Option<Option<PaintFrame>>,
    min: [f32; 2],
    size: [f32; 2],
) -> bool {
    match paint {
        Some(paint) => {
            if let Some((hit, generation)) = state.paint.frame(
                ui,
                Some(gpu),
                paint.as_ref().map(PaintFrame::view),
                min,
                size,
            ) {
                match hit {
                    crate::paint::PaintFrameHit::Button(index) => {
                        if let Some(frame) = paint.as_ref() {
                            state.session.script_paint_activate(frame, index);
                        }
                    }
                    crate::paint::PaintFrameHit::Select { key, name } => {
                        state.session.script_paint_select(&key, &name, generation);
                    }
                }
            }
            state.paint.pointer_inside_panel()
        }
        None => {
            state.paint.release_canvas(gpu);
            false
        }
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
const PANEL_TAB_WINDOWS: &[&str] = &[
    "Profiles",
    "General config",
    "Nav config",
    "Script prefs",
    "Log",
    "Debug",
];

fn dock_panel_tabs(ui: &Ui, panel: Id) {
    DockBuilder::dock_window(ui, PANEL_WINDOW, panel);
    for &title in PANEL_TAB_WINDOWS {
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
        let grid = !state.session.walkto_open && state.session.multibox && state.session.wall.grid;
        if !grid {
            state.queue_cards.clear();
        }
        if state.session.walkto_open {
            picker::draw_picker(ui, Some(gpu), &mut state.session, &mut state.walk_map);
            let (capture_enabled, capture_keys) = {
                let focus = state.session.focus.lock().unwrap();
                (focus.capture, should_capture(&focus))
            };
            let slot = state.session.core.selected();
            capture_game_mouse(
                ui,
                &state.session.core,
                &mut state.session.map_desk,
                &mut state.keyboard,
                &mut state.mouse_gate,
                MouseCaptureView {
                    slot,
                    image_min: [0.0; 2],
                    image_size: [0.0; 2],
                    image_hovered: false,
                    pointer_in_paint: false,
                    capture_enabled,
                    capture_keys,
                },
            );
        } else if grid {
            grid_pane(ui, gpu, state, avail);
        } else {
            game_pane(ui, gpu, state, avail);
        }
    });
    let open = built.is_some();
    state.session.set_game_pane_open(open);
    if !open {
        state.mouse_gate.clear();
    }
}

struct MouseCaptureView<'a> {
    slot: Option<&'a str>,
    image_min: [f32; 2],
    image_size: [f32; 2],
    image_hovered: bool,
    pointer_in_paint: bool,
    capture_enabled: bool,
    capture_keys: bool,
}

/// Capture button edges for the shared Game panes.
fn capture_game_mouse(
    ui: &Ui,
    core: &frontend_core::OperatorSession<crate::session::SlotIo>,
    map_desk: &mut frontend_core::MapDesk,
    keyboard: &mut KeyboardOwner,
    mouse_gate: &mut PaintMouseGate,
    view: MouseCaptureView<'_>,
) {
    let MouseCaptureView {
        slot,
        image_min,
        image_size,
        image_hovered,
        pointer_in_paint,
        capture_enabled,
        capture_keys,
    } = view;
    if !capture_enabled {
        mouse_gate.clear();
        return;
    }

    let mouse = ui.io().mouse_pos();
    let buttons = MouseButtons {
        left_down: ui.is_mouse_clicked(MouseButton::Left),
        right_down: ui.is_mouse_clicked(MouseButton::Right),
        left_up: ui.is_mouse_released(MouseButton::Left),
        right_up: ui.is_mouse_released(MouseButton::Right),
    };
    let applet_point = if image_hovered {
        map_image_to_applet(
            mouse[0] - image_min[0],
            mouse[1] - image_min[1],
            image_size[0],
            image_size[1],
        )
    } else {
        None
    };
    if image_hovered {
        stream_capture(
            map_desk,
            mouse[0] - image_min[0],
            mouse[1] - image_min[1],
            image_size[0],
            image_size[1],
            false,
            false,
            false,
            false,
            &[],
        );
    }

    let input = slot
        .and_then(|name| core.slot_io(name))
        .map(|slot| Arc::clone(&slot.input));
    let is_new_game_press = image_hovered
        && !pointer_in_paint
        && input.is_some()
        && (buttons.left_down || buttons.right_down);
    let sender = if is_new_game_press {
        map_desk.duplicate_input_sender()
    } else {
        None
    };
    let target = slot
        .zip(input.as_ref())
        .map(|(slot, input)| MouseTarget { slot, input });
    let frame = MouseFrame {
        target,
        sender,
        image_hovered,
        pointer_in_paint,
        applet_point,
        buttons,
    };
    mouse_gate.route(map_desk, frame, |name, input| {
        core.slot_io(name)
            .is_some_and(|slot| Arc::ptr_eq(&slot.input, input))
    });

    if capture_keys && image_hovered {
        keyboard.capture_keys(ui, map_desk);
    }
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
    let (draw, capture, capture_enabled) = {
        let focus = state.session.focus.lock().unwrap();
        (should_draw(&focus), should_capture(&focus), focus.capture)
    };
    let name = state.session.focused_name().unwrap_or_default();
    let mut image_min = [0.0; 2];
    let mut image_hovered = false;
    let mut pointer_in_paint = false;
    // A head the render service refused (its cap is full) draws nothing:
    // say so in this cell instead of showing a stale picture.
    let refused = draw
        .then(|| state.session.focused_pixels())
        .flatten()
        .and_then(|buf| buf.refusal());
    if let Some(refusal) = refused {
        ui.text_disabled(format!("renderer refused: {refusal}"));
        state.paint.release_canvas(gpu);
    } else if draw {
        let buf = state.session.focused_pixels();
        // One consumer per `FrameBuf`: `take` moves the stored frame out
        // (a `FrameOutput::Texture` hands its view off once, no Clone),
        // and `present` routes either variant — the `Texture` arm binds
        // the client's frame view directly (the shared-device seam, no
        // read-back), the `PixMap` arm uploads the CPU pixels. `take`
        // returning `None` is "no new frame since the last upload", the
        // same skip the old snapshot path gave.
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
        // The capture gate and anchor belong to the Image item. Read
        // them now: after the overlays the last submitted item is no
        // longer guaranteed to be the Image.
        let min = ui.item_rect_min();
        image_min = min;
        image_hovered = ui.is_item_hovered();
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
        pointer_in_paint = overlay_script_paint(ui, gpu, state, paint, min, size);
    } else {
        ui.text_disabled("renderer off");
        state.paint.release_canvas(gpu);
    }
    capture_game_mouse(
        ui,
        &state.session.core,
        &mut state.session.map_desk,
        &mut state.keyboard,
        &mut state.mouse_gate,
        MouseCaptureView {
            slot: (!name.is_empty()).then_some(name.as_str()),
            image_min,
            image_size: size,
            image_hovered,
            pointer_in_paint,
            capture_enabled,
            capture_keys: capture,
        },
    );
}

/// MultiBox grid-mode Game pane: same cap as the sidecar (dot, name +
/// brief, fold, ✗) over a bigger blit. Status stays visible with the
/// 330px panel collapsed. `only_render_selected` / fold hide the blit
/// only. Capture reaches the focused cell's body.
fn grid_pane(ui: &Ui, gpu: &mut Gpu, state: &mut PanelState, avail: [f32; 2]) {
    state.sync_fleet();
    let shown = state.session.fleet_model.counts().shown_loaded();
    let (capture, capture_enabled, only_selected) = {
        let focus = state.session.focus.lock().unwrap();
        (
            should_capture(&focus),
            focus.capture,
            focus.only_render_selected,
        )
    };
    if shown == 0 {
        state.queue_cards.clear();
        let loaded = state
            .session
            .fleet_model
            .rows()
            .iter()
            .any(|row| row.loaded());
        ui.text_disabled(if loaded {
            "no wall member matches the Fleet filter"
        } else {
            "no wall members"
        });
        state.paint.release_canvas(gpu);
        // No cell paints: free the tile views the cells held (the rail,
        // which also disposes them, is not drawn in Grid).
        let model = &state.session.fleet_model;
        dispose_idle_views(&mut state.views, gpu, |name| tile_shown(model, name));
        // A filtered Grid can hide its cells while a focused game press remains held.
        capture_game_mouse(
            ui,
            &state.session.core,
            &mut state.session.map_desk,
            &mut state.keyboard,
            &mut state.mouse_gate,
            MouseCaptureView {
                slot: state.session.core.selected(),
                image_min: [0.0; 2],
                image_size: [0.0; 2],
                image_hovered: false,
                pointer_in_paint: false,
                capture_enabled,
                capture_keys: capture,
            },
        );
        return;
    }
    // The cells read the model's rows while their clicks need the session:
    // lend the rows out for the frame (no allocation) and apply the one
    // clicked action once they are back.
    let model = std::mem::take(&mut state.session.fleet_model);
    let labels = std::mem::take(&mut state.fleet_labels);
    let cells = grid_cells(shown, avail);
    {
        let focus = state.session.focus.lock().unwrap();
        dispose_idle_views(&mut state.views, gpu, |name| {
            tile_shown(&model, name) && draw_for_slot(&focus, name)
        });
    }
    let debug_actions = state.session.debug_ui();
    let switch_world = state.session.can_switch_world();
    // Only the focused cell paints the script overlay (an `Arc`, no copy).
    let mut focused_paint = focused_paint(&state.session);
    let mut image_min = [0.0; 2];
    let mut image_size = [0.0; 2];
    let mut image_hovered = false;
    let mut pointer_in_paint = false;
    let mut clicked = None;
    let mut target_slot = None;
    for ((index, row), [cx, cy, cw, ch]) in model
        .shown_rows()
        .filter(|(_, row)| row.loaded())
        .zip(cells)
    {
        let labels = &labels[index];
        let name = row.name();
        let (is_focused, draw) = {
            let focus = state.session.focus.lock().unwrap();
            (
                focus.focused.as_deref() == Some(name),
                draw_for_slot(&focus, name),
            )
        };
        if is_focused {
            target_slot = Some(name);
        }
        ui.set_cursor_pos([cx, cy]);
        let preview = rail_preview_open(
            name,
            is_focused,
            only_selected,
            true,
            &state.session.ui.rail_preview,
        );
        let mut marked = model.is_marked(index);
        let restart_required = state.session.scripts.has_restart_badge(name);
        let CapClicks {
            select: cap_select,
            remove: cap_remove,
            fold: cap_fold,
            world: cap_world,
        } = rail_cap(
            ui,
            row,
            labels,
            CapDrawState {
                is_focused,
                width: cw,
                preview,
                restart_required,
                debug_actions,
                switch_world,
            },
            &mut marked,
            None,
        );
        if marked != model.is_marked(index) {
            clicked = Some(TileClick::Mark(index, marked));
        }
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
            // As in the single-bot pane: the capture gate and anchor
            // belong to the cell's Image item, read before the
            // overlays draw over it.
            let cell_image_min = ui.item_rect_min();
            if is_focused {
                image_min = cell_image_min;
                image_size = size;
                image_hovered = ui.is_item_hovered();
            }
            state
                .queue_cards
                .frame(ui, name, row.queue(), cell_image_min);
            // Only the focused cell writes the paint flag: a later open
            // cell must not clear the value the capture gate reads.
            if is_focused {
                pointer_in_paint = overlay_script_paint(
                    ui,
                    gpu,
                    state,
                    focused_paint.take(),
                    cell_image_min,
                    size,
                );
            }
        } else if is_focused {
            state.paint.release_canvas(gpu);
        }
        if cap_fold {
            let next = !preview;
            state.session.ui.rail_preview.insert(name.to_string(), next);
            let _ = state
                .session
                .persist_panel_member(&["rail_preview", name], Some(serde_json::Value::Bool(next)));
        } else if cap_remove {
            clicked = Some(TileClick::Remove(index));
        } else if cap_world {
            clicked = Some(TileClick::World(index));
        } else if cap_select || body_clicked {
            clicked = Some(TileClick::Select(index));
        }
    }
    capture_game_mouse(
        ui,
        &state.session.core,
        &mut state.session.map_desk,
        &mut state.keyboard,
        &mut state.mouse_gate,
        MouseCaptureView {
            slot: target_slot,
            image_min,
            image_size,
            image_hovered,
            pointer_in_paint,
            capture_enabled,
            capture_keys: capture,
        },
    );
    state.queue_cards.end_frame();
    state.session.fleet_model = model;
    state.fleet_labels = labels;
    apply_tile_click(&mut state.session, clicked);
}

/// Whether `name` is a loaded row the fleet model shows (a rail tile or a
/// grid cell is drawn for it).
fn tile_shown(model: &frontend_core::FleetModel, name: &str) -> bool {
    model
        .shown_rows()
        .any(|(_, row)| row.loaded() && row.name() == name)
}

/// The one fleet-row click of a rail or grid frame, by model row index.
#[derive(Clone, Copy)]
enum TileClick {
    Mark(usize, bool),
    Remove(usize),
    Select(usize),
    /// A world marker: open the world picker for the row's bot.
    World(usize),
    /// A tile dropped on another: the reorder it asks for.
    Move(ReorderOp),
}

/// Apply a frame's tile click once the session holds its fleet model again.
fn apply_tile_click(session: &mut Session, clicked: Option<TileClick>) {
    let Some(clicked) = clicked else {
        return;
    };
    let index = match clicked {
        TileClick::Mark(index, _)
        | TileClick::Remove(index)
        | TileClick::Select(index)
        | TileClick::World(index) => index,
        TileClick::Move(op) => {
            session.reorder_profiles(op);
            return;
        }
    };
    let Some(name) = session
        .fleet_model
        .rows()
        .get(index)
        .map(|row| row.name().to_string())
    else {
        return;
    };
    match clicked {
        TileClick::Mark(_, marked) => {
            session.submit(OperatorCommand::SetMark(marked), Targets::Profile(&name));
        }
        TileClick::Remove(_) => session.rail_remove(&name),
        TileClick::Select(_) => session.select(&name),
        TileClick::World(_) => {
            session.open_world_picker(Some(&name));
        }
        TileClick::Move(_) => {}
    }
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
            capacity_banner(ui, session);
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
                        // The rail's own resource card is the readout while the
                        // rail is shown; with it hidden (single bot, Grid) the
                        // strip carries the same sample.
                        if !session.rail_shown() {
                            resource_section(ui, session);
                        }
                    }
                    "profile" => profile_section(ui, session),
                    "script" => script_section(ui, session),
                    "debug" => debug_section(ui, session),
                    "log" => log_section(ui, session, log_last),
                    // The rail carries the toolbar while it is shown.
                    "fleet" if !session.rail_shown() => fleet_section(ui, session),
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
    let (w, stack) = scaled_button_row_layout(ui, avail, 3);
    if !stack {
        let total = w * 3.0 + scale_px(ui, BUTTON_GAP) * 2.0;
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
    {
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
    if !stack {
        gap_line(ui);
    }
    if ui.button_with_size("Log", [w, 0.0]) {
        session.open_window(PanelWindow::Log);
    }
    ui.set_item_tooltip("open log in a panel tab");
}

#[derive(Debug, PartialEq, Eq)]
struct LoadingText {
    description: String,
    filled: String,
    empty: String,
    percent: u8,
    caption: String,
}

fn loading_text(phase: ProgressPhase, progress: &ProgressView) -> LoadingText {
    const CELLS: u64 = 20;
    let completed = progress.completed().min(progress.total());
    let percent = if progress.total() == 0 {
        100
    } else {
        ((u128::from(completed) * 100) / u128::from(progress.total())) as u8
    };
    let filled = (u64::from(percent) * CELLS / 100) as usize;
    let description = match (phase, progress.stage()) {
        (ProgressPhase::FinalChecks, ProgressStage::FinalChecks) => {
            ProgressStage::FinalChecks.description().to_string()
        }
        (ProgressPhase::FinalChecks, stage) => {
            format!("Final checks: {}", stage.description())
        }
        (ProgressPhase::Preparing, stage) => stage.description().to_string(),
    };
    let caption = match progress.unit() {
        ProgressUnit::Bytes => {
            format!(
                "{} of {} bytes checked",
                progress.completed(),
                progress.total()
            )
        }
        ProgressUnit::Files => {
            format!(
                "{} of {} files checked",
                progress.completed(),
                progress.total()
            )
        }
        ProgressUnit::Steps => format!(
            "{} of {} steps complete",
            progress.completed(),
            progress.total()
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

fn loading_banner(ui: &Ui, phase: ProgressPhase, progress: &ProgressView) {
    let text = loading_text(phase, progress);
    ui.text_colored(ACCENT, &text.description);
    ui.text_colored(ACCENT, format!("[{}", text.filled));
    ui.same_line_with_spacing(0.0, 0.0);
    ui.text_disabled(format!("{}] {:>3}%", text.empty, text.percent));
    ui.text_disabled(text.caption);
}

fn capacity_banner(ui: &Ui, session: &Session) {
    if let Some(notice) = session.core.capacity_notice() {
        ui.text_colored(WARN, notice.notice_text());
    }
}

/// The top lines: a panel message first, then the focused bot's current
/// login notice, then (with no panel message) startup progress or the
/// bot's live state. The notice outlives the retry's state changes, so the
/// state line shows under it; when the state line says the same failure,
/// the notice says it once.
fn banner(ui: &Ui, session: &Session, progress: Option<StartupProgressView>) {
    let detail = session.core.fleet_view().detail();
    if let Some(err) = &session.error {
        ui.text_colored(ERROR, err);
    }
    if let Some(notice) = detail.and_then(|detail| detail.row().login_notice()) {
        ui.text_colored(ERROR, sentence_case(notice));
    }
    if session.error.is_some() {
        return;
    }
    if session.profile_preparing() {
        if let Some(progress) = progress {
            loading_banner(ui, progress.phase, &progress.progress);
        } else {
            loading_banner(
                ui,
                ProgressPhase::Preparing,
                &ProgressView::steps(ProgressStage::SelectingServerProfile, 0, 1),
            );
        }
    } else if let Some(detail) = detail {
        // An error state line that says the notice's failure is not repeated.
        if detail.row().login_notice() != Some(detail.state()) {
            slot_banner(ui, detail);
        }
    }
}

/// The selected bot's state line while it is on its way in (with an
/// elapsed timer when one applies), parked, or failing. Nothing while it is
/// in game or not started.
fn slot_banner(ui: &Ui, detail: &frontend_core::SlotDetail) {
    let phase = detail.row().phase();
    if matches!(phase, Phase::Ready | Phase::Offline) {
        return;
    }
    let text = sentence_case(detail.state());
    if phase.is_error() {
        ui.text_colored(ERROR, &text);
    } else if let Some(since) = detail.since() {
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
            .entry(user.clone())
            .or_default()
            .insert(id.to_string(), !open);
        let _ = session.persist_panel_member(
            &["collapsed", user.as_str(), id],
            Some(serde_json::Value::Bool(!open)),
        );
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
                    let _ = session.persist_panel_serializable(&["section_order"], &order);
                    session.ui.section_order = order;
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

#[cfg(test)]
std::thread_local! {
    /// Resource readouts drawn so far on this test thread.
    static TEST_RESOURCE_READOUTS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

fn draw_resource_rows(ui: &Ui, view: &ResourceView, show_background: bool) {
    #[cfg(test)]
    TEST_RESOURCE_READOUTS.set(TEST_RESOURCE_READOUTS.get() + 1);
    kv_row(ui, "bots", &format_bots(view.bots(), view.ingame()));
    if show_background && view.background() > 0 {
        kv_row(ui, "background", &format_background(view.background()));
    }
    kv_row(ui, "cpu", view.cpu().text());
    kv_row(ui, "ram", view.ram().text());
    kv_row(ui, "traffic", view.traffic().text());
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
    vault_pass_button(ui, session, exists);
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

/// The prompt's **Unlock vault** (a vault file exists) or **Create vault**
/// button, the full content width. A passphrase the command would refuse
/// greys it out, with a tooltip that says what to type: a greyed-out button
/// cannot be clicked, so no click is dropped without a refusal.
fn vault_pass_button(ui: &Ui, session: &mut Session, exists: bool) {
    let w = ui.content_region_avail()[0];
    let label = if exists {
        "Unlock vault"
    } else {
        "Create vault"
    };
    let usable = vault_pass_usable(&session.pass_scratch, exists);
    {
        let _off = (!usable).then(|| ui.begin_disabled());
        if ui.button_with_size(label, [w, 0.0]) {
            submit_vault_pass(session);
        }
    }
    if !usable {
        ui.set_item_tooltip(if exists {
            "type the vault passphrase"
        } else {
            "type a passphrase that is not only spaces"
        });
    }
}

/// Whether the vault prompt's button can send `typed`: Unlock takes the
/// passphrase exactly as typed (a vault made in the terminal may begin or
/// end with spaces) and needs some text; Create keeps the vault rule
/// (non-empty once trimmed). The prompt greys its button out otherwise.
fn vault_pass_usable(typed: &str, exists: bool) -> bool {
    if exists {
        !typed.is_empty()
    } else {
        !typed.trim().is_empty()
    }
}

/// Send the typed passphrase as is, for Unlock or Create; both front ends
/// store the typed text. Only an enabled button calls this
/// ([`vault_pass_usable`]).
fn submit_vault_pass(session: &mut Session) {
    let pass = Passphrase::from(session.pass_scratch.as_str());
    session.request_unlock(pass);
    session.pass_scratch.clear();
}

/// profile: orange current name (like script) plus Profiles (opens the
/// picker). Unlock prompt until the vault is open. Sits under WalkTo /
/// config, above debug.
fn profile_section(ui: &Ui, session: &mut Session) {
    if !section_open(ui, session, "profile") {
        return;
    }
    if !session.core.vault_open() {
        vault_unlock_prompt(ui, session);
        return;
    }
    profile_combo(ui, session);
    let w = ui.content_region_avail()[0];
    if ui.button_with_size("Profiles", [w, 0.0]) {
        session.open_window(PanelWindow::Profiles);
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

fn logout_enabled(vault_open: bool, eligibility: Option<LogoutEligibility>) -> bool {
    vault_open && eligibility.is_some_and(LogoutEligibility::eligible)
}

/// Log in / Log out above WalkTo. Always drawn; disabled while the vault
/// is locked or no profile is focused. Log out follows the shared
/// projection ([`LogoutEligibility`], also Fleet's and the TUI's): a
/// connected slot, a pending or retrying login, a queued place or a
/// repeat-guard park. An unloaded or logged-out profile cannot be latched
/// accidentally.
fn login_logout_row(ui: &Ui, session: &mut Session) {
    let avail = ui.content_region_avail()[0];
    let cells = scaled_button_cells(ui, avail, 2);
    let vault_open = session.core.vault_open();
    let focused = session.focused_name();
    let can_login = vault_open && focused.is_some();
    let eligibility = session.focused_logout_eligibility();
    let can_logout = logout_enabled(vault_open, eligibility);
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
        if ui.button_with_size("Log out", [cells[1].0, 0.0]) {
            if let Some(name) = focused.as_deref() {
                session.logout(name);
            }
        }
        ui.set_item_tooltip(if !vault_open {
            "unlock the vault first"
        } else if focused.is_none() {
            "pick a profile"
        } else {
            match eligibility {
                Some(LogoutEligibility::RetryingLogin) => {
                    "stop the focused slot's login retries — it stays in the picker"
                }
                Some(LogoutEligibility::Queued) => "cancel the focused slot's queued login",
                Some(LogoutEligibility::LoggingIn) => "cancel the focused slot's login",
                Some(LogoutEligibility::GuardParked) => {
                    "end the parked session — its held script work is discarded"
                }
                Some(LogoutEligibility::LoggedIn) => {
                    "log out the focused slot — it stays in the picker"
                }
                Some(LogoutEligibility::NotLoggedIn) | None => "the focused slot is not logged in",
            }
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
        session.open_window(PanelWindow::Fleet);
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

/// Local-engine debug cheats. Omitted on rs2b2t. A cheat button draws
/// disabled, its tooltip naming why, while a Debug command has no bot to go to
/// ([`Session::debug_send_target`]: no focused bot, no play session; maxme
/// follows the Debug panel's target scope), so a click always has a target
/// when it reaches [`Session::cheat_focused`] or the bulk send.
fn debug_section(ui: &Ui, session: &mut Session) {
    if !session.debug_ui() {
        return;
    }
    if !section_open(ui, session, "debug") {
        return;
    }
    let show_tutskip = session.focused_tutorial_skipped() == Some(false);
    let main = debug_main_buttons_for(session.profile_class(), show_tutskip);
    let cheat_block = session.debug_send_target().err();
    let maxme_block = crate::debug_panel::send_block(session);
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
                    session.open_window(PanelWindow::Debug);
                }
                ui.set_item_tooltip("DebugPanel v2 — full cheat catalog");
            }
            "TutSkip" => {
                let _off = cheat_block.is_some().then(|| ui.begin_disabled());
                if ui.button_with_size(caption, [w, 0.0]) {
                    session.cheat_focused("setvar tutorial 1000");
                    session.mark_tutorial_skipped();
                }
                ui.set_item_tooltip(cheat_block.unwrap_or("setvar tutorial 1000"));
            }
            "Lumbridge" => {
                let _off = cheat_block.is_some().then(|| ui.begin_disabled());
                if ui.button_with_size(caption, [w, 0.0]) {
                    session.cheat_focused("~home");
                }
                ui.set_item_tooltip(cheat_block.unwrap_or("~home — Lumbridge courtyard"));
            }
            "maxme" => {
                let _off = maxme_block.is_some().then(|| ui.begin_disabled());
                if ui.button_with_size(caption, [w, 0.0]) {
                    session.open_window(PanelWindow::Debug);
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
                ui.set_item_tooltip(
                    maxme_block.unwrap_or("19× setstat 99 — confirmation is required"),
                );
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
        // The destinations are rail cheats like Lumbridge: disabled, the
        // tooltip naming why, while no bot can take one.
        let cheat_block = session.debug_send_target().err();
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
            let _off = cheat_block.is_some().then(|| ui.begin_disabled());
            if ui.button_with_size(dest.label, [bw, 0.0]) {
                session.cheat_focused(dest.cheat);
                ui.close_current_popup();
            }
            ui.set_item_tooltip(cheat_block.unwrap_or(dest.tooltip));
        }
    });
}

/// script: native Browse/Start/Pause/Stop controls with the rs2b0t disable rules.
/// `active` = Running|Paused|Stopping: Start and Browse are disabled while
/// a script holds the slot; Pause/Resume is enabled only while Running or
/// Paused (label switches to "Resume"); Stop is enabled while active but
/// not already Stopping. JS builds also expose Load and Reload.
fn script_section(ui: &Ui, session: &mut Session) {
    if !section_open(ui, session, "script") {
        return;
    }
    let state = session.focused_script_state();
    let active = script_active(state);
    let paused = state == ScriptRunState::Paused;

    let name = session
        .script_sel
        .as_ref()
        .map(|sel| {
            #[cfg(feature = "js")]
            {
                match sel.loaded_card() {
                    Some((source, identity)) => session
                        .scripts
                        .card(source, identity)
                        .map(|card| card.name().to_owned())
                        .unwrap_or_else(|| {
                            std::path::Path::new(identity)
                                .file_name()
                                .map(|name| name.to_string_lossy().into_owned())
                                .unwrap_or_else(|| identity.to_owned())
                        }),
                    None => sel.label().to_owned(),
                }
            }
            #[cfg(not(feature = "js"))]
            {
                sel.label().to_owned()
            }
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
    #[cfg(feature = "js")]
    if let (Some((source, sel_name)), Some(front)) = (
        session
            .script_sel
            .as_ref()
            .and_then(ScriptChoice::loaded_card),
        session.transpile_front(),
    ) {
        if let Some(line) = card_transpile_label(
            Some(front),
            source,
            sel_name,
            session.transpile_progress().0,
            session.transpile_progress().1,
        ) {
            ui.same_line();
            ui.text_disabled(line);
        }
    }
    script_parameter_draft_recovery(ui, session);
    let avail = ui.content_region_avail()[0];
    #[cfg(feature = "js")]
    let (w, stack) = scaled_button_row_layout(ui, avail, 3);
    #[cfg(not(feature = "js"))]
    let (w, _stack) = scaled_button_row_layout(ui, avail, 1);
    {
        let _browse = if active {
            Some(ui.begin_disabled())
        } else {
            None
        };
        if ui.button_with_size("Browse…", [w, 0.0]) {
            session.open_window(PanelWindow::Browse);
            #[cfg(feature = "js")]
            session.on_script_browse_open();
        }
        #[cfg(feature = "js")]
        ui.set_item_tooltip("pick a compiled script or a loaded JS bot");
        #[cfg(not(feature = "js"))]
        ui.set_item_tooltip("pick a compiled script");
    }
    #[cfg(feature = "js")]
    {
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
        let pending = session.reload_validation_pending();
        {
            let _pending = pending.then(|| ui.begin_disabled());
            let label = if pending {
                "Reloading…"
            } else if confirming {
                "Confirm"
            } else {
                "Reload"
            };
            if ui.button_with_size(label, [w, 0.0]) {
                session.begin_script_reload_clicked();
            }
        }
        ui.set_item_tooltip(if pending {
            "preparing and validating candidate script off the UI thread"
        } else if confirming {
            "confirm reload: restart listed running bots and stop listed paused bots"
        } else {
            "hash source and supported siblings; unchanged skips transpile"
        });
        if (confirming || pending) && ui.button_with_size("Cancel reload", [avail, 0.0]) {
            session.cancel_reload();
        }
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
            .is_some_and(|name| session.scripts.start_waiting(name));
        let _stop = if script_stop_enabled(state) || queued {
            None
        } else {
            Some(ui.begin_disabled())
        };
        if ui.button_with_size("Stop", [sw, 0.0]) {
            session.script_stop();
        }
    }
    let native = session
        .core
        .fleet_view()
        .detail()
        .and_then(|detail| detail.native_status());
    let status = frontend_core::views::script_status_label(state, native);
    if let Some(native) = native {
        kv_row(ui, "status", status);
        if let Some(reason) = frontend_core::views::script_status_reason(native) {
            ui.text_wrapped(reason);
        }
        for (label, value) in frontend_core::views::script_status_rows(native) {
            kv_row(ui, label, value);
        }
    } else {
        match session.focused_script_last_error() {
            Some(err) => kv_row(ui, "status", &format!("{status}: {err}")),
            None => kv_row(ui, "status", status),
        }
    }
    if let Some(reason) = session
        .core
        .fleet_view()
        .detail()
        .and_then(|detail| detail.walk_risk())
    {
        kv_row(ui, "Walk risk", reason);
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
                    let _ = session.persist_panel_serializable(&["script_category_order"], &order);
                    session.ui.script_category_order = order;
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
) -> Option<ScriptChoice> {
    let selection = card.selection();
    let selected = session.script_sel.as_ref() == Some(&selection);
    let id = match card.compiled_id() {
        Some(id) => format!("##scard-compiled-{}", id.as_str()),
        #[cfg(feature = "js")]
        None => card
            .loaded()
            .map(|loaded| format!("##scard-{}", loaded.identity_key()))
            .unwrap_or_else(|| format!("##scard-{}", card.name())),
        #[cfg(not(feature = "js"))]
        None => format!("##scard-{}", card.name()),
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
            #[cfg(feature = "js")]
            let title_color = if card.unloadable().is_some() {
                TEXT_DIM
            } else {
                ACCENT
            };
            #[cfg(not(feature = "js"))]
            let title_color = ACCENT;
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
            #[cfg(feature = "js")]
            if let Some(loaded) = card.loaded() {
                if let Some(line) = card_transpile_label(
                    session.transpile_front(),
                    loaded.source(),
                    loaded.name(),
                    session.transpile_progress().0,
                    session.transpile_progress().1,
                ) {
                    let _dim = ui.push_style_color(StyleColor::Text, TEXT_DIM);
                    ui.text_disabled(line);
                }
                if let Some(failure) = session.scripts.load_failure(&loaded.identity_key()) {
                    ui.text_colored(ERROR, format!("failed {}", failure.stage().as_str()));
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

fn browse_card_grid(ui: &Ui, session: &Session, cards: &[&BrowseCard<'_>]) -> Option<ScriptChoice> {
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
    #[cfg(feature = "js")]
    {
        let w = ui.content_region_avail()[0];
        let pending = session.reload_validation_pending();
        let label = if pending {
            "Refreshing catalog…"
        } else if session.scripts.catalog_refresh_confirm() {
            "Confirm catalog reload"
        } else {
            "Refresh catalog"
        };
        {
            let _pending = pending.then(|| ui.begin_disabled());
            if ui.button_with_size(label, [w, 0.0]) {
                session.begin_refresh_catalog();
            }
        }
        if (session.scripts.catalog_refresh_confirm() || pending)
            && ui.button_with_size("Cancel reload", [w, 0.0])
        {
            session.cancel_reload();
        }
        ui.spacing();
        if frontend_core::scripts::paths::rs2b0t_import_deferred() {
            if ui.button_with_size("Import catalog…", [w, 0.0]) {
                session.open_rs2b0t_catalog_picker(false);
            }
            ui.spacing();
        }
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
        let _ = session.persist_panel_serializable(&["script_category_order"], &order);
    }
    if !order.is_empty() {
        script_category_chips(ui, session, &order);
        ui.spacing();
    }
    #[cfg(feature = "js")]
    {
        let busy = session.transpile_front().is_some();
        let need = session.scripts.cards_needing_transpile().len();
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
                        session.transpile_progress().0,
                        session.transpile_progress().1,
                    ) {
                        ui.text_disabled(line);
                    }
                }
            }
            ui.spacing();
        }
        let named_failures = session.scripts.named_failure_output();
        if !named_failures.is_empty() {
            // Collapsed by default: the list is long (dim catalog cards are
            // expected misses) and pushes the script browser off screen.
            let red = ui.push_style_color(StyleColor::Text, ERROR);
            let open = ui.collapsing_header(
                format!(
                    "{} failed###script-failures",
                    session.scripts.load_failure_count()
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
        focus_if_requested(ui, session, PanelWindow::Browse);
        ui.window(PanelWindow::Browse.title())
            .opened(&mut open)
            .flags(WindowFlags::NO_COLLAPSE | WindowFlags::NO_SCROLLBAR)
            .position(pos, Condition::FirstUseEver)
            .size(size, Condition::FirstUseEver)
            .build(|| browse_window_body(ui, session));
        session.script_browse_open = open;
    }
    // A newly opened picker must appear above the Browse window that requested it.
    #[cfg(feature = "js")]
    file_dialog_windows(ui, session);
}

#[cfg(feature = "js")]
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
        match mode {
            DialogMode::File => {
                let path = session.script_load_dir.clone();
                let _ = session.persist_panel_serializable(&["script_load_last_dir"], &path);
            }
            DialogMode::Folder => {
                let path = session.rs2b0t_catalog_dir.clone();
                let _ = session.persist_panel_serializable(&["script_catalog_last_dir"], &path);
            }
        }
    }
}

#[cfg(feature = "js")]
fn file_dialog_windows(ui: &Ui, session: &mut Session) {
    let size = scale_size(ui, [FILE_DIALOG_FIRST_W, FILE_DIALOG_FIRST_H]);
    let pos = overlay_spawn_pos(ui, session, size);
    if session.script_load_open {
        let mut open = true;
        focus_if_requested(ui, session, PanelWindow::LoadScript);
        ui.window(PanelWindow::LoadScript.title())
            .opened(&mut open)
            .flags(WindowFlags::NO_COLLAPSE)
            .position(pos, Condition::FirstUseEver)
            .size(size, Condition::FirstUseEver)
            .build(|| file_dialog_body(ui, session, DialogMode::File));
        session.script_load_open &= open;
    }
    if session.rs2b0t_catalog_open {
        let mut open = true;
        focus_if_requested(ui, session, PanelWindow::ImportCatalog);
        ui.window(PanelWindow::ImportCatalog.title())
            .opened(&mut open)
            .flags(WindowFlags::NO_COLLAPSE)
            .position(pos, Condition::FirstUseEver)
            .size(size, Condition::FirstUseEver)
            .build(|| file_dialog_body(ui, session, DialogMode::Folder));
        session.rs2b0t_catalog_open &= open;
    }
}

#[cfg(feature = "js")]
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
    let home = frontend_core::scripts::paths::bot_home();
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
                    let label = format!("{glyph}  {}##fdrow{i}", row.name());
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
                                DialogMode::File => session.script_load_dir.push(row.name()),
                                DialogMode::Folder => session.rs2b0t_catalog_dir.push(row.name()),
                            }
                            persist_dialog_cwd(session, mode);
                        } else if mode == DialogMode::File {
                            session.load_js(&cwd.join(row.name()));
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
                            session.load_js(&cwd.join(row.name()));
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

/// Routing group of the Nav config window: the global routing permissions and
/// the Danger routing drop-down. The drop-down spans the full width so its
/// level names never clip at the docked 330 px panel width; the held middle
/// level explains itself in a wrapped note under the control. Returns whether
/// a setting changed.
fn routing_group(ui: &Ui, nav: &mut crate::nav_settings::NavSettings) -> bool {
    let mut changed = false;
    ui.text_colored(ACCENT, "Routing");
    {
        let _dim = ui.push_style_color(StyleColor::Text, TEXT_DIM);
        ui.text_wrapped(frontend_core::ROUTING_SCOPE_NOTE);
    }
    #[cfg(test)]
    record_test_routing_item(ui);
    if ui.checkbox(
        frontend_core::GLOBAL_PERMISSION_LABELS[0].1,
        &mut nav.allow_teleports,
    ) {
        changed = true;
    }
    #[cfg(test)]
    record_test_routing_item(ui);
    ui.set_item_tooltip("Global routing permission for every walk.");
    if ui.checkbox(
        frontend_core::GLOBAL_PERMISSION_LABELS[1].1,
        &mut nav.allow_wilderness,
    ) {
        changed = true;
    }
    #[cfg(test)]
    record_test_routing_item(ui);
    ui.set_item_tooltip(
        "Global routing permission. rs2b0t-compatible scripts always allow wilderness and bank fetch.",
    );
    if ui.checkbox(
        frontend_core::GLOBAL_PERMISSION_LABELS[2].1,
        &mut nav.allow_bank_fetch,
    ) {
        changed = true;
    }
    #[cfg(test)]
    record_test_routing_item(ui);
    ui.set_item_tooltip(
        "BankBudget fetch is available to manual WalkTo only. rs2b0t-compatible scripts always allow wilderness and bank fetch.",
    );
    ui.same_line();
    ui.text_disabled(frontend_core::BANK_FETCH_PERMISSION_SCOPE);
    #[cfg(test)]
    record_test_routing_item(ui);
    let danger_level = nav.danger_level();
    ui.text(frontend_core::GLOBAL_PERMISSION_LABELS[3].1);
    #[cfg(test)]
    record_test_routing_item(ui);
    ui.set_next_item_width(-1.0);
    let danger_combo = ui.begin_combo("##danger-routing", danger_level.label());
    #[cfg(test)]
    record_test_routing_item(ui);
    ui.set_item_tooltip(match danger_level {
        frontend_core::DangerLevel::Never => {
            "Danger-zone routes are refused unless this walk or its script overrides."
        }
        frontend_core::DangerLevel::WhenSurvivable => {
            frontend_core::walk_permissions::survivable_routing_tooltip()
        }
        frontend_core::DangerLevel::Always => frontend_core::GLOBAL_DANGER_WARNING,
    });
    if let Some(_open) = danger_combo {
        for level in frontend_core::walk_permissions::DANGER_ROUTING_LEVELS {
            let selected = level == danger_level;
            let picked = ui
                .selectable_config(level.label())
                .selected(selected)
                .build();
            #[cfg(test)]
            record_test_routing_choice(ui);
            if picked {
                changed |= choose_danger_level(nav, level);
            }
            if selected {
                ui.set_item_default_focus();
            }
        }
    }
    if frontend_core::walk_permissions::danger_routing_held(danger_level) {
        let _dim = ui.push_style_color(StyleColor::Text, TEXT_DIM);
        ui.text_wrapped(frontend_core::walk_permissions::WHEN_SURVIVABLE_HELD_NOTE);
        #[cfg(test)]
        record_test_routing_item(ui);
    }
    if danger_level == frontend_core::DangerLevel::Always {
        ui.text_colored(ERROR, frontend_core::GLOBAL_DANGER_WARNING);
        #[cfg(test)]
        record_test_routing_item(ui);
    }
    changed
}

/// Stores a level picked from the Danger routing drop-down. Returns whether
/// the stored level changed; the caller persists the Nav config as before.
fn choose_danger_level(
    nav: &mut crate::nav_settings::NavSettings,
    level: frontend_core::DangerLevel,
) -> bool {
    if nav.danger_level() == level {
        return false;
    }
    nav.set_danger_level(level);
    true
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
    session.refresh_walk_permissions();
    let mut open = true;
    let panel_class = panel_window_class();
    ui.set_next_window_class(&panel_class);
    if let Some(id) = panel_dock {
        ui.set_next_window_dock_id_with_cond(id, Condition::FirstUseEver);
    }
    focus_if_requested(ui, session, PanelWindow::NavConfig);
    ui.window(PanelWindow::NavConfig.title())
        .opened(&mut open)
        .flags(WindowFlags::NO_COLLAPSE)
        .size(scale_size(ui, [360.0, 480.0]), Condition::FirstUseEver)
        .build(|| {
            if !session.panel_preferences_ready() {
                ui.text_disabled("Loading preferences…");
            } else if session.preferences_pending() {
                ui.text_disabled("Saving preferences…");
            }
            let previous_nav = session.ui.nav.clone();
            let mut nav = previous_nav.clone();
            let mut changed = false;
            changed |= routing_group(ui, &mut nav);
            if !nav.script_scope_notice_ack {
                ui.separator();
                ui.text_wrapped(frontend_core::SCRIPT_SCOPE_NOTICE);
                if ui.button("Dismiss walk permissions notice") {
                    nav.script_scope_notice_ack = true;
                    changed = true;
                }
            }

            if !nav.survivable_routing_notice_ack {
                ui.separator();
                ui.text_wrapped(frontend_core::walk_permissions::survivable_routing_notice());
                if ui.button("Dismiss danger-routing notice") {
                    nav.survivable_routing_notice_ack = true;
                    changed = true;
                }
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
            let mut ask =
                session.map_desk.bake_gate().choice() == frontend_core::MapBakeChoice::Ask;
            if ui.checkbox("ask before baking terrain", &mut ask) {
                session.set_map_bake_choice(if ask {
                    frontend_core::MapBakeChoice::Ask
                } else {
                    frontend_core::MapBakeChoice::Always
                });
            }

            if changed {
                let result = commit_nav_settings(session, &previous_nav, nav);
                match result {
                    Ok(()) => {
                        if session
                            .error
                            .as_deref()
                            .is_some_and(|error| error.starts_with("Nav config: "))
                        {
                            session.error = None;
                        }
                        session.refresh_walk_permissions();
                    }
                    Err(error) => {
                        session.error = Some(format!("Nav config: {error}"));
                    }
                }
            }
        });
    session.nav_settings_open = open;
}

/// Commit an edited Nav config: every operator preference that changed
/// is its own command (`SetNavPreference`, `SetDangerLevel`); panel-owned
/// presentation values submit only their changed members.
fn commit_nav_settings(
    session: &mut Session,
    before: &crate::nav_settings::NavSettings,
    after: crate::nav_settings::NavSettings,
) -> Result<(), String> {
    use frontend_core::NavPreference as P;
    let mut failure = None;
    let mut submit = |session: &mut Session, command: OperatorCommand| {
        if let Some(submitted) = session.submit(command, Targets::Application) {
            if let Some(error) = session.core.command_failure(submitted.request()) {
                failure.get_or_insert_with(|| error.to_string());
            }
        }
    };
    let flags = [
        (
            P::AllowTeleports,
            "allow_teleports",
            before.allow_teleports,
            after.allow_teleports,
        ),
        (
            P::AllowWilderness,
            "allow_wilderness",
            before.allow_wilderness,
            after.allow_wilderness,
        ),
        (
            P::AllowBankFetch,
            "allow_bank_fetch",
            before.allow_bank_fetch,
            after.allow_bank_fetch,
        ),
        (
            P::ScriptScopeNoticeAck,
            "script_scope_notice_ack",
            before.script_scope_notice_ack,
            after.script_scope_notice_ack,
        ),
        (
            P::SurvivableRoutingNoticeAck,
            "survivable_routing_notice_ack",
            before.survivable_routing_notice_ack,
            after.survivable_routing_notice_ack,
        ),
        (
            P::PauseScriptOnManualWalkAbort,
            "pause_script_on_manual_walk_abort",
            before.pause_script_on_manual_walk_abort,
            after.pause_script_on_manual_walk_abort,
        ),
        (
            P::ShowSpecialAreas,
            "show_special_areas",
            before.show_special_areas,
            after.show_special_areas,
        ),
    ];
    for (preference, key, was, now) in flags {
        if was != now {
            session.remember_nav_preference(key, now);
            submit(session, OperatorCommand::SetNavPreference(preference, now));
        }
    }
    if before.danger_level() != after.danger_level() {
        session.remember_nav_preference("allow_danger_zones", after.allow_danger_zones);
        session.remember_nav_preference("survivable_routing", after.survivable_routing);
        submit(
            session,
            OperatorCommand::SetDangerLevel(after.danger_level()),
        );
    }

    macro_rules! persist_panel_nav_member {
        ($field:ident) => {
            if before.$field != after.$field {
                if let Err(error) =
                    session.persist_panel_serializable(&["nav", stringify!($field)], &after.$field)
                {
                    failure.get_or_insert(error);
                }
            }
        };
    }
    persist_panel_nav_member!(show_nav_path);
    persist_panel_nav_member!(hop_labels);
    persist_panel_nav_member!(hop_label_px);
    persist_panel_nav_member!(color_path);
    persist_panel_nav_member!(color_transport);
    persist_panel_nav_member!(color_click);
    persist_panel_nav_member!(color_text);
    persist_panel_nav_member!(collision_fill);
    persist_panel_nav_member!(nsew_labels);
    persist_panel_nav_member!(client_trail);
    persist_panel_nav_member!(color_collision);
    persist_panel_nav_member!(color_client);
    persist_panel_nav_member!(color_client_run_alt);
    persist_panel_nav_member!(component_flood);
    persist_panel_nav_member!(camera_follow);

    session.route_through_zones = frontend_core::WalkGlobalsView::new(
        after.walk_globals(),
        after.script_scope_notice_ack,
        after.survivable_routing_notice_ack,
    )
    .danger_this_walk(session.route_through_zones);
    session.ui.nav = after;
    match failure {
        Some(error) => Err(error),
        None => Ok(()),
    }
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
    let Some(sel) = &session.script_sel else {
        return;
    };
    #[cfg(feature = "js")]
    if let Some((source, name)) = sel.loaded_card() {
        if let Some(card) = session.scripts.card(source, name) {
            let bag = session.merged_settings_bag(source, name, card.schema());
            for (label, value) in card.schema().parameter_rows(&bag) {
                kv_row(ui, &label, &value);
            }
        }
    }
    if let Some(id) = sel.compiled_id() {
        if let frontend_core::scripts::SchemaView::Ready { fields, .. } = session
            .scripts
            .compiled_schema(&session.core, session.focused_name().as_deref(), id)
        {
            match session.scripts.compiled_edit_bag(
                &session.core,
                session.focused_name().as_deref(),
                id,
            ) {
                Ok(bag) => {
                    for (label, value) in fields.parameter_rows(&bag) {
                        kv_row(ui, &label, &value);
                    }
                }
                Err(error) => ui.text_disabled(error),
            }
        }
    }
}

/// All parameter controls share the same account-scoped persistence path.
fn persist_profile_setting(
    session: &mut Session,
    selection: &ScriptChoice,
    id: &str,
    value: serde_json::Value,
) {
    if selection.compiled_id().is_some() {
        let Some(profile) = session.focused_name() else {
            session.refuse_command(
                OperatorCommand::SaveScriptSettings(frontend_core::command::CardKind::Native),
                &[],
                Some(crate::session::ErrorNeed::FocusedBot),
                "parameters: select a profile first",
            );
            return;
        };
        session
            .scripts
            .stage_setting(Some(&profile), selection, id, value);
        session.submit_script_command(
            OperatorCommand::SaveScriptSettings(frontend_core::command::CardKind::Native),
            Targets::Profile(&profile),
        );
        #[cfg(feature = "js")]
        {
            return;
        }
    }
    #[cfg(feature = "js")]
    if let Some((source, lookup)) = selection.loaded_card() {
        let Some((name, path)) = session
            .scripts
            .card(source, lookup)
            .map(|card| (card.name().to_owned(), card.path().to_path_buf()))
        else {
            return;
        };
        if let Some(profile) = session.focused_name() {
            session.set_profile_setting(&profile, source, &name, &path, id, value);
        } else {
            session.scripts.stage_setting(None, selection, id, value);
            session.submit_script_command(
                OperatorCommand::SaveLegacyScriptSettings,
                Targets::Application,
            );
        }
    }
}

fn filtered_parameter_indices<'a>(
    options: &'a frontend_core::scripts::ParameterOptions,
    query: &'a str,
) -> impl Iterator<Item = usize> + 'a {
    options
        .selectable()
        .iter()
        .enumerate()
        .filter_map(move |(index, _)| options.matches_query(index, query).then_some(index))
}

fn unavailable_parameter_text(
    label: &str,
    def: SettingView<'_>,
    bag: &serde_json::Map<String, serde_json::Value>,
    options: &frontend_core::scripts::ParameterOptions,
) -> String {
    if def.options_from() == Some("loadouts") {
        return format!("{label} (no loadouts available)");
    }
    if def.ty() == "string" && options.preserved() > 0 {
        let current = bag
            .get(def.id())
            .and_then(serde_json::Value::as_str)
            .or(def.default())
            .unwrap_or("");
        return format!(
            "{label} (options unavailable · {})",
            options.label_for(current)
        );
    }
    format!("{label} (options unavailable)")
}

fn parameter_array_editor_enabled(
    def: SettingView<'_>,
    bag: &serde_json::Map<String, serde_json::Value>,
    options: &frontend_core::scripts::ParameterOptions,
) -> bool {
    def.ty() == "string[]"
        && options.can_edit_array(
            bag.get(def.id())
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(serde_json::Value::as_str),
        )
}

fn script_parameter_text_input(
    ui: &Ui,
    session: &mut Session,
    selection: &ScriptChoice,
    profile: Option<&str>,
    def: SettingView<'_>,
    options: &[String],
    current: Option<serde_json::Value>,
) {
    script_parameter_label(ui, def, &mut session.script_parameter_label);
    let initial_text = current
        .as_ref()
        .map(format_setting_value)
        .or_else(|| def.default().map(str::to_owned))
        .unwrap_or_default();
    let changed = {
        let text = session.scripts.parameter_text_mut_for(
            profile,
            selection,
            def.id(),
            &initial_text,
            current.clone(),
        );
        ui.set_next_item_width(-1.0);
        ui.input_text(format!("##param-{id}", id = def.id()), text)
            .build()
    };
    let active = ui.is_item_active();
    let deactivated = ui.is_item_deactivated_after_edit();
    let enter = ui.is_key_pressed(Key::Enter) && (active || deactivated);
    let escape = ui.is_key_pressed(Key::Escape) && (active || deactivated);
    let commit = !escape && (deactivated || enter);
    if escape {
        let key = frontend_core::scripts::ParameterEditKey::new(profile, selection, def.id());
        session.scripts.stage_parameter_discard(key);
        session.submit_script_command(OperatorCommand::DiscardParameterText, Targets::Application);
    } else if changed || commit {
        if commit && profile.is_none() && selection.compiled_id().is_some() {
            session.refuse_command(
                OperatorCommand::SaveScriptSettings(frontend_core::command::CardKind::Native),
                &[],
                Some(crate::session::ErrorNeed::FocusedBot),
                "parameters: select a profile first",
            );
            return;
        }
        let key = frontend_core::scripts::ParameterEditKey::new(profile, selection, def.id());
        session.scripts.stage_parameter_validation(
            key,
            selection,
            &initial_text,
            current.clone(),
            def,
            options,
        );
        if !commit {
            session.submit_script_command(OperatorCommand::EditParameterText, Targets::Application);
        } else if let Some(profile) = profile {
            let kind = if selection.compiled_id().is_some() {
                frontend_core::command::CardKind::Native
            } else {
                frontend_core::command::CardKind::Script
            };
            session.submit_script_command(
                OperatorCommand::SaveScriptSettings(kind),
                Targets::Profile(profile),
            );
        } else {
            session.submit_script_command(
                OperatorCommand::SaveLegacyScriptSettings,
                Targets::Application,
            );
        }
    }
}

/// Labels and number bounds have their own wrapped rows, never the clipped
/// label area to the right of an ImGui input.
fn script_parameter_label(ui: &Ui, def: SettingView<'_>, text: &mut String) {
    use std::fmt::Write as _;
    ui.text_wrapped(def.display_label());
    if def.ty() == "number" {
        text.clear();
        let _ = match def.bounds() {
            (Some(min), Some(max), _) => write!(text, "Valid range: {min}–{max}"),
            (Some(min), None, _) => write!(text, "Minimum: {min}"),
            (None, Some(max), _) => write!(text, "Maximum: {max}"),
            (None, None, _) => return,
        };
        ui.text_wrapped(text);
    }
}

fn script_parameter_error(
    ui: &Ui,
    session: &Session,
    profile: Option<&str>,
    selection: &ScriptChoice,
    field: &str,
) {
    if let Some(error) = session
        .scripts
        .parameter_text_error_for(profile, selection, field)
    {
        let _color = ui.push_style_color(dear_imgui_rs::StyleColor::Text, ERROR);
        ui.text_wrapped(error);
    }
}

/// Hidden drafts remain recoverable after closing prefs or changing cards.
fn script_parameter_draft_recovery(ui: &Ui, session: &mut Session) {
    use std::fmt::Write as _;
    let Some(profile) = session.focused_name() else {
        return;
    };
    if !session.scripts.has_parameter_draft(&profile) {
        return;
    }
    ui.text_wrapped("Unsaved settings prevent Start. In Script prefs, press Enter to save a field or Escape to discard it.");
    for (card, field, reason) in session.scripts.parameter_draft_details(&profile) {
        session.script_parameter_label.clear();
        let _ = write!(session.script_parameter_label, "{card}: {field}: {reason}");
        ui.text_wrapped(&session.script_parameter_label);
    }
    if ui.button("Discard unsaved settings") {
        session.scripts.stage_profile_parameter_discard(&profile);
        session.submit_script_command(OperatorCommand::DiscardParameterText, Targets::Application);
    }
}

/// Script prefs window: text buffers and commit state are shared with TUI.
fn script_parameter_editors(ui: &Ui, session: &mut Session) {
    session.refresh_walk_permissions();
    let Some(selection) = session.script_sel.as_ref().map(ScriptChoice::duplicate) else {
        ui.text_wrapped("select a script with a parameter schema");
        return;
    };
    let profile = session.focused_name();
    if profile
        .as_deref()
        .is_some_and(|profile| session.scripts.restart_required_for(profile, &selection))
    {
        ui.text_disabled("Restart required to apply saved settings.");
    }
    // The editors below need `session` mutably while they walk the schema,
    // so the schema is copied into a retained buffer (unchanged schemas copy
    // nothing) and put back afterwards.
    let mut schema = std::mem::take(&mut session.script_param_schema);
    if let Some(bag) =
        script_parameter_schema(ui, session, &selection, profile.as_deref(), &mut schema)
    {
        script_parameter_rows(ui, session, &selection, profile, schema.as_ref(), bag);
    }
    session.script_param_schema = schema;
    // A new draft must not move the active field out of the visible rows.
    script_parameter_draft_recovery(ui, session);
}

/// Fill `schema` with the selected card's fields and return its bag, or say
/// why the parameters are unavailable.
fn script_parameter_schema(
    ui: &Ui,
    session: &mut Session,
    selection: &ScriptChoice,
    profile: Option<&str>,
    schema: &mut frontend_core::scripts::OwnedSchema,
) -> Option<serde_json::Map<String, serde_json::Value>> {
    #[cfg(feature = "js")]
    if let Some((source, name)) = selection.loaded_card() {
        let Some((card_name, path)) = session.scripts.card(source, name).map(|card| {
            schema.copy_from(card.schema());
            (card.name().to_owned(), card.path().to_path_buf())
        }) else {
            ui.text_disabled("(parameters unavailable)");
            return None;
        };
        return Some(if let Some(profile) = profile {
            session.merged_profile_bag(profile, source, &card_name, &path, schema.as_ref())
        } else {
            session.merged_settings_bag(source, name, schema.as_ref())
        });
    }
    #[cfg(not(feature = "js"))]
    if selection.loaded_card().is_some() {
        ui.text_disabled("JavaScript runtime unavailable");
        return None;
    }
    let id = selection.compiled_id()?;
    match session.scripts.compiled_schema(&session.core, profile, id) {
        frontend_core::scripts::SchemaView::Ready { fields, .. } => schema.copy_from(fields),
        frontend_core::scripts::SchemaView::Unavailable(reason) => {
            ui.text_disabled(reason);
            return None;
        }
    }
    match session
        .scripts
        .compiled_edit_bag(&session.core, profile, id)
    {
        Ok(bag) => Some(bag),
        Err(error) => {
            ui.text_disabled(error);
            None
        }
    }
}

/// The typed editor rows for `schema`.
fn script_parameter_rows(
    ui: &Ui,
    session: &mut Session,
    selection: &ScriptChoice,
    profile: Option<String>,
    schema: SchemaRef<'_>,
    mut bag: serde_json::Map<String, serde_json::Value>,
) {
    let filter_field_visible = session
        .script_parameter_filter_id
        .as_ref()
        .is_some_and(|id| {
            schema
                .iter()
                .any(|def| def.id() == id.as_str() && def.visible(&bag))
        });
    if !filter_field_visible {
        session.script_parameter_filter.clear();
        session.script_parameter_filter_id = None;
    }
    if schema.is_empty() {
        ui.text_disabled("(no parameters)");
        return;
    }
    let game_data = session.selected_game_data();
    let game_data_ref = game_data.as_deref();
    let mut last_group: Option<&str> = None;
    for def in schema.iter() {
        if !def.visible(&bag) {
            continue;
        }
        let permission_global = session.walk_permissions.permission_enabled(def.id());
        let group = if permission_global.is_some() {
            Some("Walk permissions")
        } else {
            def.group()
        };
        if group != last_group {
            last_group = group;
            if let Some(group) = last_group {
                ui.separator();
                ui.text(group);
            }
        }
        let label = def.display_label();
        let resolved = frontend_core::scripts::resolve_parameter_options(
            def,
            &bag,
            session.loadouts.store(),
            game_data_ref,
        );
        let array_editable = parameter_array_editor_enabled(def, &bag, &resolved);
        if def.options_from().is_some() && resolved.is_empty() && !array_editable {
            ui.text_wrapped(unavailable_parameter_text(label, def, &bag, &resolved));
            script_parameter_error(ui, session, profile.as_deref(), selection, def.id());
            if session.script_parameter_filter_id.as_deref() == Some(def.id()) {
                session.script_parameter_filter.clear();
                session.script_parameter_filter_id = None;
            }
            continue;
        }
        match def.ty() {
            "boolean" => {
                script_parameter_label(ui, def, &mut session.script_parameter_label);
                let global = permission_global;
                let mut value = bag
                    .get(def.id())
                    .and_then(|v| v.as_bool())
                    .unwrap_or_else(|| def.default() == Some("true"));
                if global == Some(true) {
                    ui.text_wrapped(format!(
                        "{label}: On — inherited globally; this script cannot veto the permission."
                    ));
                } else {
                    if global == Some(false) {
                        ui.text_disabled(
                            "Allow for this script even when the global setting is off.",
                        );
                    }
                    if ui.checkbox(format!("Enabled##param-{}", def.id()), &mut value) {
                        persist_profile_setting(
                            session,
                            selection,
                            def.id(),
                            serde_json::json!(value),
                        );
                        bag.insert(def.id().to_owned(), serde_json::json!(value));
                    }
                }
            }
            "number" => {
                script_parameter_text_input(
                    ui,
                    session,
                    selection,
                    profile.as_deref(),
                    def,
                    resolved.selectable(),
                    bag.get(def.id()).cloned(),
                );
            }
            "string" if !resolved.is_empty() => {
                script_parameter_label(ui, def, &mut session.script_parameter_label);
                let opts = resolved.selectable();
                let stored = bag
                    .get(def.id())
                    .and_then(|v| v.as_str())
                    .or(def.default())
                    .unwrap_or("");
                let current = resolved.value_for(stored);
                ui.set_next_item_width(-1.0);
                let combo_opts = ComboBoxOptions::new().preview_mode(ComboBoxPreviewMode::Preview);
                let preview = resolved.label_for(&current);
                if let Some(_popup) = ui.begin_combo_with_flags(
                    format!("##param-{id}", id = def.id()),
                    preview,
                    combo_opts,
                ) {
                    let first_open =
                        session.script_parameter_filter_id.as_deref() != Some(def.id());
                    if first_open {
                        session.script_parameter_filter_id = Some(def.id().to_owned());
                        session.script_parameter_filter.clear();
                        ui.set_keyboard_focus_here();
                    }
                    let mut filter = std::mem::take(&mut session.script_parameter_filter);
                    let searchable = opts.len() > 16;
                    if searchable {
                        ui.set_next_item_width(-1.0);
                        ui.input_text(format!("##param-filter-{id}", id = def.id()), &mut filter)
                            .hint("Filter options")
                            .build();
                        ui.separator();
                    } else {
                        filter.clear();
                    }
                    let mut matches = 0;
                    for index in filtered_parameter_indices(&resolved, &filter) {
                        let Some(opt) = opts.get(index) else {
                            continue;
                        };
                        matches += 1;
                        let selected = opt == &current;
                        let shown = format!("{}##{opt}", resolved.label_for(opt));
                        if ui.selectable_config(&shown).selected(selected).build() {
                            let value = resolved.value_for(opt);
                            persist_profile_setting(
                                session,
                                selection,
                                def.id(),
                                serde_json::json!(value),
                            );
                            bag.insert(def.id().to_owned(), serde_json::json!(value));
                        }
                    }
                    if searchable && matches == 0 {
                        ui.text_disabled("no matching options");
                    }
                    session.script_parameter_filter = filter;
                } else if session.script_parameter_filter_id.as_deref() == Some(def.id()) {
                    session.script_parameter_filter.clear();
                    session.script_parameter_filter_id = None;
                }
            }
            "string[]" if array_editable => {
                script_parameter_label(ui, def, &mut session.script_parameter_label);
                let mut selected: Vec<String> = bag
                    .get(def.id())
                    .and_then(|value| value.as_array())
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|item| item.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default();
                let mut changed = false;
                for (index, opt) in resolved.values().iter().enumerate() {
                    let mut on = selected
                        .iter()
                        .any(|value| resolved.matches_option(value, opt));
                    // Preserved rows (an unavailable quest with its reason)
                    // can only be cleared, never checked.
                    let _off =
                        (!on && !resolved.is_selectable_index(index)).then(|| ui.begin_disabled());
                    let shown = resolved.label_for(opt);
                    let toggled =
                        ui.checkbox(format!("##param-{id}-{opt}", id = def.id()), &mut on);
                    ui.same_line();
                    ui.text_wrapped(shown);
                    if toggled || ui.is_item_clicked() {
                        changed |= resolved.toggle_array_choice(&mut selected, index);
                    }
                }
                if changed {
                    let coerced = def.coerce(&serde_json::json!(selected));
                    persist_profile_setting(session, selection, def.id(), coerced.clone());
                    bag.insert(def.id().to_owned(), coerced);
                }
            }
            "string" | "tile" | "list" | "string[]" => {
                script_parameter_text_input(
                    ui,
                    session,
                    selection,
                    profile.as_deref(),
                    def,
                    resolved.selectable(),
                    bag.get(def.id()).cloned(),
                );
            }
            _ => {
                ui.text_wrapped(format!("{label} (unsupported type {})", def.ty()));
            }
        }
        if let Some(help) = def.help() {
            ui.set_item_tooltip(help);
        }
        script_parameter_error(ui, session, profile.as_deref(), selection, def.id());
    }
    if session.focused_name().is_some() {
        apply_to_all_section(ui, session);
    }
}

/// Apply to same-card members: copy the focused profile's parameters for
/// its card to the bots the fleet buttons cover, through the fleet plan
/// flow. Its confirmation (naming the frozen scope) and its result are the
/// fleet plan row and result line, drawn by every fleet toolbar; while no
/// toolbar is on screen they are drawn here, for this action only.
fn apply_to_all_section(ui: &Ui, session: &mut Session) {
    ui.separator();
    if ui.button("Apply to same-card members…") {
        session.copy_focused_settings_to_same_card();
    }
    ui.set_item_tooltip(
        "copy these parameters to the marked bots (every loaded bot shown when none is marked) \
         assigned this card; a confirmation names them first, and bots on another card are skipped",
    );
    if !session.fleet_toolbar_shown() {
        let is_copy = |action| action == frontend_core::FleetAction::CopySettings;
        if session
            .core
            .fleet_plan()
            .is_some_and(|plan| is_copy(plan.action()))
        {
            crate::fleet_toolbar::plan_row(ui, session);
        }
        if session
            .core
            .fleet_result()
            .is_some_and(|result| is_copy(result.action()))
        {
            crate::fleet_toolbar::result_line(ui, session);
        }
    }
}

/// Draw the Scripts window's Apply to same-card members section for a test.
#[cfg(all(test, feature = "js"))]
pub(crate) fn draw_test_apply_section(ui: &Ui, session: &mut Session) {
    apply_to_all_section(ui, session);
}

const RESTART_SETTINGS_POPUP: &str = "##script-restart-required";

fn restart_settings_popup(ui: &Ui, session: &mut Session) {
    if session.scripts.pending_restart_prompt().is_some()
        && !ui.is_popup_open(RESTART_SETTINGS_POPUP)
    {
        ui.open_popup(RESTART_SETTINGS_POPUP);
    }
    ui.modal_popup(RESTART_SETTINGS_POPUP, || {
        let Some(summary) = session
            .scripts
            .pending_restart_prompt()
            .map(|prompt| prompt.summary())
        else {
            ui.close_current_popup();
            return;
        };
        ui.text_wrapped(&summary);
        if ui.button("Restart##script-settings") {
            session.restart_pending_settings();
            ui.close_current_popup();
        }
        ui.same_line();
        if ui.button("Later##script-settings") {
            session.defer_settings_restart();
            ui.close_current_popup();
        }
    });
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
    focus_if_requested(ui, session, PanelWindow::ScriptPrefs);
    ui.window(PanelWindow::ScriptPrefs.title())
        .opened(&mut open)
        .flags(WindowFlags::NO_COLLAPSE)
        .size(scale_size(ui, [360.0, 480.0]), Condition::FirstUseEver)
        .build(|| {
            if ui.checkbox(
                "Show parameters in rail",
                &mut session.ui.show_parameters_rail,
            ) {
                let _ = session.persist_panel_member(
                    &["show_parameters_rail"],
                    Some(serde_json::Value::Bool(session.ui.show_parameters_rail)),
                );
            }
            ui.spacing();
            ui.text_wrapped(
                "Text fields: Enter saves, Escape discards. Leaving a field saves a valid edit. Other controls save immediately. A failed save restores the saved value. Native cards can require a restart.",
            );
            ui.spacing();
            if edit_parameters_enabled() {
                script_parameter_editors(ui, session);
            } else {
                ui.text_disabled("parameter editors not available");
            }
            if session.script_sel.as_ref().and_then(ScriptChoice::compiled_id)
                == Some(CompiledCardId::QUESTER)
            {
                quest_paths_section(ui, session);
            }
        });
    session.script_prefs_open = open;
}

/// Quest Paths settings. They are host-wide and apply to every bot, so the
/// Quester card shows them under its parameters.
fn quest_paths_section(ui: &Ui, session: &mut Session) {
    ui.spacing();
    ui.separator();
    ui.text_colored(ACCENT, frontend_core::QUEST_PATHS_HEADING);
    ui.text_disabled("Folder Paths reload here and again when a Quester starts.");
    let mut paths_changed = false;
    let path_reload_running = session.quester_paths.is_running();
    let _path_controls_disabled = if path_reload_running {
        Some(ui.begin_disabled())
    } else {
        None
    };
    let mut enabled = session.quester_paths.settings().enabled();
    if ui.checkbox(frontend_core::LOAD_PATHS_LABEL, &mut enabled) {
        paths_changed = true;
    }
    ui.text("Folder");
    ui.set_next_item_width(-1.0);
    let folder_edited = ui
        .input_text(
            "##quester-path-folder",
            &mut session.quester_paths_folder_edit,
        )
        .build();
    if folder_edited {
        session.quester_paths_folder_dirty = true;
    }
    let mut folder_committed = false;
    if ui.is_item_deactivated_after_edit() {
        folder_committed = true;
        paths_changed |= Path::new(&session.quester_paths_folder_edit)
            != session.quester_paths.settings().folder();
        if !paths_changed {
            session.quester_paths_folder_dirty = false;
        }
    }
    drop(_path_controls_disabled);

    let mut reload_requested = false;
    if path_reload_running {
        let _disabled = ui.begin_disabled();
        ui.button(frontend_core::RELOAD_PATHS_LABEL);
        ui.set_item_tooltip("Wait for the current Path reload to finish.");
    } else {
        reload_requested = ui.button(frontend_core::RELOAD_PATHS_LABEL);
    }
    if paths_changed {
        let _ = session.persist_quester_paths_change(enabled, folder_committed);
    } else if reload_requested {
        session.start_quester_paths_reload();
    }
    if let Some(notice) = session.quester_paths.notice() {
        match notice {
            Ok(summary) => ui.text_wrapped(summary.as_ref()),
            Err(error) => ui.text_colored(ERROR, error.as_ref()),
        }
    }
}

/// Why a selected Loaded card is not in the library: its catalog has not been
/// read yet (or could not be), versus a card that is simply missing. Shared by
/// the Script prefs hint and the parameters rail so neither calls a card that
/// was never loaded "no parameters".
#[cfg(feature = "js")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MissingCard {
    CatalogUnavailable,
    CardUnavailable,
}

#[cfg(feature = "js")]
impl MissingCard {
    fn of(session: &Session, source: CardSource) -> Self {
        if source == CardSource::Catalog && !session.scripts.catalog_filled() {
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
    let Some(sel) = &session.script_sel else {
        return Some("(no script selected)");
    };
    #[cfg(feature = "js")]
    if let Some((source, name)) = sel.loaded_card() {
        return match session.scripts.card(source, name) {
            None => Some(MissingCard::of(session, source).rail_text()),
            Some(card) if card.schema().is_empty() => Some("(no parameters)"),
            Some(_) => None,
        };
    }
    #[cfg(not(feature = "js"))]
    if sel.loaded_card().is_some() {
        return Some("(JavaScript runtime unavailable)");
    }
    let id = sel.compiled_id()?;
    match session
        .scripts
        .compiled_schema(&session.core, session.focused_name().as_deref(), id)
    {
        frontend_core::scripts::SchemaView::Unavailable(_) => Some("(parameters unavailable)"),
        frontend_core::scripts::SchemaView::Ready { fields, .. } if fields.is_empty() => {
            Some("(no parameters)")
        }
        frontend_core::scripts::SchemaView::Ready { .. } => None,
    }
}

/// Tooltip when Script prefs is disabled (empty schema, unavailable card, or no selection).
fn script_prefs_disabled_hint(session: &Session) -> Option<&'static str> {
    let Some(sel) = &session.script_sel else {
        return Some("select a script first");
    };
    if let Some(id) = sel.compiled_id() {
        return match session.scripts.compiled_schema(
            &session.core,
            session.focused_name().as_deref(),
            id,
        ) {
            frontend_core::scripts::SchemaView::Unavailable(reason) => Some(reason),
            frontend_core::scripts::SchemaView::Ready { fields, .. } if fields.is_empty() => {
                Some("selected script has no parameters")
            }
            frontend_core::scripts::SchemaView::Ready { .. } => None,
        };
    }
    #[cfg(feature = "js")]
    {
        let (source, name) = sel.loaded_card()?;
        let Some(card) = session.scripts.card(source, name) else {
            return Some(MissingCard::of(session, source).prefs_hint());
        };
        if card.schema().is_empty() {
            Some("selected script has no parameters")
        } else {
            None
        }
    }
    #[cfg(not(feature = "js"))]
    {
        sel.loaded_card().map(|_| "JavaScript runtime unavailable")
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
                        session.open_window(PanelWindow::GeneralConfig);
                    }
                    ui.set_item_tooltip("slot render + global cadence / capture");
                }
                "Nav config" => {
                    if ui.button_with_size("Nav config", [w, 0.0]) {
                        session.open_window(PanelWindow::NavConfig);
                    }
                    ui.set_item_tooltip("nav debug paints and labels");
                }
                "Loadouts" => {
                    // Only a closed Loadouts starts on the first preset; an
                    // open one keeps its selection and unsaved draft.
                    if ui.button_with_size("Loadouts", [w, 0.0])
                        && session.open_window(PanelWindow::Loadouts)
                    {
                        crate::loadouts::select(session, 0);
                    }
                    ui.set_item_tooltip("equipment and inventory presets");
                }
                "Script prefs" => {
                    if let Some(hint) = script_prefs_disabled_hint(session) {
                        mock_button(ui, "Script prefs", hint, [w, 0.0]);
                    } else {
                        if ui.button_with_size("Script prefs", [w, 0.0]) {
                            session.open_window(PanelWindow::ScriptPrefs);
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
    // The offer must survive closing the picker (or folding the status rows).
    if let Some(notice) = session.focused_memory_notice().filter(|n| n.differs()) {
        ui.text_wrapped(notice.notice_text());
        memory_relog_button(ui, session, notice);
    }
    if !section_open(ui, session, "status") {
        return;
    }
    let walk = session.walk_status_text();
    let mem = frontend_core::MemoryNotice::status_text(
        session.focused_lowmem(),
        session.focused_memory_notice(),
    );
    let Some(d) = session.core.fleet_view().detail() else {
        kv_row(ui, "state", "no bot selected");
        status_kv_row(ui, "walk", &walk);
        kv_row(ui, "mem", &mem);
        return;
    };
    status_detail_rows(ui, d, &walk, &mem);
}

fn status_detail_rows(ui: &Ui, d: &frontend_core::SlotDetail, walk: &str, mem: &str) {
    kv_row(ui, "state", d.state());
    if let Some(script) = d.script_phase() {
        kv_row(ui, "script", script.label());
        if let Some(phase) = script.phase() {
            kv_row(ui, "phase", phase);
        }
        if let Some(reason) = script.reason() {
            kv_row(ui, "reason", reason);
        }
    }
    kv_row(
        ui,
        "player",
        if d.player().is_empty() {
            "?"
        } else {
            d.player()
        },
    );
    if let Some(world) = d.row().world() {
        kv_row(ui, "world", &format!("w{world}"));
    }
    kv_row(ui, "tile", &format!("{} {}", d.tile().0, d.tile().1));
    status_kv_row(ui, "walk", walk);
    if let Some(reason) = d.manual_walk_risk() {
        status_kv_row(ui, "Walk risk", reason);
    }
    let queue = d
        .row()
        .queue()
        .map_or_else(|| "—".to_string(), |q| q.to_string());
    status_kv_row(ui, "queue", &queue);
    if matches!(d.row().phase(), Phase::Ready) {
        status_kv_row(ui, "modals", &d.modal().to_string());
    }
    if let Some(welcome) = d.welcome() {
        kv_row(ui, "welcome", welcome);
    }
    if let Some(random) = d.random() {
        kv_row(ui, "random", random);
    }
    // The current login notice takes the history row's place while it
    // stands (the terminal app's status pane does the same).
    if let Some(notice) = d.login_notice_line() {
        ui.text_colored(ERROR, notice);
    } else if let Some(error) = d.row().last_error() {
        kv_row(ui, "last error", error);
    }
    if let Some(op) = d.row().last_op() {
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

/// fleet: the fleet toolbar while the rail is hidden (Grid, or MultiBox
/// off). Its header folds the toolbar to its summary, plan and result with
/// the same saved choice as the rail's, and drags to reorder like the other
/// headings.
fn fleet_section(ui: &Ui, session: &mut Session) {
    let open = crate::fleet_toolbar::header(ui, session, "fleet");
    ui.set_item_tooltip("fold the fleet toolbar; drag to reorder");
    heading_dnd(ui, session, "fleet");
    crate::fleet_toolbar::body(ui, session, crate::fleet_toolbar::Site::Strip, open);
}

/// log: the shared structured log (see [`crate::log_pane`]).
fn log_section(ui: &Ui, session: &mut Session, last: bool) {
    if !section_open(ui, session, "log") {
        return;
    }
    if session.log_window_body_drawn {
        ui.text_disabled("Log is open in its tab");
    } else {
        crate::log_pane::log_body(ui, session, last);
    }
}

/// Draw the shared log as a dockable panel tab.
fn log_window(ui: &Ui, session: &mut Session, panel_dock: Option<Id>) {
    log_window_with_body(ui, session, panel_dock, |ui, session| {
        crate::log_pane::log_body(ui, session, true)
    });
}

fn log_window_with_body(
    ui: &Ui,
    session: &mut Session,
    panel_dock: Option<Id>,
    body: impl FnOnce(&Ui, &mut Session),
) {
    session.log_window_body_drawn = false;
    if !session.log_window_open {
        return;
    }
    let mut open = true;
    let panel_class = panel_window_class();
    ui.set_next_window_class(&panel_class);
    if let Some(id) = panel_dock {
        ui.set_next_window_dock_id_with_cond(id, Condition::FirstUseEver);
    }
    focus_if_requested(ui, session, PanelWindow::Log);
    ui.window(PanelWindow::Log.title())
        .opened(&mut open)
        .flags(WindowFlags::NO_COLLAPSE)
        .size(
            scale_size(ui, [PANEL_WIDTH, 480.0]),
            Condition::FirstUseEver,
        )
        .size_constraints(
            scale_size(ui, [280.0, 180.0]),
            [f32::MAX, scale_px(ui, 720.0)],
        )
        .build(|| {
            session.log_window_body_drawn = true;
            body(ui, session);
        });
    session.log_window_open = open;
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

fn memory_relog_button(ui: &Ui, session: &mut Session, notice: frontend_core::MemoryNotice) {
    if !notice.can_relog() {
        return;
    }
    if session.focused_memory_relog_warning() {
        ui.text_wrapped("Relog interrupts running or queued script work. Click twice to confirm.");
    }
    let label = if session.mem_relog_armed() {
        "Relog now (confirm interruption)"
    } else {
        "Relog now"
    };
    if ui.button_with_size(label, [0.0, 0.0]) {
        session.request_focused_memory_relog();
    }
}

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
        // Self-sized popups need an explicit wrap edge; do not let their
        // short mode buttons determine the width of a multi-line notice.
        popup_text(ui, frontend_core::MemoryNotice::NEXT_LOGIN_NOTE);
        if let Some(notice) = session.focused_memory_notice().filter(|n| n.differs()) {
            if notice.relog_pending {
                popup_text(ui, notice.notice_text());
            }
            memory_relog_button(ui, session, notice);
        }
    });
}

/// none / GPU / CPU row. Click the mem button (current highmem/lowmem) for
/// the sticky picker, like Teles.
fn raster_picker(ui: &Ui, session: &mut Session) {
    let cur = session.focused_raster();
    let avail = ui.content_region_avail()[0];
    let cells = scaled_button_cells(ui, avail, 3);
    if inverted_button(
        ui,
        "none",
        cur == frontend_core::command::RasterMode::Off,
        [cells[0].0, 0.0],
    ) {
        session.request_focused_raster(frontend_core::command::RasterMode::Off);
    }
    if cells[1].1 {
        gap_line(ui);
    }
    if inverted_button(
        ui,
        "GPU",
        cur == frontend_core::command::RasterMode::Gpu,
        [cells[1].0, 0.0],
    ) {
        session.request_focused_raster(frontend_core::command::RasterMode::Gpu);
    }
    if cells[2].1 {
        gap_line(ui);
    }
    if inverted_button(
        ui,
        "CPU",
        cur == frontend_core::command::RasterMode::Cpu,
        [cells[2].0, 0.0],
    ) {
        session.request_focused_raster(frontend_core::command::RasterMode::Cpu);
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
        "Queue highmem / lowmem for the next login. Textures, tabs and sound stay unchanged until then. Relog now is offered in the picker and status.",
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
        let _ = session.persist_panel_member(
            &["config_collapsed", id],
            Some(serde_json::Value::Bool(!open)),
        );
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
            let value = serde_json::Value::Bool(visible);
            if *id == "parameters" {
                let _ = session.persist_panel_member(&["show_parameters_rail"], Some(value));
            } else {
                let _ = session.persist_panel_member(&["panel_sections", id], Some(value));
            }
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
        macro_rules! persist_chrome_member {
            ($field:ident) => {
                if session.ui.chrome.$field != chrome.$field {
                    let _ = session.persist_panel_serializable(
                        &["chrome", stringify!($field)],
                        &chrome.$field,
                    );
                }
            };
        }
        persist_chrome_member!(accent);
        persist_chrome_member!(accent_hover);
        persist_chrome_member!(bg);
        persist_chrome_member!(bg_deep);
        persist_chrome_member!(text);
        persist_chrome_member!(text_dim);
        persist_chrome_member!(frame);
        persist_chrome_member!(hover_fill);
        persist_chrome_member!(active_fill);
        persist_chrome_member!(border);
        persist_chrome_member!(warn);
        persist_chrome_member!(error);
        persist_chrome_member!(green);
        session.ui.chrome = chrome;
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
    if !session.panel_preferences_ready() {
        ui.text_disabled("Loading preferences…");
    } else if session.preferences_pending() {
        ui.text_disabled("Saving preferences…");
    }
    if let Some(error) = session.panel_preference_write_error() {
        ui.text_colored(session.ui.chrome.error_rgba(), "Preference save failed:");
        ui.text_wrapped(error);
    }
    ui.spacing();
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
    if ui.checkbox("only render focused", &mut only) {
        only_render_selected_clicked(session, only);
    }
    let live = session.core.live_runner().status().is_some();
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
    ui.text_wrapped("Game pane only. GPU↔CPU reattaches the renderer without a logout. Memory mode is queued for the next login; textures, tabs and sound stay unchanged until then.");
    raster_picker(ui, session);
}

/// What the profile's auto-login does (`SlotArm::new(uid, auto_login)` arms
/// the login on spawn; a login keeps it armed for a disconnect; an active
/// script wants the login regardless).
const AUTO_LOGIN_TOOLTIP: &str = "Log in as soon as this bot starts (after unlocking the vault or loading the profile), and log back in after a disconnect. Off: the bot waits at the login screen until you press Log in; a running script still logs in.";

fn slot_capture_section(ui: &Ui, session: &mut Session) {
    let mut auto_cur = session.profile_editor.draft().settings().auto_login();
    // Label only: the saved setting stays `ProfileSettings.auto_login`.
    let toggled = ui.checkbox("Log in automatically", &mut auto_cur);
    ui.set_item_tooltip(AUTO_LOGIN_TOOLTIP);
    if toggled {
        *session
            .profile_editor
            .draft_mut()
            .settings_mut()
            .auto_login_mut() = auto_cur;
        if let Some(name) = session
            .profile_editor
            .target()
            .filter(|n| !n.is_empty())
            .map(str::to_owned)
        {
            if let Some(submitted) = session.submit(
                OperatorCommand::SetAutoLogin(auto_cur),
                Targets::Profile(&name),
            ) {
                session.error = session
                    .core
                    .command_failure(submitted.request())
                    .map(|reason| reason.to_string());
            }
        }
        let _ = session.submit(
            OperatorCommand::EditProfileDraft(frontend_core::command::ProfileDraftField::Settings),
            Targets::Application,
        );
    }
    let mut world_changed = false;
    if let Some(worlds) = session
        .launcher
        .bound()
        .and_then(|p| p.public_world_numbers())
    {
        let preview = session
            .profile_editor
            .draft()
            .settings()
            .world()
            .map_or_else(|| "auto".to_string(), |number| format!("w{number}"));
        ui.text_disabled("world (next slot start)");
        ui.set_next_item_width(-1.0);
        if let Some(_open) = ui.begin_combo("##account-world", &preview) {
            if ui
                .selectable_config("auto")
                .selected(session.profile_editor.draft().settings().world().is_none())
                .build()
            {
                *session
                    .profile_editor
                    .draft_mut()
                    .settings_mut()
                    .world_mut() = None;
                world_changed = true;
            }
            for number in worlds {
                let label = format!("w{number}");
                if ui
                    .selectable_config(&label)
                    .selected(session.profile_editor.draft().settings().world() == Some(number))
                    .build()
                {
                    *session
                        .profile_editor
                        .draft_mut()
                        .settings_mut()
                        .world_mut() = Some(number);
                    world_changed = true;
                }
            }
        }
    }
    if world_changed {
        let _ = session.submit(
            OperatorCommand::EditProfileDraft(frontend_core::command::ProfileDraftField::Settings),
            Targets::Application,
        );
    }
    ui.text_wrapped("this profile; handshake on spawn unless latched out");
}

/// The smallest height of the rail's tiles region at scale 1. While the
/// open toolbar leaves less, the rail window scrolls as a whole; folding the
/// toolbar gives the tiles the rest.
const RAIL_TILES_MIN_H: f32 = 120.0;

/// Sidecar rail window: only while MultiBox is on and Grid is off. The fleet
/// toolbar under its fold header ([`crate::fleet_toolbar`]) stays at the
/// top; below it one region scrolls on its own: the only-render-selected
/// checkbox (unchecking needs the render-all confirm), one tile per shown
/// wall member (cap only while only-render-selected, else cap + 1 fps body
/// or renderer-off placeholder), the folded "Not loaded" group, Profiles…
/// and the 1 Hz resource card.
fn rail_window(ui: &Ui, gpu: &mut Gpu, state: &mut PanelState) {
    let mut open = true;
    ui.window(format!(
        "{}-rail###{RAIL_WINDOW}",
        state.session.app_title()
    ))
    .opened(&mut open)
    .flags(WindowFlags::NO_COLLAPSE | WindowFlags::NO_RESIZE)
    .build(|| {
        rail_toolbar(ui, &mut state.session);
        ui.separator();
        let height = ui.content_region_avail()[1].max(scale_px(ui, RAIL_TILES_MIN_H));
        ui.child_window("##rail-tiles")
            .size([0.0, height])
            .build(ui, || {
                render_gate(ui, &mut state.session);
                rail_tiles(ui, gpu, state);
                add_bot_button(ui, state);
                resource_card(ui, state.session.core.resources());
                #[cfg(test)]
                {
                    let (pos, size) = (ui.window_pos(), ui.window_size());
                    TEST_RAIL_TILES.set(Some(RailTilesRecord {
                        scroll_y: ui.scroll_y(),
                        scroll_max_y: ui.scroll_max_y(),
                        rect: [pos[0], pos[1], pos[0] + size[0], pos[1] + size[1]],
                    }));
                }
            });
    });
    if !open {
        state.session.set_multibox(false);
    }
}

/// The rail's fleet toolbar: the "Fleet" fold header and the toolbar (or,
/// folded, its summary, plan and result).
fn rail_toolbar(ui: &Ui, session: &mut Session) {
    let open = crate::fleet_toolbar::header(ui, session, "Fleet###fleet-toolbar");
    crate::fleet_toolbar::body(ui, session, crate::fleet_toolbar::Site::Rail, open);
}

/// Draw the rail's toolbar for a test, in the current window.
#[cfg(test)]
pub(crate) fn draw_test_rail_toolbar(ui: &Ui, session: &mut Session) {
    rail_toolbar(ui, session);
}

/// Draw the strip's Local-engine debug cheat buttons for a test, in the
/// current window.
#[cfg(test)]
pub(crate) fn draw_test_debug_section(ui: &Ui, session: &mut Session) {
    debug_section(ui, session);
}

/// Draw the right-hand strip for a test at `pos` and `size` (its own size is
/// the dock node's in the product).
#[cfg(test)]
pub(crate) fn draw_test_panel_window(
    ui: &Ui,
    session: &mut Session,
    pos: [f32; 2],
    size: [f32; 2],
) {
    // Fix the window's place and size first: the same window, empty.
    ui.window(format!("{}###{PANEL_WINDOW}", session.app_title()))
        .position(pos, Condition::Always)
        .size(size, Condition::Always)
        .build(|| {});
    panel_window(ui, session, None);
}

/// Apply an "only render focused" checkbox change: re-checking on is
/// immediate; unchecking keeps the safe default and opens the scary
/// confirm instead, unless the operator saved "Don't show again" (`acked`),
/// when it applies at once. Returns `(new_only_render_selected, open_warn)`.
pub fn apply_only_render_selected(current: bool, checked: bool, acked: bool) -> (bool, bool) {
    if checked && !current {
        // user just checked "only render focused" — apply immediately
        return (true, false);
    }
    if !checked && current {
        if acked {
            return (false, false);
        }
        // user just unchecked — do not apply; open warning
        return (true, true);
    }
    (current, false)
}

/// The "only render focused" checkbox was clicked to `checked`. The policy
/// flips every wall member's draw state, so the slots are kicked and parked
/// threads apply the change within a frame (slot threads read it every frame).
fn only_render_selected_clicked(session: &mut Session, checked: bool) {
    let current = session.focus.lock().unwrap().only_render_selected;
    let (next, open_warn) =
        apply_only_render_selected(current, checked, session.ui.render_all_warning_ack);
    if next != current {
        session.set_render_all(!next);
    }
    if open_warn {
        session.wall.render_all_warn_open = true;
    }
}

/// The render gate at the top of the rail's tiles region: the
/// only-render-selected checkbox. Unchecking does not write `false` until
/// the render-all warning is accepted; re-checking on is immediate (slot
/// threads apply `set_draw` from it every frame).
fn render_gate(ui: &Ui, session: &mut Session) {
    let current = session.focus.lock().unwrap().only_render_selected;
    let mut only = current;
    if ui.checkbox("only render focused", &mut only) {
        only_render_selected_clicked(session, only);
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

/// Scary confirm before "only render focused" can be unchecked: OK and
/// "Don't show again" stay disabled until "I understand" is ticked. Both
/// write `only_render_selected = false`; "Don't show again" also saves the
/// acknowledgement so the box unchecks at once from then on. Cancel keeps the
/// safe default; the box that triggered this stays checked either way.
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
            #[cfg(test)]
            record_test_render_all_item(ui);
            ui.spacing();
            let avail = ui.content_region_avail()[0];
            let gap = scale_px(ui, BUTTON_GAP);
            let remember_w = ui.current_font().calc_text_size(
                ui.current_font_size(),
                f32::MAX,
                0.0,
                "Don't show again",
            )[0] + 2.0 * ui.clone_style().frame_padding()[0];
            let side_w = ((avail - remember_w - 2.0 * gap) / 2.0).max(1.0);
            let stack = side_w < scale_px(ui, MIN_BUTTON);
            let (side_w, remember_w) = if stack {
                (avail, avail)
            } else {
                (side_w, remember_w)
            };
            let ok_clicked = {
                let _disabled = ui.begin_disabled_with_cond(!understood);
                ui.button_with_size("OK", [side_w, 0.0])
            };
            #[cfg(test)]
            record_test_render_all_item(ui);
            if !stack {
                gap_line(ui);
            }
            let cancelled = ui.button_with_size("Cancel", [side_w, 0.0]);
            #[cfg(test)]
            record_test_render_all_item(ui);
            if !stack {
                gap_line(ui);
            }
            let remember_clicked = {
                let _disabled = ui.begin_disabled_with_cond(!understood);
                ui.button_with_size("Don't show again", [remember_w, 0.0])
            };
            #[cfg(test)]
            record_test_render_all_item(ui);
            if (ok_clicked || remember_clicked) && understood {
                session.confirm_render_all(remember_clicked);
            }
            if cancelled {
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

#[cfg(test)]
std::thread_local! {
    static TEST_RENDER_ALL_ITEMS: std::cell::RefCell<Vec<[f32; 4]>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(test)]
fn record_test_render_all_item(ui: &Ui) {
    let bounds = item_bounds(ui);
    TEST_RENDER_ALL_ITEMS.with_borrow_mut(|items| items.push(bounds));
}

/// One frame of the render-all warning. Returns the bounds of its controls in
/// order: "I understand", OK, Cancel, "Don't show again"; empty while closed.
#[cfg(test)]
fn draw_test_render_all_warning(ui: &Ui, session: &mut Session) -> Vec<[f32; 4]> {
    TEST_RENDER_ALL_ITEMS.with_borrow_mut(Vec::clear);
    render_all_warn_window(ui, session);
    TEST_RENDER_ALL_ITEMS.with_borrow_mut(std::mem::take)
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

/// One tile per shown wall member, in the fleet model's order and filter
/// (the Fleet window's): cap (world number or local traffic-light dot,
/// name + brief, ✗) then a `TILE_W`×`TILE_H` body. The body blits the
/// slot's `FrameBuf` when `draw_for_slot` says this member paints;
/// otherwise it shows the renderer-off placeholder. While
/// `only_render_selected` is on (the safe default) the strip is collapsed:
/// cap only, no body. Clicking the name or body focuses the member; the ✗
/// (a sibling button, never part of the name click) removes it. Shown
/// profiles that are not loaded follow in a folded "Not loaded" group of
/// markable names (no tile, no texture).
fn rail_tiles(ui: &Ui, gpu: &mut Gpu, state: &mut PanelState) {
    ui.spacing();
    state.sync_fleet();
    // The tiles read the model's rows while their clicks need the session:
    // lend the rows out for the frame (no allocation) and apply the one
    // clicked action once they are back.
    let model = std::mem::take(&mut state.session.fleet_model);
    let labels = std::mem::take(&mut state.fleet_labels);
    let debug_actions = state.session.debug_ui();
    let switch_world = state.session.can_switch_world();
    let only_selected = state.session.focus.lock().unwrap().only_render_selected;
    {
        let focus = state.session.focus.lock().unwrap();
        dispose_idle_views(&mut state.views, gpu, |name| {
            tile_shown(&model, name) && draw_for_slot(&focus, name)
        });
    }
    let mut clicked = None;
    // Dragging a tile reorders the saved Load order, so only that view
    // offers it.
    let reorder = model.order() == FleetOrder::LoadOrder;
    for (index, row) in model.shown_rows().filter(|(_, row)| row.loaded()) {
        let labels = &labels[index];
        let name = row.name();
        let (is_focused, draw) = {
            let focus = state.session.focus.lock().unwrap();
            (
                focus.focused.as_deref() == Some(name),
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
        let mut marked = model.is_marked(index);
        let restart_required = state.session.scripts.has_restart_badge(name);
        let mut drag = reorder.then(|| CapDrag {
            identity: model.identities()[index],
            dropped: None,
            dragging: false,
        });
        let CapClicks {
            select: cap_select,
            remove: cap_remove,
            fold: cap_fold,
            world: cap_world,
        } = rail_cap(
            ui,
            row,
            labels,
            CapDrawState {
                is_focused,
                width: avail,
                preview,
                restart_required,
                debug_actions,
                switch_world,
            },
            &mut marked,
            drag.as_mut(),
        );
        if marked != model.is_marked(index) {
            clicked = Some(TileClick::Mark(index, marked));
        }
        let mut dropped = drag.as_ref().and_then(|drag| drag.dropped);
        let body_clicked = if preview {
            let body_clicked = rail_body(ui, gpu, state, name, draw);
            if reorder && dropped.is_none() {
                dropped = crate::fleet::profile_drop_target(ui);
            }
            body_clicked
        } else {
            false
        };
        if cap_fold {
            let next = !preview;
            state.session.ui.rail_preview.insert(name.to_string(), next);
            let _ = state
                .session
                .persist_panel_member(&["rail_preview", name], Some(serde_json::Value::Bool(next)));
        } else if cap_remove {
            clicked = Some(TileClick::Remove(index));
        } else if cap_world {
            clicked = Some(TileClick::World(index));
        } else if cap_select || body_clicked {
            clicked = Some(TileClick::Select(index));
        }
        if let Some(op) = dropped.and_then(|moving| crate::fleet::drop_op(&model, moving, index)) {
            clicked = Some(TileClick::Move(op));
        }
        ui.spacing();
    }
    if model.counts().shown_unloaded() > 0
        && ui.collapsing_header(&state.not_loaded_label, TreeNodeFlags::NONE)
    {
        for (index, row) in model.shown_rows().filter(|(_, row)| !row.loaded()) {
            let _id = ui.push_id(row.name());
            let mut marked = model.is_marked(index);
            if ui.checkbox("##not-loaded-mark", &mut marked) {
                clicked = Some(TileClick::Mark(index, marked));
            }
            ui.set_item_tooltip("mark, then Log in from the fleet toolbar, to load and log in");
            ui.same_line();
            ui.text_disabled(row.name());
        }
    }
    state.session.fleet_model = model;
    state.fleet_labels = labels;
    apply_tile_click(&mut state.session, clicked);
}

/// Cap row: the active public-world number (or a status dot for local
/// profiles) in the row's light, the member's name plus its brief status
/// (click selects), and a small red ✗ (rail remove: logout arm then
/// `stop_slot`, never `vault`). `width` is the strip the row must fit (rail
/// avail or grid cell width). Every string is the row's cached label: a
/// frame formats nothing.
#[derive(Clone, Copy)]
struct CapDrawState {
    is_focused: bool,
    width: f32,
    preview: bool,
    restart_required: bool,
    /// The profile's server allows Debug, so a mark also feeds Debug actions.
    debug_actions: bool,
    /// The bound server has public worlds, so the world marker opens the
    /// world picker (a local server's marker is not clickable).
    switch_world: bool,
}

/// What a click on a cap asked for. At most one is set in a frame: they are
/// separate items.
#[derive(Default)]
struct CapClicks {
    /// The name: focus the member.
    select: bool,
    /// The ✗: drop the member from the wall.
    remove: bool,
    /// The fold button: fold or show the preview.
    fold: bool,
    /// The world marker: open the world picker.
    world: bool,
}

/// One rail cap's reorder gesture, offered only while the fleet shows Load
/// order: the cap name is a drag source for `identity` and a drop target.
struct CapDrag {
    identity: ProfileIdentity,
    /// Set by [`rail_cap`]: the profile dropped on the name this frame
    /// ([`ProfileIdentity::raw`]).
    dropped: Option<u64>,
    /// Set by [`rail_cap`]: the name is being dragged now.
    dragging: bool,
}

fn rail_cap(
    ui: &Ui,
    row: &FleetRow,
    labels: &CapLabels,
    draw: CapDrawState,
    marked: &mut bool,
    drag: Option<&mut CapDrag>,
) -> CapClicks {
    const BTN: f32 = 28.0;
    const DOT_W: f32 = 18.0;
    const MARK_W: f32 = 18.0;
    let _id = ui.push_id(row.name());
    let [screen_x, screen_y] = ui.cursor_screen_pos();
    // Fractional scaled spacing can leave the next rail row between pixels.
    ui.set_cursor_screen_pos([screen_x, screen_y.round()]);
    let light = row.light();
    let colour = light_rgb(light);
    let btn = scale_px(ui, BTN);
    let dot_w = scale_px(ui, DOT_W);
    let mark_w = scale_px(ui, MARK_W);
    let gap = scale_px(ui, BUTTON_GAP);
    let restart_w = if draw.restart_required {
        scale_px(ui, 56.0) + gap
    } else {
        0.0
    };
    let marker_x = ui.cursor_pos_x();
    let mut world = false;
    if labels.world.is_empty() {
        draw_status_dot(ui, light, dot_w);
    } else {
        world = world_marker(ui, &labels.world, colour, dot_w, draw.switch_world);
        if draw.switch_world {
            world_switch_tooltip(ui, &labels.world_tip);
        } else {
            ui.set_item_tooltip(&labels.world_tip);
        }
    }
    #[cfg(test)]
    record_test_rail_item(ui);
    ui.same_line_with_pos(marker_x + dot_w + gap);
    ui.checkbox("##rail-mark", marked);
    #[cfg(test)]
    record_test_rail_item(ui);
    let mark_tip = rail_mark_tooltip(draw.debug_actions);
    ui.set_item_tooltip(mark_tip);
    #[cfg(test)]
    TEST_RAIL_MARK_TOOLTIP.set(mark_tip);
    ui.same_line_with_pos(marker_x + dot_w + gap + mark_w + gap);
    let name_w =
        (draw.width - btn * 2.0 - dot_w - mark_w - gap * 4.0 - restart_w).max(scale_px(ui, 10.0));
    let mut clicked = ui
        .selectable_config(&labels.title)
        .selected(draw.is_focused)
        .size([name_w, 0.0])
        .build();
    #[cfg(test)]
    record_test_rail_item(ui);
    if let Some(drag) = drag {
        drag.dragging = crate::fleet::profile_drag_source(ui, drag.identity, row.name());
        drag.dropped = crate::fleet::profile_drop_target(ui);
        // A drag is not a click: it must not focus the bot.
        clicked &= !drag.dragging;
    }
    if draw.restart_required {
        ui.same_line_with_spacing(0.0, gap);
        ui.text_colored(WARN, "[restart]");
        ui.set_item_tooltip("saved settings need a restart to apply");
    }
    gap_line(ui);
    let fold_g = if draw.preview {
        FOLD_GLYPH
    } else {
        UNFOLD_GLYPH
    };
    let folded = ui.button_with_size(fold_g, [btn, 0.0]);
    #[cfg(test)]
    record_test_rail_item(ui);
    ui.set_item_tooltip(if draw.preview {
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
    CapClicks {
        select: clicked,
        remove: removed,
        fold: folded,
        world,
    }
}

#[cfg(test)]
std::thread_local! {
    static TEST_RAIL_ITEM_BOUNDS: std::cell::RefCell<Vec<[f32; 4]>> =
        const { std::cell::RefCell::new(Vec::new()) };
    /// The rail's scrolling tiles region as the last frame drew it.
    static TEST_RAIL_TILES: std::cell::Cell<Option<RailTilesRecord>> =
        const { std::cell::Cell::new(None) };
}

/// The rail's scrolling tiles region as a test sees it.
#[cfg(test)]
#[derive(Debug, Clone, Copy)]
struct RailTilesRecord {
    scroll_y: f32,
    scroll_max_y: f32,
    /// `[min_x, min_y, max_x, max_y]`.
    rect: [f32; 4],
}

#[cfg(test)]
fn record_test_rail_item(ui: &Ui) {
    let min = ui.item_rect_min();
    let max = ui.item_rect_max();
    TEST_RAIL_ITEM_BOUNDS.with_borrow_mut(|bounds| bounds.push([min[0], min[1], max[0], max[1]]));
}

#[cfg(test)]
pub(crate) fn draw_test_rail_cap(ui: &Ui, width: f32) -> (Vec<[f32; 4]>, [f32; 4]) {
    let row = FleetRow::for_profile("pixel-layout");
    let labels = CapLabels {
        title: "pixel-layout".into(),
        ..CapLabels::default()
    };
    let colour = light_rgb(row.light());
    TEST_RAIL_ITEM_BOUNDS.with_borrow_mut(Vec::clear);
    let mut marked = false;
    let _ = rail_cap(
        ui,
        &row,
        &labels,
        CapDrawState {
            is_focused: true,
            width,
            preview: false,
            restart_required: false,
            debug_actions: true,
            switch_world: false,
        },
        &mut marked,
        None,
    );
    let bounds = TEST_RAIL_ITEM_BOUNDS.with_borrow_mut(std::mem::take);
    (bounds, colour)
}

/// Tooltip of a cap's mark checkbox. Debug actions exist only on servers that
/// allow Debug, so the others don't mention them.
fn rail_mark_tooltip(debug_actions: bool) -> &'static str {
    if debug_actions {
        "mark for Fleet and Debug actions"
    } else {
        "mark for Fleet actions"
    }
}

#[cfg(test)]
std::thread_local! {
    static TEST_RAIL_MARK_TOOLTIP: std::cell::Cell<&'static str> = const { std::cell::Cell::new("") };
}

/// The tooltip a cap's mark checkbox gets for a server that does or doesn't
/// allow Debug, as `rail_cap` draws it.
#[cfg(test)]
fn draw_test_rail_mark_tooltip(ui: &Ui, debug_actions: bool) -> &'static str {
    let row = FleetRow::for_profile("mark-tip");
    let labels = CapLabels {
        title: "mark-tip".into(),
        ..CapLabels::default()
    };
    let mut marked = false;
    let _ = rail_cap(
        ui,
        &row,
        &labels,
        CapDrawState {
            is_focused: false,
            width: 300.0,
            preview: false,
            restart_required: false,
            debug_actions,
            switch_world: false,
        },
        &mut marked,
        None,
    );
    TEST_RAIL_MARK_TOOLTIP.get()
}

#[cfg(test)]
std::thread_local! {
    static TEST_ROUTING_ITEM_BOUNDS: std::cell::RefCell<Vec<[f32; 4]>> =
        const { std::cell::RefCell::new(Vec::new()) };
    static TEST_ROUTING_CHOICE_BOUNDS: std::cell::RefCell<Vec<[f32; 4]>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(test)]
fn item_bounds(ui: &Ui) -> [f32; 4] {
    let min = ui.item_rect_min();
    let max = ui.item_rect_max();
    [min[0], min[1], max[0], max[1]]
}

#[cfg(test)]
fn record_test_routing_item(ui: &Ui) {
    let bounds = item_bounds(ui);
    TEST_ROUTING_ITEM_BOUNDS.with_borrow_mut(|items| items.push(bounds));
}

#[cfg(test)]
fn record_test_routing_choice(ui: &Ui) {
    let bounds = item_bounds(ui);
    TEST_ROUTING_CHOICE_BOUNDS.with_borrow_mut(|choices| choices.push(bounds));
}

/// One frame of the Routing group as a test sees it.
#[cfg(test)]
struct TestRouting {
    /// Bounds of each routing row: the checkboxes, the drop-down, and the
    /// notes under it.
    items: Vec<[f32; 4]>,
    /// Bounds of each drop-down entry while the popup is open.
    choices: Vec<[f32; 4]>,
    /// Whether this frame changed a setting.
    changed: bool,
}

/// Draws only the Routing group, so a test can check the docked width and
/// drive the drop-down without the rest of the Nav config window.
#[cfg(test)]
fn draw_test_nav_routing(ui: &Ui, nav: &mut crate::nav_settings::NavSettings) -> TestRouting {
    TEST_ROUTING_ITEM_BOUNDS.with_borrow_mut(Vec::clear);
    TEST_ROUTING_CHOICE_BOUNDS.with_borrow_mut(Vec::clear);
    let changed = routing_group(ui, nav);
    TestRouting {
        items: TEST_ROUTING_ITEM_BOUNDS.with_borrow_mut(std::mem::take),
        choices: TEST_ROUTING_CHOICE_BOUNDS.with_borrow_mut(std::mem::take),
        changed,
    }
}

/// Public world marker: a disc in the status colour, centred in the
/// `width`-wide status cell and on the text line, with the world number
/// knocked out in the background colour. Drawn as geometry so the digit is
/// the rail's own font and the disc size does not depend on glyph metrics.
/// Occupies one text line, like the local-profile status marker. With
/// `clickable` (the server has public worlds to switch to) it is the world
/// bubble: a button that rings in the accent colour under the pointer, and
/// a click returns `true`.
fn world_marker(ui: &Ui, number: &str, colour: [f32; 4], width: f32, clickable: bool) -> bool {
    let line_h = ui.text_line_height();
    let [x, y] = ui.cursor_screen_pos();
    let center = [x + width * 0.5, y + line_h * 0.5];
    let radius = (line_h * 0.5 + scale_px(ui, 1.5)).min(width * 0.5);
    let [text_w, text_h] =
        ui.current_font()
            .calc_text_size(ui.current_font_size(), f32::MAX, 0.0, number);
    {
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
    }
    if !clickable {
        ui.dummy([width, line_h]);
        return false;
    }
    let clicked = ui.invisible_button("##world", [width, line_h]);
    if ui.is_item_hovered() {
        ui.get_window_draw_list()
            .add_circle(center, radius + scale_px(ui, 1.0), crate::theme::ACCENT)
            .thickness(scale_px(ui, 1.5))
            .build();
    }
    clicked
}

/// The world marker's tooltip while it opens the world picker: the usual
/// `wN · status` line, and that a click switches world.
fn world_switch_tooltip(ui: &Ui, tip: &str) {
    if ui.is_item_hovered() {
        ui.tooltip(|| {
            ui.text(tip);
            ui.text_disabled("click to switch world");
        });
    }
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
    // A head the render service refused (its cap is full) draws nothing:
    // its own cell says so.
    let refused = draw
        .then(|| state.session.core.slot_io(name))
        .flatten()
        .and_then(|io| io.pixels.refusal());
    if !draw || refused.is_some() {
        if let Some(tv) = state.views.remove(name) {
            tv.view.dispose(gpu);
        }
        let _id = ui.push_id(name);
        let label = match refused {
            Some(refusal) => format!("renderer refused: {refusal}"),
            None => "renderer off".to_string(),
        };
        return ui.selectable_config(&label).size(size).build();
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

/// Rail **Profiles…**: opens or focuses the profile picker (same window as
/// the strip's Profiles button).
fn add_bot_button(ui: &Ui, state: &mut PanelState) {
    ui.spacing();
    let w = ui.content_region_avail()[0];
    if ui.button_with_size("Profiles…", [w, 0.0]) {
        state.session.open_window(PanelWindow::Profiles);
    }
    ui.set_item_tooltip(
        "Open or focus Profiles to add, edit, load or log in a bot. Selecting a profile does not start its script.",
    );
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

/// Consume an opener's one-shot focus request for `window` (see
/// [`Session::open_window`]). Call immediately before its `Begin`: an
/// existing window comes to the front and a docked one has its tab
/// selected; a window ImGui has not created yet is focused by appearing.
pub(crate) fn focus_if_requested(ui: &Ui, session: &mut Session, window: PanelWindow) {
    if session.take_window_focus(window) {
        ui.set_window_focus(Some(window.title()));
    }
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
    if session.profile_editor.pending().is_none() && !open {
        return;
    }
    if !open {
        ui.open_popup(PROFILE_EDIT_SWITCH_POPUP);
    }
    ui.popup(PROFILE_EDIT_SWITCH_POPUP, || {
        let Some(switch) = session.profile_editor.pending() else {
            ui.close_current_popup();
            return;
        };
        if ui.is_key_pressed(Key::Escape) {
            session.cancel_pending_edit_switch();
            ui.close_current_popup();
            return;
        }
        popup_text(ui, switch.prompt());
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

/// The Profiles editor pane's narrowest width. The edit form is laid out
/// for the 330 px docked panel, so its content width there (`PANEL_WIDTH`
/// less the window's 8 px padding on each side) is the narrowest it reads
/// at.
const PROFILES_EDITOR_MIN_W: f32 = PANEL_WIDTH - 16.0;
/// The profile list's narrowest width beside the editor: a short name and
/// its Edit and ✕.
const PROFILES_LIST_MIN_W: f32 = 180.0;
/// The draggable divider between the list and the editor pane.
const PROFILES_DIVIDER: f32 = 8.0;
/// From this content width (314 + 8 + 180 = 502 logical px) the list and
/// the editor pane sit side by side; narrower, as in the 330 px dock, the
/// editor sits under the list.
const PROFILES_SIDE_BY_SIDE_W: f32 = PROFILES_EDITOR_MIN_W + PROFILES_DIVIDER + PROFILES_LIST_MIN_W;
/// The least height a Profiles region keeps.
const PROFILES_REGION_MIN_H: f32 = 72.0;
/// One profile row.
const PROFILE_ROW_H: f32 = 24.0;

/// What the operator did in the profile list this frame.
#[derive(Default)]
struct ListPick {
    picked: Option<String>,
    removed: Option<String>,
    edit: Option<String>,
}

/// Profile picker (single-bot and MultiBox). Click a row to focus it
/// (and load onto the wall while MultiBox is on); Edit opens the form;
/// ✕ deletes the vault row only. Load all is MultiBox-only. The list and
/// the editor pane (the edit form or the rs2b0t import, one at a time)
/// each scroll on their own, side by side when the window is wide enough
/// ([`PROFILES_SIDE_BY_SIDE_W`]) and stacked otherwise, with a draggable
/// divider between them; Close stays at the bottom.
fn chooser_window(ui: &Ui, session: &mut Session, panel_dock: Option<Id>) {
    if !session.wall.chooser_open {
        #[cfg(feature = "js")]
        if session.profile_import_open || !session.profile_import.is_fresh() {
            session.close_profile_import();
        }
        return;
    }
    // An opener asked for Profiles, or a leave prompt was staged from
    // elsewhere (the tab's ✕, MultiBox) and draws inside this window: bring
    // the window (its dock tab) forward once.
    focus_if_requested(ui, session, PanelWindow::Profiles);
    let mut open = true;
    ui.set_next_window_class(&panel_window_class());
    if let Some(id) = chooser_dock_id(panel_dock) {
        ui.set_next_window_dock_id_with_cond(id, Condition::Appearing);
    }
    ui.window(PanelWindow::Profiles.title())
        .opened(&mut open)
        .flags(WindowFlags::NO_COLLAPSE)
        .size(scale_size(ui, [PANEL_WIDTH, 560.0]), Condition::FirstUseEver)
        .build(|| {
            let right = chooser_right_edge(ui);
            // Wrap text where the on-screen part of the window ends.
            let wrap =
                ui.cursor_screen_pos()[0] + on_screen_avail(ui, right) - ui.window_pos()[0];
            let _wrap = ui.push_text_wrap_pos(wrap);
            if !session.core.vault_open() {
                vault_unlock_prompt(ui, session);
            } else {
                profiles_toolbar(ui, session, right);
                let footer = profiles_footer_height(ui, session);
                let region_h = (ui.content_region_avail()[1] - footer)
                    .max(scale_px(ui, PROFILES_REGION_MIN_H));
                let pick = profiles_regions(ui, session, right, region_h);
                if let Some(name) = pick.edit {
                    session.begin_edit_profile(Some(&name));
                } else if let Some(name) = pick.removed {
                    session.delete_understood = false;
                    session.pending_profile_delete = Some(name);
                    ui.open_popup(PROFILE_DELETE_POPUP);
                } else if let Some(name) = pick.picked {
                    // Row pick is pure focus (a wall load in MultiBox): the
                    // picker and the editor pane stay exactly as they are.
                    session.pick_profile(&name);
                }
                edit_switch_popup(ui, session);
            }
            ui.spacing();
            if let Some(record) = session.core.saves_in_flight().last() {
                let name = record.destination();
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

/// Load all (MultiBox), then New profile; JavaScript builds also expose
/// import. While an import's writes are being saved the editor pane keeps
/// its report, so New profile waits.
fn profiles_toolbar(ui: &Ui, session: &mut Session, right: f32) {
    let w = on_screen_avail(ui, right);
    if session.multibox && ui.button_with_size("Load all", [w, 0.0]) {
        session.load_all();
    }
    #[cfg(feature = "js")]
    let (bw, stack) = scaled_button_row_layout(ui, w, 2);
    #[cfg(not(feature = "js"))]
    let bw = w;
    #[cfg(feature = "js")]
    let writing = session.import_writing();
    #[cfg(feature = "js")]
    let _waits = ui.begin_disabled_with_cond(writing);
    if ui.button_with_size("New profile", [bw, 0.0]) {
        session.begin_edit_profile(None);
    }
    #[cfg(feature = "js")]
    {
        drop(_waits);
        if !stack {
            gap_line(ui);
        }
        let _open = ui.begin_disabled_with_cond(session.profile_import_open);
        if ui.button_with_size("Import from rs2b0t…", [bw, 0.0]) {
            session.request_open_import();
        }
        if writing {
            ui.text_disabled("Editing waits until the import's writes are saved.");
        }
    }
}

/// The height under the split regions: the spacing, the saving line and
/// Close.
fn profiles_footer_height(ui: &Ui, session: &Session) -> f32 {
    let spacing = ui.clone_style().item_spacing()[1];
    let saving = if session.core.saves_in_flight().is_empty() {
        0.0
    } else {
        ui.text_line_height_with_spacing()
    };
    spacing + saving + ui.frame_height_with_spacing()
}

/// The profile list and, when the form or the import is open, the editor
/// pane, in `region_h`: side by side from [`PROFILES_SIDE_BY_SIDE_W`],
/// stacked below it. Returns what the list was asked to do.
fn profiles_regions(ui: &Ui, session: &mut Session, right: f32, region_h: f32) -> ListPick {
    let w = on_screen_avail(ui, right);
    #[cfg(feature = "js")]
    let import_open = session.profile_import_open;
    #[cfg(not(feature = "js"))]
    let import_open = false;
    if !session.profile_editor.is_open() && !import_open {
        return profile_list(ui, session, right, [w, region_h]);
    }
    let divider = scale_px(ui, PROFILES_DIVIDER);
    if w >= scale_px(ui, PROFILES_SIDE_BY_SIDE_W) {
        let list_w = (w * session.profiles_split[0])
            .clamp(
                scale_px(ui, PROFILES_LIST_MIN_W),
                w - divider - scale_px(ui, PROFILES_EDITOR_MIN_W),
            )
            .floor();
        let pick = profile_list(ui, session, right, [list_w, region_h]);
        ui.same_line_with_spacing(0.0, 0.0);
        if let Some(dx) = profiles_divider(ui, "##profiles-divider-x", [divider, region_h], true) {
            session.profiles_split[0] = ((list_w + dx) / w).clamp(0.0, 1.0);
        }
        ui.same_line_with_spacing(0.0, 0.0);
        ui.group(|| profiles_editor(ui, session, [w - list_w - divider, region_h]));
        pick
    } else {
        // The list and the divider are each followed by item spacing.
        let gaps = 2.0 * ui.clone_style().item_spacing()[1];
        let min_h = scale_px(ui, PROFILES_REGION_MIN_H);
        let max_list = (region_h - divider - gaps - min_h).max(min_h);
        let list_h = (region_h * session.profiles_split[1])
            .clamp(min_h, max_list)
            .min(profile_list_height(ui, session))
            .floor();
        let pick = profile_list(ui, session, right, [w, list_h]);
        if let Some(dy) = profiles_divider(ui, "##profiles-divider-y", [w, divider], false) {
            session.profiles_split[1] = ((list_h + dy) / region_h).clamp(0.0, 1.0);
        }
        profiles_editor(
            ui,
            session,
            [w, (region_h - list_h - divider - gaps).max(min_h)],
        );
        pick
    }
}

/// The height the whole profile list needs: its heading and every row.
fn profile_list_height(ui: &Ui, session: &Session) -> f32 {
    let rows = session.core.profiles().count().max(1);
    ui.text_line_height_with_spacing() + scale_px(ui, rows as f32 * PROFILE_ROW_H + 8.0)
}

/// A draggable divider of `size` between the list and the editor pane
/// (`vertical`: a column between them side by side). Returns how far it
/// was dragged this frame.
fn profiles_divider(ui: &Ui, id: &str, size: [f32; 2], vertical: bool) -> Option<f32> {
    let min = ui.cursor_screen_pos();
    ui.invisible_button(id, size);
    let hovered = ui.is_item_hovered();
    let active = ui.is_item_active();
    if hovered || active {
        ui.set_mouse_cursor(Some(if vertical {
            dear_imgui_rs::MouseCursor::ResizeEW
        } else {
            dear_imgui_rs::MouseCursor::ResizeNS
        }));
    }
    let colour = if hovered || active { ACCENT } else { TEXT_DIM };
    let draw = ui.get_window_draw_list();
    if vertical {
        let x = (min[0] + size[0] * 0.5).floor();
        draw.add_line([x, min[1]], [x, min[1] + size[1]], colour)
            .build();
    } else {
        let y = (min[1] + size[1] * 0.5).floor();
        draw.add_line([min[0], y], [min[0] + size[0], y], colour)
            .build();
    }
    let delta = ui.io().mouse_delta()[usize::from(!vertical)];
    (active && delta != 0.0).then_some(delta)
}

/// The profile list in its own scrolling region of `size`.
fn profile_list(ui: &Ui, session: &Session, right: f32, size: [f32; 2]) -> ListPick {
    let names: Vec<String> = session
        .core
        .profiles()
        .map(|p| p.username().to_string())
        .collect();
    let focused = session.focused_name();
    let members = session.core.members();
    let multibox = session.multibox;
    // The JavaScript import pane keeps its report while writes settle.
    #[cfg(feature = "js")]
    let edit_waits = session.import_writing();
    #[cfg(not(feature = "js"))]
    let edit_waits = false;
    let mut pick = ListPick::default();
    ui.child_window("##profiles-list")
        .size(size)
        .child_flags(ChildFlags::NAV_FLATTENED)
        .build(ui, || {
            ui.text_colored(ACCENT, "Vault profiles");
            if names.is_empty() {
                ui.text_disabled("vault is empty — New profile then Save");
                return;
            }
            for name in &names {
                let selected = if multibox {
                    members.iter().any(|m| m == name)
                } else {
                    focused.as_deref() == Some(name.as_str())
                };
                // A profile whose first save is still being written is not
                // selectable, editable or deletable until the write settles.
                let saving = session.core.profile_saving(name);
                let _saving = saving.then(|| ui.begin_disabled());
                let (p, r, e) = chooser_row(ui, name, selected, right, edit_waits);
                if saving {
                    continue;
                }
                if e {
                    pick.edit = Some(name.clone());
                } else if r {
                    pick.removed = Some(name.clone());
                } else if p {
                    pick.picked = Some(name.clone());
                }
            }
        });
    pick
}

/// The editor pane in `size`: the edit form or the rs2b0t import. Its body
/// scrolls on its own; its buttons (and the form's save notice) stay under
/// it.
fn profiles_editor(ui: &Ui, session: &mut Session, size: [f32; 2]) {
    let [w, h] = size;
    let min_h = scale_px(ui, PROFILES_REGION_MIN_H) * 0.5;
    // The body is followed by item spacing, then the buttons.
    let h = h - ui.clone_style().item_spacing()[1];
    #[cfg(feature = "js")]
    if session.profile_import_open {
        let bar = profile_import_bar_height(ui, w);
        let mut preview = false;
        ui.child_window("##profiles-editor")
            .size([w, (h - bar).max(min_h)])
            .child_flags(ChildFlags::NAV_FLATTENED)
            .build(ui, || preview = profile_import_body(ui, session));
        profile_import_bar(ui, session, w, preview);
        return;
    }
    if session.profile_editor.is_open() {
        let bar = profile_form_bar_height(ui, session, w);
        ui.child_window("##profiles-editor")
            .size([w, (h - bar).max(min_h)])
            .child_flags(ChildFlags::NAV_FLATTENED)
            .build(ui, || profile_form_body(ui, session));
        profile_form_bar(ui, session, w);
    }
}

/// The edit form's fields, in the editor pane.
fn profile_form_body(ui: &Ui, session: &mut Session) {
    {
        let edit = session.profile_editor.target().unwrap_or("");
        if edit.is_empty() {
            ui.text_colored(ACCENT, "New profile");
        } else {
            ui.text_colored(ACCENT, "Editing");
            ui.same_line();
            ui.text_colored(ACCENT, edit);
        }
    }
    // One id scope per opened target: a text field still active from the
    // previous target cannot write its edit buffer into this one's fields.
    let _form = ui.push_id(session.profile_editor.generation());
    ui.text_disabled("user");
    let username_changed = ui
        .input_text("##cred-user", session.profile_editor.username_mut())
        .hint("username")
        .build();
    if username_changed {
        let _ = session.submit(
            OperatorCommand::EditProfileDraft(frontend_core::command::ProfileDraftField::Username),
            Targets::Application,
        );
    }
    ui.text_disabled("pass");
    if ui
        .input_text("##cred-pass", session.profile_editor.password_mut())
        .password(true)
        .hint("password")
        .build()
    {
        let _ = session.submit(
            OperatorCommand::EditProfileDraft(frontend_core::command::ProfileDraftField::Password),
            Targets::Application,
        );
    }
    slot_capture_section(ui, session);
    slot_random_section(ui, session);
    slot_survival_section(ui, session);
    slot_clue_section(ui, session);
}

/// The height of one wrapped block of `text` at `width`, with its spacing.
fn wrapped_height(ui: &Ui, text: &str, width: f32) -> f32 {
    ui.current_font()
        .calc_text_size(ui.current_font_size(), f32::MAX, width.max(1.0), text)[1]
        + ui.clone_style().item_spacing()[1]
}

/// The rows a two-button bar of `width` takes.
fn button_rows(ui: &Ui, width: f32) -> f32 {
    let (_, stack) = scaled_button_row_layout(ui, width, 2);
    if stack {
        2.0
    } else {
        1.0
    }
}

fn profile_form_bar_height(ui: &Ui, session: &Session, width: f32) -> f32 {
    let notice = match session.profile_editor.save_state().notice() {
        Some(notice) if notice.is_saved() => wrapped_height(ui, notice.text(), width),
        Some(notice) => notice.error().map_or(0.0, |reason| {
            wrapped_height(ui, reason, width)
                + wrapped_height(ui, frontend_core::NOTHING_SAVED, width)
        }),
        None => 0.0,
    };
    notice + button_rows(ui, width) * ui.frame_height_with_spacing()
}

/// Save's outcome and Save / Cancel, under the form's body.
fn profile_form_bar(ui: &Ui, session: &mut Session, width: f32) {
    let _form = ui.push_id(session.profile_editor.generation());
    // Save's outcome, next to Save: a refusal or a write that failed after
    // it was queued (nothing was written; the banner is hidden while
    // Profiles is open) or, once the write is durable, `Saved <name>.`.
    match session.profile_editor.save_state().notice() {
        Some(notice) if notice.is_saved() => ui.text_colored(GREEN, notice.text()),
        Some(notice) => {
            if let Some(reason) = notice.error() {
                ui.text_colored(ERROR, reason);
                ui.text_disabled(frontend_core::NOTHING_SAVED);
            }
        }
        None => {}
    }
    let (bw, stack) = scaled_button_row_layout(ui, width, 2);
    // One Save at a time: the form follows a rename or a new profile only
    // once its write is durable.
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

#[cfg(feature = "js")]
const IMPORT_DELETE_POPUP: &str = "Delete the rs2b0t export file?";

/// The rs2b0t profile import in the editor pane (vault unlocked): the
/// export file's path, then a preview with the choices (nothing is written
/// until Import), then the report while the writes are saved and after.
/// The flow is frontend-core's, shared with the TUI. Returns whether Enter
/// asked for the preview.
#[cfg(feature = "js")]
fn profile_import_body(ui: &Ui, session: &mut Session) -> bool {
    use frontend_core::rs2b0t_import::{ConflictPolicy, Stage, PROFILE_FILE_NAME};
    ui.text_colored(ACCENT, "Import rs2b0t profiles");
    let stage = session.profile_import.stage();
    let mut entered = false;
    match stage {
        Stage::Path => {
            ui.text_wrapped(format!(
                "Path of the {PROFILE_FILE_NAME} file rs2b0t exported (its Settings → export). \
                 The preview lists every account and setting before anything is written."
            ));
            ui.set_next_item_width(-1.0);
            let path_changed = ui
                .input_text("##rs2b0t-import-path", session.profile_import.path_mut())
                .hint(PROFILE_FILE_NAME)
                .enter_returns_true(true)
                .build();
            if path_changed {
                let _ =
                    session.submit(OperatorCommand::EditProfileImportPath, Targets::Application);
            }
            entered = path_changed;
        }
        Stage::Preview => {
            ui.text_disabled("Existing profiles:");
            let policy = session.profile_import.policy();
            let mut next = policy;
            // The two choices share a line when it fits them.
            let style = ui.clone_style();
            let radio = |label: &str| {
                ui.frame_height()
                    + style.item_inner_spacing()[0]
                    + ui.current_font()
                        .calc_text_size(ui.current_font_size(), f32::MAX, 0.0, label)[0]
            };
            let one_line = radio("Skip (keep them)") + style.item_spacing()[0] + radio("Overwrite")
                <= ui.content_region_avail()[0];
            if ui.radio_button("Skip (keep them)", policy == ConflictPolicy::Skip) {
                next = ConflictPolicy::Skip;
            }
            if one_line {
                ui.same_line();
            }
            if ui.radio_button("Overwrite", policy == ConflictPolicy::Overwrite) {
                next = ConflictPolicy::Overwrite;
            }
            if next != policy {
                let _ = session.submit(
                    OperatorCommand::SetProfileImportPolicy(next),
                    Targets::Application,
                );
            }
            // The preview's lines say which accounts and why.
            let unusable = session
                .profile_import
                .plan()
                .map_or(0, |plan| plan.unusable());
            if unusable > 0 {
                let mut include = session.profile_import.include_unusable();
                if ui.checkbox("Import unusable accounts", &mut include) {
                    let _ = session.submit(
                        OperatorCommand::SetProfileImportIncludeUnusable(include),
                        Targets::Application,
                    );
                }
            }
        }
        Stage::Report => {
            if !session.profile_import_settled {
                session.refresh_profile_import();
            }
        }
    }
    if let Some(error) = session.profile_import.error() {
        ui.text_colored(ERROR, error);
    }
    if stage != Stage::Path {
        ui.separator();
        for line in &session.profile_import_lines {
            ui.text_wrapped(line);
        }
    }
    entered
}

#[cfg(feature = "js")]
fn profile_import_bar_height(ui: &Ui, width: f32) -> f32 {
    button_rows(ui, width) * ui.frame_height_with_spacing()
}

/// The import's buttons under its body, and the delete-file confirm.
#[cfg(feature = "js")]
fn profile_import_bar(ui: &Ui, session: &mut Session, width: f32, entered: bool) {
    use frontend_core::rs2b0t_import::Stage;
    let (bw, stack) = scaled_button_row_layout(ui, width, 2);
    let mut changed = false;
    match session.profile_import.stage() {
        Stage::Path => {
            if ui.button_with_size("Preview", [bw, 0.0]) || entered {
                // Script names map through the catalog: register it from its
                // known root first (no folder prompt).
                let _ = session.submit(OperatorCommand::ImportCatalog, Targets::Application);
                let _ = session.submit(OperatorCommand::PreviewProfileImport, Targets::Application);
                changed = true;
            }
            if !stack {
                gap_line(ui);
            }
            if ui.button_with_size("Cancel##rs2b0t-import", [bw, 0.0]) {
                session.close_profile_import();
                return;
            }
        }
        Stage::Preview => {
            if ui.button_with_size("Import", [bw, 0.0]) {
                match session
                    .profile_import
                    .prepare_confirmation(&session.core, &session.scripts)
                {
                    Ok(()) => {
                        let planned = session
                            .profile_import
                            .planned_writes()
                            .map(|(index, name, command)| (index, name.to_owned(), command))
                            .collect::<Vec<_>>();
                        for (index, name, planned_command) in planned {
                            let staged = match planned_command {
                                OperatorCommand::CreateProfile => {
                                    session.profile_import.stage_create_account(index)
                                }
                                OperatorCommand::SaveProfile => {
                                    session.profile_import.stage_save_account(index)
                                }
                                _ => {
                                    session.refuse_command(
                                        planned_command,
                                        &[&name],
                                        None,
                                        &format!(
                                            "Profile import account {name} has an unsupported writer."
                                        ),
                                    );
                                    break;
                                }
                            };
                            if let Err(error) = staged {
                                session.refuse_command(
                                    planned_command,
                                    &[&name],
                                    None,
                                    &error.to_string(),
                                );
                                break;
                            }
                            let targets = if planned_command == OperatorCommand::CreateProfile {
                                Targets::Application
                            } else {
                                Targets::Profile(&name)
                            };
                            let _ = session.submit(planned_command, targets);
                            session.profile_import.cancel_staged_account(index);
                        }
                    }
                    // Import replans the preview against the current vault
                    // before any account row exists; a refused replan is
                    // refused as the preview it repeats.
                    Err(error) => session.refuse_command(
                        OperatorCommand::PreviewProfileImport,
                        &[],
                        None,
                        &error.to_string(),
                    ),
                }
                changed = true;
            }
            if !stack {
                gap_line(ui);
            }
            if ui.button_with_size("Back##rs2b0t-import", [bw, 0.0]) {
                let _ = session.submit(
                    OperatorCommand::ReturnToProfileImportPath,
                    Targets::Application,
                );
                changed = true;
            }
        }
        Stage::Report => {
            if session.profile_import.can_delete(&session.core) {
                if ui.button_with_size("Delete export file…", [bw, 0.0]) {
                    session.import_delete_understood = false;
                    let _ = session.submit(
                        OperatorCommand::SetProfileImportDeletePrompt(true),
                        Targets::Application,
                    );
                    if session.profile_import.delete_asked() {
                        ui.open_popup(IMPORT_DELETE_POPUP);
                    }
                }
                if !stack {
                    gap_line(ui);
                }
            }
            {
                // The report stays until its writes are saved.
                let _saving = ui.begin_disabled_with_cond(session.import_writing());
                if ui.button_with_size("Done##rs2b0t-import", [bw, 0.0]) {
                    session.close_profile_import();
                    return;
                }
            }
            let delete_prompt_asked = session.profile_import.delete_asked();
            let body = match session.profile_import.export_path() {
                Some(path) => format!(
                    "Delete {}? It holds your rs2b0t passwords in plain text. This cannot be undone.",
                    path.display()
                ),
                None => "Delete the export file?".into(),
            };
            if scary_confirm_popup(
                ui,
                IMPORT_DELETE_POPUP,
                &body,
                "Delete",
                &mut session.import_delete_understood,
            ) {
                let _ = session.submit(
                    OperatorCommand::DeleteProfileImportExport,
                    Targets::Application,
                );
                changed = true;
            } else if delete_prompt_asked && !ui.is_popup_open(IMPORT_DELETE_POPUP) {
                let _ = session.submit(
                    OperatorCommand::SetProfileImportDeletePrompt(false),
                    Targets::Application,
                );
            }
        }
    }
    if changed {
        session.refresh_profile_import();
    }
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
    focus_if_requested(ui, session, PanelWindow::GeneralConfig);
    ui.window(PanelWindow::GeneralConfig.title())
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
    focus_if_requested(ui, session, PanelWindow::Debug);
    ui.window(PanelWindow::Debug.title())
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
            session
                .profile_editor
                .draft_mut()
                .settings_mut()
                .clue_duel_partner_mut(),
        )
        .hint("other account name")
        .build()
    {
        let _ = session.submit(
            OperatorCommand::EditProfileDraft(frontend_core::command::ProfileDraftField::Settings),
            Targets::Application,
        );
    }
    ui.text_wrapped(
        "run this account in Duel Arena Clue helper mode when clue 3554 needs the crossing",
    );
}

/// Random-event toggles, per-event Guardian policies and lamp reward settings.
/// Profile choices use the same vault save path as the existing random-event
/// and lamp controls.
fn slot_random_section(ui: &Ui, session: &mut Session) {
    let mut events = session.profile_editor.draft().settings().random_events();
    if ui.checkbox("random events", &mut events) {
        *session
            .profile_editor
            .draft_mut()
            .settings_mut()
            .random_events_mut() = events;
        if let Some(name) = session
            .profile_editor
            .target()
            .filter(|n| !n.is_empty())
            .map(str::to_owned)
        {
            let skill = session
                .profile_editor
                .draft()
                .settings()
                .lamp_skill()
                .to_string();
            let lamp_auto = session.profile_editor.draft().settings().lamp_auto();
            session
                .core
                .stage_guardian(frontend_core::GuardianEdit::fields(
                    events, &skill, lamp_auto,
                ));
            if let Some(submitted) =
                session.submit(OperatorCommand::SetGuardian, Targets::Profile(&name))
            {
                session.error = session
                    .core
                    .command_failure(submitted.request())
                    .map(|reason| reason.to_string());
            }
        }
        let _ = session.submit(
            OperatorCommand::EditProfileDraft(frontend_core::command::ProfileDraftField::Settings),
            Targets::Application,
        );
    }
    ui.text_wrapped("Off: show event status only; do not handle or hold events.");
    let policies_open = ui.collapsing_header("Policies", TreeNodeFlags::NONE);
    ui.set_item_tooltip(
        "Per-event response: Flee is available; Fight and Ignore are disabled, and saved choices are treated as Flee.",
    );
    if policies_open {
        for (event, label) in frontend_core::GUARDIAN_EVENTS {
            ui.text_disabled(label);
            ui.same_line();
            let mut policy = session
                .profile_editor
                .draft()
                .settings()
                .random_event_policy(event);
            if random_event_policy_combo(ui, event, &mut policy) {
                session
                    .profile_editor
                    .draft_mut()
                    .settings_mut()
                    .set_random_event_policy(event, policy);
                if let Some(name) = session
                    .profile_editor
                    .target()
                    .filter(|n| !n.is_empty())
                    .map(str::to_owned)
                {
                    session
                        .core
                        .stage_guardian(frontend_core::GuardianEdit::policy(event, policy));
                    if let Some(submitted) =
                        session.submit(OperatorCommand::SetGuardian, Targets::Profile(&name))
                    {
                        session.error = session
                            .core
                            .command_failure(submitted.request())
                            .map(|reason| reason.to_string());
                    }
                }
                let _ = session.submit(
                    OperatorCommand::EditProfileDraft(
                        frontend_core::command::ProfileDraftField::Settings,
                    ),
                    Targets::Application,
                );
            }
        }
    }
    let mut auto = session.profile_editor.draft().settings().lamp_auto();
    if ui.checkbox("lamp auto", &mut auto) {
        *session
            .profile_editor
            .draft_mut()
            .settings_mut()
            .lamp_auto_mut() = auto;
        if let Some(name) = session
            .profile_editor
            .target()
            .filter(|n| !n.is_empty())
            .map(str::to_owned)
        {
            let events = session.profile_editor.draft().settings().random_events();
            let skill = session
                .profile_editor
                .draft()
                .settings()
                .lamp_skill()
                .to_string();
            session
                .core
                .stage_guardian(frontend_core::GuardianEdit::fields(events, &skill, auto));
            if let Some(submitted) =
                session.submit(OperatorCommand::SetGuardian, Targets::Profile(&name))
            {
                session.error = session
                    .core
                    .command_failure(submitted.request())
                    .map(|reason| reason.to_string());
            }
        }
        let _ = session.submit(
            OperatorCommand::EditProfileDraft(frontend_core::command::ProfileDraftField::Settings),
            Targets::Application,
        );
    }
    ui.text_wrapped(
        "this profile; claim a lamp reward without a confirmation click (guardian rubs when lamp auto is on).",
    );
    let mut skill = session
        .profile_editor
        .draft()
        .settings()
        .lamp_skill()
        .to_string();
    if lamp_skill_combo(ui, &mut skill) {
        *session
            .profile_editor
            .draft_mut()
            .settings_mut()
            .lamp_skill_mut() = skill.clone();
        if let Some(name) = session
            .profile_editor
            .target()
            .filter(|n| !n.is_empty())
            .map(str::to_owned)
        {
            let events = session.profile_editor.draft().settings().random_events();
            let lamp_auto = session.profile_editor.draft().settings().lamp_auto();
            session
                .core
                .stage_guardian(frontend_core::GuardianEdit::fields(
                    events, &skill, lamp_auto,
                ));
            if let Some(submitted) =
                session.submit(OperatorCommand::SetGuardian, Targets::Profile(&name))
            {
                session.error = session
                    .core
                    .command_failure(submitted.request())
                    .map(|reason| reason.to_string());
            }
        }
        let _ = session.submit(
            OperatorCommand::EditProfileDraft(frontend_core::command::ProfileDraftField::Settings),
            Targets::Application,
        );
    }
}

/// The Survival and supplies section: the profile's Cure poison row.
/// The choice uses the same vault save path as the random-event and
/// lamp controls, and a running slot sees it within one frame.
fn slot_survival_section(ui: &Ui, session: &mut Session) {
    ui.text_disabled("Survival and supplies");
    ui.text_disabled("Cure poison");
    ui.same_line();
    let mut policy = session
        .profile_editor
        .draft()
        .settings()
        .survival()
        .cure_poison();
    if cure_poison_combo(ui, &mut policy) {
        session
            .profile_editor
            .draft_mut()
            .settings_mut()
            .survival_mut()
            .set_cure_poison(policy);
        if let Some(name) = session
            .profile_editor
            .target()
            .filter(|n| !n.is_empty())
            .map(str::to_owned)
        {
            session
                .core
                .stage_survival(frontend_core::SurvivalEdit::cure_poison(policy));
            if let Some(submitted) =
                session.submit(OperatorCommand::SetSurvival, Targets::Profile(&name))
            {
                session.error = session
                    .core
                    .command_failure(submitted.request())
                    .map(|reason| reason.to_string());
            }
        }
        let _ = session.submit(
            OperatorCommand::EditProfileDraft(frontend_core::command::ProfileDraftField::Settings),
            Targets::Application,
        );
    }
    ui.text_wrapped(policy.label());
}

fn cure_poison_combo(ui: &Ui, policy: &mut frontend_core::CurePoison) -> bool {
    use frontend_core::CurePoison::{Always, Never, WhenNeeded};

    let preview = match *policy {
        WhenNeeded => "When needed",
        Always => "Always",
        Never => "Never",
    };
    let options = ComboBoxOptions::new().preview_mode(ComboBoxPreviewMode::Preview);
    let mut changed = false;
    ui.set_next_item_width(-1.0);
    if let Some(_open) = ui.begin_combo_with_flags("##cure-poison", preview, options) {
        for (option, label) in [
            (WhenNeeded, "When needed"),
            (Always, "Always"),
            (Never, "Never"),
        ] {
            let selected = option == *policy;
            if ui.selectable_config(label).selected(selected).build() {
                *policy = option;
                changed = true;
            }
            if selected {
                ui.set_item_default_focus();
            }
        }
    }
    changed
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

fn random_event_policy_combo(
    ui: &Ui,
    event: &str,
    policy: &mut frontend_core::GuardianPolicy,
) -> bool {
    use frontend_core::GuardianPolicy::{Fight, Flee, Ignore};

    let preview = match *policy {
        Flee => "Flee",
        Fight => "Fight (not available yet)",
        Ignore => "Ignore (not available yet)",
    };
    let id = format!("##random-event-policy-{event}");
    let options = ComboBoxOptions::new().preview_mode(ComboBoxPreviewMode::Preview);
    let mut changed = false;
    ui.set_next_item_width(-1.0);
    if let Some(_open) = ui.begin_combo_with_flags(&id, preview, options) {
        for (option, label) in [
            (Flee, "Flee"),
            (Fight, "Fight (not available yet)"),
            (Ignore, "Ignore (not available yet)"),
        ] {
            let selected = option == *policy;
            let _disabled = (option != Flee).then(|| ui.begin_disabled());
            if ui.selectable_config(label).selected(selected).build() && option == Flee {
                *policy = option;
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
/// on screen when the docked window runs past the display. `edit_waits`
/// disables Edit (an import's writes keep the editor pane).
fn chooser_row(
    ui: &Ui,
    name: &str,
    selected: bool,
    right: f32,
    edit_waits: bool,
) -> (bool, bool, bool) {
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
    let edit = {
        let _waits = ui.begin_disabled_with_cond(edit_waits);
        ui.button_with_size(format!("Edit##edit-{name}"), [edit_w, 0.0])
    };
    gap_line(ui);
    let _red = ui.push_style_color(StyleColor::Text, ERROR);
    let removed = ui.button_with_size(format!("✕##{name}"), [x_w, 0.0]);
    (loaded, removed, edit)
}

/// Offline prepare: mint isolated identity, write server-native `.sav` via
/// `tools/harness`, write durable fixture identity — no live engine/window.
fn run_offline_prepare_fixture(args: &PanelArgs, scenario: &str) -> Result<(), String> {
    let scenario_value = frontend_core::live::Scenario::get(scenario)
        .ok_or_else(|| format!("unknown scenario {scenario}"))?;
    let view = scenario_value.view();
    let profile_count = view.profile_count();
    if profile_count == 0 {
        return Err(format!("scenario {scenario} has zero seed profiles"));
    }
    let server_root = args
        .server_root
        .clone()
        .or_else(|| std::env::var_os("BOT_SERVER_ROOT").map(PathBuf::from))
        .ok_or("offline fixture preparation requires --server-root or BOT_SERVER_ROOT")?;
    if !server_root.join("data/pack/server/obj.dat").is_file() {
        return Err(format!(
            "server root missing pack data: {} (pass --server-root or BOT_SERVER_ROOT)",
            server_root.display()
        ));
    }
    let identity_path = args
        .fixture_path
        .clone()
        .unwrap_or_else(|| scenario_value.default_fixture_path());
    let sav_dir = identity_path
        .parent()
        .map(|path| path.join(view.name()))
        .unwrap_or_else(|| scenario_value.default_fixture_sav_dir());
    let profiles = frontend_core::live::LiveProfiles::mint(profile_count);
    let passwords = profiles
        .entries()
        .iter()
        .map(|(_, password)| password.clone())
        .collect();
    let identity = scenario_value.prepare_offline_fixture(
        server_root,
        identity_path.clone(),
        sav_dir,
        profiles.names().to_vec(),
        passwords,
        profiles.passphrase().to_owned(),
    )?;
    let accounts = identity.accounts();
    println!(
        "[panel] offline fixture prepared: scenario={} identity={} accounts={}",
        identity.scenario_name(),
        identity_path.display(),
        accounts.len()
    );
    for (username, sav_path, sha256, bytes) in accounts {
        println!("[panel]   {username} sav={sav_path} sha256={sha256} bytes={bytes}");
    }
    println!("PASS: offline prepare fixture {scenario}");
    Ok(())
}

fn init_panel_running(
    args: &PanelArgs,
    shot_state: Arc<Mutex<crate::window::ShotState>>,
    permit: InstancePermit,
) -> Result<(PanelState, StartupPreparation), window::PanelError> {
    let mut state = PanelState::with_session(Session::with_instance(permit), shot_state);
    state.session.set_memory_override(args.memory_override);
    state.session.set_nav_paints_override(args.nav_paints);
    state.session.set_catalog_core_enabled(args.catalog_core);
    state.session.set_pair_core_enabled(args.pair_core);
    #[cfg(feature = "js")]
    state.session.set_external_core_enabled(args.external_core);
    #[cfg(feature = "js")]
    state.session.set_external_ts(args.external_ts.clone());
    crate::log_pane::apply_session_log_pref(&mut state.session);
    let fixture_mode = if args.run_prepared {
        frontend_core::live::FixtureMode::RunPrepared
    } else {
        frontend_core::live::FixtureMode::Default
    };
    state
        .session
        .set_fixture_boot(fixture_mode, args.fixture_path.clone());
    state
        .session
        .configure_profile(args.profile.duplicate())
        .map_err(|error| window::PanelError::ServerProfile(error.to_string()))?;
    let vault_pass = if args.vault_pass_stdin && matches!(args.mode, RunMode::Interactive) {
        let exists = state
            .session
            .default_vault_exists()
            .map_err(|error| window::PanelError::ServerProfile(error.to_string()))?;
        match frontend_core::launch::obtain_passphrase("panel-play", true, exists) {
            Ok(pass) => Some(pass),
            Err(msg) => {
                eprintln!("FAIL: {msg}");
                exit_after_work_shutdown(1);
            }
        }
    } else {
        None
    };
    let boot = boot_for(&args.mode, vault_pass);
    #[cfg(feature = "memory-profile")]
    let boot = match frontend_core::bench::BenchConfig::from_env() {
        Ok(Some(config)) => {
            client::profiling::enable();
            // The profile is not bound yet (an unbound session reads as
            // Remote), so check the class of the profile it will bind.
            let class = state
                .session
                .launcher
                .configured_class(state.session.ui.server_revision);
            let live = class.map_err(|error| error.to_string()).and_then(|class| {
                frontend_core::bench::require_live_benchmark(class)
                    .map_err(|error| error.to_string())
            });
            if let Err(error) = live {
                eprintln!("FAIL: {error}");
                exit_after_work_shutdown(1);
            }
            Some(Boot::Memory(config))
        }
        Ok(None) => boot,
        Err(error) => {
            eprintln!("FAIL: {error}");
            exit_after_work_shutdown(1);
        }
    };
    Ok((state, StartupPreparation::new(boot)))
}

/// `Some(true)` continue, `Some(false)` exit, `None` still open.
fn instance_conflict_choice(ui: &Ui, conflict: &InstanceConflict) -> Option<bool> {
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
            ui.text_wrapped(conflict.message());
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

/// Resolve a normal event-loop return against a failure latched by a terminal
/// frame. The failure remains fatal if the window closes during that frame.
fn finish_panel_run(
    result: Result<(), window::PanelError>,
    failure: Option<window::PanelError>,
) -> Result<(), window::PanelError> {
    result.and_then(|()| failure.map_or(Ok(()), Err))
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
            exit_after_work_shutdown(1);
        });
    }
    let skip_lock = !matches!(args.mode, RunMode::Interactive);
    #[cfg(feature = "memory-profile")]
    let skip_lock = skip_lock
        || frontend_core::bench::BenchConfig::from_env()
            .ok()
            .flatten()
            .is_some();
    let shot_state = Arc::new(Mutex::new(crate::window::ShotState::default()));
    let mut instance_prompt = None;
    let mut running = None;
    match frontend_core::launch::acquire_instance(InstanceKind::Panel, skip_lock) {
        Ok(Ok(permit)) => {
            running = Some(init_panel_running(&args, Arc::clone(&shot_state), permit)?);
        }
        Ok(Err(conflict)) => {
            instance_prompt = Some(conflict);
        }
        Err(e) => {
            eprintln!("panel: instance lock: {e}");
            exit_after_work_shutdown(1);
        }
    }
    let running = Rc::new(RefCell::new(running));
    let mut presented = false;

    let mut cfg = runner_config();
    let mut window_title = running
        .borrow()
        .as_ref()
        .map(|(state, _)| state.session.app_title())
        .unwrap_or_else(|| "274bot".into());
    cfg.window_title.clone_from(&window_title);
    let os_window: Arc<Mutex<Option<Arc<winit::window::Window>>>> = Arc::new(Mutex::new(None));
    let os_window_init = Arc::clone(&os_window);
    let terminal_failure = Arc::new(Mutex::new(None));
    let terminal_failure_frame = Arc::clone(&terminal_failure);
    let frame_running = Rc::clone(&running);
    let result = window::run(
        cfg,
        amber_style,
        move |window, device, queue, _| {
            client::render::backend::inject_device(device.clone(), queue.clone());
            *os_window_init.lock().unwrap() = Some(Arc::clone(window));
        },
        panel_gpu_shutdown(Rc::clone(&running)),
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
                            InstancePermit::skip(),
                        ) {
                            Ok(ready) => *frame_running.borrow_mut() = Some(ready),
                            Err(e) => {
                                eprintln!("panel: {e}");
                                exit_after_work_shutdown(1);
                            }
                        }
                    }
                }
            }
            let mut running = frame_running.borrow_mut();
            let Some((state, startup)) = running.as_mut() else {
                return false;
            };
            if presented {
                if let Some(error) = drive_startup(state, startup) {
                    *terminal_failure_frame.lock().unwrap() = Some(error);
                    return true;
                }
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
            ui_frame(ui, gpu, state, progress, &terminal_failure_frame)
        },
    );
    let failure = terminal_failure.lock().unwrap().take();
    finish_panel_run(result, failure)
}

/// The window's GPU shutdown hook: release the walk map's textures while
/// the renderer that registered them is still live.
fn panel_gpu_shutdown(
    running: Rc<RefCell<Option<(PanelState, StartupPreparation)>>>,
) -> impl FnMut(&mut Gpu) + 'static {
    move |gpu| {
        if let Some((state, _)) = running.borrow_mut().as_mut() {
            state.walk_map.release(Some(gpu));
        }
    }
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
            match frontend_core::live::create_shot_run_dir() {
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
            match frontend_core::live::write_shot(
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

/// Headed recording (`HEADED_RECORD=1`): arm once the live run's shot dir
/// exists, stage one whole-window readback when the pacer fires, and move
/// completed frames into the background encoder. When the switch is off
/// this is a single disabled branch; harness runs only (interactive F12
/// shots never arm a recording).
fn pump_record(state: &mut PanelState) {
    if state.record.needs_arm() && state.live.is_some() {
        if let Some(dir) = state.shot_dir.clone() {
            if let Some(scenario) = state.session.core.live_runner().scenario_name() {
                state.record.arm(&dir, scenario);
            }
        }
    }
    if !state.record.is_active() {
        return;
    }
    if state.record.is_due(Instant::now()) {
        state.shot_state.lock().unwrap().arm_record();
    }
    let frames: Vec<RecordFrame> = state
        .shot_state
        .lock()
        .unwrap()
        .take_record()
        .into_iter()
        .map(|capture| RecordFrame {
            width: capture.width,
            height: capture.height,
            rgba: capture.rgba,
        })
        .collect();
    if frames.is_empty() {
        return;
    }
    let step = record_step_info(&state.session);
    state.record.pump_frames(frames, &step);
}

/// The runner position for the next recording sidecar line. Read only for
/// frames actually staged, so the lock never costs a non-recording frame.
fn record_step_info(session: &Session) -> StepInfo {
    session
        .core
        .live_runner()
        .record_step()
        .map(|(step, total, name, tick)| StepInfo {
            step,
            total,
            name: name.to_string(),
            tick,
        })
        .unwrap_or(StepInfo {
            step: 0,
            total: 0,
            name: "no-runner".to_string(),
            tick: 0,
        })
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
    session
        .core
        .with_slot_snapshot(actor, |snapshot| {
            if !snapshot.ingame() || snapshot.scene_state() != 2 {
                return None;
            }
            actor_snapshot_json(actor, snapshot).ok()
        })
        .flatten()
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

/// The ordered, non-drawing front of the per-frame UI body: theme, shots,
/// recording, the session pump, Quester Paths and the live harness tick.
/// Returns `true` when the live harness ends the run. `ui_frame` calls it
/// right after it resets the input staging and before it draws anything; the
/// order of these calls is the frame's contract (the pump publishes the
/// status the live tick reads).
fn frame_pumps(
    state: &mut PanelState,
    terminal_failure: &Mutex<Option<window::PanelError>>,
) -> bool {
    state.session.poll_panel_preferences();
    apply_amber_current(&state.session.ui.chrome);
    let wrote_shots = pump_shots(state);
    pump_record(state);
    state.session.pump_status();
    #[cfg(feature = "memory-profile")]
    if let Some(run) = state.memory.as_mut() {
        state.session.memory_focus(run);
        match run.poll(&state.session.core) {
            Ok(true) => {
                // Quit joins workers before the GPU window and driver close.
                state.session.retire_play();
                eprintln!("PASS: memory panel observation complete");
                return true;
            }
            Ok(false) => {}
            Err(error) => {
                state.session.retire_play();
                eprintln!("FAIL: memory panel: {error}");
                *terminal_failure.lock().unwrap() =
                    Some(window::PanelError::MemoryRunFailure(error.to_string()));
                return true;
            }
        }
    }
    state.session.poll_quester_paths_reload();
    if let Some(live) = state.live.as_mut() {
        let terminal_shot_status = state
            .session
            .core
            .live_runner()
            .terminal_shot()
            .map(|label| state.shot_state.lock().unwrap().status(label))
            .unwrap_or(ShotStatus::Missing);
        let failure = live.tick(
            &mut state.session,
            &terminal_shot_status,
            Some(&state.shot_state),
            wrote_shots,
        );
        if let Some(code) = live_exit_code(live, failure.as_deref()) {
            // PASS and FAIL both finalize the recording now, so the mp4
            // trailer lands before the window exits (window close is
            // covered by the recorder's `Drop`).
            state.record.finish();
            if code == 1 {
                if let Some(msg) = failure.as_deref().or_else(|| live.failure()) {
                    eprintln!("FAIL: {msg}");
                    *terminal_failure.lock().unwrap() =
                        Some(window::PanelError::LiveHarnessFailure(msg.to_owned()));
                }
            }
            return true;
        }
    }
    false
}

/// The per-frame UI body: the pump phase ([`frame_pumps`]), dock host,
/// chrome, game pane, the staged GameInput submit, rail.
fn ui_frame(
    ui: &Ui,
    gpu: &mut Gpu,
    state: &mut PanelState,
    progress: Option<StartupProgressView>,
    terminal_failure: &Mutex<Option<window::PanelError>>,
) -> bool {
    state.session.map_desk.begin_game_input_frame();
    if frame_pumps(state, terminal_failure) {
        return true;
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
    log_window(ui, &mut state.session, state.panel_dock_node);
    ui.set_next_window_class(&panel_class);
    panel_window(ui, &mut state.session, progress);
    crate::fleet::window(ui, &mut state.session);
    {
        let focus = state.session.focus.lock().unwrap();
        if let Some(slot) = focus
            .focused
            .as_deref()
            .and_then(|name| state.session.core.slot_io(name))
        {
            slot.input
                .set_keyboard_enabled(crate::input_capture::game_keyboard_available(ui));
        }
        state
            .keyboard
            .process_ownership(ui, &mut state.session.map_desk, focus.focused.as_deref());
    }
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
    if state.session.map_desk.has_staged_game_input() {
        if let Some(submitted) = state
            .session
            .submit(OperatorCommand::GameInput, Targets::Selected)
        {
            if let Some(error) = state.session.core.command_failure(submitted.request()) {
                state.session.error = Some(error.to_owned());
            }
        }
    }
    if state.session.multibox && !state.session.wall.grid {
        ui.set_next_window_class(&rail_window_class());
        rail_window(ui, gpu, state);
    }
    // Called every frame; each returns early while closed and applies an
    // opener's one-shot focus request just before its own Begin.
    chooser_window(ui, &mut state.session, state.panel_dock_node);
    settings_window(ui, &mut state.session, state.panel_dock_node);
    debug_panel_window(ui, &mut state.session, state.panel_dock_node);
    browse_window(ui, &mut state.session);
    nav_settings_window(ui, &mut state.session, state.panel_dock_node);
    script_prefs_window(ui, &mut state.session, state.panel_dock_node);
    restart_settings_popup(ui, &mut state.session);
    crate::world_picker::popup(ui, &mut state.session);
    crate::loadouts::window(ui, &mut state.session);
    render_all_warn_window(ui, &mut state.session);
    background_ack_window(ui, &mut state.session);
    discard_unconsumed_native_capture();
    false
}

#[cfg(test)]
mod parameter_options_tests {
    use super::*;
    fn compiled_schema(
        session: &crate::session::Session,
        name: &str,
    ) -> frontend_core::scripts::SchemaRef<'static> {
        let id = frontend_core::scripts::CompiledCardId::lookup(name)
            .unwrap_or_else(|| panic!("compiled card {name} is registered"));
        match session.scripts.compiled_schema(&session.core, None, id) {
            frontend_core::scripts::SchemaView::Ready { fields, .. } => fields,
            frontend_core::scripts::SchemaView::Unavailable(reason) => {
                panic!("compiled card {name} schema unavailable: {reason}")
            }
        }
    }

    #[test]
    fn panel_site_filter_uses_selectable_shared_search_results() {
        let session = crate::session::Session::new();
        let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        let loadouts = frontend_core::scripts::Loadouts::default();
        let schema = compiled_schema(&session, "Gatherer");
        let site = schema
            .get("site")
            .expect("Gatherer schema includes the Site setting");
        let mut bag = serde_json::Map::new();
        bag.insert("skill".into(), serde_json::json!("Woodcutting"));
        bag.insert("woodcuttingResources".into(), serde_json::json!(["normal"]));
        bag.insert("site".into(), serde_json::json!("woodcutting.draynor"));
        let options = frontend_core::scripts::resolve_parameter_options(
            site,
            &bag,
            &loadouts,
            Some(data.as_ref()),
        );

        let matches = filtered_parameter_indices(&options, "varrock").collect::<Vec<_>>();
        assert!(
            !matches.is_empty() && matches.len() < options.selectable().len(),
            "the filter narrows the site list to the rows that match: {} of {}",
            matches.len(),
            options.selectable().len()
        );
        assert!(matches
            .iter()
            .all(|index| options.matches_query(*index, "varrock")));
        assert_eq!(
            bag.get("site"),
            Some(&serde_json::json!("woodcutting.draynor")),
            "resolving and filtering do not alter the saved site"
        );
    }

    #[test]
    fn disabled_panel_site_keeps_the_saved_reason_read_only() {
        let session = crate::session::Session::new();
        let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        let loadouts = frontend_core::scripts::Loadouts::default();
        let schema = compiled_schema(&session, "Gatherer");
        let site = schema
            .get("site")
            .expect("Gatherer schema includes the Site setting");
        let mut bag = serde_json::Map::new();
        bag.insert("skill".into(), serde_json::json!("Mining"));
        bag.insert("miningResources".into(), serde_json::json!(["rune stones"]));
        bag.insert("site".into(), serde_json::json!("mining.varrock_east.se"));
        let options = frontend_core::scripts::resolve_parameter_options(
            site,
            &bag,
            &loadouts,
            Some(data.as_ref()),
        );

        assert!(options.is_empty());
        let disabled = unavailable_parameter_text("Site", site, &bag, &options);
        assert!(disabled.contains("options unavailable"));
        assert!(disabled.contains("not for the selected resources"));
        assert_eq!(
            bag.get("site"),
            Some(&serde_json::json!("mining.varrock_east.se")),
            "disabled rendering does not rewrite the saved site"
        );
    }

    #[test]
    fn preserved_only_priority_array_is_editable_for_removal() {
        let session = crate::session::Session::new();
        let loadouts = frontend_core::scripts::Loadouts::default();
        let schema = compiled_schema(&session, "Quester");
        let priority = schema
            .get("order_override")
            .expect("Quester has an order override array");
        let mut bag = serde_json::Map::new();
        bag.insert("quests".into(), serde_json::json!(["hauntedmine"]));
        bag.insert("order_override".into(), serde_json::json!(["hauntedmine"]));
        let options =
            frontend_core::scripts::resolve_parameter_options(priority, &bag, &loadouts, None);

        assert!(
            options.is_empty(),
            "the saved priority has no selectable options"
        );
        assert!(parameter_array_editor_enabled(priority, &bag, &options));
        let haunted = options
            .values()
            .iter()
            .position(|value| value == "hauntedmine")
            .expect("the saved unavailable priority remains displayed");
        let mut selected = vec!["hauntedmine".to_owned()];
        assert!(options.toggle_array_choice(&mut selected, haunted));
        assert!(selected.is_empty());
        assert!(
            !options.toggle_array_choice(&mut selected, haunted),
            "the same preserved row cannot be selected again"
        );

        let persisted = priority.coerce(&serde_json::json!(selected));
        bag.insert(priority.id().to_owned(), persisted.clone());
        assert_eq!(persisted, serde_json::json!([]));
        let options =
            frontend_core::scripts::resolve_parameter_options(priority, &bag, &loadouts, None);
        assert!(
            !parameter_array_editor_enabled(priority, &bag, &options),
            "after removal there is no refused stored row to edit"
        );
    }
}

#[cfg(test)]
#[path = "app_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "app_window_tests.rs"]
mod window_tests;

#[cfg(test)]
#[path = "app_teardown_tests.rs"]
mod teardown_tests;

#[cfg(test)]
#[path = "app_frame_tests.rs"]
mod frame_tests;
