use dear_imgui_rs::{Style, StyleColor};

/// Amber accent color (#FFB000) for the 274 panel.
pub const ACCENT: [f32; 4] = [1.0, 176.0 / 255.0, 0.0, 1.0]; // #FFB000
/// Hover variant of the amber accent (#FFC14D).
pub const ACCENT_HOVER: [f32; 4] = [1.0, 193.0 / 255.0, 77.0 / 255.0, 1.0]; // #FFC14D
/// Panel background (#111).
pub const BG: [f32; 4] = [0.067, 0.067, 0.067, 1.0]; // #111
/// Deeper chrome (#0a0a0a) for title bars / empty dock.
pub const BG_DEEP: [f32; 4] = [0.039, 0.039, 0.039, 1.0]; // #0a0a0a
/// Body text (#ddd).
pub const TEXT: [f32; 4] = [0.867, 0.867, 0.867, 1.0];
/// Dim text.
pub const TEXT_DIM: [f32; 4] = [0.4, 0.4, 0.4, 1.0];
/// Frame / button fill.
pub const FRAME: [f32; 4] = [0.102, 0.102, 0.102, 1.0]; // #1a1a1a
/// Hover/selected fill — dark amber, so #ddd text stays readable (not white-on-#FFB000).
pub const HOVER_FILL: [f32; 4] = [0.18, 0.12, 0.04, 1.0];
/// Pressed fill.
pub const ACTIVE_FILL: [f32; 4] = [0.24, 0.16, 0.04, 1.0];
/// Borders (#333).
pub const BORDER: [f32; 4] = [0.2, 0.2, 0.2, 1.0];
/// Warn banner text (#e0d75b).
pub const WARN: [f32; 4] = [224.0 / 255.0, 215.0 / 255.0, 91.0 / 255.0, 1.0];
/// Error banner text (#ff7b7b).
pub const ERROR: [f32; 4] = [1.0, 123.0 / 255.0, 123.0 / 255.0, 1.0];
/// Panel window title.
pub const TITLE: &str = "274bot";
/// Dim build line under the title (no fake git hash).
pub const BUILD_LINE: &str = "campaign 2 · mock chrome";
/// Right-hand chrome width, matching rs2b0t's 330px panel.
pub const PANEL_WIDTH: f32 = 330.0;
/// Stable ImGui window name for the right-hand chrome.
pub const PANEL_WINDOW: &str = "274bot";

/// Integer UI scale for ImGui chrome. Never mutates 765×503. Do **not** also
/// multiply the Game Image by this — HiDpi already maps logical pixels.
pub fn integer_ui_scale(dpi: f32) -> f32 {
    dpi.max(1.0).round().max(1.0)
}

/// Largest 765:503 box that fits `avail` (rs2b0t `#game-stage`).
pub fn fit_applet(avail: [f32; 2]) -> [f32; 2] {
    const AW: f32 = 765.0;
    const AH: f32 = 503.0;
    let w = avail[0].max(1.0);
    let h = avail[1].max(1.0);
    let scale = (w / AW).min(h / AH);
    [AW * scale, AH * scale]
}

/// Right-split ratio for a 330px-class panel in a window of `width`.
pub fn panel_split_ratio(width: f32) -> f32 {
    (PANEL_WIDTH / width.max(1.0)).clamp(0.18, 0.40)
}

/// Title bar of the Game pane: focused vault username, else `"Game"`.
pub fn game_window_title(focused: Option<&str>) -> String {
    match focused {
        Some(name) if !name.is_empty() => name.to_string(),
        _ => "Game".into(),
    }
}

/// Amber CRT palette. Replaces the default ImGui blue title/tab chrome.
pub fn apply_amber(style: &mut Style) {
    style.set_color(StyleColor::Text, TEXT);
    style.set_color(StyleColor::TextDisabled, TEXT_DIM);
    style.set_color(StyleColor::WindowBg, BG);
    style.set_color(StyleColor::ChildBg, BG_DEEP);
    style.set_color(StyleColor::PopupBg, BG);
    style.set_color(StyleColor::Border, BORDER);
    style.set_color(StyleColor::BorderShadow, [0.0, 0.0, 0.0, 0.0]);
    style.set_color(StyleColor::FrameBg, FRAME);
    style.set_color(StyleColor::FrameBgHovered, HOVER_FILL);
    style.set_color(StyleColor::FrameBgActive, ACTIVE_FILL);
    style.set_color(StyleColor::TitleBg, BG_DEEP);
    style.set_color(StyleColor::TitleBgActive, [0.12, 0.09, 0.02, 1.0]);
    style.set_color(StyleColor::TitleBgCollapsed, BG_DEEP);
    style.set_color(StyleColor::MenuBarBg, BG_DEEP);
    style.set_color(StyleColor::ScrollbarBg, BG_DEEP);
    style.set_color(StyleColor::ScrollbarGrab, [0.35, 0.25, 0.05, 1.0]);
    style.set_color(StyleColor::ScrollbarGrabHovered, ACCENT_HOVER);
    style.set_color(StyleColor::ScrollbarGrabActive, ACCENT);
    style.set_color(StyleColor::CheckMark, ACCENT);
    style.set_color(StyleColor::CheckboxSelectedBg, HOVER_FILL);
    style.set_color(StyleColor::SliderGrab, ACCENT);
    style.set_color(StyleColor::SliderGrabActive, ACCENT);
    style.set_color(StyleColor::Button, FRAME);
    style.set_color(StyleColor::ButtonHovered, HOVER_FILL);
    style.set_color(StyleColor::ButtonActive, ACTIVE_FILL);
    style.set_color(StyleColor::Header, HOVER_FILL);
    style.set_color(StyleColor::HeaderHovered, HOVER_FILL);
    style.set_color(StyleColor::HeaderActive, ACTIVE_FILL);
    style.set_color(StyleColor::Separator, BORDER);
    style.set_color(StyleColor::SeparatorHovered, ACCENT);
    style.set_color(StyleColor::SeparatorActive, ACCENT);
    style.set_color(StyleColor::ResizeGrip, FRAME);
    style.set_color(StyleColor::ResizeGripHovered, ACCENT);
    style.set_color(StyleColor::ResizeGripActive, ACCENT_HOVER);
    style.set_color(StyleColor::Tab, [0.14, 0.10, 0.02, 1.0]);
    style.set_color(StyleColor::TabHovered, HOVER_FILL);
    style.set_color(StyleColor::TabSelected, ACTIVE_FILL);
    style.set_color(StyleColor::TabSelectedOverline, ACCENT);
    style.set_color(StyleColor::TabDimmed, BG_DEEP);
    style.set_color(StyleColor::TabDimmedSelected, [0.16, 0.11, 0.02, 1.0]);
    style.set_color(StyleColor::DockingPreview, [ACCENT[0], ACCENT[1], ACCENT[2], 0.45]);
    style.set_color(StyleColor::DockingEmptyBg, BG_DEEP);
    style.set_color(StyleColor::TextSelectedBg, [ACCENT[0], ACCENT[1], ACCENT[2], 0.35]);
    style.set_color(StyleColor::NavCursor, ACCENT);
}
