use dear_imgui_rs::{Direction, Style, StyleColor};

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
/// Running cap dot (#4cd964), the green cap state.
pub const GREEN: [f32; 4] = [76.0 / 255.0, 217.0 / 255.0, 100.0 / 255.0, 1.0];
/// Panel window title.
pub const TITLE: &str = "274bot";
/// Dim build line under the title is [`crate::build_info::build_line`]
/// (`alpha 1 ·` git stamp; hover is crate version + full commit).
/// Right-hand chrome width, matching rs2b0t's 330px panel. Locked: the
/// strip does not grow with the OS window, only taller.
pub const PANEL_WIDTH: f32 = 330.0;
/// Native 274 applet in logical pixels. Same numbers as
/// [`crate::game_view::APPLET_W`] / `APPLET_H` (those are `u32` for
/// textures). HiDpi (`winit` + imgui Default) maps these to the
/// framebuffer; we do not ScaleAllSizes on top.
const STAGE_W: f32 = 765.0;
const STAGE_H: f32 = 503.0;
/// Stable ImGui window name for the right-hand chrome.
pub const PANEL_WINDOW: &str = "274bot";
/// Stable ImGui window name for the MultiBox sidecar rail.
pub const RAIL_WINDOW: &str = "274bot-rail";

/// Integer UI scale for ImGui chrome. Never mutates 765×503. Do **not** also
/// multiply the Game Image by this — HiDpi already maps logical pixels.
pub fn integer_ui_scale(dpi: f32) -> f32 {
    dpi.max(1.0).round().max(1.0)
}

/// Native applet size. Non-grid Game blit is always this; host-window
/// zoom/resize does not scale the client.
pub fn native_applet() -> [f32; 2] {
    [STAGE_W, STAGE_H]
}

/// Offset of the native applet inside the Game pane: flush to the panel
/// (right) and vertically centred. Extra width sits on the left.
pub fn applet_offset(avail: [f32; 2], size: [f32; 2]) -> [f32; 2] {
    [
        (avail[0] - size[0]).max(0.0),
        ((avail[1] - size[1]) * 0.5).max(0.0),
    ]
}

/// Grid-mode only: 765:503 stage fitted into `avail`.
pub fn fit_applet(avail: [f32; 2]) -> [f32; 2] {
    let w = avail[0].max(1.0);
    let h = avail[1].max(1.0);
    let scale = (w / STAGE_W).min(h / STAGE_H).max(0.01);
    [STAGE_W * scale, STAGE_H * scale]
}

/// Right-split ratio so the panel stays [`PANEL_WIDTH`] px at `width`.
pub fn panel_split_ratio(width: f32) -> f32 {
    (PANEL_WIDTH / width.max(1.0)).clamp(0.05, 0.85)
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
    style.set_color(StyleColor::ResizeGripHovered, HOVER_FILL);
    style.set_color(StyleColor::ResizeGripActive, ACTIVE_FILL);
    // Top-left orange "flag" is the dock tab-bar window-menu button, not
    // a resize grip. Off. Game AUTO_HIDEs its tab strip; the rail keeps a
    // tab so its window X can close the strip. Node X stays off so a
    // config tab cannot close the whole 274bot pane.
    style.set_window_menu_button_position(Direction::None);
    style.set_docking_node_has_close_button(false);
    style.set_color(StyleColor::Tab, [0.14, 0.10, 0.02, 1.0]);
    style.set_color(StyleColor::TabHovered, HOVER_FILL);
    style.set_color(StyleColor::TabSelected, ACTIVE_FILL);
    style.set_color(StyleColor::TabSelectedOverline, ACCENT);
    style.set_color(StyleColor::TabDimmed, BG_DEEP);
    style.set_color(StyleColor::TabDimmedSelected, [0.16, 0.11, 0.02, 1.0]);
    style.set_color(
        StyleColor::DockingPreview,
        [ACCENT[0], ACCENT[1], ACCENT[2], 0.45],
    );
    style.set_color(StyleColor::DockingEmptyBg, BG_DEEP);
    style.set_color(
        StyleColor::TextSelectedBg,
        [ACCENT[0], ACCENT[1], ACCENT[2], 0.35],
    );
    style.set_color(StyleColor::NavCursor, ACCENT);
}
