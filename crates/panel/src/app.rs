//! dear-app shell: docking enabled, multi-viewport disabled, amber chrome.

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dear_app::{AddOns, RedrawMode, Theme};
use dear_imgui_rs::internal::RawWrapper;
use dear_imgui_rs::{
    Condition, DockBuilder, DockNodeFlags, Id, Key, MouseButton, SplitDirection, TreeNodeFlags, Ui,
    WindowClass, WindowFlags,
};

use crate::chrome::{
    button_row_layout, multibox_tooltip, BUTTON_GAP, PARAM_ROW, SCRIPT_ROW,
};
use crate::focus::{draw_for_slot, should_capture, should_draw};
use crate::game_view::{frame_pixels, GameView};
use crate::grid::grid_cells;
use crate::overlay::{draw_focused_queue_card, PathOverlay};
use crate::picker;
use crate::queue_card::queue_k_of_n;
use crate::rail::{
    os_window_size, traffic_light, Light, BASE_WINDOW_H, BASE_WINDOW_W, RAIL_W, TILE_H, TILE_W,
};
use host::debug_enabled;

use crate::resource::{
    cpu_from_delta, format_bots, format_rss_caption, sample_process, traffic_from_samples, Metric,
};
use crate::session::{
    combo_index, script_active, script_pause_enabled, script_status_text, script_stop_enabled,
    stream_capture, Session, PROCESS,
};
use crate::theme::{
    apply_amber, fit_applet, game_window_title, integer_ui_scale, panel_split_ratio, ACCENT, BG,
    BUILD_LINE, ERROR, PANEL_WINDOW, RAIL_WINDOW, TEXT_DIM, TITLE,
};

/// Runner configuration: docking on, viewports off, amber CRT, 50 fps cap.
/// `auto_dockspace` is off so we own the split (game left, 330px panel right).
/// Default dear-app `RedrawMode::Poll` spins the UI thread and starves the
/// 20 ms slot; WaitUntil matches the client tick.
pub fn runner_config() -> dear_app::RunnerConfig {
    // Viewports stay off: dear-app renders into the single main viewport only.
    dear_app::RunnerConfig {
        window_title: "274bot".into(),
        window_size: (BASE_WINDOW_W as f64, BASE_WINDOW_H as f64),
        clear_color: BG,
        theme: Some(Theme::Dark),
        redraw: RedrawMode::WaitUntil { fps: 50.0 },
        ini_filename: Some(PathBuf::from("274bot-panel.ini")),
        restore_previous_geometry: false,
        docking: dear_app::DockingConfig {
            enable: true,
            auto_dockspace: false,
            dockspace_flags: DockNodeFlags::AUTO_HIDE_TAB_BAR,
            ..Default::default()
        },
        io_config_flags: Some(dear_imgui_rs::ConfigFlags::DOCKING_ENABLE),
        ..Default::default()
    }
}

/// Scale all ImGui style sizes for a window DPI. Held for Task 7: dear-app runs
/// under `HiDpiMode::Default`, which already sets `display_framebuffer_scale`,
/// so applying this on top of that would double every size on Retina.
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
    /// Cached path overlay; rebuilt at the 1 s raster cadence or on a new
    /// arm (see `overlay`).
    overlay: PathOverlay,
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
    /// Headed `--live` watch (`null_raster` or `stress50`); `None` interactive.
    live: Option<LiveHarness>,
    /// winit window cloned from `on_gpu_init` so MultiBox can grow/shrink
    /// the OS inner size by [`RAIL_W`] without shrinking the Game pane.
    os_window: Option<std::sync::Arc<winit::window::Window>>,
    /// Last `request_inner_size` rail-open flag; skip no-op resizes.
    rail_window_applied: Option<bool>,
}

/// Headed live harness: null_raster (2 slots), stress50 (50 slots), or a
/// shared scenario (`script_<name>`).
enum LiveHarness {
    Null(LiveNull),
    Stress(LiveStress),
    Script(LiveScript),
}

/// Headed `null_raster` harness state. `started` is the 120s login clock.
struct LiveNull {
    started: Instant,
    saw_scene2: bool,
    passed: bool,
}

/// Headed `stress50` harness. `started` is the 600s login clock.
struct LiveStress {
    started: Instant,
    last_announced: u8,
    passed: bool,
}

/// Headed `script_<name>` watch. The shared `ScenarioRunner` lives on the
/// `Session` (the slot thread ticks it); this struct only mirrors the
/// last-reported step for progress lines and latches the terminal state.
struct LiveScript {
    name: String,
    passed: bool,
    failed: Option<String>,
    last_step: Option<(usize, usize)>,
}

/// One rail/grid tile's GPU texture (uploaded when the slot's `FrameBuf`
/// hands a new frame to `take`).
struct TileView {
    view: GameView,
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
            views: HashMap::new(),
            last_proc: None,
            res_cpu: Metric::Measuring,
            res_ram: Metric::Measuring,
            res_traffic: Metric::Measuring,
            last_traffic: None,
            live: None,
            os_window: None,
            rail_window_applied: None,
        }
    }
}

const LIVE_USAGE: &str = "usage: panel-play [--live null_raster|stress50|script_<name>]";

/// `--live NAME` wins over `BOT_LIVE`. Empty env is ignored.
/// Unknown flags/names → `Err((2, msg))`; `--help`/`-h` → `Err((0, usage))`.
pub fn parse_live_args(
    args: impl IntoIterator<Item = impl AsRef<str>>,
    env_live: Option<&str>,
) -> Result<Option<String>, (i32, String)> {
    let mut live = env_live.filter(|s| !s.is_empty()).map(str::to_string);
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_ref() {
            "--live" => {
                let Some(name) = it.next() else {
                    return Err((2, "panel-play: --live needs a name".into()));
                };
                live = Some(name.as_ref().to_string());
            }
            "--help" | "-h" => return Err((0, LIVE_USAGE.into())),
            other => return Err((2, format!("panel-play: unknown {other}"))),
        }
    }
    if let Some(name) = live.as_deref() {
        let script_ok = name
            .strip_prefix("script_")
            .is_some_and(|n| scenario::get(n).is_some());
        if name != "null_raster" && name != "stress50" && !script_ok {
            return Err((2, LIVE_USAGE.into()));
        }
    }
    Ok(live)
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
/// then 50. At 50 print PASS and stay open. Timeout 600s. Does **not**
/// freeze-assert (operator may click). Does **not** fail on RSS magnitude.
fn live_stress_tick(live: &mut LiveStress, statuses: &[host_play::SlotStatus]) -> Option<String> {
    if live.passed {
        return None;
    }
    let n = statuses.iter().filter(|s| s.is_up()).count();
    if n >= 1 && live.last_announced < 1 {
        println!("live stress50: 1/50 up");
        live.last_announced = 1;
    }
    if n >= 10 && live.last_announced < 10 {
        println!("live stress50: 10/50 up");
        live.last_announced = 10;
    }
    if n >= 50 {
        let (rss, _) = sample_process();
        println!("PASS: live stress50 rss={rss} up50");
        live.last_announced = 50;
        live.passed = true;
        return None;
    }
    if live.started.elapsed() >= Duration::from_secs(600) {
        return Some(format!("live stress50: {n}/50 up after 600s"));
    }
    None
}

/// Headed script watch: mirror the shared `ScenarioRunner` each frame.
/// PASS prints the JSON evidence record and keeps the window open (visual
/// debug); FAIL prints the record and returns the message (the caller
/// exits 1, the existing live FAIL contract).
fn live_script_tick(live: &mut LiveScript, session: &mut Session) -> Option<String> {
    if live.passed || live.failed.is_some() {
        return None;
    }
    let (status, evidence) = {
        let guard = session.scenario.lock().unwrap();
        (
            guard.as_ref().map(|r| r.status()),
            guard.as_ref().and_then(|r| r.evidence().cloned()),
        )
    };
    let record = |evidence: &Option<scenario::Evidence>| {
        evidence
            .as_ref()
            .map(|ev| ev.to_json())
            .unwrap_or_default()
    };
    match status {
        Some(scenario::RunnerStatus::Passed) => {
            println!("PASS: live {} {}", live.name, record(&evidence));
            live.passed = true;
            None
        }
        Some(scenario::RunnerStatus::Failed(msg)) => {
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
                    println!(
                        "live {}: running step {}/{}",
                        live.name,
                        step + 1,
                        total
                    );
                }
            }
            None
        }
        None => None,
    }
}

/// Leaf dock nodes hide the tab bar while they host a single window.
fn single_bot_window_class() -> WindowClass {
    WindowClass::new(Id::from(1u32))
        .dock_node_flags_override_set(DockNodeFlags::AUTO_HIDE_TAB_BAR)
        .docking_always_tab_bar(false)
}

/// Grow/shrink the OS window with the sidecar rail. Game pane width is
/// unchanged: the extra pixels are the rail.
fn sync_os_window_size(state: &mut PanelState, rail_open: bool) {
    if state.rail_window_applied == Some(rail_open) {
        return;
    }
    let Some(window) = state.os_window.as_ref() else {
        return;
    };
    let (w, h) = os_window_size(rail_open);
    let _ = window.request_inner_size(winit::dpi::LogicalSize::new(w as f64, h as f64));
    state.rail_window_applied = Some(rail_open);
}

/// Fullscreen dock host: game fills the left, 330px-class panel on the right.
/// MultiBox (rail mode) splits an extra `RAIL_W` node on the far right and
/// grows the OS window by that width so the Game pane stays 765×503.
fn dock_host(ui: &Ui, state: &mut PanelState, game_title: &str) {
    let viewport = ui.main_viewport();
    let pos = viewport.pos();
    let rail_open = state.session.multibox && !state.session.wall.grid;
    sync_os_window_size(state, rail_open);
    let (want_w, want_h) = os_window_size(rail_open);
    let vs = viewport.size();
    // Prefer the live viewport once winit has applied the resize; otherwise
    // split against the target so the first rail frame does not steal Game
    // width.
    let size = if (vs[0] - want_w).abs() < 16.0 {
        vs
    } else {
        [want_w, want_h]
    };
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
            // Single-bot default: each split leaf hosts one window, so hide
            // the tab strip. MultiBox (campaign 4) can stack windows in a
            // node and the bar comes back.
            let _ = ui.dock_space_with_class(
                dock_id,
                [0.0, 0.0],
                DockNodeFlags::AUTO_HIDE_TAB_BAR,
                None,
            );
            let want = if state.session.multibox && !state.session.wall.grid {
                DockLayout::Rail
            } else {
                DockLayout::Single
            };
            if state.dock_layout != Some(want) {
                // A MultiBox toggle rebuilds the whole tree. The next
                // frame's DockSpace call above re-creates the node, so
                // windows are re-docked a frame after the toggle.
                DockBuilder::remove_node(ui, dock_id);
                state.dock_layout = Some(want);
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
                        let rail_ratio = (RAIL_W / size[0]).clamp(0.1, 0.9);
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
fn game_window(ui: &Ui, addons: &mut AddOns, state: &mut PanelState, title: &str) {
    let built = ui
        .window(title)
        .flags(WindowFlags::NO_COLLAPSE | WindowFlags::HORIZONTAL_SCROLLBAR)
        .build(|| {
            let avail = ui.content_region_avail();
            if state.session.multibox && state.session.wall.grid {
                grid_pane(ui, addons, state, avail);
            } else {
                game_pane(ui, addons, state, avail);
            }
        });
    state.session.set_game_pane_open(built.is_some());
}

/// Single-bot Game pane: the focused slot's applet, 765:503 fitted and
/// centred, with the nav overlay, capture, and queue card.
fn game_pane(ui: &Ui, addons: &mut AddOns, state: &mut PanelState, avail: [f32; 2]) {
    let size = fit_applet(avail);
    let cursor = ui.cursor_pos();
    ui.set_cursor_pos([
        cursor[0] + ((avail[0] - size[0]) * 0.5).max(0.0),
        cursor[1] + ((avail[1] - size[1]) * 0.5).max(0.0),
    ]);
    if state.game_view.is_none() {
        state.game_view = Some(GameView::init(&mut addons.gpu));
    }
    let (draw, capture) = {
        let focus = state.session.focus.lock().unwrap();
        (should_draw(&focus), should_capture(&focus))
    };
    if draw {
        let buf = state.session.focused_pixels();
        // One consumer per `FrameBuf`: `take` moves the stored frame out
        // (a `FrameOutput::Texture` hands its view off once, no Clone),
        // and `frame_pixels` packs either variant — the `Texture` arm
        // reads the GPU frame back through the client's device, the
        // `PixMap` arm packs the CPU pixels. `take` returning `None` is
        // "no new frame since the last upload", the same skip the old
        // snapshot path gave.
        let name = state.session.focused_name().unwrap_or_default();
        let gen = buf.as_ref().map(|p| p.generation()).unwrap_or(0);
        let dirty = state.last_upload.as_ref() != Some(&(name.clone(), gen));
        if dirty {
            if let Some(frame) = buf.as_ref().and_then(|p| p.take()) {
                let pixels = frame_pixels(frame);
                if let Some(view) = state.game_view.as_mut() {
                    view.upload(&addons.gpu, &pixels);
                }
            }
            state.last_upload = Some((name, gen));
        }
        let view = state.game_view.as_ref().expect("game view initialized");
        ui.image(view.tex_id, size);
        // Nav path overlay: amber polyline of the armed route's
        // remaining tiles, drawn over the Image (rebuilds at the
        // 1 s raster cadence or on a new arm, not per frame).
        state
            .overlay
            .frame(ui, &state.session, ui.item_rect_min(), size);
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

/// MultiBox grid-mode Game pane: one cell per wall member, row-major from
/// [`grid_cells`]. Clicking a cell selects the member; capture reaches
/// only the focused cell; the queue card overlays the focused cell. While
/// `only_render_selected` is on (the safe default) each cell collapses to
/// a cap row (dot, name, ✕) with no preview body.
fn grid_pane(ui: &Ui, addons: &mut AddOns, state: &mut PanelState, avail: [f32; 2]) {
    let members = state.session.wall.members.clone();
    if members.is_empty() {
        ui.text_disabled("no wall members");
        return;
    }
    let cells = grid_cells(members.len(), avail);
    // Drop textures for members that left the wall (grid ✕ / wall change).
    state
        .views
        .retain(|name, _| members.iter().any(|m| m == name));
    let (focused, capture) = {
        let focus = state.session.focus.lock().unwrap();
        (focus.focused.clone(), should_capture(&focus))
    };
    let only_selected = state.session.focus.lock().unwrap().only_render_selected;
    let statuses = state.session.statuses();
    for (i, name) in members.iter().enumerate() {
        let [cx, cy, cw, ch] = cells[i];
        let is_focused = focused.as_deref() == Some(name.as_str());
        ui.set_cursor_pos([cx, cy]);
        if only_selected {
            // Collapsed cell: cap row only, no preview body.
            let status = statuses.iter().find(|s| &s.username == name);
            let light = traffic_light(
                status.is_some_and(|s| s.ingame),
                status.is_some_and(|s| s.error.is_some()),
                status.is_some_and(|s| s.queue_position >= 1),
            );
            let (cap_select, cap_remove) = rail_cap(ui, name, light, focused.as_deref(), cw);
            if cap_remove {
                state.session.rail_remove(name);
            } else if cap_select {
                state.session.select(name);
            }
            continue;
        }
        let draw = draw_for_slot(&state.session.focus.lock().unwrap(), name);
        let clicked = cell_body(ui, addons, state, name, [cw, ch], draw);
        // Capture only on the focused cell; a click on another cell is a
        // select, not a click-through.
        if is_focused && capture && ui.is_item_hovered() {
            let mouse = ui.io().mouse_pos();
            let min = ui.item_rect_min();
            stream_capture(
                &state.session.capture_tx,
                mouse[0] - min[0],
                mouse[1] - min[1],
                cw,
                ch,
                ui.is_mouse_clicked(MouseButton::Left),
                ui.is_mouse_clicked(MouseButton::Right),
                ui.is_mouse_released(MouseButton::Left),
                ui.is_mouse_released(MouseButton::Right),
                &capture_keys(ui),
            );
        }
        if clicked {
            state.session.select(name);
        }
        if is_focused {
            draw_focused_queue_card(ui, &state.session, ui.item_rect_min());
        }
    }
}

/// Map hovered ImGui keys to GameShell `ch` values (arrows 1–4, ASCII).
fn capture_keys(ui: &Ui) -> Vec<(bool, i32)> {
    let shift = ui.is_key_down(Key::LeftShift) || ui.is_key_down(Key::RightShift);
    let mut keys = Vec::new();
    const NAMED: &[(Key, i32)] = &[
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
    for &(key, ch) in NAMED {
        if ui.is_key_pressed_with_repeat(key, false) {
            keys.push((true, ch));
        }
        if ui.is_key_released(key) {
            keys.push((false, ch));
        }
    }
    const LETTERS: [Key; 26] = [
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
    for (i, &key) in LETTERS.iter().enumerate() {
        let ch = if shift {
            (b'A' + i as u8) as i32
        } else {
            (b'a' + i as u8) as i32
        };
        if ui.is_key_pressed_with_repeat(key, false) {
            keys.push((true, ch));
        }
        if ui.is_key_released(key) {
            keys.push((false, ch));
        }
    }
    const DIGITS: [(Key, u8, u8); 10] = [
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
    for &(key, unshifted, shifted) in &DIGITS {
        let ch = if shift {
            shifted as i32
        } else {
            unshifted as i32
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
    ui.window(PANEL_WINDOW).build(|| {
        let _width = ui.push_item_width(-1.0);
        let _wrap = ui.push_text_wrap_pos(0.0);
        title_row(ui, session);
        ui.text_colored(TEXT_DIM, BUILD_LINE);
        banner(ui, session);
        profile_section(ui, session);
        credentials_section(ui, session);
        walkto_button(ui, session);
        script_section(ui, session);
        parameters_section(ui, session);
        status_section(ui, session);
        log_section(ui, session);
        rendering_section(ui, session);
        input_section(ui, session);
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
        ui.same_line();
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

fn kv_row(ui: &Ui, key: &str, value: &str) {
    ui.text_disabled(key);
    ui.same_line();
    ui.text_wrapped(value);
}

fn mock_button(ui: &Ui, label: &str, hint: &str, size: [f32; 2]) {
    let _disabled = ui.begin_disabled();
    ui.button_with_size(label, size);
    // SetItemTooltip: only this widget, including while disabled. `tooltip_text`
    // is SetTooltip and every mock dumps into one always-on blob.
    ui.set_item_tooltip(hint);
}

fn mock_button_row(ui: &Ui, labels: &[&str], hint: &str) {
    let avail = ui.content_region_avail()[0];
    let (w, stack) = button_row_layout(avail, labels.len());
    for (i, label) in labels.iter().enumerate() {
        if !stack && i > 0 {
            ui.same_line();
        }
        mock_button(ui, label, hint, [w, 0.0]);
    }
}

/// profile: vault combo; password prompt until unlocked.
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
        if ui.button_with_size("Unlock vault", [w, 0.0]) {
            let pass = session.pass_scratch.trim().to_string();
            if !pass.is_empty() {
                session.unlock(&pass);
                session.pass_scratch.clear();
            }
        }
        return;
    }
    let names = session.profile_names();
    if names.is_empty() {
        ui.text_disabled("no profiles in vault");
    } else if let Some(mut idx) = combo_index(session.focused_name().as_deref(), &names) {
        if ui.combo("##profile", &mut idx, &names, |n: &String| {
            Cow::Borrowed(n.as_str())
        }) {
            session.select(&names[idx]);
        }
    } else {
        ui.text_disabled("no focused profile");
    }
}

/// credentials: editable user/pass fields. Save upserts the vault profile,
/// spawns the slot if it is not running, then selects it. Usable with an
/// empty first-run vault (no focused profile required). Log in focuses an
/// already-spawned slot; Logout (enabled while the focused slot is ingame)
/// arms a clean IF logout and latches auto-login; Clear empties the two
/// fields without touching the vault. Panel does not auto-create test/test.
fn credentials_section(ui: &Ui, session: &mut Session) {
    if !section_open(ui, session, "credentials") {
        return;
    }
    if session.vault.is_none() {
        ui.text_disabled("vault locked");
        return;
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
    let (w, stack) = button_row_layout(avail, 2);
    // Save, same_line Clear
    if ui.button_with_size("Save", [w, 0.0]) {
        session.save_credentials();
    }
    if !stack {
        ui.same_line();
    }
    if ui.button_with_size("Clear", [w, 0.0]) {
        session.clear_credentials();
    }
    // Log in, same_line Logout (disabled rules unchanged)
    if ui.button_with_size("Log in", [w, 0.0]) {
        let name = session.cred_user.trim().to_string();
        if !name.is_empty() {
            session.login(&name);
        }
    }
    if !stack {
        ui.same_line();
    }
    {
        let focused = session.focused_name();
        let _logout_disabled = if focused.is_some() && session.focused_ingame() {
            None
        } else {
            Some(ui.begin_disabled())
        };
        if ui.button_with_size("Logout", [w, 0.0]) {
            if let Some(name) = focused {
                session.logout(&name);
            }
        }
        ui.set_item_tooltip("log out the focused slot — it stays in the combo");
    }
    // Auto-login follows the focused profile's vault setting; toggling
    // upserts it (never spawns a slot).
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
}

/// WalkTo: main-chrome button that opens the collision-dot tile picker.
fn walkto_button(ui: &Ui, session: &mut Session) {
    ui.spacing();
    let w = ui.content_region_avail()[0];
    if ui.button_with_size("WalkTo", [w, 0.0]) {
        session.walkto_open = true;
    }
    ui.set_item_tooltip("open tile picker");
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
            session.script_browse_open = true;
        }
        ui.set_item_tooltip("pick a compiled script or a loaded JS bot");
    }
    if !stack {
        ui.same_line();
    }
    {
        let _load = if active {
            Some(ui.begin_disabled())
        } else {
            None
        };
        if ui.button_with_size("Load", [w, 0.0]) {
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
        ui.same_line();
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
        ui.same_line();
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

/// True while the script Browse picker was wanted last frame; drives the
/// rising-edge `open_popup` (same latch as the chooser) so Esc cannot be
/// defeated by a per-frame reopen.
static PREV_BROWSE: AtomicBool = AtomicBool::new(false);

/// True while the script Load picker was wanted last frame (same rising-
/// edge latch as the chooser and Browse).
static PREV_LOAD: AtomicBool = AtomicBool::new(false);

/// Browse picker: one row per compiled script, then one per loaded JS card
/// (tagged "JS"). Clicking a row only stores the selection — selecting
/// never Starts.
fn browse_window(ui: &Ui, session: &mut Session) {
    let want = session.script_browse_open;
    let (open_popup, new_prev) =
        chooser_should_open_popup(want, PREV_BROWSE.load(Ordering::Relaxed));
    PREV_BROWSE.store(new_prev, Ordering::Relaxed);
    if open_popup {
        ui.open_popup("274bot-browse");
    }
    let mut open = want;
    if let Some(_t) = ui
        .begin_modal_popup_config("274bot-browse")
        .opened(&mut open)
        .begin()
    {
        let ids = script::compiled_ids();
        let cards = session.js.cards();
        let w = ui.content_region_avail()[0];
        if ids.is_empty() && cards.is_empty() {
            ui.text_disabled("no scripts — Browse is empty");
        }
        for id in ids {
            let selected = session.script_sel == Some(script::ScriptSel::Compiled(*id));
            if ui
                .selectable_config(id.0)
                .selected(selected)
                .close_popups(false)
                .size([w, 0.0])
                .build()
            {
                session.script_sel = Some(script::ScriptSel::Compiled(*id));
            }
        }
        for card in cards {
            let selected = session.script_sel == Some(script::ScriptSel::Loaded(card.name.clone()));
            if ui
                .selectable_config(format!("{}  (JS)", card.name))
                .selected(selected)
                .close_popups(false)
                .size([w, 0.0])
                .build()
            {
                session.script_sel = Some(script::ScriptSel::Loaded(card.name.clone()));
            }
        }
        ui.spacing();
        if ui.button_with_size("Close", [w, 0.0]) {
            ui.close_current_popup();
        }
    }
    session.script_browse_open = open;
}

/// Load modal: a filesystem path to an out-of-tree JS bot. Load registers
/// the card (same name overwrites; compiled ids reserved), persists the
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

/// parameters: uncollapses the selected compiled script's default key/value
/// rows (`script::defaults`; `(no parameters)` until ports fill a schema),
/// then the Edit button — always gray until `edit_parameters_enabled`
/// flips. Loaded JS cards have no parameter schema.
fn parameters_section(ui: &Ui, session: &mut Session) {
    if !section_open(ui, session, "parameters") {
        return;
    }
    let pairs = match &session.script_sel {
        Some(script::ScriptSel::Compiled(id)) => script::defaults(*id),
        _ => Vec::new(),
    };
    if pairs.is_empty() {
        ui.text_disabled("(no parameters)");
    } else {
        for (k, v) in pairs {
            kv_row(ui, &k, &v);
        }
    }
    let w = ui.content_region_avail()[0];
    if edit_parameters_enabled() {
        ui.button_with_size("Edit parameters", [w, 0.0]);
    } else {
        mock_button(ui, "Edit parameters", "not in v1", [w, 0.0]);
    }
    mock_button_row(ui, PARAM_ROW, "campaign 5");
}

/// Parameter editing is a v1 gap: always `false` until a params modal ships.
fn edit_parameters_enabled() -> bool {
    false
}

/// status: rs2b0t key/value rows (state, player, tile, modals), wrapped.
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

/// rendering: game renderer checkbox; `set_draw` is applied by the slot
/// threads from the shared focus on every frame.
fn rendering_section(ui: &Ui, session: &mut Session) {
    if !section_open(ui, session, "rendering") {
        return;
    }
    let on = session.focus.lock().unwrap().renderer;
    let mut cur = on;
    if ui.checkbox("game renderer", &mut cur) {
        session.set_renderer(cur);
    }
    ui.text_wrapped(if on {
        "1 fps watch; capture raises the focused slot to 50 fps. Never pauses the bot."
    } else {
        "renderer off — bot still runs."
    });
    // Sidecar 50 fps: a render-cadence pref for wall/grid members (the
    // focused slot's 50 fps is the capture path, not this knob).
    let mut sidecar = session.focus.lock().unwrap().sidecar_50;
    if ui.checkbox("sidecar 50 fps", &mut sidecar) {
        session.set_sidecar_50(sidecar);
    }
    ui.text_wrapped("wall/grid members repaint every 20 ms, not 1 s");
    // Music / SFX: the focused profile's vault lowmem. The toggle
    // retargets the focused slot's cpal speaker live.
    let mut music = !session.focused_lowmem();
    if ui.checkbox("Music / SFX", &mut music) {
        session.set_focused_lowmem(!music);
    }
    ui.text_wrapped("highmem audio; the focused slot's speaker opens live");
}

/// input: per-focused-bot capture toggle. Off = watch-only, zero input work.
fn input_section(ui: &Ui, session: &mut Session) {
    if !section_open(ui, session, "input") {
        return;
    }
    let on = session.focus.lock().unwrap().capture;
    let mut cur = on;
    if ui.checkbox("capture input", &mut cur) {
        session.set_capture(cur);
    }
    ui.text_wrapped(if on {
        "click-through on; at most one keyboard"
    } else {
        "watch-only; no input work"
    });
}

/// Sidecar rail window: only while MultiBox is on and Grid is off. Bulk
/// Login all / Logout all, the only-render-selected checkbox (unchecking
/// needs the render-all confirm), one tile per wall member (cap only while
/// only-render-selected, else cap + 1 fps body or renderer-off
/// placeholder), `+ add bot`, and the 1 Hz resource card.
fn rail_window(ui: &Ui, addons: &mut AddOns, state: &mut PanelState) {
    state.sample_resources();
    ui.window(RAIL_WINDOW)
        .flags(WindowFlags::NO_COLLAPSE)
        .build(|| {
            rail_bulk_row(ui, state);
            rail_tiles(ui, addons, state);
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

/// True while the render-all warning was wanted last frame; drives the
/// rising-edge `open_popup` (same latch as the chooser) so Esc cannot be
/// defeated by a per-frame reopen.
static PREV_RENDER_ALL_WARN: AtomicBool = AtomicBool::new(false);

/// Scary confirm before "only render selected" can be unchecked: OK stays
/// disabled until "I understand" is ticked, then writes
/// `only_render_selected = false`. Cancel/Esc keep the safe default; the
/// box that triggered this stays checked either way.
fn render_all_warn_window(ui: &Ui, session: &mut Session) {
    let want = session.wall.render_all_warn_open;
    let (open_popup, new_prev) =
        chooser_should_open_popup(want, PREV_RENDER_ALL_WARN.load(Ordering::Relaxed));
    PREV_RENDER_ALL_WARN.store(new_prev, Ordering::Relaxed);
    if open_popup {
        ui.open_popup("Render all wall members?");
    }
    let mut open = want;
    if let Some(_t) = ui
        .begin_modal_popup_config("Render all wall members?")
        .opened(&mut open)
        .begin()
    {
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
            // Every member now draws; kick the parked slots so the
            // renderers start within a frame.
            session.wake_all_slots();
            ui.close_current_popup();
        }
        if !stack {
            ui.same_line();
        }
        if ui.button_with_size("Cancel", [w, 0.0]) {
            ui.close_current_popup();
        }
    }
    session.wall.render_all_warn_open = open;
    if !open {
        session.wall.render_all_understood = false;
    }
}

/// One tile per wall member, in wall order: cap (traffic-light dot, name,
/// ✕) then a `TILE_W`×`TILE_H` body. The body blits the slot's `FrameBuf`
/// when `draw_for_slot` says this member paints, else the renderer-off
/// placeholder. While `only_render_selected` is on (the safe default) the
/// strip is collapsed: cap only, no body. Clicking the name or the body
/// focuses the member; the ✕ (a sibling button, never part of the name
/// click) removes it.
fn rail_tiles(ui: &Ui, addons: &mut AddOns, state: &mut PanelState) {
    ui.spacing();
    let members = state.session.wall.members.clone();
    let statuses = state.session.statuses();
    // Drop textures for members that left the rail (rail ✕ or wall change).
    state
        .views
        .retain(|name, _| members.iter().any(|m| m == name));
    let only_selected = state.session.focus.lock().unwrap().only_render_selected;
    for name in &members {
        let status = statuses.iter().find(|s| &s.username == name);
        let light = traffic_light(
            status.is_some_and(|s| s.ingame),
            status.is_some_and(|s| s.error.is_some()),
            status.is_some_and(|s| s.queue_position >= 1),
        );
        let (focused, draw) = {
            let focus = state.session.focus.lock().unwrap();
            (focus.focused.clone(), draw_for_slot(&focus, name))
        };
        let avail = ui.content_region_avail()[0];
        let (cap_select, cap_remove) = rail_cap(ui, name, light, focused.as_deref(), avail);
        let body_clicked = if only_selected {
            // Collapsed strip: no preview body until "render all" is accepted.
            false
        } else {
            rail_body(ui, addons, state, name, draw)
        };
        if cap_remove {
            state.session.rail_remove(name);
        } else if cap_select || body_clicked {
            state.session.select(name);
        }
        ui.spacing();
    }
}

/// Cap row: the traffic-light dot, the member's name (click selects), and
/// a small ✕ (rail remove: logout arm then `stop_slot`, never `vault`).
/// `width` is the strip the row must fit (rail avail or grid cell width).
/// Returns `(selected, removed)`.
fn rail_cap(ui: &Ui, name: &str, light: Light, focused: Option<&str>, width: f32) -> (bool, bool) {
    const X_W: f32 = 24.0;
    const DOT_W: f32 = 16.0;
    ui.text_colored(light.rgb(), "●");
    ui.same_line();
    let selected = focused == Some(name);
    let clicked = ui
        .selectable_config(name)
        .selected(selected)
        .size([(width - X_W - DOT_W).max(10.0), 0.0])
        .build();
    ui.same_line();
    let removed = ui.button_with_size(format!("✕##{name}"), [X_W, 0.0]);
    (clicked, removed)
}

/// Tile body: the member's `FrameBuf` blitted into a `size` box via a
/// cached [`GameView`] per name (uploaded when the mailbox hands a new
/// frame to `take`), or the renderer-off placeholder. Returns whether the
/// box was clicked (the grid cell / rail tile select path).
fn cell_body(
    ui: &Ui,
    addons: &mut AddOns,
    state: &mut PanelState,
    name: &str,
    size: [f32; 2],
    draw: bool,
) -> bool {
    if !draw {
        return ui
            .selectable_config(format!("renderer off##{name}"))
            .size(size)
            .build();
    }
    let tv = state
        .views
        .entry(name.to_string())
        .or_insert_with(|| TileView {
            view: GameView::init(&mut addons.gpu),
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
        // `take` moves the stored frame out; `frame_pixels` packs the
        // `PixMap` (CPU) or reads the wgpu frame back (`Texture`).
        if let Some(frame) = state
            .session
            .slots
            .get(name)
            .and_then(|s| s.pixels.take())
        {
            let pixels = frame_pixels(frame);
            tv.view.upload(&addons.gpu, &pixels);
        }
    }
    ui.image(tv.view.tex_id, size);
    ui.is_item_clicked_with_button(MouseButton::Left)
}

/// Rail tile body: the fixed `TILE_W`×`TILE_H` case of [`cell_body`].
fn rail_body(ui: &Ui, addons: &mut AddOns, state: &mut PanelState, name: &str, draw: bool) -> bool {
    cell_body(ui, addons, state, name, [TILE_W, TILE_H], draw)
}

/// `+ add bot`: opens the chooser modal again (first MultiBox-on opened it
/// once already; this button always reopens).
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

/// True while the chooser modal was wanted last frame; drives the
/// rising-edge `open_popup` so Esc cannot be defeated by a per-frame reopen.
static PREV_CHOOSER: AtomicBool = AtomicBool::new(false);

/// Rising-edge helper: `(open_popup, new_prev)`. `open_popup` is true only
/// on the `want` false→true edge, so `+ add bot` reopens after a close;
/// `new_prev` tracks `want` on **both** values, or a closed chooser would
/// keep a stale `true` and the next reopen would never fire.
pub fn chooser_should_open_popup(want: bool, prev: bool) -> (bool, bool) {
    (want && !prev, want)
}

/// Chooser modal: one row per vault profile. Click a row to load it onto
/// the wall; the row ✕ deletes the vault profile only (a live wall member
/// is untouched); Load all loads every profile. Esc (native popup
/// behavior) closes without loading.
fn chooser_window(ui: &Ui, session: &mut Session) {
    let want = session.wall.chooser_open;
    // Rising edge only: re-calling OpenPopup every frame would re-open the
    // modal the moment Esc closes it (BeginPopupModal writes `opened` false
    // when the popup is not open). The prev latch is updated on both true
    // and false so a later `+ add bot` is a fresh edge.
    let (open_popup, new_prev) =
        chooser_should_open_popup(want, PREV_CHOOSER.load(Ordering::Relaxed));
    PREV_CHOOSER.store(new_prev, Ordering::Relaxed);
    if open_popup {
        ui.open_popup("274bot-chooser");
    }
    let mut open = want;
    if let Some(_t) = ui
        .begin_modal_popup_config("274bot-chooser")
        .opened(&mut open)
        .begin()
    {
        let names: Vec<String> = session
            .vault
            .as_ref()
            .map(|v| v.profiles().map(|p| p.username.clone()).collect())
            .unwrap_or_default();
        let w = ui.content_region_avail()[0];
        if ui.button_with_size("Load all", [w, 0.0]) {
            session.load_all();
        }
        ui.spacing();
        if names.is_empty() {
            ui.text_disabled("vault is empty — Save creates the first profile");
        }
        for name in &names {
            let on_wall = session.wall.members.iter().any(|m| m == name);
            let (loaded, removed) = chooser_row(ui, name, on_wall);
            if loaded {
                session.load(name);
            }
            if removed {
                session.vault_remove(name);
            }
        }
        ui.spacing();
        let w = ui.content_region_avail()[0];
        if ui.button_with_size("Close", [w, 0.0]) {
            ui.close_current_popup();
        }
    }
    session.wall.chooser_open = open;
}

/// One chooser row: a selectable name (click loads; stays open so more
/// rows can be clicked) plus a small ✕ (vault row delete). The ✕ is a
/// sibling item, so its click never also loads the row.
fn chooser_row(ui: &Ui, name: &str, on_wall: bool) -> (bool, bool) {
    let avail = ui.content_region_avail()[0];
    let loaded = ui
        .selectable_config(name)
        .selected(on_wall)
        .close_popups(false)
        .size([(avail - 26.0).max(10.0), 0.0])
        .build();
    ui.same_line();
    let removed = ui.button_with_size(format!("✕##{name}"), [24.0, 0.0]);
    (loaded, removed)
}

/// Open the 274bot panel window. Call after the vault has been started.
/// `live` is `Some("null_raster")` or `Some("stress50")` (temp vault;
/// `BOT_VAULT_PASS` is unused on those paths).
pub fn run_panel(live: Option<String>) -> Result<(), dear_app::DearAppError> {
    let scale = Arc::new(AtomicU32::new(1.0f32.to_bits()));
    let frame_scale = Arc::clone(&scale);
    let mut state = PanelState::default();

    match live.as_deref() {
        Some("null_raster") => {
            if let Err(e) = state.session.live_prepare_null_raster() {
                eprintln!("FAIL: {e}");
                std::process::exit(1);
            }
            state.live = Some(LiveHarness::Null(LiveNull {
                started: Instant::now(),
                saw_scene2: false,
                passed: false,
            }));
        }
        Some("stress50") => {
            if let Err(e) = state.session.live_prepare_stress50() {
                eprintln!("FAIL: {e}");
                std::process::exit(1);
            }
            state.live = Some(LiveHarness::Stress(LiveStress {
                started: Instant::now(),
                last_announced: 0,
                passed: false,
            }));
        }
        Some(name) if name.starts_with("script_") => {
            let scenario_name = &name["script_".len()..];
            let Some(scenario) = scenario::get(scenario_name) else {
                eprintln!("FAIL: unknown scenario {scenario_name}");
                std::process::exit(1);
            };
            if let Err(e) = state.session.live_prepare_script(scenario) {
                eprintln!("FAIL: {e}");
                std::process::exit(1);
            }
            state.live = Some(LiveHarness::Script(LiveScript {
                name: name.to_string(),
                passed: false,
                failed: None,
                last_step: None,
            }));
        }
        _ => {
            if let Ok(pass) = std::env::var("BOT_VAULT_PASS") {
                // Interactive/headless env flow: unlock before the window so slots
                // spawn before the first frame. The in-panel prompt covers typing.
                if !state.session.unlock(&pass) {
                    eprintln!(
                        "panel: vault: {}",
                        state.session.error.clone().unwrap_or_default()
                    );
                }
            }
        }
    }

    let cfg = runner_config();
    let os_window: Arc<Mutex<Option<Arc<winit::window::Window>>>> = Arc::new(Mutex::new(None));
    let os_window_init = Arc::clone(&os_window);
    dear_app::AppBuilder::new()
        .with_config(cfg)
        .on_style(amber_style)
        .on_gpu_init(move |window, _, _, _| {
            scale.store(
                integer_ui_scale(window.scale_factor() as f32).to_bits(),
                Ordering::Relaxed,
            );
            *os_window_init.lock().unwrap() = Some(Arc::clone(window));
        })
        .on_frame(move |ui, addons| {
            if state.os_window.is_none() {
                state.os_window = os_window.lock().unwrap().clone();
            }
            let _scale = f32::from_bits(frame_scale.load(Ordering::Relaxed));
            state.session.pump_status();
            let statuses = state.session.statuses();
            if let Some(live) = state.live.as_mut() {
                let fail = match live {
                    LiveHarness::Null(n) => live_null_tick(n, &statuses),
                    LiveHarness::Stress(s) => live_stress_tick(s, &statuses),
                    LiveHarness::Script(ls) => live_script_tick(ls, &mut state.session),
                };
                if let Some(msg) = fail {
                    eprintln!("FAIL: {msg}");
                    std::process::exit(1);
                }
            }
            let title = game_window_title(state.session.focused_name().as_deref());
            dock_host(ui, &mut state, &title);
            let class = single_bot_window_class();
            ui.set_next_window_class(&class);
            panel_window(ui, &mut state.session);
            if state.session.walkto_open {
                picker::picker_window(ui, &mut state.session);
            }
            ui.set_next_window_class(&class);
            game_window(ui, addons, &mut state, &title);
            if state.session.multibox && !state.session.wall.grid {
                ui.set_next_window_class(&class);
                rail_window(ui, addons, &mut state);
            }
            // Every frame, not only while open: the prev latches must track
            // the close so the next open is a fresh rising edge.
            chooser_window(ui, &mut state.session);
            browse_window(ui, &mut state.session);
            load_window(ui, &mut state.session);
            render_all_warn_window(ui, &mut state.session);
        })
        .run()
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use dear_imgui_rs::ConfigFlags;

    use super::{
        apply_only_render_selected, apply_ui_scale, chooser_should_open_popup,
        edit_parameters_enabled, live_null_tick, live_script_tick, live_stress_tick,
        parse_live_args, runner_config, LiveNull, LiveScript, LiveStress, BASE_WINDOW_H,
        BASE_WINDOW_W, LIVE_USAGE,
    };
    use crate::theme::{fit_applet, game_window_title, panel_split_ratio};
    use dear_app::RedrawMode;

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
    fn edit_parameters_is_a_v1_gap() {
        assert!(!edit_parameters_enabled(), "no params modal in v1");
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
        assert_eq!(fit_applet([765.0, 503.0]), [765.0, 503.0]);
        // Small pane must not downscale — native 765×503 floor.
        assert_eq!(fit_applet([382.5, 251.5]), [765.0, 503.0]);
        let wide = fit_applet([2000.0, 503.0]);
        assert!((wide[1] - 503.0).abs() < 0.01);
        assert!(wide[0] <= 2000.0);
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
        assert!((r - 330.0 / 1120.0).abs() < 0.001);
        assert!(r < 0.4);
    }

    #[test]
    fn parse_live_args_none_without_flag_or_env() {
        assert_eq!(parse_live_args([] as [&str; 0], None), Ok(None));
        assert_eq!(parse_live_args([] as [&str; 0], Some("")), Ok(None));
    }

    #[test]
    fn parse_live_args_env_and_flag_null_raster() {
        assert_eq!(
            parse_live_args([] as [&str; 0], Some("null_raster")),
            Ok(Some("null_raster".into()))
        );
        assert_eq!(
            parse_live_args(["--live", "null_raster"], None),
            Ok(Some("null_raster".into()))
        );
    }

    #[test]
    fn parse_live_args_env_and_flag_stress50() {
        assert_eq!(
            parse_live_args([] as [&str; 0], Some("stress50")),
            Ok(Some("stress50".into()))
        );
        assert_eq!(
            parse_live_args(["--live", "stress50"], None),
            Ok(Some("stress50".into()))
        );
    }

    #[test]
    fn parse_live_args_flag_wins_over_env() {
        assert_eq!(
            parse_live_args(["--live", "null_raster"], Some("other")),
            Ok(Some("null_raster".into()))
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
            Ok(Some("script_walk".into()))
        );
        assert_eq!(
            parse_live_args([] as [&str; 0], Some("script_walk")),
            Ok(Some("script_walk".into()))
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
            vec![],
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

    /// Headed contract: PASS keeps the window open (returns `None`),
    /// FAIL returns the message the caller turns into exit 1.
    #[test]
    fn live_script_tick_keeps_window_open_on_pass_and_reports_fail() {
        use scenario::{Proof, RunnerStatus, Scenario, ScenarioRunner, Seed, Step, StepKind, Wait};

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
        };
        let mut runner = ScenarioRunner::new(pass);
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
        };
        assert_eq!(
            live_script_tick(&mut live, &mut s),
            None,
            "PASS keeps the window open"
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
        };
        let mut runner = ScenarioRunner::new(fail_scenario);
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
        };
        let msg = live_script_tick(&mut live, &mut s).expect("FAIL returns the message");
        assert!(msg.contains("not seen within 1 ticks"), "msg: {msg}");
        assert!(live.failed.is_some());
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
        }
    }

    fn ready_n(n: usize) -> Vec<host_play::SlotStatus> {
        (0..n)
            .map(|i| st(&format!("s{i:02}"), true, 2))
            .collect()
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
}
