//! dear-app shell: docking enabled, multi-viewport disabled, amber chrome.

use std::borrow::Cow;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use dear_app::AddOns;
use dear_imgui_rs::internal::RawWrapper;
use dear_imgui_rs::{Condition, StyleColor, Ui};

use crate::chrome::MOCK_BUTTONS;
use crate::focus::{should_capture, should_draw};
use crate::game_view::{game_pixels, GameView};
use crate::session::{Session, maybe_send_click};
use crate::theme::{ACCENT, BG, TITLE, integer_ui_scale};

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
    ui.window("Game")
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
                        // Click-through: only map/enqueue while capture is
                        // on; capture off skips the coord math entirely.
                        if capture && ui.is_item_clicked() {
                            let io = ui.io();
                            let mouse = io.mouse_pos();
                            let min = ui.item_rect_min();
                            maybe_send_click(
                                &state.session.capture_tx,
                                mouse[0] - min[0],
                                mouse[1] - min[1],
                                size[0],
                                size[1],
                            );
                        }
                    } else {
                        ui.text_disabled("renderer off");
                    }
                });
        });
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
        return;
    }
    let mut idx = session
        .focused_name()
        .and_then(|n| names.iter().position(|x| *x == n))
        .unwrap_or(0);
    if ui.combo("##profile", &mut idx, &names, |n: &String| Cow::Borrowed(n.as_str())) {
        session.select(&names[idx]);
    }
    let mut mainland = session.mainland.load(Ordering::Relaxed);
    if ui.checkbox("mainland hop", &mut mainland) {
        session.mainland.store(mainland, Ordering::Relaxed);
    }
    if let Some(err) = &session.error {
        ui.text_colored(ERROR, format!("vault: {err}"));
    }
}

/// credentials: the focused profile's stored user/pass from the vault.
fn credentials_section(ui: &Ui, session: &Session) {
    ui.text_disabled("credentials");
    let name = match session.focused_name() {
        Some(n) => n,
        None => {
            ui.text_disabled("no focused profile");
            return;
        }
    };
    match session.vault.as_ref().and_then(|v| v.get(&name)) {
        Some(p) => {
            ui.text(format!("user: {}", p.username));
            ui.text(format!("pass: {}", p.password));
        }
        None => ui.text_disabled("running slot outside vault"),
    }
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
