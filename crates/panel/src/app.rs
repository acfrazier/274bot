//! dear-app shell: docking enabled, multi-viewport disabled, amber chrome.

use std::borrow::Cow;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use dear_app::AddOns;
use dear_imgui_rs::internal::RawWrapper;
use dear_imgui_rs::{Condition, Key, MouseButton, StyleColor, Ui};

use crate::chrome::MOCK_BUTTONS;
use crate::focus::{should_capture, should_draw};
use crate::game_view::{game_pixels, GameView};
use crate::session::{combo_index, stream_capture, Session};
use crate::theme::{integer_ui_scale, ACCENT, BG, TITLE};

/// Runner configuration from the task brief: docking on, viewports off.
pub fn runner_config() -> dear_app::RunnerConfig {
    let mut cfg = dear_app::RunnerConfig::default();
    cfg.window_title = "274bot".into();
    cfg.window_size = (1100.0, 560.0);
    cfg.clear_color = BG;
    cfg.docking = dear_app::DockingConfig {
        enable: true,
        auto_dockspace: true,
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

/// Push the amber accent into hover/header/tab style colors.
fn amber_style(ctx: &mut dear_imgui_rs::Context) {
    let style = ctx.style_mut();
    style.set_color(StyleColor::ButtonHovered, ACCENT);
    style.set_color(StyleColor::Header, ACCENT);
    style.set_color(StyleColor::Tab, ACCENT);
}

/// Game window: RGBA8 765×503 texture in a black viewport. Texture data is
/// always 765×503; the widget size is 765*dpi × 503*dpi.
fn game_image_size(scale: f32) -> [f32; 2] {
    [765.0 * scale.max(1.0), 503.0 * scale.max(1.0)]
}

/// Per-frame panel state: lazily-created game texture and the session (vault,
/// running slots, focus).
struct PanelState {
    game_view: Option<GameView>,
    session: Session,
}

impl Default for PanelState {
    fn default() -> Self {
        Self {
            game_view: None,
            session: Session::new(),
        }
    }
}

fn game_window(ui: &Ui, addons: &mut AddOns, scale: f32, state: &mut PanelState) {
    let built = ui
        .window("Game")
        .size([500.0, 400.0], Condition::FirstUseEver)
        .build(|| {
            let _bg = ui.push_style_color(StyleColor::ChildBg, [0.0, 0.0, 0.0, 1.0]);
            ui.child_window("game_viewport")
                .size(game_image_size(scale))
                .build(ui, || {
                    if state.game_view.is_none() {
                        state.game_view = Some(GameView::init(&mut addons.gpu));
                    }
                    let (draw, capture) = {
                        let focus = state.session.focus.lock().unwrap();
                        (should_draw(&focus), should_capture(&focus))
                    };
                    if draw {
                        let pixels = state.session.focused_pixels();
                        let pixels = game_pixels(&pixels);
                        if let Some(view) = &state.game_view {
                            view.upload(&addons.gpu, &pixels);
                        }
                        let view = state.game_view.as_ref().expect("game view initialized");
                        let size = game_image_size(scale);
                        ui.image(view.tex_id, size);
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

/// Left panel: wired vault/profile/status/log/rendering/input chrome plus the
/// mocked script/parameters sections.
fn panel_window(ui: &Ui, session: &mut Session) {
    ui.window("274bot")
        .size([360.0, 520.0], Condition::FirstUseEver)
        .build(|| {
            ui.text_colored(ACCENT, TITLE);
            ui.separator();
            profile_section(ui, session);
            ui.separator();
            credentials_section(ui, session);
            ui.separator();
            script_section(ui);
            ui.separator();
            parameters_section(ui);
            ui.separator();
            status_section(ui, session);
            ui.separator();
            log_section(ui, session);
            ui.separator();
            rendering_section(ui, session);
            ui.separator();
            input_section(ui, session);
        });
}

/// profile: vault combo + mainland checkbox; password prompt until unlocked.
fn profile_section(ui: &Ui, session: &mut Session) {
    ui.text_disabled("profile");
    if session.vault.is_none() {
        ui.input_text("##vault-pass", &mut session.pass_scratch)
            .password(true)
            .hint("vault passphrase")
            .build();
        if ui.button("Unlock vault") {
            let pass = session.pass_scratch.trim().to_string();
            if !pass.is_empty() {
                session.unlock(&pass);
                session.pass_scratch.clear();
            }
        }
        if let Some(err) = &session.error {
            ui.text_colored(ERROR, format!("vault: {err}"));
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
    if let Some(err) = &session.error {
        ui.text_colored(ERROR, format!("vault: {err}"));
    }
}

/// credentials: editable user/pass fields. Save upserts the vault profile,
/// spawns the slot if it is not running, then selects it. Usable with an
/// empty first-run vault (no focused profile required). Log in focuses an
/// already-spawned slot; Clear empties the two fields without touching the
/// vault. Panel does not auto-create test/test.
fn credentials_section(ui: &Ui, session: &mut Session) {
    ui.text_disabled("credentials");
    if session.vault.is_none() {
        ui.text_disabled("vault locked");
        return;
    }
    ui.input_text("##cred-user", &mut session.cred_user)
        .hint("username")
        .build();
    ui.input_text("##cred-pass", &mut session.cred_pass)
        .password(true)
        .hint("password")
        .build();
    if ui.button("Save") {
        session.save_credentials();
    }
    ui.same_line();
    if ui.button("Log in") {
        let name = session.cred_user.trim().to_string();
        if !name.is_empty() {
            session.login(&name);
        }
    }
    ui.same_line();
    if ui.button("Clear") {
        session.clear_credentials();
    }
    ui.text_disabled("Save writes the vault and spawns the slot if needed.");
}

/// script: mocked until campaign 5.
fn script_section(ui: &Ui) {
    ui.text_disabled("script");
    mock_buttons(ui, &MOCK_BUTTONS[..4], "campaign 5");
}

/// parameters: mocked until campaign 5.
fn parameters_section(ui: &Ui) {
    ui.text_disabled("parameters");
    ui.text_disabled("(no parameters)");
    mock_buttons(ui, &MOCK_BUTTONS[4..], "campaign 5");
}

/// Disabled, dimmed mock buttons with the owning campaign as a tooltip.
fn mock_buttons(ui: &Ui, buttons: &[&str], hint: &str) {
    let _disabled = ui.begin_disabled();
    for (i, name) in buttons.iter().enumerate() {
        if i > 0 {
            ui.same_line();
        }
        ui.button(*name);
        ui.tooltip_text(hint);
    }
}

/// status: one row per running slot from the shared `SlotStatus` list.
fn status_section(ui: &Ui, session: &Session) {
    ui.text_disabled("status");
    let statuses = session.statuses();
    if statuses.is_empty() {
        ui.text_disabled("no slots");
        return;
    }
    for s in &statuses {
        let state = if s.ingame {
            format!("ingame scene {}", s.scene_state)
        } else if let Some(err) = &s.error {
            format!("login {err}")
        } else if s.login_started.is_some() {
            "logging in…".to_string()
        } else {
            "waiting".to_string()
        };
        let player = if s.player.is_empty() { "?" } else { &s.player };
        ui.text(format!(
            "{} — {state} | {player} @ {},{} | modal {}",
            s.username, s.tile_x, s.tile_z, s.main_modal_id
        ));
        if s.ingame {
            ui.text_disabled(format!("energy {} run_sends {}", s.runenergy, s.run_sends));
        }
    }
}

/// log: status-transition lines, scrollable.
fn log_section(ui: &Ui, session: &Session) {
    ui.text_disabled("log");
    let log = session.log.lock().unwrap();
    ui.child_window("panel-log")
        .size([320.0, 120.0])
        .build(ui, || {
            for line in log.iter() {
                ui.text_disabled(line);
            }
        });
}

/// rendering: game renderer checkbox; `set_draw` is applied by the slot
/// threads from the shared focus on every frame.
fn rendering_section(ui: &Ui, session: &mut Session) {
    ui.text_disabled("rendering");
    let on = session.focus.lock().unwrap().renderer;
    let mut cur = on;
    if ui.checkbox("game renderer", &mut cur) {
        session.set_renderer(cur);
    }
    ui.text_disabled(if on {
        "Rendering never pauses the bot."
    } else {
        "renderer off"
    });
}

/// input: per-focused-bot capture toggle. Off = watch-only, zero input work.
fn input_section(ui: &Ui, session: &mut Session) {
    ui.text_disabled("input");
    let on = session.focus.lock().unwrap().capture;
    let mut cur = on;
    if ui.checkbox("capture input", &mut cur) {
        session.set_capture(cur);
    }
    ui.text_disabled(if on {
        "click-through on"
    } else {
        "watch-only; no input work"
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
            let scale = f32::from_bits(frame_scale.load(Ordering::Relaxed));
            state.session.pump_status();
            panel_window(ui, &mut state.session);
            game_window(ui, addons, scale, &mut state);
        })
        .run()
}

/// Error tint for vault/login failures in the panel banner.
const ERROR: [f32; 4] = [1.0, 0.5, 0.5, 1.0];

#[cfg(test)]
mod tests {
    use dear_imgui_rs::ConfigFlags;

    use super::{apply_ui_scale, game_image_size, runner_config};

    #[test]
    fn runner_config_docks_without_viewports() {
        let c = runner_config();
        assert!(c.docking.enable);
        let flags = c.io_config_flags.expect("flags");
        assert!(flags.contains(ConfigFlags::DOCKING_ENABLE));
        assert!(!flags.contains(ConfigFlags::VIEWPORTS_ENABLE));
    }

    #[test]
    fn apply_ui_scale_scales_padding_for_retina() {
        let mut ctx = dear_imgui_rs::Context::create();
        let before = ctx.style().window_padding();
        apply_ui_scale(ctx.style_mut(), 2.0);
        let after = ctx.style().window_padding();
        assert_eq!(before, [8.0, 8.0]);
        assert_eq!(after, [16.0, 16.0]);
    }

    #[test]
    fn game_image_size_scales_up_for_retina() {
        assert_eq!(game_image_size(2.0), [1530.0, 1006.0]);
        assert_eq!(game_image_size(1.0), [765.0, 503.0]);
    }
}
