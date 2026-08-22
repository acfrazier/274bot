//! dear-app shell: docking enabled, multi-viewport disabled, amber mocks.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use dear_app::AddOns;
use dear_imgui_rs::internal::RawWrapper;
use dear_imgui_rs::{Condition, StyleColor, Ui};
use host::PixelBuf;

use crate::chrome::{MOCK_BUTTONS, sections};
use crate::focus::{Focus, should_draw};
use crate::game_view::{game_pixels, GameView};
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

/// Left panel: amber title, chrome section placeholders, disabled mock buttons.
fn panel_window(ui: &Ui) {
    ui.window("274bot")
        .size([360.0, 520.0], Condition::FirstUseEver)
        .build(|| {
            ui.text_colored(ACCENT, TITLE);
            for section in sections() {
                ui.separator();
                ui.text_disabled(section.id);
                if let Some(hint) = section.campaign_hint {
                    ui.tooltip_text(hint);
                }
            }
            ui.separator();
            for name in MOCK_BUTTONS {
                let _disabled = ui.begin_disabled();
                ui.button(*name);
            }
        });
}

/// Game window: RGBA8 765×503 texture in a black viewport. Texture data is
/// always 765×503; the widget size is 765*dpi × 503*dpi.
fn game_image_size(scale: f32) -> [f32; 2] {
    [765.0 * scale.max(1.0), 503.0 * scale.max(1.0)]
}

/// Per-frame panel state: lazily-created game texture, focus policy, and the
/// focused slot's pixels (None until a slot is wired).
struct PanelState {
    game_view: Option<GameView>,
    focus: Focus,
    pixels: Option<Arc<PixelBuf>>,
}

impl Default for PanelState {
    fn default() -> Self {
        Self {
            game_view: None,
            // No slot is focused yet: the game pane shows "renderer off" and
            // the texture is not uploaded until a bot connects.
            focus: Focus {
                focused: None,
                renderer: true,
                game_pane_open: true,
                capture: false,
            },
            pixels: None,
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
                    if should_draw(&state.focus) {
                        let pixels = game_pixels(&state.pixels);
                        if let Some(view) = &state.game_view {
                            view.upload(&addons.gpu, &pixels);
                        }
                        let view = state.game_view.as_ref().expect("game view initialized");
                        ui.image(view.tex_id, game_image_size(scale));
                    } else {
                        ui.text_disabled("renderer off");
                    }
                });
        });
}

/// Open the 274bot panel window. Call after the vault has been started.
pub fn run_panel() -> Result<(), dear_app::DearAppError> {
    let scale = Arc::new(AtomicU32::new(1.0f32.to_bits()));
    let frame_scale = Arc::clone(&scale);
    let mut state = PanelState::default();

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
            panel_window(ui);
            game_window(ui, addons, scale, &mut state);
        })
        .run()
}

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
