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

/// `#RRGGBB` from an imgui RGBA quad (alpha ignored).
pub fn rgba_to_hex(c: [f32; 4]) -> String {
    format!(
        "#{:02X}{:02X}{:02X}",
        (c[0] * 255.0).round().clamp(0.0, 255.0) as u8,
        (c[1] * 255.0).round().clamp(0.0, 255.0) as u8,
        (c[2] * 255.0).round().clamp(0.0, 255.0) as u8
    )
}

/// Named CRT palette as persisted hex (`PanelUiState.chrome`). Defaults
/// match the [`ACCENT`] / [`BG`] / … constants. `#[serde(default)]` so an
/// old `panel-ui.json` still loads.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ChromeColors {
    pub accent: String,
    pub accent_hover: String,
    pub bg: String,
    pub bg_deep: String,
    pub text: String,
    pub text_dim: String,
    pub frame: String,
    pub hover_fill: String,
    pub active_fill: String,
    pub border: String,
    pub warn: String,
    pub error: String,
    pub green: String,
}

impl Default for ChromeColors {
    fn default() -> Self {
        Self {
            accent: rgba_to_hex(ACCENT),
            accent_hover: rgba_to_hex(ACCENT_HOVER),
            bg: rgba_to_hex(BG),
            bg_deep: rgba_to_hex(BG_DEEP),
            text: rgba_to_hex(TEXT),
            text_dim: rgba_to_hex(TEXT_DIM),
            frame: rgba_to_hex(FRAME),
            hover_fill: rgba_to_hex(HOVER_FILL),
            active_fill: rgba_to_hex(ACTIVE_FILL),
            border: rgba_to_hex(BORDER),
            warn: rgba_to_hex(WARN),
            error: rgba_to_hex(ERROR),
            green: rgba_to_hex(GREEN),
        }
    }
}

impl ChromeColors {
    pub fn accent_rgba(&self) -> [f32; 4] {
        hex_rgba(&self.accent, ACCENT)
    }
    pub fn accent_hover_rgba(&self) -> [f32; 4] {
        hex_rgba(&self.accent_hover, ACCENT_HOVER)
    }
    pub fn bg_rgba(&self) -> [f32; 4] {
        hex_rgba(&self.bg, BG)
    }
    pub fn bg_deep_rgba(&self) -> [f32; 4] {
        hex_rgba(&self.bg_deep, BG_DEEP)
    }
    pub fn text_rgba(&self) -> [f32; 4] {
        hex_rgba(&self.text, TEXT)
    }
    pub fn text_dim_rgba(&self) -> [f32; 4] {
        hex_rgba(&self.text_dim, TEXT_DIM)
    }
    pub fn frame_rgba(&self) -> [f32; 4] {
        hex_rgba(&self.frame, FRAME)
    }
    pub fn hover_fill_rgba(&self) -> [f32; 4] {
        hex_rgba(&self.hover_fill, HOVER_FILL)
    }
    pub fn active_fill_rgba(&self) -> [f32; 4] {
        hex_rgba(&self.active_fill, ACTIVE_FILL)
    }
    pub fn border_rgba(&self) -> [f32; 4] {
        hex_rgba(&self.border, BORDER)
    }
    pub fn warn_rgba(&self) -> [f32; 4] {
        hex_rgba(&self.warn, WARN)
    }
    pub fn error_rgba(&self) -> [f32; 4] {
        hex_rgba(&self.error, ERROR)
    }
    pub fn green_rgba(&self) -> [f32; 4] {
        hex_rgba(&self.green, GREEN)
    }
}

fn hex_rgba(hex: &str, fallback: [f32; 4]) -> [f32; 4] {
    let fb = [
        (fallback[0] * 255.0).round().clamp(0.0, 255.0) as u8,
        (fallback[1] * 255.0).round().clamp(0.0, 255.0) as u8,
        (fallback[2] * 255.0).round().clamp(0.0, 255.0) as u8,
    ];
    let [r, g, b] = crate::nav_settings::parse_html_color(hex, fb);
    [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0]
}

/// Amber CRT palette. Replaces the default ImGui blue title/tab chrome.
/// Named colours come from [`ChromeColors`]; unnamed mixes (title-active,
/// tab, scrollbar grab) stay as literals.
pub fn apply_amber(style: &mut Style, chrome: &ChromeColors) {
    let accent = chrome.accent_rgba();
    let accent_hover = chrome.accent_hover_rgba();
    let bg = chrome.bg_rgba();
    let bg_deep = chrome.bg_deep_rgba();
    let text = chrome.text_rgba();
    let text_dim = chrome.text_dim_rgba();
    let frame = chrome.frame_rgba();
    let hover_fill = chrome.hover_fill_rgba();
    let active_fill = chrome.active_fill_rgba();
    let border = chrome.border_rgba();
    style.set_color(StyleColor::Text, text);
    style.set_color(StyleColor::TextDisabled, text_dim);
    style.set_color(StyleColor::WindowBg, bg);
    style.set_color(StyleColor::ChildBg, bg_deep);
    style.set_color(StyleColor::PopupBg, bg);
    style.set_color(StyleColor::Border, border);
    style.set_color(StyleColor::BorderShadow, [0.0, 0.0, 0.0, 0.0]);
    style.set_color(StyleColor::FrameBg, frame);
    style.set_color(StyleColor::FrameBgHovered, hover_fill);
    style.set_color(StyleColor::FrameBgActive, active_fill);
    style.set_color(StyleColor::TitleBg, bg_deep);
    style.set_color(StyleColor::TitleBgActive, [0.12, 0.09, 0.02, 1.0]);
    style.set_color(StyleColor::TitleBgCollapsed, bg_deep);
    style.set_color(StyleColor::MenuBarBg, bg_deep);
    style.set_color(StyleColor::ScrollbarBg, bg_deep);
    style.set_color(StyleColor::ScrollbarGrab, [0.35, 0.25, 0.05, 1.0]);
    style.set_color(StyleColor::ScrollbarGrabHovered, accent_hover);
    style.set_color(StyleColor::ScrollbarGrabActive, accent);
    style.set_color(StyleColor::CheckMark, accent);
    style.set_color(StyleColor::CheckboxSelectedBg, hover_fill);
    style.set_color(StyleColor::SliderGrab, accent);
    style.set_color(StyleColor::SliderGrabActive, accent);
    style.set_color(StyleColor::Button, frame);
    style.set_color(StyleColor::ButtonHovered, hover_fill);
    style.set_color(StyleColor::ButtonActive, active_fill);
    style.set_color(StyleColor::Header, hover_fill);
    style.set_color(StyleColor::HeaderHovered, hover_fill);
    style.set_color(StyleColor::HeaderActive, active_fill);
    style.set_color(StyleColor::Separator, border);
    style.set_color(StyleColor::SeparatorHovered, accent);
    style.set_color(StyleColor::SeparatorActive, accent);
    style.set_color(StyleColor::ResizeGrip, frame);
    style.set_color(StyleColor::ResizeGripHovered, hover_fill);
    style.set_color(StyleColor::ResizeGripActive, active_fill);
    // Top-left orange "flag" is the dock tab-bar window-menu button, not
    // a resize grip. Off. Game AUTO_HIDEs its tab strip; the rail keeps a
    // tab so its window X can close the strip. Node X stays off so a
    // config tab cannot close the whole 274bot pane.
    style.set_window_menu_button_position(Direction::None);
    style.set_docking_node_has_close_button(false);
    style.set_color(StyleColor::Tab, [0.14, 0.10, 0.02, 1.0]);
    style.set_color(StyleColor::TabHovered, hover_fill);
    style.set_color(StyleColor::TabSelected, active_fill);
    style.set_color(StyleColor::TabSelectedOverline, accent);
    style.set_color(StyleColor::TabDimmed, bg_deep);
    style.set_color(StyleColor::TabDimmedSelected, [0.16, 0.11, 0.02, 1.0]);
    style.set_color(
        StyleColor::DockingPreview,
        [accent[0], accent[1], accent[2], 0.45],
    );
    style.set_color(StyleColor::DockingEmptyBg, bg_deep);
    style.set_color(
        StyleColor::TextSelectedBg,
        [accent[0], accent[1], accent[2], 0.35],
    );
    style.set_color(StyleColor::NavCursor, accent);
}

/// Re-apply the CRT palette onto the current ImGui context (live chrome
/// pickers). Same pointer cast [`dear_imgui_rs::Context::style_mut`] uses.
pub fn apply_amber_current(chrome: &ChromeColors) {
    unsafe {
        let style_ptr = dear_imgui_rs::sys::igGetStyle();
        if !style_ptr.is_null() {
            apply_amber(&mut *(style_ptr as *mut Style), chrome);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{apply_amber, rgba_to_hex, ChromeColors, ACCENT};

    #[test]
    fn rgba_to_hex_roundtrips_accent() {
        assert_eq!(rgba_to_hex(ACCENT), "#FFB000");
    }

    #[test]
    fn apply_amber_honours_chrome_accent() {
        let _guard = crate::IMGUI_CTX_TEST_GUARD.lock().unwrap();
        let mut ctx = dear_imgui_rs::Context::create();
        let chrome = ChromeColors {
            accent: "#00FF00".into(),
            ..Default::default()
        };
        apply_amber(ctx.style_mut(), &chrome);
        let check = ctx.style().color(dear_imgui_rs::StyleColor::CheckMark);
        assert!((check[1] - 1.0).abs() < 0.01, "custom accent is checkmark");
        assert!(check[0] < 0.05 && check[2] < 0.05);
    }
}
