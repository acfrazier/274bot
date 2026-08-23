//! dear-app shell: docking enabled, multi-viewport disabled, amber chrome.

use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use dear_app::{AddOns, RedrawMode, Theme};
use dear_imgui_rs::internal::RawWrapper;
use dear_imgui_rs::{
    Condition, DockBuilder, DockNodeFlags, Id, Key, MouseButton, SplitDirection, Ui, WindowClass,
    WindowFlags,
};

use crate::chrome::{button_row_layout, multibox_tooltip, BUTTON_GAP, PARAM_ROW, SCRIPT_ROW};
use crate::focus::{should_capture, should_draw};
use crate::game_view::{game_pixels, GameView};
use crate::overlay::PathOverlay;
use crate::picker;
use crate::queue_card::queue_k_of_n;
use crate::rail::{RAIL_W, RAIL_WINDOW};
use crate::session::{combo_index, stream_capture, Session};
use crate::theme::{
    apply_amber, fit_applet, game_window_title, integer_ui_scale, panel_split_ratio, ACCENT, BG,
    BUILD_LINE, ERROR, PANEL_WINDOW, TEXT_DIM, TITLE,
};

/// Runner configuration: docking on, viewports off, amber CRT, 50 fps cap.
/// `auto_dockspace` is off so we own the split (game left, 330px panel right).
/// Default dear-app `RedrawMode::Poll` spins the UI thread and starves the
/// 20 ms slot; WaitUntil matches the client tick.
pub fn runner_config() -> dear_app::RunnerConfig {
    let mut cfg = dear_app::RunnerConfig::default();
    cfg.window_title = "274bot".into();
    cfg.window_size = (1120.0, 580.0);
    cfg.clear_color = BG;
    cfg.theme = Some(Theme::Dark);
    cfg.redraw = RedrawMode::WaitUntil { fps: 50.0 };
    cfg.ini_filename = Some(PathBuf::from("274bot-panel.ini"));
    cfg.restore_previous_geometry = false;
    cfg.docking = dear_app::DockingConfig {
        enable: true,
        auto_dockspace: false,
        dockspace_flags: DockNodeFlags::AUTO_HIDE_TAB_BAR,
        ..Default::default()
    };
    // Viewports stay off: dear-app renders into the single main viewport only.
    cfg.io_config_flags = Some(dear_imgui_rs::ConfigFlags::DOCKING_ENABLE);
    cfg
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
        }
    }
}

/// Leaf dock nodes hide the tab bar while they host a single window.
fn single_bot_window_class() -> WindowClass {
    WindowClass::new(Id::from(1u32))
        .dock_node_flags_override_set(DockNodeFlags::AUTO_HIDE_TAB_BAR)
        .docking_always_tab_bar(false)
}

/// Fullscreen dock host: game fills the left, 330px-class panel on the right.
/// MultiBox (rail mode) splits an extra `RAIL_W` node on the far right.
///
/// OS-grow note: dear-app's `AddOns` does **not** expose the winit window
/// (no `set_inner_size` on `AddOns`/`GpuApi`/`DockingApi`), so the plan's
/// fallback path landed: the rail is split inside the current window and
/// the game/panel shrink by `RAIL_W`. A later dear-app release that exposes
/// the window could swap this for `window.set_inner_size` + `RAIL_W`.
fn dock_host(ui: &Ui, state: &mut PanelState, game_title: &str) {
    let viewport = ui.main_viewport();
    let pos = viewport.pos();
    let size = viewport.size();
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

fn game_window(ui: &Ui, addons: &mut AddOns, state: &mut PanelState, title: &str) {
    let built = ui
        .window(title)
        .flags(WindowFlags::NO_COLLAPSE | WindowFlags::NO_SCROLLBAR)
        .build(|| {
            let avail = ui.content_region_avail();
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
                let name = state.session.focused_name().unwrap_or_default();
                let gen = buf.as_ref().map(|p| p.generation()).unwrap_or(0);
                let dirty = state.last_upload.as_ref() != Some(&(name.clone(), gen));
                if dirty {
                    let pixels = game_pixels(&buf);
                    if let Some(view) = state.game_view.as_mut() {
                        view.upload(&addons.gpu, &pixels);
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
        });
    state.session.set_game_pane_open(built.is_some());
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
        script_section(ui);
        parameters_section(ui);
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
    // Grid is a MultiBox submode: hide the rail, Game pane lays members
    // (cells land in Task 12). Unreachable until MultiBox is on.
    let _grid_disabled = if session.multibox {
        None
    } else {
        Some(ui.begin_disabled())
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
        ui.text_colored(ERROR, format!("{err}"));
    }
}

fn section_title(ui: &Ui, id: &str) {
    ui.spacing();
    ui.text_disabled(id);
    ui.separator();
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

/// profile: vault combo + mainland checkbox; password prompt until unlocked.
fn profile_section(ui: &Ui, session: &mut Session) {
    section_title(ui, "profile");
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
    let mut mainland = session.mainland.load(Ordering::Relaxed);
    if ui.checkbox("mainland hop", &mut mainland) {
        session.mainland.store(mainland, Ordering::Relaxed);
    }
}

/// credentials: editable user/pass fields. Save upserts the vault profile,
/// spawns the slot if it is not running, then selects it. Usable with an
/// empty first-run vault (no focused profile required). Log in focuses an
/// already-spawned slot; Logout (enabled while the focused slot is ingame)
/// arms a clean IF logout and latches auto-login; Clear empties the two
/// fields without touching the vault. Panel does not auto-create test/test.
fn credentials_section(ui: &Ui, session: &mut Session) {
    section_title(ui, "credentials");
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
    let (w, stack) = button_row_layout(avail, 4);
    if ui.button_with_size("Save", [w, 0.0]) {
        session.save_credentials();
    }
    if !stack {
        ui.same_line();
    }
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
    if !stack {
        ui.same_line();
    }
    if ui.button_with_size("Clear", [w, 0.0]) {
        session.clear_credentials();
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

/// script: mocked until campaign 5. Layout matches BotPanel (name+Browse,
/// then Start/Pause/Stop, then a status row).
fn script_section(ui: &Ui) {
    section_title(ui, "script");
    ui.text_colored(ACCENT, "(none)");
    ui.same_line();
    let rest = ui.content_region_avail()[0];
    mock_button(ui, "Browse…", "campaign 5", [rest.max(1.0), 0.0]);
    mock_button_row(ui, SCRIPT_ROW, "campaign 5");
    kv_row(ui, "status", "idle");
}

/// parameters: mocked until campaign 5.
fn parameters_section(ui: &Ui) {
    section_title(ui, "parameters");
    ui.text_disabled("(no parameters)");
    let w = ui.content_region_avail()[0];
    mock_button(ui, "Edit parameters", "campaign 5", [w, 0.0]);
    mock_button_row(ui, PARAM_ROW, "campaign 5");
}

/// status: rs2b0t key/value rows (state, player, tile, modals), wrapped.
fn status_section(ui: &Ui, session: &Session) {
    section_title(ui, "status");
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
    let player = if s.player.is_empty() { "?" } else { s.player.as_str() };
    kv_row(ui, "state", &state);
    kv_row(ui, "player", player);
    kv_row(ui, "tile", &format!("{} {}", s.tile_x, s.tile_z));
    kv_row(ui, "walk", &session.walk_status_text());
    let queue = queue_k_of_n(s.queue_position, s.queue_total).unwrap_or_else(|| "—".into());
    kv_row(ui, "queue", &queue);
    kv_row(ui, "modals", &format!("{}", s.main_modal_id));
}

/// log: status-transition lines, scrollable.
fn log_section(ui: &Ui, session: &Session) {
    section_title(ui, "log");
    let log = session.log.lock().unwrap();
    ui.child_window("panel-log")
        .size([0.0, 80.0])
        .build(ui, || {
            let _wrap = ui.push_text_wrap_pos(0.0);
            for line in log.iter() {
                ui.text_wrapped(line);
            }
        });
}

/// rendering: game renderer checkbox; `set_draw` is applied by the slot
/// threads from the shared focus on every frame.
fn rendering_section(ui: &Ui, session: &mut Session) {
    section_title(ui, "rendering");
    let on = session.focus.lock().unwrap().renderer;
    let mut cur = on;
    if ui.checkbox("game renderer", &mut cur) {
        session.set_renderer(cur);
    }
    ui.text_wrapped(if on {
        "1 fps rail (CPU). Capture raises it to 50 fps. Never pauses the bot."
    } else {
        "renderer off — bot still runs."
    });
}

/// input: per-focused-bot capture toggle. Off = watch-only, zero input work.
fn input_section(ui: &Ui, session: &mut Session) {
    section_title(ui, "input");
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

/// Sidecar rail window: only while MultiBox is on and Grid is off. Task 11
/// paints the member tiles; this task shows the stub strip.
fn rail_window(ui: &Ui, session: &Session) {
    ui.window(RAIL_WINDOW)
        .flags(WindowFlags::NO_COLLAPSE | WindowFlags::NO_SCROLLBAR)
        .build(|| {
            ui.text_colored(ACCENT, "rail");
            ui.text_wrapped(format!(
                "{} member(s) · tiles land in Task 11",
                session.wall.members.len()
            ));
        });
}

/// Open the 274bot panel window. Call after the vault has been started.
pub fn run_panel() -> Result<(), dear_app::DearAppError> {
    let scale = Arc::new(AtomicU32::new(1.0f32.to_bits()));
    let frame_scale = Arc::clone(&scale);
    let mut state = PanelState::default();

    // Headless env flow: unlock before the window opens so the slots spawn
    // before the first frame. The in-panel passphrase prompt covers the
    // interactive flow.
    if let Ok(pass) = std::env::var("BOT_VAULT_PASS") {
        if !state.session.unlock(&pass) {
            eprintln!(
                "panel: vault: {}",
                state.session.error.clone().unwrap_or_default()
            );
        }
    }

    let cfg = runner_config();
    dear_app::AppBuilder::new()
        .with_config(cfg)
        .on_style(amber_style)
        .on_gpu_init(move |window, _, _, _| {
            scale.store(
                integer_ui_scale(window.scale_factor() as f32).to_bits(),
                Ordering::Relaxed,
            );
        })
        .on_frame(move |ui, addons| {
            let _scale = f32::from_bits(frame_scale.load(Ordering::Relaxed));
            state.session.pump_status();
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
                rail_window(ui, &state.session);
            }
        })
        .run()
}



#[cfg(test)]
mod tests {
    use dear_imgui_rs::ConfigFlags;

    use super::{apply_ui_scale, runner_config};
    use crate::theme::{fit_applet, game_window_title, panel_split_ratio};
    use dear_app::RedrawMode;

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
        let small = fit_applet([382.5, 251.5]);
        assert!((small[0] / small[1] - 765.0 / 503.0).abs() < 0.01);
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
}
