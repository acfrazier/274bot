//! Panel shell: docking enabled, multi-viewport disabled, amber chrome,
//! running on the panel-owned window loop in `crate::window`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use crate::window::{self, Gpu, RedrawMode, Theme};
use dear_imgui_rs::internal::RawWrapper;
use dear_imgui_rs::{
    ColorDisplayMode, ComboBoxOptions, ComboBoxPreviewMode, Condition, DockBuilder, DockNodeFlags,
    DragDropTargetFlags, Id, Key, MouseButton, SplitDirection, StyleColor, TreeNodeFlags, Ui,
    WindowClass, WindowFlags,
};

use crate::chrome::{
    button_cells, button_cells_min, button_row_layout, equal_button_width, move_heading,
    multibox_tooltip, resolve_heading_order, BUTTON_GAP, CONFIG_MIN, CONFIG_ROW, MIN_BUTTON,
    SCRIPT_ROW,
};
use crate::focus::{draw_for_slot, should_capture, should_draw};
use crate::game_view::GameView;
use crate::grid::grid_cells;
use crate::overlay::{draw_focused_queue_card, PathOverlay};
use crate::paint::PaintOverlay;
use crate::picker;
use crate::queue_card::queue_k_of_n;
use crate::script_picker::{
    self, kind_badge, move_category, resolve_category_order, source_badge, BROWSE_WINDOW_TITLE,
};
use crate::rail::{
    cap_title, os_window_size, rail_preview_open, rail_split_ratio, traffic_light, Light,
    BASE_WINDOW_H, BASE_WINDOW_W, FOLD_GLYPH, RAIL_W, REMOVE_GLYPH, STATUS_GLYPH, TILE_H, TILE_W,
    UNFOLD_GLYPH,
};
use host::debug_enabled;

use crate::resource::{
    cpu_from_delta, format_bots, format_rss_caption, sample_process, traffic_from_samples, Metric,
};
use crate::session::{
    debug_dest_cheats, debug_main_buttons, debug_maxme_cheats, script_active, script_pause_enabled,
    script_status_text, script_stop_enabled, stream_capture, Session, PROCESS,
};
use crate::theme::{
    applet_offset, apply_amber, fit_applet, game_window_title, integer_ui_scale, native_applet,
    panel_split_ratio, ACCENT, ACCENT_HOVER, BG, ERROR, PANEL_WIDTH, PANEL_WINDOW, RAIL_WINDOW,
    TEXT, TEXT_DIM, TITLE,
};

/// Runner configuration: docking on, viewports off, amber CRT, 50 fps cap.
/// `auto_dockspace` is off so we own the split (game left, 330px panel right).
/// Default `RedrawMode::Poll` spins the UI thread and starves the
/// 20 ms slot; WaitUntil matches the client tick.
pub fn runner_config() -> window::PanelConfig {
    // Viewports stay off: the panel renders into the single main viewport only.
    window::PanelConfig {
        window_title: "274bot".into(),
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

/// Scale all ImGui style sizes for a window DPI. Held for Task 7: the loop
/// runs under `HiDpiMode::Default`, which already sets
/// `display_framebuffer_scale`, so applying this on top of that would double
/// every size on Retina.
pub fn apply_ui_scale(style: &mut dear_imgui_rs::Style, dpi: f32) {
    let s = integer_ui_scale(dpi);
    unsafe {
        dear_imgui_rs::sys::ImGuiStyle_ScaleAllSizes(style.raw_mut(), s);
    }
}

/// Push the amber CRT palette over Theme::Dark (kills default imgui blue).
fn amber_style(ctx: &mut dear_imgui_rs::Context) {
    apply_amber(ctx.style_mut());
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
    game_view: Option<GameView>,
    session: Session,
    dock_inited: bool,
    /// Which dock layout the tree was last built with; `None` before the
    /// first init. A MultiBox toggle rebuilds the tree when this differs.
    dock_layout: Option<DockLayout>,
    game_dock_node: Option<Id>,
    docked_game_title: String,
    last_upload: Option<(String, u64)>,
    /// Cached queue-card overlay for the focused slot (see `overlay`).
    overlay: PathOverlay,
    /// Cached script-paint overlay over the Game chatbox (see `paint`).
    paint: PaintOverlay,
    /// One cached tile texture per wall member (blitted at TILE_W×TILE_H).
    views: HashMap<String, TileView>,
    /// Last 1 Hz process sample `(instant, cpu secs)`; `None` before the
    /// first sample or after a sampler failure (CPU then re-measures).
    last_proc: Option<(Instant, f64)>,
    /// Cached 1 Hz CPU metric for the resource card.
    res_cpu: Metric,
    /// Cached 1 Hz RAM metric for the resource card.
    res_ram: Metric,
    /// Cached 1 Hz traffic metric for the resource card.
    res_traffic: Metric,
    /// Last traffic sample `(instant, sum, n_slots)`; `None` until the
    /// first 1 Hz pass (first rate needs two samples).
    last_traffic: Option<(Instant, u64, usize)>,
    /// Headed `--live` watch (`null_raster`, `stress50`, `stress50_full`,
    /// `script_<name>`) or `--smoke`; `None` interactive.
    live: Option<LiveHarness>,
    /// Last dock-host viewport size; a width/height change rebuilds the
    /// split so the panel stays 330 and the rail 264.
    dock_size: Option<[f32; 2]>,
    /// winit window so MultiBox can grow the OS inner size when the rail
    /// would cover the 765×503 blit.
    os_window: Option<std::sync::Arc<winit::window::Window>>,
    /// Whole-window shot coordination: the scenario sink (slot thread)
    /// → the render readback (`window::ShotState`) → the shot files.
    shot_state: Arc<Mutex<crate::window::ShotState>>,
    /// Per-run shot dir (`~/.274bot/smoke/<runId>`), created when a
    /// `--live script_*` run starts or lazily on the first write (the
    /// interactive F12 capture has no run start to hook).
    shot_dir: Option<PathBuf>,
}

/// Headed live harness: null_raster (2 slots), stress50 / stress50_full
/// (50 slots), a shared scenario (`script_<name>`), or `--smoke` (one
/// whole-window shot at scene 2, then exit 0).
enum LiveHarness {
    Null(LiveNull),
    Stress(LiveStress),
    Script(LiveScript),
    Smoke(LiveSmoke),
}

/// Headed `null_raster` harness state. `started` is the 120s login clock.
struct LiveNull {
    started: Instant,
    saw_scene2: bool,
    passed: bool,
}

/// Headed `stress50` / `stress50_full` harness. `started` is the 600s
/// login clock. `name` is the `--live` token used in PASS/FAIL lines.
struct LiveStress {
    started: Instant,
    last_announced: u8,
    passed: bool,
    name: &'static str,
    host: String,
    port: u16,
}

/// Headed `script_<name>` watch. The shared `ScenarioRunner` lives on the
/// `Session` (the slot thread ticks it); this struct only mirrors the
/// last-reported step for progress lines and latches the terminal state.
/// PASS → the caller exits 0; FAIL → exit 1. A terminal shot holds either
/// exit until the capture writes (or [`NAV_FULL_SHOT_DRAIN`] lapses).
struct LiveScript {
    name: String,
    passed: bool,
    failed: Option<String>,
    last_step: Option<(usize, usize)>,
    /// Terminal-shot drain: the instant the runner first reported a
    /// terminal status while a shot was armed. The watch keeps pumping
    /// (returning `None`) until the shot writes or [`NAV_FULL_SHOT_DRAIN`]
    /// lapses, so the screenshot lands before the process exits.
    drain_started: Option<Instant>,
}

/// Headed `--smoke` watch. The `render_smoke` scenario's shot sink fires
/// the tick the focused slot reaches scene 2, but the capture is held
/// [`SMOKE_SETTLE`] so the slot's 1 fps renderer can rasterize the world
/// first; this watch latches `passed` once `pump_shots` has written the
/// PNG (the caller exits 0), mirrors the shared runner's failures, and
/// FAILs when scene 2 never arrives within [`SMOKE_DEADLINE`].
struct LiveSmoke {
    started: Instant,
    last_step: Option<(usize, usize)>,
    failed: Option<String>,
    /// The instant the pure trigger first saw the focused slot at scene 2
    /// (`None` before). The settle gate holds the shot until
    /// [`SMOKE_SETTLE`] has elapsed; the deadline message distinguishes a
    /// stuck login from a missing write.
    saw_scene2_at: Option<Instant>,
    /// The scene2 PNG landed (`pump_shots` wrote it): the caller exits 0.
    passed: bool,
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
    /// `BOT_VAULT_PASS` interactive/headed flow. Failure is non-fatal: the
    /// in-panel prompt covers typing.
    Unlock { pass: String },
    /// Live harness spawns. Failure is fatal (`FAIL:` + exit).
    Live(LiveBoot),
}

/// Which live harness the boot starts.
#[derive(Debug)]
enum LiveBoot {
    NullRaster,
    Stress50,
    /// Same 50-head wall as [`LiveBoot::Stress50`], every member painting
    /// at 50 fps (Game + sidecar).
    Stress50Full,
    Script {
        name: String,
    },
    /// `--smoke`: the `render_smoke` scenario with the shot sink armed.
    /// The scenario's own settings carry the 300 s deadline and the off
    /// mainland-base gate; `live_smoke_tick` keeps its outer ceiling.
    Smoke,
}

impl LiveBoot {
    /// Prepare the harness session and install the harness state. `Smoke`
    /// and `Script` arm the whole-window shot sink too.
    fn run(self, state: &mut PanelState) -> Result<(), String> {
        match self {
            LiveBoot::NullRaster => {
                state.session.live_prepare_null_raster()?;
                state.live = Some(LiveHarness::Null(LiveNull {
                    started: Instant::now(),
                    saw_scene2: false,
                    passed: false,
                }));
            }
            LiveBoot::Stress50 => {
                state.session.live_prepare_stress50()?;
                state.live = Some(LiveHarness::Stress(LiveStress {
                    started: Instant::now(),
                    last_announced: 0,
                    passed: false,
                    name: "stress50",
                    host: state.session.play_options().host.clone(),
                    port: state.session.play_options().port,
                }));
            }
            LiveBoot::Stress50Full => {
                state.session.live_prepare_stress50_full()?;
                state.live = Some(LiveHarness::Stress(LiveStress {
                    started: Instant::now(),
                    last_announced: 0,
                    passed: false,
                    name: "stress50_full",
                    host: state.session.play_options().host.clone(),
                    port: state.session.play_options().port,
                }));
            }
            LiveBoot::Script { name } => {
                let scenario_name = name.strip_prefix("script_").unwrap_or(&name);
                let scenario = scenario::get(scenario_name)
                    .ok_or_else(|| format!("unknown scenario {scenario_name}"))?;
                state.session.live_prepare_script(scenario)?;
                arm_scenario_shots(state);
                state.live = Some(LiveHarness::Script(LiveScript {
                    name,
                    passed: false,
                    failed: None,
                    last_step: None,
                    drain_started: None,
                }));
            }
            LiveBoot::Smoke => {
                let scenario = scenario::get("render_smoke")
                    .ok_or_else(|| "unknown scenario render_smoke".to_string())?;
                state.session.live_prepare_script(scenario)?;
                arm_scenario_shots(state);
                state.live = Some(LiveHarness::Smoke(LiveSmoke {
                    started: Instant::now(),
                    last_step: None,
                    failed: None,
                    saw_scene2_at: None,
                    passed: false,
                }));
            }
        }
        Ok(())
    }
}

/// The deferred boot for a [`run_panel`] call, derived from the run mode
/// and `BOT_VAULT_PASS` — pure, so the mapping is testable (the boot
/// itself runs after GPU init; nothing here spawns or unlocks).
fn boot_for(mode: &RunMode, vault_pass: Option<&str>) -> Option<Boot> {
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
        _ => vault_pass.map(|pass| Boot::Unlock {
            pass: pass.to_string(),
        }),
    }
}

/// Execute a deferred [`Boot`] (slot spawns) after GPU init. Returns the
/// fatal `FAIL:` message for live-harness failures; the vault unlock
/// failure is non-fatal (the in-panel prompt covers typing).
fn boot_execute(state: &mut PanelState, boot: Boot) -> Result<(), String> {
    match boot {
        Boot::Unlock { pass } => {
            if !state.session.unlock(&pass) {
                eprintln!(
                    "panel: vault: {}",
                    state.session.error.clone().unwrap_or_default()
                );
            }
            Ok(())
        }
        Boot::Live(live) => live.run(state),
    }
}

/// One rail/grid tile's GPU texture (uploaded when the slot's `FrameBuf`
/// hands a new frame to `take`).
struct TileView {
    view: GameView,
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
    /// 1 Hz process + stream-byte sample for the resource card. CPU needs
    /// a wall+CPU delta, so the first sample is [`Metric::Measuring`]; RAM
    /// is available from the start. Traffic needs two samples of summed
    /// `bytes_in+bytes_out`; zero slots stay Measuring (never fake 0 B/s).
    /// A process-sampler failure flips CPU/RAM to [`Metric::Error`] and
    /// re-baselines them, but traffic still samples from statuses.
    fn sample_resources(&mut self) {
        let now = Instant::now();
        // Prefer last_proc for the 1 Hz gate; if the process sampler failed
        // and cleared it, fall back to last_traffic so we keep sampling
        // stream bytes without spinning every frame.
        let due = match &self.last_proc {
            Some((t, _)) => now.duration_since(*t).as_secs_f64() >= 1.0,
            None => match &self.last_traffic {
                Some((t, ..)) => now.duration_since(*t).as_secs_f64() >= 1.0,
                None => true,
            },
        };
        if !due {
            return;
        }

        let statuses = self.session.statuses();
        let n = statuses.len();
        let sum: u64 = statuses
            .iter()
            .map(|s| s.bytes_in.wrapping_add(s.bytes_out))
            .sum();
        match self.last_traffic {
            Some((t0, sum0, n_prev)) => {
                let dt = now.duration_since(t0).as_secs_f64();
                self.res_traffic = traffic_from_samples(sum, sum0, dt, n, n_prev);
            }
            None => self.res_traffic = Metric::Measuring,
        }
        self.last_traffic = Some((now, sum, n));

        let (rss, cpu) = sample_process();
        if debug_enabled() {
            eprintln!("[panel] rss={} traffic_sum={}", rss, sum);
        }
        if rss == 0 && cpu == 0.0 {
            self.res_cpu = Metric::Error("process sample failed".into());
            self.res_ram = Metric::Error("process sample failed".into());
            self.last_proc = None;
            return;
        }
        match self.last_proc {
            Some((t0, cpu0)) => {
                let wall = now.duration_since(t0).as_secs_f64();
                let ncpu = std::thread::available_parallelism()
                    .map(|n| n.get() as u32)
                    .unwrap_or(1)
                    .max(1);
                self.res_cpu = cpu_from_delta(cpu - cpu0, wall, ncpu);
            }
            None => self.res_cpu = Metric::Measuring,
        }
        self.res_ram = Metric::Available(format_rss_caption(rss));
        self.last_proc = Some((now, cpu));
    }
}

impl Default for PanelState {
    fn default() -> Self {
        Self {
            game_view: None,
            session: Session::new(),
            dock_inited: false,
            dock_layout: None,
            game_dock_node: None,
            docked_game_title: String::new(),
            last_upload: None,
            overlay: PathOverlay::new(),
            paint: PaintOverlay::new(),
            views: HashMap::new(),
            last_proc: None,
            res_cpu: Metric::Measuring,
            res_ram: Metric::Measuring,
            res_traffic: Metric::Measuring,
            last_traffic: None,
            live: None,
            dock_size: None,
            os_window: None,
            shot_state: Arc::new(Mutex::new(crate::window::ShotState::default())),
            shot_dir: None,
        }
    }
}

const LIVE_USAGE: &str =
    "usage: panel-play [--smoke] [--live null_raster|stress50|stress50_full|nav_full|script_<name>]";

/// What `panel-play` should do this run: the normal interactive panel, a
/// `--live NAME` harness, or `--smoke` (one whole-window shot at scene 2,
/// then exit 0).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunMode {
    Interactive,
    Live(String),
    Smoke,
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
}

/// `--smoke` login+world deadline: the focused slot must reach scene 2
/// (and the shot write must land) within this window or the run FAILs
/// with exit 1. Generous — scene 2 usually lands inside the first two
/// minutes.
const SMOKE_DEADLINE: Duration = Duration::from_secs(300);

/// `nav_full` FAIL shot drain: after a `Failed` status the panel holds
/// the exit up to this long so the terminal whole-window capture — asked
/// for by the slot thread, written by the render readback — lands before
/// exit 1.
const NAV_FULL_SHOT_DRAIN: Duration = Duration::from_secs(10);

/// `--smoke` render-settle window: after the focused slot reaches scene 2,
/// the whole-window shot request waits this long before reaching the render
/// readback — the slot's renderer rasterizes at 1 fps on the CPU backend
/// (slower in a debug build), so an immediate capture would still show the
/// title/loading screen.
const SMOKE_SETTLE: Duration = Duration::from_secs(3);

/// `--smoke` trigger: fire exactly once, on the frame the focused slot
/// first reaches `ingame && scene_state == 2`. Pure so the trigger path
/// is a table test; the smoke watch calls it against the focused slot's
/// status every frame.
fn smoke_should_fire(
    smoke_armed: bool,
    already_fired: bool,
    ingame: bool,
    scene_state: i32,
) -> bool {
    smoke_armed && !already_fired && ingame && scene_state == 2
}

/// `--smoke` render-settle predicate: the scene2 shot request is released
/// to the render readback only once the focused slot has held scene 2 for
/// [`SMOKE_SETTLE`] (wall-clock, so the slot's 1 fps renderer has rasterized
/// the world) and is still ingame. `saw_scene2_at` is `None` until the
/// trigger latches scene 2.
fn smoke_settled(saw_scene2_at: Option<Instant>, now: Instant, ingame: bool) -> bool {
    match saw_scene2_at {
        Some(t) => ingame && now.saturating_duration_since(t) >= SMOKE_SETTLE,
        None => false,
    }
}

/// The interactive F12 capture label: `manual-<stamp>` (the 377 stamp so
/// shot files sort chronologically). Already in the 377 safe alphabet, so
/// `safe_label` is a no-op on it.
fn manual_shot_label(now: SystemTime) -> String {
    format!("manual-{}", scenario::shot::stamp_utc(now))
}

/// F12 whole-window shot: enqueue `(manual-<stamp>, default snapshot)` on
/// the same `shot_state.requests` path the scenario sink uses, so the
/// render readback and `pump_shots` handle it with no scenario involved.
/// The per-run shot dir is created lazily by `pump_shots` on the first
/// write.
fn enqueue_manual_shot(state: &mut PanelState) {
    let label = manual_shot_label(SystemTime::now());
    let json =
        serde_json::to_string_pretty(&api::snapshot::GameSnapshot::new()).unwrap_or_default();
    state
        .shot_state
        .lock()
        .unwrap()
        .requests
        .push((label.clone(), json));
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
        let script_ok = name
            .strip_prefix("script_")
            .is_some_and(|n| scenario::get(n).is_some());
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

/// Headed watch: wait until both slots are scene 2, print RSS/counters, PASS.
/// Does **not** freeze-assert (operator may click the rail). Null freeze is
/// the headless `e2e` twin.
fn live_null_tick(live: &mut LiveNull, statuses: &[host_play::SlotStatus]) -> Option<String> {
    if live.passed {
        return None;
    }
    let ready = statuses
        .iter()
        .filter(|s| s.ingame && s.scene_state == 2)
        .count();
    if ready < 2 {
        if live.started.elapsed() >= Duration::from_secs(120) {
            return Some(format!(
                "live null_raster: {ready}/2 slot(s) ingame scene 2 after 120s"
            ));
        }
        return None;
    }
    let (rss, _) = sample_process();
    let Some(test2) = statuses.iter().find(|s| s.username == "test2") else {
        return Some("live null_raster: missing test2".into());
    };
    let Some(test) = statuses.iter().find(|s| s.username == "test") else {
        return Some("live null_raster: missing test".into());
    };
    println!("live null_raster: rss={rss}");
    println!(
        "live null_raster test2 bytes={}/{}",
        test2.bytes_in, test2.bytes_out
    );
    println!(
        "live null_raster test  bytes={}/{}",
        test.bytes_in, test.bytes_out
    );
    println!("PASS: live null_raster");
    live.saw_scene2 = true;
    live.passed = true;
    None
}

/// Headed watch: count Clients that are up — every slot is a full Client,
/// so "up" is `ingame && scene_state==2` for all of them. Announce 1, 10,
/// then 50. At 50 print PASS (with RSS) and stay open. Timeout 600s. Does
/// **not** freeze-assert (operator may click). Does **not** fail on RSS
/// magnitude — `stress50` is the release RAM check; `stress50_full` is
/// the same wall with every renderer at 50 fps.
fn live_stress_tick(live: &mut LiveStress, statuses: &[host_play::SlotStatus]) -> Option<String> {
    if live.passed {
        return None;
    }
    let name = live.name;
    let n = statuses.iter().filter(|s| s.is_up()).count();
    if n >= 1 && live.last_announced < 1 {
        println!("live {name}: 1/50 up");
        live.last_announced = 1;
    }
    if n >= 10 && live.last_announced < 10 {
        println!("live {name}: 10/50 up");
        live.last_announced = 10;
    }
    if n >= 50 {
        let (rss, _) = sample_process();
        let workers = client::io::OnDemand::live_workers_for(&live.host, live.port);
        let tcp = host_play::count_tcp_to(&live.host, live.port);
        let tcp_s = tcp.map(|c| c.to_string()).unwrap_or_else(|| "?".into());
        println!("PASS: live {name} rss={rss} up={n}/50 ondemand={workers} tcp={tcp_s}");
        live.last_announced = 50;
        live.passed = true;
        return None;
    }
    if live.started.elapsed() >= Duration::from_secs(600) {
        return Some(format!("live {name}: {n}/50 up after 600s"));
    }
    None
}

/// Hold a terminal-shot exit until `pump_shots` writes (or the drain
/// lapses). Returns true while the caller must keep pumping.
fn hold_terminal_shot(live: &mut LiveScript, wants: bool, wrote_shots: usize) -> bool {
    if !wants {
        return false;
    }
    let held = match live.drain_started {
        Some(t0) => t0.elapsed() >= NAV_FULL_SHOT_DRAIN,
        // The frame the terminal status is first seen: the request is
        // already in `shot_state`, but `pump_shots` ran before this tick,
        // so the write needs at least one more frame.
        None => false,
    };
    if !held && wrote_shots == 0 {
        if live.drain_started.is_none() {
            live.drain_started = Some(Instant::now());
        }
        return true;
    }
    false
}

/// Headed script watch: mirror the shared `ScenarioRunner` each frame.
/// PASS prints the JSON evidence record and latches `passed` (the caller
/// exits 0); FAIL prints the record and returns the message (exit 1).
/// When the runner armed a terminal shot, either outcome holds until the
/// capture writes (or [`NAV_FULL_SHOT_DRAIN`] lapses).
fn live_script_tick(
    live: &mut LiveScript,
    session: &mut Session,
    wrote_shots: usize,
) -> Option<String> {
    if live.passed || live.failed.is_some() {
        return None;
    }
    let (status, evidence, wants_terminal_shot) = {
        let guard = session.scenario.lock().unwrap();
        (
            guard.as_ref().map(|r| r.status()),
            guard.as_ref().and_then(|r| r.evidence().cloned()),
            guard.as_ref().is_some_and(|r| r.terminal_shot().is_some()),
        )
    };
    let record = |evidence: &Option<scenario::Evidence>| {
        evidence.as_ref().map(|ev| ev.to_json()).unwrap_or_default()
    };
    match status {
        Some(scenario::RunnerStatus::Passed) => {
            if hold_terminal_shot(live, wants_terminal_shot, wrote_shots) {
                return None;
            }
            println!("PASS: live {} {}", live.name, record(&evidence));
            live.passed = true;
            None
        }
        Some(scenario::RunnerStatus::Failed(msg)) => {
            if hold_terminal_shot(live, wants_terminal_shot, wrote_shots) {
                return None;
            }
            eprintln!("FAIL: live {} {}", live.name, record(&evidence));
            live.failed = Some(msg.clone());
            Some(msg)
        }
        Some(scenario::RunnerStatus::Seeding) => None,
        Some(scenario::RunnerStatus::Running { step, total }) => {
            if live.last_step != Some((step, total)) {
                live.last_step = Some((step, total));
                if step >= total {
                    println!("live {}: proving proof predicate", live.name);
                } else {
                    println!("live {}: running step {}/{}", live.name, step + 1, total);
                }
            }
            None
        }
        None => None,
    }
}

/// The focused slot's status row, `None` when nothing is focused or the
/// focused username has no row this pump.
fn focused_slot<'a>(
    session: &Session,
    statuses: &'a [host_play::SlotStatus],
) -> Option<&'a host_play::SlotStatus> {
    let focused = session.focused_name();
    statuses
        .iter()
        .find(|s| focused.as_deref() == Some(s.username.as_str()))
}

/// Headed `--smoke` watch. `wrote_shots` is the count `pump_shots` wrote
/// this frame: the smoke's single scene2 shot landing passes the run (the
/// caller exits 0). Before that, the pure trigger latches scene 2 on the
/// focused slot for the render-settle gate and a precise deadline message,
/// and the shared runner's failures surface unchanged. The 300 s ceiling
/// is the run's own clock: it also catches a runner that passed but whose
/// PNG never got written (e.g. a failed render readback).
fn live_smoke_tick(
    live: &mut LiveSmoke,
    session: &mut Session,
    statuses: &[host_play::SlotStatus],
    wrote_shots: usize,
) -> Option<String> {
    if live.passed || live.failed.is_some() {
        return None;
    }
    if wrote_shots > 0 {
        println!("PASS: panel-play --smoke (scene2 whole-window shot written)");
        live.passed = true;
        return None;
    }
    let slot = focused_slot(session, statuses);
    if smoke_should_fire(
        true,
        live.saw_scene2_at.is_some(),
        slot.is_some_and(|s| s.ingame),
        slot.map_or(0, |s| s.scene_state),
    ) {
        live.saw_scene2_at = Some(Instant::now());
        println!(
            "smoke: focused slot at scene 2; waiting for the render to settle before the capture"
        );
    }
    // Mirror the shared runner so a scenario failure is the failure.
    let status = session
        .scenario
        .lock()
        .unwrap()
        .as_ref()
        .map(|r| r.status());
    match status {
        Some(scenario::RunnerStatus::Failed(msg)) => {
            live.failed = Some(msg.clone());
            return Some(msg);
        }
        Some(scenario::RunnerStatus::Running { step, total })
            if live.last_step != Some((step, total)) =>
        {
            live.last_step = Some((step, total));
            println!("smoke: running step {step}/{total}");
        }
        _ => {}
    }
    if live.started.elapsed() >= SMOKE_DEADLINE {
        let msg = if live.saw_scene2_at.is_some() {
            "panel-play --smoke: scene2 shot never written within 300s".to_string()
        } else {
            "panel-play --smoke: focused slot never reached scene 2 within 300s".to_string()
        };
        live.failed = Some(msg.clone());
        return Some(msg);
    }
    None
}

/// Dock leaves: no splitter, no undock, no extra splits. Tab bar hides
/// when a node has one window.
fn dock_flags() -> DockNodeFlags {
    DockNodeFlags::AUTO_HIDE_TAB_BAR
        | DockNodeFlags::NO_RESIZE
        | DockNodeFlags::NO_UNDOCKING
        | DockNodeFlags::NO_DOCKING_SPLIT
}

/// Leaf dock nodes hide the tab bar while they host a single window.
fn single_bot_window_class() -> WindowClass {
    WindowClass::new(Id::from(1u32))
        .dock_node_flags_override_set(dock_flags())
        .docking_always_tab_bar(false)
}

fn size_changed(prev: Option<[f32; 2]>, size: [f32; 2]) -> bool {
    match prev {
        None => true,
        Some(p) => (p[0] - size[0]).abs() > 1.0 || (p[1] - size[1]).abs() > 1.0,
    }
}

/// Grow the OS window when the rail (or a too-narrow host) would cover
/// the native blit. Never shrinks a larger window.
fn ensure_window_fits(state: &mut PanelState, rail_open: bool) {
    let Some(window) = state.os_window.as_ref() else {
        return;
    };
    let (need_w, need_h) = os_window_size(rail_open);
    let scale = window.scale_factor();
    let logical: winit::dpi::LogicalSize<f64> = window.inner_size().to_logical(scale);
    let w = logical.width.max(need_w as f64);
    let h = logical.height.max(need_h as f64);
    if (w - logical.width).abs() > 1.0 || (h - logical.height).abs() > 1.0 {
        let _ = window.request_inner_size(winit::dpi::LogicalSize::new(w, h));
    }
}

/// Fullscreen dock host: game left, 330px panel right, optional 264px rail.
/// Rebuilds when MultiBox/grid toggles or the OS window size changes so
/// panel/rail widths stay fixed; they only grow vertically.
fn dock_host(ui: &Ui, state: &mut PanelState, game_title: &str) {
    let rail_open = state.session.multibox && !state.session.wall.grid;
    ensure_window_fits(state, rail_open);
    let viewport = ui.main_viewport();
    let pos = viewport.pos();
    let vs = viewport.size();
    let (need_w, _) = os_window_size(rail_open);
    // Until winit applies a grow, lay out against the need so the blit
    // is not parked under the panel/rail for a frame.
    let size = [vs[0].max(need_w), vs[1].max(1.0)];
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
                        let ratio = panel_split_ratio(size[0]);
                        let (right, left) =
                            DockBuilder::split_node(ui, dock_id, SplitDirection::Right, ratio);
                        DockBuilder::dock_window(ui, PANEL_WINDOW, right);
                        DockBuilder::dock_window(ui, game_title, left);
                        state.game_dock_node = Some(left);
                    }
                    DockLayout::Rail => {
                        let rail_ratio = rail_split_ratio(size[0]);
                        let (rail, main) =
                            DockBuilder::split_node(ui, dock_id, SplitDirection::Right, rail_ratio);
                        let panel_ratio = panel_split_ratio((size[0] - RAIL_W).max(1.0));
                        let (panel, game) =
                            DockBuilder::split_node(ui, main, SplitDirection::Right, panel_ratio);
                        DockBuilder::dock_window(ui, RAIL_WINDOW, rail);
                        DockBuilder::dock_window(ui, PANEL_WINDOW, panel);
                        DockBuilder::dock_window(ui, game_title, game);
                        state.game_dock_node = Some(game);
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
            picker::draw_picker(ui, &mut state.session);
        } else {
            picker::note_closed();
            if state.session.multibox && state.session.wall.grid {
                grid_pane(ui, gpu, state, avail);
            } else {
                game_pane(ui, gpu, state, avail);
            }
        }
    });
    state.session.set_game_pane_open(built.is_some());
}

/// Single-bot / rail Game pane: native 765×503, flush to the panel (right).
/// Does not scale with the host window; grid mode fits cells to avail.
fn game_pane(ui: &Ui, gpu: &mut Gpu, state: &mut PanelState, avail: [f32; 2]) {
    let size = native_applet();
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
            if let Some(frame) = buf.as_ref().and_then(|p| p.take()) {
                if let Some(view) = state.game_view.as_mut() {
                    view.present(gpu, frame);
                }
            }
            state.last_upload = Some((name, gen));
        }
        let view = state.game_view.as_ref().expect("game view initialized");
        ui.image(view.tex_id, size);
        let min = ui.item_rect_min();
        // Queue-card overlay: the armed route's remaining tiles are
        // painted by the client's 3D renderer and on the pack map, so the
        // Image only carries the focused slot's queue card.
        state.overlay.frame(ui, &state.session, min, size);
        // Script-paint overlay: the focused slot's paint renders in an
        // ImGui window over the chatbox rect — never on the game texture.
        let statuses = state.session.statuses();
        if let Some(slot) = focused_slot(&state.session, &statuses) {
            state.paint.frame(ui, slot.script_paint.as_ref(), min, size);
        }
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
    }
}

/// MultiBox grid-mode Game pane: same cap as the sidecar (dot, name +
/// brief, fold, ✗) over a bigger blit. Status stays visible with the
/// 330px panel collapsed. `only_render_selected` / fold hide the blit
/// only. Capture reaches the focused cell's body.
fn grid_pane(ui: &Ui, gpu: &mut Gpu, state: &mut PanelState, avail: [f32; 2]) {
    let members = state.session.wall.members.clone();
    if members.is_empty() {
        ui.text_disabled("no wall members");
        return;
    }
    let cells = grid_cells(members.len(), avail);
    let (focused, capture) = {
        let focus = state.session.focus.lock().unwrap();
        (focus.focused.clone(), should_capture(&focus))
    };
    let only_selected = state.session.focus.lock().unwrap().only_render_selected;
    {
        let focus = state.session.focus.lock().unwrap();
        dispose_idle_views(&mut state.views, gpu, |name| {
            members.iter().any(|m| m == name) && draw_for_slot(&focus, name)
        });
    }
    let statuses = state.session.statuses();
    for (i, name) in members.iter().enumerate() {
        let [cx, cy, cw, ch] = cells[i];
        let is_focused = focused.as_deref() == Some(name.as_str());
        ui.set_cursor_pos([cx, cy]);
        let status = statuses.iter().find(|s| &s.username == name);
        let running = status.is_some_and(|s| s.walk_x != -1)
            || state
                .session
                .play
                .as_ref()
                .is_some_and(|p| p.script_state(name) == script::RunState::Running);
        let light = traffic_light(
            status.is_some_and(|s| s.ingame),
            status.is_some_and(|s| s.error.is_some()),
            running,
        );
        let preview = rail_preview_open(
            name,
            focused.as_deref(),
            only_selected,
            true,
            &state.session.ui.rail_preview,
        );
        let (cap_select, cap_remove, cap_fold) =
            rail_cap(ui, name, light, focused.as_deref(), cw, preview);
        let mut body_clicked = false;
        if preview {
            let after = ui.cursor_pos();
            let remain_h = (cy + ch - after[1]).max(1.0);
            let size = fit_applet([cw, remain_h]);
            ui.set_cursor_pos([
                cx + ((cw - size[0]) * 0.5).max(0.0),
                after[1] + ((remain_h - size[1]) * 0.5).max(0.0),
            ]);
            let draw = draw_for_slot(&state.session.focus.lock().unwrap(), name);
            body_clicked = cell_body(ui, gpu, state, name, size, draw);
            if is_focused && capture && ui.is_item_hovered() {
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
            if is_focused {
                draw_focused_queue_card(ui, &state.session, ui.item_rect_min());
                if let Some(slot) = statuses.iter().find(|s| s.username == *name) {
                    state
                        .paint
                        .frame(ui, slot.script_paint.as_ref(), ui.item_rect_min(), size);
                }
            }
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
}

/// Named GameShell `ch` values (arrows 1–4, ASCII controls, space).
const CAPTURE_NAMED: &[(Key, i32)] = &[
    (Key::LeftArrow, 1),
    (Key::RightArrow, 2),
    (Key::UpArrow, 3),
    (Key::DownArrow, 4),
    (Key::Backspace, 8),
    (Key::Delete, 8),
    (Key::Tab, 9),
    (Key::Enter, 10),
    (Key::Escape, 27),
    (Key::Space, 32),
];
const CAPTURE_LETTERS: [Key; 26] = [
    Key::A,
    Key::B,
    Key::C,
    Key::D,
    Key::E,
    Key::F,
    Key::G,
    Key::H,
    Key::I,
    Key::J,
    Key::K,
    Key::L,
    Key::M,
    Key::N,
    Key::O,
    Key::P,
    Key::Q,
    Key::R,
    Key::S,
    Key::T,
    Key::U,
    Key::V,
    Key::W,
    Key::X,
    Key::Y,
    Key::Z,
];
const CAPTURE_DIGITS: [(Key, u8, u8); 10] = [
    (Key::Key0, b'0', b')'),
    (Key::Key1, b'1', b'!'),
    (Key::Key2, b'2', b'@'),
    (Key::Key3, b'3', b'#'),
    (Key::Key4, b'4', b'$'),
    (Key::Key5, b'5', b'%'),
    (Key::Key6, b'6', b'^'),
    (Key::Key7, b'7', b'&'),
    (Key::Key8, b'8', b'*'),
    (Key::Key9, b'9', b'('),
];
/// Punctuation: same `ch` as client-play `key_codes::lookup` / KeyCodes.ts
/// 48–80. Without this, `::` (`:`) and `~` never reach chat.
const CAPTURE_PUNCT: [(Key, u8, u8); 11] = [
    (Key::GraveAccent, b'`', b'~'),
    (Key::Minus, b'-', b'_'),
    (Key::Equal, b'=', b'+'),
    (Key::LeftBracket, b'[', b'{'),
    (Key::RightBracket, b']', b'}'),
    (Key::Backslash, b'\\', b'|'),
    (Key::Semicolon, b';', b':'),
    (Key::Apostrophe, b'\'', b'"'),
    (Key::Comma, b',', b'<'),
    (Key::Period, b'.', b'>'),
    (Key::Slash, b'/', b'?'),
];

/// GameShell `ch` for one ImGui key, Shift applied the way client-play
/// maps DOM `KeyboardEvent.key` (`:` is 58, `~` is 126).
fn capture_key_ch(key: Key, shift: bool) -> Option<i32> {
    for &(k, ch) in CAPTURE_NAMED {
        if k == key {
            return Some(ch);
        }
    }
    for (i, &k) in CAPTURE_LETTERS.iter().enumerate() {
        if k == key {
            let base = if shift { b'A' } else { b'a' };
            return Some((base + i as u8) as i32);
        }
    }
    for &(k, unshifted, shifted) in &CAPTURE_DIGITS {
        if k == key {
            return Some(if shift { shifted } else { unshifted } as i32);
        }
    }
    for &(k, unshifted, shifted) in &CAPTURE_PUNCT {
        if k == key {
            return Some(if shift { shifted } else { unshifted } as i32);
        }
    }
    None
}

/// Map hovered ImGui keys to GameShell `ch` values (arrows 1–4, ASCII).
fn capture_keys(ui: &Ui) -> Vec<(bool, i32)> {
    let shift = ui.is_key_down(Key::LeftShift) || ui.is_key_down(Key::RightShift);
    let mut keys = Vec::new();
    let all = CAPTURE_NAMED
        .iter()
        .map(|&(k, _)| k)
        .chain(CAPTURE_LETTERS)
        .chain(CAPTURE_DIGITS.iter().map(|&(k, _, _)| k))
        .chain(CAPTURE_PUNCT.iter().map(|&(k, _, _)| k));
    for key in all {
        let Some(ch) = capture_key_ch(key, shift) else {
            continue;
        };
        if ui.is_key_pressed_with_repeat(key, false) {
            keys.push((true, ch));
        }
        if ui.is_key_released(key) {
            keys.push((false, ch));
        }
    }
    keys
}

/// Right panel: rs2b0t chrome squished into the 330px strip. Vertical scroll
/// only — wrap/clip, never a horizontal bar.
fn panel_window(ui: &Ui, session: &mut Session) {
    ui.window(PANEL_WINDOW)
        .flags(WindowFlags::NO_RESIZE | WindowFlags::NO_COLLAPSE)
        .build(|| {
            let _width = ui.push_item_width(-1.0);
            let _wrap = ui.push_text_wrap_pos(0.0);
            title_row(ui, session);
            ui.text_colored(TEXT_DIM, crate::build_info::build_line());
            ui.set_item_tooltip(crate::build_info::build_tooltip());
            banner(ui, session);
            login_logout_row(ui, session);
            walkto_button(ui, session);
            slot_config_row(ui, session);
            let order = resolve_heading_order(&session.ui.section_order);
            for id in order {
                match id.as_str() {
                    "status" => status_section(ui, session),
                    "profile" => profile_section(ui, session),
                    "script" => script_section(ui, session),
                    "parameters" => parameters_section(ui, session),
                    "debug" => debug_section(ui, session),
                    "log" => log_section(ui, session),
                    _ => {}
                }
            }
        });
}

fn title_row(ui: &Ui, session: &mut Session) {
    ui.text_colored(ACCENT, TITLE);
    ui.same_line();
    let avail = ui.content_region_avail()[0];
    let (w, stack) = button_row_layout(avail, 2);
    if !stack {
        let total = w * 2.0 + BUTTON_GAP;
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

fn banner(ui: &Ui, session: &Session) {
    if let Some(err) = &session.error {
        ui.text_colored(ERROR, err);
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

/// Same-line gap that matches [`equal_button_width`]'s `BUTTON_GAP`.
fn gap_line(ui: &Ui) {
    ui.same_line_with_spacing(0.0, BUTTON_GAP);
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

/// profile: orange current name (like script) plus Profiles (opens the
/// picker). Unlock prompt until the vault is open. Sits under WalkTo /
/// config, above debug.
fn profile_section(ui: &Ui, session: &mut Session) {
    if !section_open(ui, session, "profile") {
        return;
    }
    if session.vault.is_none() {
        ui.input_text("##vault-pass", &mut session.pass_scratch)
            .password(true)
            .hint("vault passphrase")
            .build();
        let w = ui.content_region_avail()[0];
        let exists = Session::default_vault_exists();
        let label = if exists {
            "Unlock vault"
        } else {
            "Create vault"
        };
        if ui.button_with_size(label, [w, 0.0]) {
            let pass = session.pass_scratch.trim().to_string();
            if !pass.is_empty() {
                session.unlock(&pass);
                session.pass_scratch.clear();
            }
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
    if h <= 2.0 {
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

/// Log in / Logout above WalkTo. Always drawn; disabled while the vault
/// is locked or no profile is focused. Logout also needs ingame.
fn login_logout_row(ui: &Ui, session: &mut Session) {
    let avail = ui.content_region_avail()[0];
    let cells = button_cells(avail, 2);
    let vault_open = session.vault.is_some();
    let focused = session.focused_name();
    let can_login = vault_open && focused.is_some();
    let can_logout = can_login && session.focused_ingame();
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
        } else {
            "log out the focused slot — it stays in the picker"
        });
    }
}

/// WalkTo: main-chrome button that opens the collision-dot tile picker.
fn walkto_button(ui: &Ui, session: &mut Session) {
    let w = ui.content_region_avail()[0];
    if ui.button_with_size("WalkTo", [w, 0.0]) {
        session.walkto_open = !session.walkto_open;
    }
    ui.set_item_tooltip("open the tile picker — close with the window ✕");
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
    let main = debug_main_buttons(show_tutskip);
    let avail = ui.content_region_avail()[0];
    // Packed one row even when a scrollbar trims avail below MIN_BUTTON —
    // stacking turns DebugPanel/Lumbridge/maxme/Teles into four strip-width
    // buttons.
    let w = equal_button_width(avail, main.len());
    for (i, label) in main.iter().enumerate() {
        if i > 0 {
            gap_line(ui);
        }
        let caption = debug_caption(label);
        match *label {
            "DebugPanel" => {
                let _off = ui.begin_disabled();
                let _ = ui.button_with_size(caption, [w, 0.0]);
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
                    for cmd in debug_maxme_cheats() {
                        session.cheat_focused(cmd);
                    }
                }
                ui.set_item_tooltip("19× setstat 99");
            }
            "Teles" if ui.button_with_size(caption, [w, 0.0]) => {
                ui.open_popup("##debug-teles");
            }
            _ => {}
        }
    }
    ui.popup("##debug-teles", || {
        let dests = debug_dest_cheats();
        let avail = PANEL_WIDTH;
        let cols = (1usize..=6)
            .rev()
            .find(|&n| equal_button_width(avail, n) >= MIN_BUTTON)
            .unwrap_or(1);
        let (bw, _) = button_row_layout(avail, cols);
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
        .map(|sel| sel.label())
        .unwrap_or_else(|| "(none)".to_string());
    ui.text_colored(ACCENT, name);
    ui.same_line();
    let avail = ui.content_region_avail()[0];
    let (w, stack) = button_row_layout(avail, 2);
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
            session.script_load_open = true;
        }
        ui.set_item_tooltip("load an out-of-tree JS bot file (native tick or defineBot)");
    }

    let (sw, sstack) = button_row_layout(ui.content_region_avail()[0], SCRIPT_ROW.len());
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
        let _stop = if script_stop_enabled(state) {
            None
        } else {
            Some(ui.begin_disabled())
        };
        if ui.button_with_size("Stop", [sw, 0.0]) {
            session.script_stop();
        }
    }

    let status = script_status_text(state);
    match session.focused_script_last_error() {
        Some(err) => kv_row(ui, "status", &format!("{status}: {err}")),
        None => kv_row(ui, "status", status),
    }
}

/// True while the script Load picker was wanted last frame (same rising-
/// edge latch as the chooser and Browse).
static PREV_LOAD: AtomicBool = AtomicBool::new(false);

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

fn card_categories(cards: &[script::JsCard]) -> Vec<String> {
    let mut out = Vec::new();
    for card in cards {
        let cat = if card.category.is_empty() {
            "Uncategorized".to_string()
        } else {
            card.category.clone()
        };
        if !out.iter().any(|c| c == &cat) {
            out.push(cat);
        }
    }
    out
}

fn category_chip_dnd(ui: &Ui, session: &mut Session, cat: &str) {
    if let Some(_tip) = ui
        .drag_drop_source_config("274bot-script-cat")
        .begin_payload(category_key(cat))
    {
        ui.text(cat);
    }
    if let Some(tgt) = ui.drag_drop_target() {
        if let Some(Ok(p)) = tgt.accept_payload::<[u8; 32], _>(
            "274bot-script-cat",
            DragDropTargetFlags::NONE,
        ) {
            if p.delivery {
                if let Some(from) = category_from_key(p.data) {
                    let present = card_categories(session.js.cards());
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

fn browse_script_card_row(ui: &Ui, session: &mut Session, card: &script::JsCard, w: f32) {
    let selected = session.script_sel
        == Some(script::ScriptSel::Loaded(card.source, card.name.clone()));
    let label = format!(
        "{}  [{}] [{}]",
        card.name,
        kind_badge(card.kind),
        source_badge(card.source)
    );
    if ui
        .selectable_config(label)
        .selected(selected)
        .size([w, 0.0])
        .build()
    {
        session.script_sel = Some(script::ScriptSel::Loaded(
            card.source,
            card.name.clone(),
        ));
    }
    if !card.description.is_empty() {
        ui.text_disabled(&card.description);
    }
    let cat = if card.category.is_empty() {
        "Uncategorized"
    } else {
        card.category.as_str()
    };
    ui.text_disabled(format!("category: {cat}"));
    if !card.tags.is_empty() {
        ui.text_disabled(format!("tags: {}", card.tags.join(", ")));
    }
}

fn browse_window_body(ui: &Ui, session: &mut Session) {
    let w = ui.content_region_avail()[0];
    if script::rs2b0t_import_deferred() {
        if ui.button_with_size("Import catalog…", [w, 0.0]) {
            session.rs2b0t_catalog_open = true;
            session.rs2b0t_catalog_dir = std::env::var("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("/"));
        }
        ui.spacing();
    }
    let cards: Vec<script::JsCard> = session.js.cards().to_vec();
    let present = card_categories(&cards);
    let order = resolve_category_order(&session.ui.script_category_order, &present);
    if order != session.ui.script_category_order {
        session.ui.script_category_order = order.clone();
        crate::ui_state::save(&session.ui);
    }
    if !order.is_empty() {
        ui.text_disabled("categories (drag to reorder):");
        let all_sel = session.browse_category_filter.is_none();
        if ui.selectable_config("All").selected(all_sel).build() {
            session.browse_category_filter = None;
        }
        ui.same_line();
        for cat in &order {
            let sel = session.browse_category_filter.as_deref() == Some(cat.as_str());
            if ui.selectable_config(cat).selected(sel).build() {
                session.browse_category_filter = Some(cat.clone());
            }
            ui.same_line();
            category_chip_dnd(ui, session, cat);
            ui.same_line();
        }
        ui.new_line();
        ui.spacing();
    }
    let ids = script::compiled_ids();
    let filter = session.browse_category_filter.clone();
    let mut any = false;
    for id in ids {
        any = true;
        let selected = session.script_sel == Some(script::ScriptSel::Compiled(*id));
        if ui
            .selectable_config(id.0)
            .selected(selected)
            .size([w, 0.0])
            .build()
        {
            session.script_sel = Some(script::ScriptSel::Compiled(*id));
        }
    }
    for card in &cards {
        let cat = if card.category.is_empty() {
            "Uncategorized"
        } else {
            card.category.as_str()
        };
        if filter.as_deref().is_some_and(|f| f != cat) {
            continue;
        }
        any = true;
        browse_script_card_row(ui, session, card, w);
    }
    if !any {
        ui.text_disabled("no scripts — Browse is empty");
    }
}

/// Browse picker: script cards with category chips and badges. Non-modal
/// window (same pattern as Nav config).
fn browse_window(ui: &Ui, session: &mut Session) {
    rs2b0t_catalog_window(ui, session);
    if !session.script_browse_open {
        return;
    }
    let mut open = true;
    ui.window(BROWSE_WINDOW_TITLE)
        .opened(&mut open)
        .flags(WindowFlags::NO_COLLAPSE)
        .size([480.0, 520.0], Condition::FirstUseEver)
        .build(|| browse_window_body(ui, session));
    session.script_browse_open = open;
}

/// First-run folder browser for the rs2b0t clone root (`index.ts` required).
fn rs2b0t_catalog_window(ui: &Ui, session: &mut Session) {
    if !session.rs2b0t_catalog_open {
        return;
    }
    let mut open = true;
    ui.window("Import rs2b0t catalog")
        .opened(&mut open)
        .flags(WindowFlags::NO_COLLAPSE)
        .size([420.0, 420.0], Condition::FirstUseEver)
        .build(|| {
            let w = ui.content_region_avail()[0];
            ui.text_wrapped(
                "Choose the rs2b0t clone root (must contain src/bot/scripts/index.ts):",
            );
            ui.text(&session.rs2b0t_catalog_dir.to_string_lossy());
            ui.spacing();
            let dir = session.rs2b0t_catalog_dir.clone();
            if dir.parent().is_some() {
                if ui.button_with_size("Up", [w, 0.0]) {
                    if let Some(parent) = dir.parent() {
                        session.rs2b0t_catalog_dir = parent.to_path_buf();
                    }
                }
                ui.spacing();
            }
            let mut subdirs = Vec::new();
            if let Ok(read) = std::fs::read_dir(&dir) {
                for entry in read.flatten() {
                    if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                        subdirs.push(entry.path());
                    }
                }
            }
            subdirs.sort();
            for path in subdirs {
                let name = path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("?");
                if ui.selectable_config(name).build() {
                    session.rs2b0t_catalog_dir = path;
                }
            }
            ui.spacing();
            let has_index = script_picker::rs2b0t_root_has_index(&session.rs2b0t_catalog_dir);
            if has_index {
                ui.text_colored(ACCENT, "catalog index found");
            } else {
                ui.text_disabled("no src/bot/scripts/index.ts here");
            }
            let half = w / 2.0 - BUTTON_GAP / 2.0;
            if has_index {
                if ui.button_with_size("Use this folder", [half, 0.0]) {
                    let root = session.rs2b0t_catalog_dir.clone();
                    if let Err(e) = session.import_rs2b0t_catalog(&root) {
                        session.error = Some(e);
                    } else {
                        session.error = None;
                    }
                }
                ui.same_line();
            }
            if ui.button_with_size("Not now", [half, 0.0]) {
                session.defer_rs2b0t_catalog();
            }
        });
    session.rs2b0t_catalog_open = open;
}

/// Load modal: a filesystem path to an out-of-tree JS bot. Load registers
/// the card (same name overwrites; only WalkTo reserved), persists the
/// store, and selects the card for Start. The isolate is spawned only on
/// Start, never here.
fn load_window(ui: &Ui, session: &mut Session) {
    let want = session.script_load_open;
    let (open_popup, new_prev) = chooser_should_open_popup(want, PREV_LOAD.load(Ordering::Relaxed));
    PREV_LOAD.store(new_prev, Ordering::Relaxed);
    if open_popup {
        ui.open_popup("274bot-load");
    }
    let mut open = want;
    if let Some(_t) = ui
        .begin_modal_popup_config("274bot-load")
        .opened(&mut open)
        .begin()
    {
        let w = ui.content_region_avail()[0];
        ui.text("path to a JS bot file:");
        ui.input_text("##load-path", &mut session.load_scratch)
            .hint("e.g. ~/bot.js")
            .build();
        if ui.button_with_size("Load", [w / 2.0, 0.0]) {
            let path = session.load_scratch.clone();
            session.load_js(&path);
            if session.error.is_none() {
                ui.close_current_popup();
            }
        }
        ui.same_line();
        if ui.button_with_size("Cancel", [w / 2.0, 0.0]) {
            session.load_scratch.clear();
            ui.close_current_popup();
        }
    }
    session.script_load_open = open;
}

/// Nav config window: Routing, Display, Path paint (only while the path
/// is shown), and Debug groups. Every toggle or colour writes
/// `session.ui.nav` + `ui_state::save`. Not a blocking modal.
fn nav_settings_window(ui: &Ui, session: &mut Session) {
    if !session.nav_settings_open {
        return;
    }
    let mut open = true;
    ui.window("Nav config")
        .opened(&mut open)
        .flags(WindowFlags::NO_COLLAPSE)
        .size([360.0, 480.0], Condition::FirstUseEver)
        .build(|| {
            let mut nav = session.ui.nav.clone();
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

            if changed {
                session.ui.nav = nav;
                crate::ui_state::save(&session.ui);
            }
        });
    session.nav_settings_open = open;
}

/// Nav paint colour: imgui color-edit (preview square opens the picker)
/// plus hex display. The stored string stays `#RRGGBB`.
fn nav_color_field(ui: &Ui, label: &str, color: &mut String) -> bool {
    let [r, g, b] = crate::nav_settings::parse_html_color(color, [0xff, 0xff, 0xff]);
    let mut rgb = [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0];
    ui.set_next_item_width(148.0);
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

/// parameters: uncollapses the selected JS card's merged settings rows,
/// then Edit opens typed editors (honours `showIf` / `group`).
fn parameters_section(ui: &Ui, session: &mut Session) {
    if !section_open(ui, session, "parameters") {
        return;
    }
    let mut has_schema = false;
    match &session.script_sel {
        Some(script::ScriptSel::Loaded(source, name)) => {
            if let Some(card) = session.js.get(*source, name) {
                if card.settings_schema.is_empty() {
                    ui.text_disabled("(no parameters)");
                } else {
                    has_schema = true;
                    let bag = session.merged_settings_bag(*source, name, &card.settings_schema);
                    for (label, value) in script::parameter_rows(&card.settings_schema, &bag) {
                        kv_row(ui, &label, &value);
                    }
                }
            } else {
                ui.text_disabled("(no parameters)");
            }
        }
        Some(script::ScriptSel::Compiled(id)) => {
            let pairs = script::defaults(*id);
            if pairs.is_empty() {
                ui.text_disabled("(no parameters)");
            } else {
                for (k, v) in pairs {
                    kv_row(ui, &k, &v);
                }
            }
        }
        None => ui.text_disabled("(no script selected)"),
    }
    let w = ui.content_region_avail()[0];
    if edit_parameters_enabled() {
        if has_schema {
            if ui.button_with_size("Edit parameters", [w, 0.0]) {
                session.params_edit_open = true;
            }
        } else {
            mock_button(ui, "Edit parameters", "select a script with a schema", [w, 0.0]);
        }
    } else {
        mock_button(ui, "Edit parameters", "not in v1", [w, 0.0]);
    }
}

/// Modal typed editors for the selected card's settings schema.
fn params_edit_window(ui: &Ui, session: &mut Session) {
    let want = session.params_edit_open;
    let mut open = want;
    if !want {
        return;
    }
    ui.open_popup("274bot-params");
    if let Some(_t) = ui
        .begin_modal_popup_config("274bot-params")
        .opened(&mut open)
        .begin()
    {
        let Some(script::ScriptSel::Loaded(source, name)) = session.script_sel.clone() else {
            ui.close_current_popup();
            session.params_edit_open = false;
            return;
        };
        let Some(card) = session.js.get(source, &name).cloned() else {
            ui.close_current_popup();
            session.params_edit_open = false;
            return;
        };
        if card.settings_schema.is_empty() {
            ui.close_current_popup();
            session.params_edit_open = false;
            return;
        }
        let mut bag = session.merged_settings_bag(source, &name, &card.settings_schema);
        let mut last_group: Option<String> = None;
        for def in &card.settings_schema {
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
            let label = def
                .label
                .as_deref()
                .unwrap_or(&def.id)
                .to_string();
            match def.ty.as_str() {
                "boolean" => {
                    let mut value = bag
                        .get(&def.id)
                        .and_then(|v| v.as_bool())
                        .unwrap_or_else(|| def.default.as_deref() == Some("true"));
                    if ui.checkbox(&label, &mut value) {
                        session
                            .script_settings
                            .set_bool(source, &name, &def.id, value);
                        let _ = session.script_settings.save();
                        bag.insert(def.id.clone(), serde_json::json!(value));
                    }
                }
                "number" => {
                    let mut value = bag
                        .get(&def.id)
                        .and_then(|v| v.as_f64())
                        .or_else(|| {
                            def.default
                                .as_deref()
                                .and_then(|s| s.parse::<f64>().ok())
                        })
                        .unwrap_or(0.0) as i32;
                    if ui.input_int(&label, &mut value) {
                        session
                            .script_settings
                            .set_num(source, &name, &def.id, value as f64);
                        let _ = session.script_settings.save();
                        bag.insert(def.id.clone(), serde_json::json!(value));
                    }
                }
                "string" if !script::resolve_setting_options(def, &session.loadouts).is_empty() => {
                    ui.text(&label);
                    let opts = script::resolve_setting_options(def, &session.loadouts);
                    let current = bag
                        .get(&def.id)
                        .and_then(|v| v.as_str())
                        .or(def.default.as_deref())
                        .unwrap_or("")
                        .to_string();
                    ui.set_next_item_width(-1.0);
                    let combo_opts = ComboBoxOptions::new().preview_mode(ComboBoxPreviewMode::Preview);
                    if let Some(_popup) =
                        ui.begin_combo_with_flags(&format!("##param-{id}", id = def.id), &current, combo_opts)
                    {
                        for opt in &opts {
                            let selected = opt == &current;
                            if ui.selectable_config(opt).selected(selected).build() {
                                session
                                    .script_settings
                                    .set_str(source, &name, &def.id, opt);
                                let _ = session.script_settings.save();
                                bag.insert(def.id.clone(), serde_json::json!(opt));
                            }
                        }
                    }
                }
                "string" | "tile" | "list" => {
                    let mut text = bag
                        .get(&def.id)
                        .map(script::format_setting_value)
                        .or(def.default.as_deref().map(String::from))
                        .unwrap_or_default();
                    if ui.input_text(&label, &mut text).build() {
                        let coerced =
                            script::coerce_setting_value(&def.ty, &serde_json::json!(text));
                        session
                            .script_settings
                            .set_value(source, &name, &def.id, coerced.clone());
                        let _ = session.script_settings.save();
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
        if ui.button("Close") {
            ui.close_current_popup();
        }
    }
    session.params_edit_open = open;
}

/// Under WalkTo, above profile: per-slot renderer/mem, nav paints,
/// Loadouts (mocked until the TS shim). Wraps so "General config" is not clipped.
fn slot_config_row(ui: &Ui, session: &mut Session) {
    let avail = ui.content_region_avail()[0];
    let cells = button_cells_min(avail, CONFIG_ROW.len(), CONFIG_MIN);
    for (i, &(w, same_line)) in cells.iter().enumerate() {
        if same_line {
            gap_line(ui);
        }
        match CONFIG_ROW[i] {
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
                    sync_loadouts_scratch(session);
                }
                ui.set_item_tooltip("equipment and inventory presets");
            }
            _ => {}
        }
    }
}

/// Copy the selected loadout into the session scratch buffers.
fn sync_loadouts_scratch(session: &mut Session) {
    if let Some(loadout) = session.loadouts.loadouts().get(session.loadouts_sel) {
        session.loadouts_name_scratch = loadout.name.clone();
        session.loadouts_worn_scratch = loadout.worn.join(", ");
        session.loadouts_carry_scratch = loadout.carry.join(", ");
    } else {
        session.loadouts_name_scratch.clear();
        session.loadouts_worn_scratch.clear();
        session.loadouts_carry_scratch.clear();
    }
}

fn split_loadout_csv(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

fn apply_loadouts_scratch(session: &mut Session) {
    let name = session.loadouts_name_scratch.trim();
    if name.is_empty() {
        return;
    }
    session.loadouts.upsert(script::Loadout {
        name: name.to_string(),
        worn: split_loadout_csv(&session.loadouts_worn_scratch),
        carry: split_loadout_csv(&session.loadouts_carry_scratch),
    });
    let _ = session.loadouts.save();
}

/// Loadouts window: list presets and edit worn/carry item names.
fn loadouts_window(ui: &Ui, session: &mut Session) {
    if !session.loadouts_open {
        return;
    }
    let mut open = true;
    ui.window("Loadouts")
        .opened(&mut open)
        .flags(WindowFlags::NO_COLLAPSE)
        .size([400.0, 360.0], Condition::FirstUseEver)
        .build(|| {
            let count = session.loadouts.loadouts().len();
            if count == 0 {
                ui.text_wrapped("No loadouts yet — add one below.");
            } else {
                ui.text("Presets");
                let names: Vec<String> = session.loadouts.names();
                for (i, name) in names.iter().enumerate() {
                    let selected = i == session.loadouts_sel;
                    if ui.selectable_config(name).selected(selected).build() {
                        session.loadouts_sel = i;
                        sync_loadouts_scratch(session);
                    }
                }
            }
            ui.spacing();
            ui.text("Edit");
            if ui.input_text("name", &mut session.loadouts_name_scratch).build() {
                apply_loadouts_scratch(session);
            }
            if ui.input_text("worn (comma-separated)", &mut session.loadouts_worn_scratch).build()
            {
                apply_loadouts_scratch(session);
            }
            if ui.input_text("carry (comma-separated)", &mut session.loadouts_carry_scratch)
                .build()
            {
                apply_loadouts_scratch(session);
            }
            if ui.button("Add loadout") {
                let next = format!("loadout-{}", session.loadouts.loadouts().len() + 1);
                session.loadouts.upsert(script::Loadout {
                    name: next.clone(),
                    worn: vec![],
                    carry: vec![],
                });
                let _ = session.loadouts.save();
                session.loadouts_sel = session.loadouts.loadouts().len().saturating_sub(1);
                sync_loadouts_scratch(session);
            }
            if count > 0 && ui.button("Delete selected") {
                if let Some(name) = session
                    .loadouts
                    .loadouts()
                    .get(session.loadouts_sel)
                    .map(|l| l.name.clone())
                {
                    session.loadouts.remove(&name);
                    let _ = session.loadouts.save();
                    if session.loadouts_sel >= session.loadouts.loadouts().len() {
                        session.loadouts_sel = session.loadouts.loadouts().len().saturating_sub(1);
                    }
                    sync_loadouts_scratch(session);
                }
            }
        });
    session.loadouts_open = open;
}

/// Parameter editing is enabled for compat cards with a settings schema.
fn edit_parameters_enabled() -> bool {
    true
}

/// `random` status-row value: `dialog: mysterious old man`, plus `(hold)`
/// while the slot freezes on the event and `(off)` when the profile toggle
/// is off (toggle-off still detects + publishes). `None` when nothing is
/// detected — the caller skips the row then.
fn random_status_text(r: &host::RandomStatus) -> Option<String> {
    let kind = r.kind?;
    let mut text = format!(
        "{}: {}",
        random_kind_name(kind),
        r.name.as_deref().unwrap_or("?")
    );
    if r.hold {
        text.push_str(" (hold)");
    }
    if !r.toggle {
        text.push_str(" (off)");
    }
    Some(text)
}

/// Status-row names for the guardian kinds (kebab-case, like the spec's
/// `evade: swarm` / `lost-tool` rows).
fn random_kind_name(kind: api::RandomKind) -> &'static str {
    use api::RandomKind::*;
    match kind {
        Dialog => "dialog",
        Pick => "pick",
        Evade => "evade",
        Maze => "maze",
        Mime => "mime",
        Box => "box",
        Lamp => "lamp",
        Hazard => "hazard",
        LostTool => "lost-tool",
        LostGear => "lost-gear",
    }
}

/// status: rs2b0t key/value rows (state, player, tile, modals, random),
/// wrapped.
fn status_section(ui: &Ui, session: &mut Session) {
    if !section_open(ui, session, "status") {
        return;
    }
    let statuses = session.statuses();
    if statuses.is_empty() {
        kv_row(ui, "state", "no slots");
        kv_row(ui, "player", "—");
        kv_row(ui, "tile", "—");
        kv_row(ui, "walk", &session.walk_status_text());
        kv_row(ui, "queue", "—");
        kv_row(ui, "modals", "—");
        kv_row(
            ui,
            "mem",
            Session::mem_status_text(session.focused_lowmem()),
        );
        return;
    }
    // Focused slot if present, else the first runner. One bot's rows, not a
    // concatenated line that overflows the 330px strip.
    let focused = session.focused_name();
    let s = statuses
        .iter()
        .find(|s| focused.as_deref() == Some(s.username.as_str()))
        .unwrap_or(&statuses[0]);
    let state = if s.ingame {
        format!("ingame scene {}", s.scene_state)
    } else if let Some(err) = &s.error {
        format!("login {err}")
    } else if s.login_started.is_some() {
        "logging in…".to_string()
    } else {
        "waiting".to_string()
    };
    let player = if s.player.is_empty() {
        "?"
    } else {
        s.player.as_str()
    };
    kv_row(ui, "state", &state);
    kv_row(ui, "player", player);
    kv_row(ui, "tile", &format!("{} {}", s.tile_x, s.tile_z));
    kv_row(ui, "walk", &session.walk_status_text());
    let queue = queue_k_of_n(s.queue_position, s.queue_total).unwrap_or_else(|| "—".into());
    kv_row(ui, "queue", &queue);
    kv_row(ui, "modals", &format!("{}", s.main_modal_id));
    if let Some(random) = random_status_text(&s.random) {
        kv_row(ui, "random", &random);
    }
    kv_row(
        ui,
        "mem",
        Session::mem_status_text(session.focused_lowmem()),
    );
}

/// log: focused slot's status-transition lines (or PROCESS when none).
fn log_section(ui: &Ui, session: &mut Session) {
    if !section_open(ui, session, "log") {
        return;
    }
    let key = session
        .focused_name()
        .unwrap_or_else(|| PROCESS.to_string());
    let log_by = session.log_by.lock().unwrap();
    let empty: Vec<String> = Vec::new();
    let lines = log_by.get(&key).unwrap_or(&empty);
    ui.child_window("panel-log")
        .size([0.0, 80.0])
        .build(ui, || {
            let _wrap = ui.push_text_wrap_pos(0.0);
            for line in lines.iter() {
                ui.text_wrapped(line);
            }
        });
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
    });
}

/// none / GPU / CPU row. Click the mem button (current highmem/lowmem) for
/// the sticky picker, like Teles.
fn raster_picker(ui: &Ui, session: &mut Session) {
    let cur = session.focused_raster();
    let avail = ui.content_region_avail()[0];
    let cells = button_cells(avail, 3);
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
        "Game pane highmem / lowmem — switching mem reattaches the renderer on the live client",
    );
    mem_popup(ui, session);
}

fn config_heading(ui: &Ui, label: &str) {
    ui.text_colored(ACCENT, label);
}

/// rendering: none/GPU/CPU picker; `set_draw` is applied by the slot
/// threads from the shared focus on every frame.
fn slot_render_section(ui: &Ui, session: &mut Session) {
    ui.text_wrapped("Game pane only. Rail members stay GPU / lowmem at 1 fps (CPU/none as fallback). Click lowmem/highmem for the sticky picker. Switching GPU↔CPU or mem drops + reattaches the renderer — the client stays logged in.");
    raster_picker(ui, session);
    let mut focused_50 = session.focus.lock().unwrap().focused_50;
    if ui.checkbox("focused 50 fps", &mut focused_50) {
        session.set_focused_50(focused_50);
    }
    ui.text_wrapped("Game pane only; this client on the rail uses the rail cadence.");
}

fn slot_capture_section(ui: &Ui, session: &mut Session) {
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
    let focused = session.focused_name();
    let auto = focused
        .as_deref()
        .and_then(|n| session.vault.as_ref().and_then(|v| v.get(n)))
        .map(|p| p.settings.auto_login)
        .unwrap_or(false);
    let mut auto_cur = auto;
    if ui.checkbox("auto-login on title", &mut auto_cur) {
        if let Some(name) = focused {
            session.set_auto_login(&name, auto_cur);
        }
    }
    ui.text_wrapped("this profile; handshake on spawn unless latched out");
}

fn global_config_section(ui: &Ui, session: &mut Session) {
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
}

/// Sidecar rail window: only while MultiBox is on and Grid is off. Bulk
/// Login all / Logout all, the only-render-selected checkbox (unchecking
/// needs the render-all confirm), one tile per wall member (cap only while
/// only-render-selected, else cap + 1 fps body or renderer-off
/// placeholder), `+ add bot`, and the 1 Hz resource card.
fn rail_window(ui: &Ui, gpu: &mut Gpu, state: &mut PanelState) {
    state.sample_resources();
    ui.window(RAIL_WINDOW)
        .flags(WindowFlags::NO_COLLAPSE | WindowFlags::NO_RESIZE | WindowFlags::NO_TITLE_BAR)
        .build(|| {
            rail_bulk_row(ui, state);
            rail_tiles(ui, gpu, state);
            add_bot_button(ui, state);
            resource_card(ui, state);
        });
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
    let (w, stack) = button_row_layout(avail, 2);
    if ui.button_with_size("Login all", [w, 0.0]) {
        state.session.login_all();
    }
    if !stack {
        ui.same_line();
    }
    if ui.button_with_size("Logout all", [w, 0.0]) {
        state.session.logout_all();
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
const VAULT_RESET_POPUP: &str = "Reset vault?";
const PROFILE_DELETE_POPUP: &str = "Delete profile?";

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
        let _wrap = ui.push_text_wrap_pos(DIALOG_W - 16.0);
        ui.text_wrapped(body);
        ui.spacing();
        ui.checkbox("I understand", understood);
        ui.spacing();
        let avail = ui.content_region_avail()[0];
        let (w, stack) = button_row_layout(avail, 2);
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
        .size_constraints([DIALOG_W, 80.0], [DIALOG_W, 720.0])
        .build(|| {
            let _wrap = ui.push_text_wrap_pos(DIALOG_W - 16.0);
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
            let (w, stack) = button_row_layout(avail, 2);
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

/// One tile per wall member, in wall order: cap (traffic-light dot,
/// name + brief, ✗) then a `TILE_W`×`TILE_H` body. The body blits the
/// slot's `FrameBuf` when `draw_for_slot` says this member paints, else
/// the renderer-off placeholder. While `only_render_selected` is on (the
/// safe default) the strip is collapsed: cap only, no body. Clicking the
/// name or the body focuses the member; the ✗ (a sibling button, never
/// part of the name click) removes it.
fn rail_tiles(ui: &Ui, gpu: &mut Gpu, state: &mut PanelState) {
    ui.spacing();
    let members = state.session.wall.members.clone();
    let statuses = state.session.statuses();
    let only_selected = state.session.focus.lock().unwrap().only_render_selected;
    {
        let focus = state.session.focus.lock().unwrap();
        dispose_idle_views(&mut state.views, gpu, |name| {
            members.iter().any(|m| m == name) && draw_for_slot(&focus, name)
        });
    }
    for name in &members {
        let status = statuses.iter().find(|s| &s.username == name);
        let running = status.is_some_and(|s| s.walk_x != -1)
            || state
                .session
                .play
                .as_ref()
                .is_some_and(|p| p.script_state(name) == script::RunState::Running);
        let light = traffic_light(
            status.is_some_and(|s| s.ingame),
            status.is_some_and(|s| s.error.is_some()),
            running,
        );
        let (focused, draw) = {
            let focus = state.session.focus.lock().unwrap();
            (focus.focused.clone(), draw_for_slot(&focus, name))
        };
        let avail = ui.content_region_avail()[0];
        let preview = rail_preview_open(
            name,
            focused.as_deref(),
            only_selected,
            false,
            &state.session.ui.rail_preview,
        );
        let (cap_select, cap_remove, cap_fold) =
            rail_cap(ui, name, light, focused.as_deref(), avail, preview);
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
}

/// Cap row: the traffic-light dot, the member's name plus brief status
/// (click selects), and a small red ✗ (rail remove: logout arm then
/// `stop_slot`, never `vault`). `width` is the strip the row must fit
/// (rail avail or grid cell width). Returns `(selected, removed)`.
fn rail_cap(
    ui: &Ui,
    name: &str,
    light: Light,
    focused: Option<&str>,
    width: f32,
    preview: bool,
) -> (bool, bool, bool) {
    const BTN: f32 = 28.0;
    const DOT_W: f32 = 18.0;
    ui.text_colored(light.rgb(), STATUS_GLYPH);
    gap_line(ui);
    let selected = focused == Some(name);
    let name_w = (width - BTN * 2.0 - DOT_W - BUTTON_GAP * 3.0).max(10.0);
    let clicked = ui
        .selectable_config(cap_title(name, light))
        .selected(selected)
        .size([name_w, 0.0])
        .build();
    gap_line(ui);
    let fold_g = if preview { FOLD_GLYPH } else { UNFOLD_GLYPH };
    let folded = ui.button_with_size(format!("{fold_g}##fold-{name}"), [BTN, 0.0]);
    ui.set_item_tooltip(if preview {
        "fold preview"
    } else {
        "show preview"
    });
    gap_line(ui);
    let red = ui.push_style_color(StyleColor::Text, ERROR);
    let removed = ui.button_with_size(format!("{REMOVE_GLYPH}##{name}"), [BTN, 0.0]);
    ui.set_item_tooltip("drop from the wall — does not delete the vault profile");
    red.pop();
    (clicked, removed, folded)
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
        return ui
            .selectable_config(format!("renderer off##{name}"))
            .size(size)
            .build();
    }
    let tv = state
        .views
        .entry(name.to_string())
        .or_insert_with(|| TileView {
            view: GameView::init(gpu),
        });
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
                .slots
                .get(name)
                .map(|s| Arc::ptr_eq(&s.pixels, &buf))
                .unwrap_or(false),
            None => false,
        }
    };
    if !game_pane_owns {
        // `take` moves the stored frame out; `present` routes it: the
        // `PixMap` (CPU) arm uploads into the tile's owned texture, the
        // `Texture` (GPU) arm binds the client's frame view directly.
        if let Some(frame) = state.session.slots.get(name).and_then(|s| s.pixels.take()) {
            tv.view.present(gpu, frame);
        }
    }
    ui.image(tv.view.tex_id, size);
    ui.is_item_clicked_with_button(MouseButton::Left)
}

/// Rail tile body: the fixed `TILE_W`×`TILE_H` case of [`cell_body`].
fn rail_body(ui: &Ui, gpu: &mut Gpu, state: &mut PanelState, name: &str, draw: bool) -> bool {
    cell_body(ui, gpu, state, name, [TILE_W, TILE_H], draw)
}

/// `+ add bot`: opens the profile picker (same window as Profiles).
fn add_bot_button(ui: &Ui, state: &mut PanelState) {
    ui.spacing();
    let w = ui.content_region_avail()[0];
    if ui.button_with_size("+ add bot", [w, 0.0]) {
        state.session.wall.chooser_open = true;
    }
}

/// Resource card at the rail bottom: bots, CPU/RAM, and traffic from
/// ClientStream byte counters (1 Hz). First CPU/traffic sample reads
/// "measuring…"; a failed process sampler shows error for CPU/RAM only.
/// The draw/paint counters moved off the slot status row (M2 Task 1), so
/// the draw row is gone until Task 4's per-slot renderer metrics.
fn resource_card(ui: &Ui, state: &mut PanelState) {
    ui.spacing();
    ui.text_disabled("resource");
    ui.separator();
    let statuses = state.session.statuses();
    let ingame = statuses.iter().filter(|s| s.ingame).count();
    kv_row(ui, "bots", &format_bots(statuses.len(), ingame));
    match &state.res_cpu {
        Metric::Measuring => kv_row(ui, "cpu", "measuring…"),
        Metric::Available(s) => kv_row(ui, "cpu", s),
        Metric::Unavailable(r) => kv_row(ui, "cpu", r),
        Metric::Error(e) => kv_row(ui, "cpu", e),
    }
    match &state.res_ram {
        Metric::Measuring => kv_row(ui, "ram", "measuring…"),
        Metric::Available(s) => kv_row(ui, "ram", s),
        Metric::Unavailable(r) => kv_row(ui, "ram", r),
        Metric::Error(e) => kv_row(ui, "ram", e),
    }
    match &state.res_traffic {
        Metric::Measuring => kv_row(ui, "traffic", "measuring…"),
        Metric::Available(s) => kv_row(ui, "traffic", s),
        Metric::Unavailable(r) => kv_row(ui, "traffic", r),
        Metric::Error(e) => kv_row(ui, "traffic", e),
    }
}

/// Rising-edge helper: `(open_popup, new_prev)`. `open_popup` is true only
/// on the `want` false→true edge, so `+ add bot` reopens after a close;
/// `new_prev` tracks `want` on **both** values, or a closed chooser would
/// keep a stale `true` and the next reopen would never fire.
pub fn chooser_should_open_popup(want: bool, prev: bool) -> (bool, bool) {
    (want && !prev, want)
}

/// Profile picker (single-bot and MultiBox). Click a row to focus it
/// (and load onto the wall while MultiBox is on); Edit opens user/pass;
/// ✕ deletes the vault row only. Load all is MultiBox-only.
fn chooser_window(ui: &Ui, session: &mut Session) {
    if !session.wall.chooser_open {
        return;
    }
    let mut open = true;
    ui.window("Profiles")
        .opened(&mut open)
        .flags(WindowFlags::NO_COLLAPSE | WindowFlags::ALWAYS_AUTO_RESIZE)
        .size_constraints([DIALOG_W, 120.0], [DIALOG_W, 720.0])
        .build(|| {
            let _wrap = ui.push_text_wrap_pos(DIALOG_W - 16.0);
            let names: Vec<String> = session
                .vault
                .as_ref()
                .map(|v| v.profiles().map(|p| p.username.clone()).collect())
                .unwrap_or_default();
            let w = ui.content_region_avail()[0];
            let focused = session.focused_name();
            let members = session.wall.members.clone();
            let multibox = session.multibox;
            if multibox && ui.button_with_size("Load all", [w, 0.0]) {
                session.load_all();
            }
            if ui.button_with_size("New profile", [w, 0.0]) {
                session.begin_edit_profile(None);
            }
            ui.spacing();
            let mut picked: Option<String> = None;
            let mut removed: Option<String> = None;
            let mut edit: Option<String> = None;
            if names.is_empty() {
                ui.text_disabled("vault is empty — New profile then Save");
            } else {
                const ROW_H: f32 = 24.0;
                let vp_h = ui.main_viewport().size()[1];
                let max_list = (vp_h - 240.0).clamp(88.0, 480.0);
                let need = (names.len() as f32) * ROW_H + 8.0;
                let list_h = need.min(max_list);
                ui.child_window("##profiles-list")
                    .size([0.0, list_h])
                    .build(ui, || {
                        for name in &names {
                            let selected = if multibox {
                                members.iter().any(|m| m == name)
                            } else {
                                focused.as_deref() == Some(name.as_str())
                            };
                            let (p, r, e) = chooser_row(ui, name, selected);
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
                if session.multibox {
                    session.load(&name);
                } else {
                    session.select(&name);
                    session.wall.chooser_open = false;
                    session.cancel_edit_profile();
                }
            }
            if let Some(edit_name) = session.chooser_edit.clone() {
                ui.spacing();
                ui.separator();
                if edit_name.is_empty() {
                    ui.text_colored(ACCENT, "new profile");
                } else {
                    ui.text_colored(ACCENT, format!("edit {edit_name}"));
                }
                ui.text_disabled("user");
                ui.input_text("##cred-user", &mut session.cred_user)
                    .hint("username")
                    .build();
                ui.text_disabled("pass");
                ui.input_text("##cred-pass", &mut session.cred_pass)
                    .password(true)
                    .hint("password")
                    .build();
                let avail = ui.content_region_avail()[0];
                let (bw, stack) = button_row_layout(avail, 2);
                if ui.button_with_size("Save", [bw, 0.0]) && session.save_credentials() {
                    session.cancel_edit_profile();
                }
                if !stack {
                    ui.same_line();
                }
                if ui.button_with_size("Cancel", [bw, 0.0]) {
                    session.cancel_edit_profile();
                }
            }
            ui.spacing();
            let w = ui.content_region_avail()[0];
            if ui.button_with_size("Close", [w, 0.0]) {
                session.wall.chooser_open = false;
                session.cancel_edit_profile();
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
                    if session.chooser_edit.as_deref() == Some(name.as_str()) {
                        session.cancel_edit_profile();
                    }
                    session.vault_remove(&name);
                }
            } else if !ui.is_popup_open(PROFILE_DELETE_POPUP) {
                session.pending_profile_delete = None;
            }
        });
    if !open {
        session.cancel_edit_profile();
    }
    session.wall.chooser_open = session.wall.chooser_open && open;
}

fn settings_window(ui: &Ui, session: &mut Session) {
    if !session.global_settings_open {
        return;
    }
    let mut open = true;
    ui.window("General config")
        .opened(&mut open)
        .flags(WindowFlags::NO_COLLAPSE | WindowFlags::ALWAYS_AUTO_RESIZE)
        .size_constraints([DIALOG_W, 80.0], [DIALOG_W, 720.0])
        .build(|| {
            config_heading(ui, "slot");
            let slot = session.focused_name().unwrap_or_else(|| "—".into());
            ui.text_disabled(slot);
            slot_capture_section(ui, session);
            ui.spacing();
            config_heading(ui, "render");
            slot_render_section(ui, session);
            ui.spacing();
            config_heading(ui, "random");
            slot_random_section(ui, session);
            ui.spacing();
            config_heading(ui, "global");
            global_config_section(ui, session);
        });
    session.global_settings_open = open;
}

/// random: the guardian's per-profile toggles + lamp reward settings
/// (guardian spec `ProfileSettings`). Checkboxes and the skill combo write
/// straight through the vault upsert (`Session::set_random_settings`); the
/// lamp auto rubs through the guardian when enabled (`lamp_auto`).
fn slot_random_section(ui: &Ui, session: &mut Session) {
    let Some(name) = session.focused_name() else {
        ui.text_wrapped("focus a profile to edit its random-event and lamp settings");
        return;
    };
    let settings = session
        .vault
        .as_ref()
        .and_then(|v| v.get(&name))
        .map(|p| p.settings.clone())
        .unwrap_or_default();
    let mut events = settings.random_events;
    if ui.checkbox("random events", &mut events) {
        session.set_random_settings(&name, events, &settings.lamp_skill, settings.lamp_auto);
    }
    ui.text_wrapped("talk random events through on this profile. Off: never talk, never hold — still detects and shows the status row.");
    let mut auto = settings.lamp_auto;
    if ui.checkbox("lamp auto", &mut auto) {
        session.set_random_settings(&name, settings.random_events, &settings.lamp_skill, auto);
    }
    ui.text_wrapped("claim a lamp reward without a confirmation click (guardian rubs when lamp auto is on).");
    let mut skill = settings.lamp_skill.clone();
    if lamp_skill_combo(ui, &mut skill) {
        session.set_random_settings(&name, settings.random_events, &skill, settings.lamp_auto);
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
fn chooser_row(ui: &Ui, name: &str, selected: bool) -> (bool, bool, bool) {
    const EDIT_W: f32 = 44.0;
    const X_W: f32 = 28.0;
    let avail = ui.content_region_avail()[0];
    let name_w = (avail - EDIT_W - X_W - BUTTON_GAP * 2.0).max(10.0);
    let loaded = ui
        .selectable_config(name)
        .selected(selected)
        .close_popups(false)
        .size([name_w, 0.0])
        .build();
    gap_line(ui);
    let edit = ui.button_with_size(format!("Edit##edit-{name}"), [EDIT_W, 0.0]);
    gap_line(ui);
    let _red = ui.push_style_color(StyleColor::Text, ERROR);
    let removed = ui.button_with_size(format!("✕##{name}"), [X_W, 0.0]);
    (loaded, removed, edit)
}

/// Install the whole-window shot plumbing for a headed scenario run: the
/// per-run shot dir plus the sink bridging the slot-threaded runner to
/// the window readback — it enqueues `(label, snapshot JSON)`, and
/// `ui_frame` hands the requests to the render pass, then writes the PNG
/// and sidecar from the returned bytes. Shared by `--live script_*` and
/// `--smoke`.
fn arm_scenario_shots(state: &mut PanelState) {
    state.shot_dir = scenario::shot::create_run_dir()
        .map(Some)
        .unwrap_or_else(|e| {
            eprintln!("[panel] shot dir: {e}; shots will be skipped");
            None
        });
    let shots = Arc::clone(&state.shot_state);
    if let Some(runner) = state.session.scenario.lock().unwrap().as_mut() {
        runner.set_shot_sink(Box::new(
            move |label: &str, snap: &api::snapshot::GameSnapshot| {
                let json = serde_json::to_string_pretty(snap).unwrap_or_default();
                shots
                    .lock()
                    .unwrap()
                    .requests
                    .push((label.to_string(), json));
            },
        ));
    }
}

/// Open the 274bot panel window. Call after the vault has been started.
/// `mode` selects the normal interactive panel, a `--live NAME` harness,
/// or `--smoke` (temp `test` vault, one whole-window shot at scene 2,
/// exit 0).
pub fn run_panel(mode: RunMode) -> Result<(), window::PanelError> {
    let scale = Arc::new(AtomicU32::new(1.0f32.to_bits()));
    let frame_scale = Arc::clone(&scale);
    let mut state = PanelState::default();

    // Deferred boot: unlock / live-harness spawn slot threads, and a slot
    // renderer is built lazily at its first paint — spawning before GPU
    // init would let a slot construct its own wgpu device ahead of
    // `on_gpu_init`'s `inject_device`. The first UI frame presents an
    // empty panel (so the OS window actually appears); boot runs on the
    // next frame, after that present. Slot `maininit` (snapshot / maps)
    // must not sit on the first-present path.
    let mut boot = boot_for(&mode, std::env::var("BOT_VAULT_PASS").ok().as_deref());
    let mut presented = false;

    let cfg = runner_config();
    let os_window: Arc<Mutex<Option<Arc<winit::window::Window>>>> = Arc::new(Mutex::new(None));
    let os_window_init = Arc::clone(&os_window);
    window::run(
        cfg,
        amber_style,
        move |window, device, queue, _| {
            // The shared-device seam: seed the client's process-wide GPU
            // context with the panel's device before any slot renderer can
            // construct (slots spawn on the first frame, after this).
            client::render::backend::inject_device(device.clone(), queue.clone());
            // Observed only: HiDpiMode::Default already maps logical
            // pixels. Applying ScaleAllSizes here would double Retina.
            scale.store(
                integer_ui_scale(window.scale_factor() as f32).to_bits(),
                Ordering::Relaxed,
            );
            *os_window_init.lock().unwrap() = Some(Arc::clone(window));
        },
        Arc::clone(&state.shot_state),
        move |ui, gpu| {
            // Frame 0 presents chrome with no slots. Frame 1+ runs boot so
            // a blocking `maininit` cannot hide the window.
            if presented {
                if let Some(boot) = boot.take() {
                    if let Err(e) = boot_execute(&mut state, boot) {
                        eprintln!("FAIL: {e}");
                        std::process::exit(1);
                    }
                }
            }
            presented = true;
            if state.os_window.is_none() {
                state.os_window = os_window.lock().unwrap().clone();
            }
            if boot.is_some() {
                if let Some(w) = state.os_window.as_ref() {
                    w.request_redraw();
                }
            }
            let _scale = f32::from_bits(frame_scale.load(Ordering::Relaxed));
            ui_frame(ui, gpu, &mut state);
        },
    )
}

/// Whole-window shots (the 377 harness pattern): write completed captures
/// to the per-run dir, then hand the scenario sink's new requests to the
/// next render pass's readback. The per-run dir is created lazily on the
/// first write (the interactive F12 capture has no run start to hook).
/// Returns how many shots were written this frame — the `--smoke` watch
/// exits 0 once its single scene2 shot lands.
fn pump_shots(state: &mut PanelState) -> usize {
    let mut shots = state.shot_state.lock().unwrap();
    let mut written = 0;
    for cap in std::mem::take(&mut shots.done) {
        if state.shot_dir.is_none() {
            state.shot_dir = scenario::shot::create_run_dir()
                .map(Some)
                .unwrap_or_else(|e| {
                    eprintln!("[panel] shot dir: {e}; shots will be skipped");
                    None
                });
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
                    written += 1;
                }
                Err(e) => eprintln!("[panel] shot {}: {e}", cap.label),
            }
        }
    }
    let requests = std::mem::take(&mut shots.requests);
    // `--smoke` render-settle gate: hold the scene2 request until the
    // focused slot has held scene 2 for [`SMOKE_SETTLE`] and is still
    // ingame — the slot's 1 fps renderer needs wall-clock time to
    // rasterize the world, and an early readback captures the title
    // screen. `script_*` and interactive shots drain as before.
    let hold = match state.live.as_ref() {
        Some(LiveHarness::Smoke(s)) => !smoke_settled(
            s.saw_scene2_at,
            Instant::now(),
            focused_slot(&state.session, &state.session.statuses()).is_some_and(|slot| slot.ingame),
        ),
        _ => false,
    };
    if !hold {
        shots.wanted.extend(requests);
    } else {
        shots.requests = requests;
    }
    written
}

/// The per-frame UI body (the former dear-app `on_frame` closure): session
/// pump, live harness ticks, dock host, chrome, game pane, rail.
fn ui_frame(ui: &Ui, gpu: &mut Gpu, state: &mut PanelState) {
    let wrote_shots = pump_shots(state);
    state.session.pump_status();
    let statuses = state.session.statuses();
    if let Some(live) = state.live.as_mut() {
        let fail = match live {
            LiveHarness::Null(n) => live_null_tick(n, &statuses),
            LiveHarness::Stress(s) => live_stress_tick(s, &statuses),
            LiveHarness::Script(ls) => live_script_tick(ls, &mut state.session, wrote_shots),
            LiveHarness::Smoke(s) => live_smoke_tick(s, &mut state.session, &statuses, wrote_shots),
        };
        if let Some(msg) = fail {
            eprintln!("FAIL: {msg}");
            std::process::exit(1);
        }
        match live {
            LiveHarness::Smoke(s) if s.passed => {
                // `pump_shots` wrote the scene2 shot: the render gate is
                // green, leave with 0.
                std::process::exit(0);
            }
            LiveHarness::Script(s) if s.passed => {
                std::process::exit(0);
            }
            _ => {}
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
    let class = single_bot_window_class();
    ui.set_next_window_class(&class);
    panel_window(ui, &mut state.session);
    ui.set_next_window_class(&class);
    game_window(ui, gpu, state, &title);
    if state.session.multibox && !state.session.wall.grid {
        ui.set_next_window_class(&class);
        rail_window(ui, gpu, state);
    }
    // Every frame, not only while open: the prev latches must track
    // the close so the next open is a fresh rising edge.
    chooser_window(ui, &mut state.session);
    settings_window(ui, &mut state.session);
    browse_window(ui, &mut state.session);
    load_window(ui, &mut state.session);
    nav_settings_window(ui, &mut state.session);
    loadouts_window(ui, &mut state.session);
    params_edit_window(ui, &mut state.session);
    render_all_warn_window(ui, &mut state.session);
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant, SystemTime};

    use dear_imgui_rs::{ConfigFlags, Key, WindowFlags};

    use super::{
        apply_only_render_selected, apply_ui_scale, boot_for, capture_key_ch,
        chooser_should_open_popup, clamp_hop_label_px, debug_caption, edit_parameters_enabled,
        game_window_flags, live_null_tick, live_script_tick, live_smoke_tick, live_stress_tick,
        manual_shot_label, parse_live_args, random_status_text, runner_config, smoke_settled,
        smoke_should_fire, Boot, LiveBoot, LiveNull, LiveScript, LiveSmoke, LiveStress, RunMode,
        BASE_WINDOW_H, BASE_WINDOW_W, LIVE_USAGE, SMOKE_DEADLINE, SMOKE_SETTLE,
    };
    use crate::theme::{
        applet_offset, fit_applet, game_window_title, native_applet, panel_split_ratio, PANEL_WIDTH,
    };
    use crate::window::RedrawMode;

    #[test]
    fn boot_is_deferred_and_maps_live_smoke_and_vault_pass() {
        // Every slot-spawning path is deferred (returns a Boot to run on
        // the first frame after GPU init), never executed eagerly: mapping
        // a mode to a Boot must not unlock or call live_prepare — those
        // run only in `boot_execute`, after the first frame presents and
        // `on_gpu_init` injected the panel's device.
        assert!(matches!(
            boot_for(&RunMode::Live("null_raster".into()), None),
            Some(Boot::Live(LiveBoot::NullRaster))
        ));
        assert!(matches!(
            boot_for(&RunMode::Live("stress50".into()), None),
            Some(Boot::Live(LiveBoot::Stress50))
        ));
        assert!(matches!(
            boot_for(&RunMode::Live("stress50_full".into()), None),
            Some(Boot::Live(LiveBoot::Stress50Full))
        ));
        match boot_for(&RunMode::Live("script_walk".into()), Some("pass")) {
            Some(Boot::Live(LiveBoot::Script { name })) => assert_eq!(name, "script_walk"),
            other => panic!("script_<name> must map to a deferred Script boot, got {other:?}"),
        }
        match boot_for(&RunMode::Live("nav_full".into()), None) {
            Some(Boot::Live(LiveBoot::Script { name })) => assert_eq!(name, "nav_full"),
            other => panic!("nav_full must map to a deferred Script boot, got {other:?}"),
        }
        match boot_for(&RunMode::Interactive, Some("hunter2")) {
            Some(Boot::Unlock { pass }) => assert_eq!(pass, "hunter2"),
            other => panic!("BOT_VAULT_PASS must map to a deferred Unlock boot, got {other:?}"),
        }
        assert!(matches!(
            boot_for(&RunMode::Smoke, None),
            Some(Boot::Live(LiveBoot::Smoke))
        ));
        assert!(
            boot_for(&RunMode::Interactive, None).is_none(),
            "no boot with no live arg and no pass"
        );
    }

    #[test]
    fn chooser_should_open_popup_table() {
        // First open: rising edge opens the popup and latches prev.
        assert_eq!(chooser_should_open_popup(true, false), (true, true));
        // Already open: no re-open while want stays true.
        assert_eq!(chooser_should_open_popup(true, true), (false, true));
        // Esc closed it: want drops to false and prev must fall so a later
        // `+ add bot` is a fresh rising edge.
        assert_eq!(chooser_should_open_popup(false, true), (false, false));
        assert_eq!(chooser_should_open_popup(false, false), (false, false));
    }

    #[test]
    fn chooser_reopens_after_a_close() {
        let mut prev = false;
        let (open, np) = chooser_should_open_popup(true, prev);
        assert!(open, "first + add opens the chooser");
        prev = np;
        let (open, np) = chooser_should_open_popup(true, prev);
        assert!(!open, "already open: no reopen");
        prev = np;
        let (open, np) = chooser_should_open_popup(false, prev);
        assert!(!open);
        prev = np;
        assert!(!prev, "prev must track the close so + add can reopen");
        let (open, _np) = chooser_should_open_popup(true, prev);
        assert!(open, "the next + add bot reopens the chooser");
    }

    #[test]
    fn edit_parameters_enabled_for_operator_bag() {
        assert!(edit_parameters_enabled(), "parameter editors ship in 0.1.6");
    }

    #[test]
    fn loadout_combo_lists_store_names() {
        use script::{Loadout, LoadoutsStore, SettingDef, resolve_setting_options};

        let dir = std::env::temp_dir().join(format!(
            "274bot-panel-loadout-combo-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut store = LoadoutsStore::at(dir.join("loadouts.json"));
        store.upsert(Loadout {
            name: "guard".into(),
            worn: vec![],
            carry: vec![],
        });
        store.upsert(Loadout {
            name: "stall".into(),
            worn: vec![],
            carry: vec![],
        });
        let def = SettingDef {
            id: "loadout".into(),
            ty: "string".into(),
            default: None,
            label: None,
            min: None,
            max: None,
            step: None,
            options: Vec::new(),
            option_labels: Vec::new(),
            group: None,
            show_if: None,
            options_from: Some("loadouts".into()),
            csv_toggle: None,
            help: None,
        };
        assert_eq!(
            resolve_setting_options(&def, &store),
            vec!["guard".to_string(), "stall".to_string()]
        );
    }

    #[test]
    fn debug_captions_fit_the_strip() {
        assert_eq!(debug_caption("DebugPanel"), "Panel");
        assert_eq!(debug_caption("Lumbridge"), "Lumb");
        assert_eq!(debug_caption("maxme"), "maxme");
        assert_eq!(debug_caption("Teles"), "Teles");
        assert_eq!(debug_caption("TutSkip"), "TutSkip");
    }

    #[test]
    fn hop_label_px_writes_clamp_to_8_28() {
        // The settings UI is the only writer of `hop_label_px`; it must
        // hold the NavSettings 8..=28 field invariant.
        assert_eq!(clamp_hop_label_px(0), 8);
        assert_eq!(clamp_hop_label_px(8), 8);
        assert_eq!(clamp_hop_label_px(11), 11);
        assert_eq!(clamp_hop_label_px(28), 28);
        assert_eq!(clamp_hop_label_px(100), 28);
    }

    #[test]
    fn apply_only_render_selected_warns_before_unchecking() {
        // Checking "only render selected" on is immediate, no dialog.
        assert_eq!(apply_only_render_selected(false, true), (true, false));
        // Unchecking does not apply: keeps the safe default, opens the
        // warning instead.
        assert_eq!(apply_only_render_selected(true, false), (true, true));
        // No-op rows: the box already matches the flag.
        assert_eq!(apply_only_render_selected(true, true), (true, false));
        assert_eq!(apply_only_render_selected(false, false), (false, false));
    }

    #[test]
    fn runner_config_docks_without_viewports() {
        let c = runner_config();
        assert!(c.docking.enable);
        assert!(
            !c.docking.auto_dockspace,
            "we own the game-left / panel-right split"
        );
        assert!(
            c.docking
                .dockspace_flags
                .contains(dear_imgui_rs::DockNodeFlags::AUTO_HIDE_TAB_BAR),
            "single-bot hides the game/panel tab strip"
        );
        assert!(
            c.docking
                .dockspace_flags
                .contains(dear_imgui_rs::DockNodeFlags::NO_RESIZE),
            "dock splitters stay off — only grid uses OS-window resize"
        );
        assert!(c.ini_filename.is_none(), "no imgui.ini to restore");
        let flags = c.io_config_flags.expect("flags");
        assert!(flags.contains(ConfigFlags::DOCKING_ENABLE));
        assert!(!flags.contains(ConfigFlags::VIEWPORTS_ENABLE));
        assert!(matches!(c.redraw, RedrawMode::WaitUntil { fps } if (fps - 50.0).abs() < 0.01));
        assert_eq!(c.window_size, (BASE_WINDOW_W as f64, BASE_WINDOW_H as f64));
    }

    #[test]
    fn single_bot_window_class_hides_tab_bar() {
        let c = super::single_bot_window_class();
        assert!(c
            .dock_node_flags_override_set
            .contains(dear_imgui_rs::DockNodeFlags::AUTO_HIDE_TAB_BAR));
        assert!(c
            .dock_node_flags_override_set
            .contains(dear_imgui_rs::DockNodeFlags::NO_RESIZE));
        assert!(!c.docking_always_tab_bar);
    }

    #[test]
    fn apply_ui_scale_scales_padding_for_retina() {
        let _guard = crate::IMGUI_CTX_TEST_GUARD.lock().unwrap();
        let mut ctx = dear_imgui_rs::Context::create();
        let before = ctx.style().window_padding();
        apply_ui_scale(ctx.style_mut(), 2.0);
        let after = ctx.style().window_padding();
        assert_eq!(before, [8.0, 8.0]);
        assert_eq!(after, [16.0, 16.0]);
    }

    #[test]
    fn fit_applet_keeps_aspect_and_does_not_dpi_double() {
        assert_eq!(native_applet(), [765.0, 503.0]);
        assert_eq!(fit_applet([765.0, 503.0]), [765.0, 503.0]);
        let off = applet_offset([1200.0, 700.0], [765.0, 503.0]);
        assert!(
            (off[0] - (1200.0 - 765.0)).abs() < 0.01,
            "flush to the panel"
        );
        assert!((off[1] - (700.0 - 503.0) * 0.5).abs() < 0.01);
        // Grid cells downscale; the non-grid Game blit stays native_applet.
        assert_eq!(fit_applet([382.5, 251.5]), [382.5, 251.5]);
        let wide = fit_applet([2000.0, 503.0]);
        assert!((wide[1] - 503.0).abs() < 0.01);
        assert!(wide[0] <= 2000.0);
    }

    #[test]
    fn game_window_flags_have_no_scrollbar() {
        let f = game_window_flags();
        assert!(f.contains(WindowFlags::NO_SCROLLBAR));
        assert!(f.contains(WindowFlags::NO_SCROLL_WITH_MOUSE));
        assert!(f.contains(WindowFlags::NO_RESIZE));
        assert!(!f.contains(WindowFlags::HORIZONTAL_SCROLLBAR));
    }

    #[test]
    fn capture_keys_pass_colon_and_tilde_like_client_play() {
        // client-play KeyCodes.ts: `:` ch 58, `~` ch 126. Panel capture
        // used to map only letters+digits, so `::` / `~` never reached chat.
        assert_eq!(capture_key_ch(Key::Semicolon, true), Some(b':' as i32));
        assert_eq!(capture_key_ch(Key::Semicolon, false), Some(b';' as i32));
        assert_eq!(capture_key_ch(Key::GraveAccent, true), Some(b'~' as i32));
        assert_eq!(capture_key_ch(Key::GraveAccent, false), Some(b'`' as i32));
        assert_eq!(capture_key_ch(Key::Comma, false), Some(b',' as i32));
        assert_eq!(capture_key_ch(Key::Minus, true), Some(b'_' as i32));
    }

    #[test]
    fn game_window_title_is_the_profile_name() {
        assert_eq!(game_window_title(Some("test")), "test");
        assert_eq!(game_window_title(None), "Game");
        assert_eq!(game_window_title(Some("")), "Game");
    }

    #[test]
    fn panel_split_is_a_thin_right_slice() {
        let r = panel_split_ratio(1120.0);
        assert!((r - PANEL_WIDTH / 1120.0).abs() < 0.001);
        let wide = panel_split_ratio(2000.0);
        assert!(
            (wide * 2000.0 - PANEL_WIDTH).abs() < 0.01,
            "panel stays 330px on a wide host window, got {}",
            wide * 2000.0
        );
    }

    #[test]
    fn parse_live_args_none_without_flag_or_env() {
        assert_eq!(
            parse_live_args([] as [&str; 0], None),
            Ok(RunMode::Interactive)
        );
        assert_eq!(
            parse_live_args([] as [&str; 0], Some("")),
            Ok(RunMode::Interactive)
        );
    }

    #[test]
    fn parse_live_args_env_and_flag_null_raster() {
        assert_eq!(
            parse_live_args([] as [&str; 0], Some("null_raster")),
            Ok(RunMode::Live("null_raster".into()))
        );
        assert_eq!(
            parse_live_args(["--live", "null_raster"], None),
            Ok(RunMode::Live("null_raster".into()))
        );
    }

    #[test]
    fn parse_live_args_env_and_flag_stress50() {
        assert_eq!(
            parse_live_args([] as [&str; 0], Some("stress50")),
            Ok(RunMode::Live("stress50".into()))
        );
        assert_eq!(
            parse_live_args(["--live", "stress50"], None),
            Ok(RunMode::Live("stress50".into()))
        );
        assert_eq!(
            parse_live_args(["--live", "stress50_full"], None),
            Ok(RunMode::Live("stress50_full".into()))
        );
        assert_eq!(
            parse_live_args([] as [&str; 0], Some("stress50_full")),
            Ok(RunMode::Live("stress50_full".into()))
        );
    }

    #[test]
    fn parse_live_args_accepts_nav_full() {
        assert_eq!(
            parse_live_args(["--live", "nav_full"], None),
            Ok(RunMode::Live("nav_full".into()))
        );
        assert_eq!(
            parse_live_args([] as [&str; 0], Some("nav_full")),
            Ok(RunMode::Live("nav_full".into()))
        );
    }

    #[test]
    fn parse_live_args_flag_wins_over_env() {
        assert_eq!(
            parse_live_args(["--live", "null_raster"], Some("other")),
            Ok(RunMode::Live("null_raster".into()))
        );
    }

    #[test]
    fn parse_live_args_smoke_flag_maps_to_smoke_mode() {
        assert_eq!(parse_live_args(["--smoke"], None), Ok(RunMode::Smoke));
        // A stray BOT_LIVE env does not demote --smoke.
        assert_eq!(
            parse_live_args(["--smoke"], Some("stress50")),
            Ok(RunMode::Smoke)
        );
        // --smoke wins over --live when both are passed.
        assert_eq!(
            parse_live_args(["--smoke", "--live", "null_raster"], None),
            Ok(RunMode::Smoke)
        );
    }

    #[test]
    fn parse_live_args_unknown_name_is_usage_exit_2() {
        assert_eq!(
            parse_live_args(["--live", "nope"], None),
            Err((2, LIVE_USAGE.into()))
        );
        assert_eq!(
            parse_live_args([] as [&str; 0], Some("other")),
            Err((2, LIVE_USAGE.into()))
        );
    }

    #[test]
    fn parse_live_args_unknown_flag_and_missing_name() {
        assert_eq!(
            parse_live_args(["--wat"], None),
            Err((2, "panel-play: unknown --wat".into()))
        );
        assert_eq!(
            parse_live_args(["--live"], None),
            Err((2, "panel-play: --live needs a name".into()))
        );
    }

    #[test]
    fn parse_live_args_help_is_usage_exit_0() {
        assert_eq!(
            parse_live_args(["--help"], None),
            Err((0, LIVE_USAGE.into()))
        );
        assert_eq!(parse_live_args(["-h"], None), Err((0, LIVE_USAGE.into())));
    }

    #[test]
    fn parse_live_args_script_walk_accepted() {
        assert_eq!(
            parse_live_args(["--live", "script_walk"], None),
            Ok(RunMode::Live("script_walk".into()))
        );
        assert_eq!(
            parse_live_args([] as [&str; 0], Some("script_walk")),
            Ok(RunMode::Live("script_walk".into()))
        );
        // The smoke scenario is a registered scenario, so the script_
        // harness can also drive it manually.
        assert_eq!(
            parse_live_args(["--live", "script_render_smoke"], None),
            Ok(RunMode::Live("script_render_smoke".into()))
        );
        assert_eq!(
            parse_live_args(["--live", "script_nav_routes"], None),
            Ok(RunMode::Live("script_nav_routes".into()))
        );
        // The courtyard paint-path scenario is a registered scenario, so
        // the script_ harness drives it like script_nav_routes.
        assert_eq!(
            parse_live_args(["--live", "script_nav_paint_path"], None),
            Ok(RunMode::Live("script_nav_paint_path".into()))
        );
    }

    #[test]
    fn parse_live_args_unknown_script_rejected() {
        assert_eq!(
            parse_live_args(["--live", "script_nope"], None),
            Err((2, LIVE_USAGE.into()))
        );
        assert_eq!(
            parse_live_args(["--live", "script_"], None),
            Err((2, LIVE_USAGE.into()))
        );
    }

    #[test]
    fn smoke_should_fire_table() {
        // Fires exactly once, only while armed, only at `ingame && scene 2`.
        let cases: &[(&str, bool, bool, bool, i32, bool)] = &[
            ("disarmed never fires", false, false, true, 2, false),
            ("fires at scene 2", true, false, true, 2, true),
            ("not before scene 2", true, false, true, 1, false),
            ("not while logged out", true, false, false, 2, false),
            ("not after scene 1", true, false, true, 0, false),
            ("not twice", true, true, true, 2, false),
            ("not after exit", true, true, true, 1, false),
        ];
        for (name, armed, fired, ingame, scene, expect) in cases {
            assert_eq!(
                smoke_should_fire(*armed, *fired, *ingame, *scene),
                *expect,
                "{name}"
            );
        }
    }

    #[test]
    fn smoke_settled_table() {
        // The scene2 shot request reaches the render readback only once
        // the focused slot has held scene 2 for the full settle window and
        // is still ingame (the 1 fps renderer needs wall-clock time to
        // rasterize the world; an early capture is the title screen).
        let t0 = Instant::now();
        let cases: &[(&str, Option<Instant>, Instant, bool, bool)] = &[
            ("no scene 2 yet", None, t0, true, false),
            (
                "before the settle window",
                Some(t0),
                t0 + Duration::from_secs(1),
                true,
                false,
            ),
            (
                "at the settle boundary",
                Some(t0),
                t0 + SMOKE_SETTLE,
                true,
                true,
            ),
            (
                "after the settle window",
                Some(t0),
                t0 + Duration::from_secs(5),
                true,
                true,
            ),
            (
                "settled but logged out",
                Some(t0),
                t0 + Duration::from_secs(5),
                false,
                false,
            ),
        ];
        for (name, at, now, ingame, expect) in cases {
            assert_eq!(smoke_settled(*at, *now, *ingame), *expect, "{name}");
        }
    }

    #[test]
    fn manual_shot_label_is_stamped_and_stays_normalized() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_787_616_000);
        let label = manual_shot_label(now);
        assert_eq!(label, "manual-2026-08-25T00-00-00");
        // The label is already in the 377 safe alphabet: the file name
        // normalizes to itself (the write path applies `safe_label`).
        assert_eq!(scenario::shot::safe_label(&label), label);
    }

    /// A synthetic client that has already seeded: ingame, scene 2, a
    /// mainland build base, and bumped family gens (same trick as the
    /// scenario crate's own tests — no live server).
    fn script_client() -> client::client::Client {
        let mut c = host::prepare_client(
            client::client::ClientConfig {
                host: "127.0.0.1".into(),
                port: 43594,
                cache_dir: "/tmp".into(),
                members: true,
                lowmem: true,
            },
            1,
            std::sync::Arc::new(client::config::Cache::default()),
            std::sync::Arc::new(vec![]),
            Vec::new(),
        );
        c.ingame = true;
        c.scene_state = 2;
        c.map_build_base_x = 3200;
        c.map_build_base_z = 3200;
        c.local_player = Some(client::dash3d::ClientPlayer::at(20, 20));
        for prot in [
            client::io::ServerProt::PLAYER_INFO,
            client::io::ServerProt::REBUILD_NORMAL,
            client::io::ServerProt::UPDATE_STAT,
        ] {
            c.bump_gens(prot);
        }
        c
    }

    /// Headed contract: PASS latches `passed` (the caller exits 0),
    /// FAIL returns the message the caller turns into exit 1.
    #[test]
    fn live_script_tick_latches_pass_and_reports_fail() {
        use scenario::{
            Proof, RunnerStatus, Scenario, ScenarioRunner, ScenarioSettings, Seed, Step, StepKind,
            Wait,
        };

        let mut s = crate::session::Session::new();
        // A runnable micro-scenario: the send sets run energy, the arm
        // waits for it, the proof asserts it.
        let pass = Scenario {
            name: "t",
            seed: Seed {
                profiles: vec![("test", "test")],
                mainland: false,
            },
            steps: vec![Step {
                name: "energy",
                kind: StepKind::Perform {
                    send: Box::new(|c, _| {
                        c.runenergy = 5;
                        true
                    }),
                },
                wait: Wait {
                    arm: Proof::Stat { id: 16, min: 5 },
                    budget_ticks: 5,
                },
            }],
            proof: Proof::Stat { id: 16, min: 5 },
            companions: vec![],
            settings: ScenarioSettings::default(),
        };
        let mut runner = ScenarioRunner::new(pass);
        runner.set_scene_settle(Duration::ZERO);
        {
            let mut c = script_client();
            runner.tick(&mut c);
            c.bump_gens(client::io::ServerProt::UPDATE_RUNENERGY);
            runner.tick(&mut c);
        }
        assert_eq!(runner.status(), RunnerStatus::Passed);
        *s.scenario.lock().unwrap() = Some(runner);
        let mut live = LiveScript {
            name: "script_t".into(),
            passed: false,
            failed: None,
            last_step: None,
            drain_started: None,
        };
        assert_eq!(
            live_script_tick(&mut live, &mut s, 0),
            None,
            "PASS latches; the caller exits 0"
        );
        assert!(live.passed);

        // FAIL: a never-satisfiable arm within a 1-tick budget.
        let fail_scenario = Scenario {
            name: "f",
            seed: Seed {
                profiles: vec![("test", "test")],
                mainland: false,
            },
            steps: vec![Step {
                name: "never",
                kind: StepKind::Perform {
                    send: Box::new(|_, _| true),
                },
                wait: Wait {
                    arm: Proof::Stat { id: 16, min: 999 },
                    budget_ticks: 1,
                },
            }],
            proof: Proof::Stat { id: 16, min: 999 },
            companions: vec![],
            settings: ScenarioSettings::default(),
        };
        let mut runner = ScenarioRunner::new(fail_scenario);
        runner.set_scene_settle(Duration::ZERO);
        {
            let mut c = script_client();
            runner.tick(&mut c);
        }
        assert!(matches!(runner.status(), RunnerStatus::Failed(_)));
        *s.scenario.lock().unwrap() = Some(runner);
        let mut live = LiveScript {
            name: "script_f".into(),
            passed: false,
            failed: None,
            last_step: None,
            drain_started: None,
        };
        // No terminal shot armed: the FAIL returns immediately.
        let msg = live_script_tick(&mut live, &mut s, 0).expect("FAIL returns the message");
        assert!(msg.contains("not seen within 1 ticks"), "msg: {msg}");
        assert!(live.failed.is_some());
    }

    /// A terminal-shot FAIL holds the exit until the shot writes (or the
    /// drain lapses): the first frame returns `None` (the request landed
    /// after `pump_shots` ran), the write frame returns the message.
    #[test]
    fn live_script_tick_holds_a_terminal_shot_fail_until_the_shot_writes() {
        use scenario::{
            Proof, RunnerStatus, Scenario, ScenarioRunner, ScenarioSettings, Seed, Step, StepKind,
            Wait,
        };
        let mut s = crate::session::Session::new();
        let fail_scenario = Scenario {
            name: "f",
            seed: Seed {
                profiles: vec![("test", "test")],
                mainland: false,
            },
            steps: vec![Step {
                name: "never",
                kind: StepKind::Perform {
                    send: Box::new(|_, _| true),
                },
                wait: Wait {
                    arm: Proof::Stat { id: 16, min: 999 },
                    budget_ticks: 1,
                },
            }],
            proof: Proof::Stat { id: 16, min: 999 },
            companions: vec![],
            settings: ScenarioSettings::default(),
        };
        let mut runner = ScenarioRunner::new(fail_scenario);
        runner.set_scene_settle(Duration::ZERO);
        runner.set_terminal_shot("t-fail");
        {
            let mut c = script_client();
            runner.tick(&mut c);
        }
        assert!(matches!(runner.status(), RunnerStatus::Failed(_)));
        *s.scenario.lock().unwrap() = Some(runner);
        let mut live = LiveScript {
            name: "nav_full".into(),
            passed: false,
            failed: None,
            last_step: None,
            drain_started: None,
        };
        assert_eq!(
            live_script_tick(&mut live, &mut s, 0),
            None,
            "the first FAIL frame holds so the shot can land"
        );
        assert!(live.drain_started.is_some());
        // The shot writes on a later frame: the FAIL is returned.
        let msg = live_script_tick(&mut live, &mut s, 1).expect("FAIL after the shot writes");
        assert!(live.failed.is_some());
        assert!(msg.contains("not seen within 1 ticks"), "msg: {msg}");
    }

    /// A terminal-shot PASS holds the exit until the shot writes, then
    /// latches `passed` so the caller exits 0.
    #[test]
    fn live_script_tick_holds_a_terminal_shot_pass_until_the_shot_writes() {
        use scenario::{
            Proof, RunnerStatus, Scenario, ScenarioRunner, ScenarioSettings, Seed, Step, StepKind,
            Wait,
        };
        let mut s = crate::session::Session::new();
        let pass = Scenario {
            name: "t",
            seed: Seed {
                profiles: vec![("test", "test")],
                mainland: false,
            },
            steps: vec![Step {
                name: "energy",
                kind: StepKind::Perform {
                    send: Box::new(|c, _| {
                        c.runenergy = 5;
                        true
                    }),
                },
                wait: Wait {
                    arm: Proof::Stat { id: 16, min: 5 },
                    budget_ticks: 5,
                },
            }],
            proof: Proof::Stat { id: 16, min: 5 },
            companions: vec![],
            settings: ScenarioSettings::default(),
        };
        let mut runner = ScenarioRunner::new(pass);
        runner.set_scene_settle(Duration::ZERO);
        runner.set_terminal_shot("t-pass");
        {
            let mut c = script_client();
            runner.tick(&mut c);
            c.bump_gens(client::io::ServerProt::UPDATE_RUNENERGY);
            runner.tick(&mut c);
        }
        assert_eq!(runner.status(), RunnerStatus::Passed);
        *s.scenario.lock().unwrap() = Some(runner);
        let mut live = LiveScript {
            name: "script_t".into(),
            passed: false,
            failed: None,
            last_step: None,
            drain_started: None,
        };
        assert_eq!(
            live_script_tick(&mut live, &mut s, 0),
            None,
            "the first PASS frame holds so the shot can land"
        );
        assert!(!live.passed);
        assert!(live.drain_started.is_some());
        assert_eq!(live_script_tick(&mut live, &mut s, 1), None);
        assert!(live.passed, "PASS after the shot writes; caller exits 0");
    }

    fn smoke_at(started: Instant) -> LiveSmoke {
        LiveSmoke {
            started,
            last_step: None,
            failed: None,
            saw_scene2_at: None,
            passed: false,
        }
    }

    #[test]
    fn live_smoke_tick_passes_when_the_shot_is_written() {
        let mut s = crate::session::Session::new();
        let mut live = smoke_at(Instant::now());
        let statuses = [st("test", true, 2)];
        // A written shot (pump_shots drained `done`) exits the smoke: the
        // tick latches passed and the caller turns it into exit 0.
        assert_eq!(live_smoke_tick(&mut live, &mut s, &statuses, 1), None);
        assert!(live.passed, "the written scene2 shot passes the smoke");
    }

    #[test]
    fn live_smoke_tick_latches_scene2_once_and_reports_a_missing_write() {
        let mut s = crate::session::Session::new();
        s.focus.lock().unwrap().focused = Some("test".into());
        let mut live = smoke_at(Instant::now());
        // Before scene 2 nothing latches.
        assert_eq!(
            live_smoke_tick(&mut live, &mut s, &[st("test", true, 1)], 0),
            None
        );
        assert!(!live.saw_scene2_at.is_some());
        // Scene 2 on the focused slot latches exactly once (the pure
        // trigger), and a late deadline names the missing write.
        let scene2 = [st("test", true, 2)];
        assert_eq!(live_smoke_tick(&mut live, &mut s, &scene2, 0), None);
        assert!(live.saw_scene2_at.is_some());
        live.started = Instant::now() - SMOKE_DEADLINE;
        let err = live_smoke_tick(&mut live, &mut s, &scene2, 0).expect("deadline");
        assert!(err.contains("never written within 300s"), "err: {err}");
    }

    #[test]
    fn live_smoke_tick_deadline_reports_scene2_never_reached() {
        let mut s = crate::session::Session::new();
        s.focus.lock().unwrap().focused = Some("test".into());
        let mut live = smoke_at(Instant::now() - SMOKE_DEADLINE);
        let err = live_smoke_tick(&mut live, &mut s, &[st("test", false, 0)], 0).expect("deadline");
        assert!(
            err.contains("never reached scene 2 within 300s"),
            "err: {err}"
        );
        assert!(live.failed.is_some(), "the deadline latches the failure");
    }

    fn st(name: &str, ingame: bool, scene: i32) -> host_play::SlotStatus {
        host_play::SlotStatus {
            username: name.into(),
            ingame,
            scene_state: scene,
            ..Default::default()
        }
    }

    fn live_at(started: Instant) -> LiveNull {
        LiveNull {
            started,
            saw_scene2: false,
            passed: false,
        }
    }

    #[test]
    fn live_null_tick_waits_until_two_scene2() {
        let mut live = live_at(Instant::now());
        let statuses = [st("test", true, 1), st("test2", false, 0)];
        assert_eq!(live_null_tick(&mut live, &statuses), None);
        assert!(!live.saw_scene2);
    }

    #[test]
    fn live_null_tick_timeout_before_scene2() {
        let mut live = live_at(Instant::now() - Duration::from_secs(120));
        let statuses = [st("test", true, 2)];
        let err = live_null_tick(&mut live, &statuses).expect("timeout");
        assert!(err.contains("1/2"), "{err}");
        assert!(err.contains("120s"), "{err}");
    }

    #[test]
    fn live_null_tick_passes_at_scene2_without_freeze() {
        let mut live = live_at(Instant::now());
        let scene2 = [st("test", true, 2), st("test2", true, 2)];
        assert_eq!(live_null_tick(&mut live, &scene2), None);
        assert!(live.passed);
        assert!(live.saw_scene2);
        assert_eq!(live_null_tick(&mut live, &scene2), None, "stay passed");
    }

    fn stress_at(started: Instant) -> LiveStress {
        LiveStress {
            started,
            last_announced: 0,
            passed: false,
            name: "stress50",
            host: "127.0.0.1".into(),
            port: 43594,
        }
    }

    fn ready_n(n: usize) -> Vec<host_play::SlotStatus> {
        (0..n).map(|i| st(&format!("s{i:02}"), true, 2)).collect()
    }

    #[test]
    fn live_stress_tick_announces_1_10_50_and_stays_passed() {
        let mut live = stress_at(Instant::now());
        assert_eq!(live_stress_tick(&mut live, &[]), None);
        assert_eq!(live.last_announced, 0);
        assert!(!live.passed);

        assert_eq!(live_stress_tick(&mut live, &ready_n(1)), None);
        assert_eq!(live.last_announced, 1);
        assert!(!live.passed);

        assert_eq!(live_stress_tick(&mut live, &ready_n(10)), None);
        assert_eq!(live.last_announced, 10);
        assert!(!live.passed);

        assert_eq!(live_stress_tick(&mut live, &ready_n(50)), None);
        assert_eq!(live.last_announced, 50);
        assert!(live.passed);
        assert_eq!(
            live_stress_tick(&mut live, &ready_n(50)),
            None,
            "stay passed"
        );
        assert!(live.passed);
    }

    #[test]
    fn live_stress_tick_timeout_before_50() {
        let mut live = stress_at(Instant::now() - Duration::from_secs(600));
        let err = live_stress_tick(&mut live, &ready_n(1)).expect("timeout");
        assert_eq!(err, "live stress50: 1/50 up after 600s");
        assert!(!live.passed);
        assert_eq!(live.last_announced, 1);
    }

    #[test]
    fn live_stress_tick_full_name_in_timeout() {
        let mut live = LiveStress {
            started: Instant::now() - Duration::from_secs(600),
            last_announced: 0,
            passed: false,
            name: "stress50_full",
            host: "127.0.0.1".into(),
            port: 43594,
        };
        let err = live_stress_tick(&mut live, &ready_n(1)).expect("timeout");
        assert_eq!(err, "live stress50_full: 1/50 up after 600s");
    }

    #[test]
    fn live_stress_tick_counts_full_clients_up() {
        let mut live = stress_at(Instant::now());
        // Every member is a full Client: "up" requires scene 2, so loading
        // slots do not count toward the 50.
        let mut rows = vec![st("s00", true, 2)];
        for i in 1..50 {
            rows.push(st(&format!("s{i:02}"), true, 1));
        }
        assert_eq!(live_stress_tick(&mut live, &rows), None);
        assert!(!live.passed, "49 loading Clients are not 50 up");
        assert_eq!(live.last_announced, 1);
        for r in rows.iter_mut() {
            r.scene_state = 2;
        }
        assert_eq!(live_stress_tick(&mut live, &rows), None);
        assert!(live.passed, "50 scene-2 Clients pass");
        assert_eq!(live.last_announced, 50);
    }

    #[test]
    fn random_status_text_names_kind_hold_and_off() {
        let r = host::RandomStatus {
            kind: Some(api::RandomKind::Dialog),
            name: Some("mysterious old man".into()),
            toggle: true,
            hold: true,
            ..Default::default()
        };
        assert_eq!(
            random_status_text(&r).as_deref(),
            Some("dialog: mysterious old man (hold)")
        );
        let r = host::RandomStatus {
            kind: Some(api::RandomKind::Dialog),
            name: Some("mysterious old man".into()),
            toggle: false,
            ..Default::default()
        };
        assert_eq!(
            random_status_text(&r).as_deref(),
            Some("dialog: mysterious old man (off)")
        );
        let r = host::RandomStatus {
            kind: Some(api::RandomKind::Lamp),
            name: Some("genie".into()),
            toggle: true,
            ..Default::default()
        };
        assert_eq!(random_status_text(&r).as_deref(), Some("lamp: genie"));
        let r = host::RandomStatus::default();
        assert_eq!(random_status_text(&r), None, "no event, no row");
    }

    #[test]
    fn random_status_text_kebab_cases_lost_kinds() {
        let r = host::RandomStatus {
            kind: Some(api::RandomKind::LostTool),
            toggle: true,
            ..Default::default()
        };
        assert_eq!(random_status_text(&r).as_deref(), Some("lost-tool: ?"));
        let r = host::RandomStatus {
            kind: Some(api::RandomKind::LostGear),
            toggle: true,
            ..Default::default()
        };
        assert_eq!(random_status_text(&r).as_deref(), Some("lost-gear: ?"));
    }
}
