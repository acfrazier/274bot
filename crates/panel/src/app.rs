//! dear-app shell: docking enabled, multi-viewport disabled, amber mocks.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use dear_imgui_rs::internal::RawWrapper;
use dear_imgui_rs::{Condition, StyleColor, Ui};

use crate::chrome::{MOCK_BUTTONS, sections};
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

/// Game window: black placeholder viewport until Task 7 wires the 765×503
/// texture; logical size divides by scale so the physical render stays 765×503.
fn game_window(ui: &Ui, scale: f32) {
    let s = scale.max(1.0);
    ui.window("Game")
        .size([500.0, 400.0], Condition::FirstUseEver)
        .build(|| {
            let _bg = ui.push_style_color(StyleColor::ChildBg, [0.0, 0.0, 0.0, 1.0]);
            ui.child_window("game_viewport")
                .size([765.0 / s, 503.0 / s])
                .build(ui, || {
                    ui.text("no bot focused");
                });
        });
}

/// Open the 274bot panel window. Call after the vault has been started.
pub fn run_panel() -> Result<(), dear_app::DearAppError> {
    let scale = Arc::new(AtomicU32::new(1.0f32.to_bits()));
    let frame_scale = Arc::clone(&scale);

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
        .on_frame(move |ui, _addons| {
            let scale = f32::from_bits(frame_scale.load(Ordering::Relaxed));
            panel_window(ui);
            game_window(ui, scale);
        })
        .run()
}

#[cfg(test)]
mod tests {
    use dear_imgui_rs::ConfigFlags;

    use super::{apply_ui_scale, runner_config};

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
}
