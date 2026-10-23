//! Style data for a script's paint frame: the applet-space metrics and
//! the sRGB palette the front ends draw with, plus the parse of the
//! script's accent colour text.
//!
//! The values follow rs2b0t's `Paint.ts` (the PAINT-RS2B0T-LOOK-1022
//! design, section 3). Every length is in applet pixels at scale 1 on
//! the 765x503 stage. A renderer multiplies each length by its frame
//! scale. This module holds plain data only. It names no toolkit type.

use super::paint::PaintView;

/// The applet-space dock the paint fills: `(x, y, w, h)` of the client's
/// chatbox on the 765x503 stage.
pub const PANEL_RECT: [f32; 4] = [8.0, 345.0, 506.0, 150.0];

/// Body inset from the panel edge to the text and the button flow.
pub const PAD: f32 = 8.0;
/// One text row's height. Row `n` starts at `top + n * LINE`.
pub const LINE: f32 = 16.0;
/// The text top inside one row, below the row top.
pub const TEXT_DY: f32 = 2.0;
/// The title bar height. The bar also holds the strip and the toggle.
pub const TITLE_H: f32 = 20.0;
/// The text top inside the title bar, below the bar top.
pub const TITLE_TEXT_DY: f32 = 3.0;
/// The collapse toggle square at the right end of the title bar.
pub const TOGGLE_W: f32 = 20.0;
/// The toggle glyph's left edge inside the toggle square.
pub const TOGGLE_TEXT_DX: f32 = 7.0;
/// The last body row's height when the paint advertises a footer.
pub const FOOTER_H: f32 = 16.0;
/// The footer text top inside its row, below the row top. This is the
/// offset the panel used before this layout (a quarter of the row).
pub const FOOTER_TEXT_DY: f32 = 4.0;

/// The rail column width. The body starts one [`PAD`] right of it.
pub const RAIL_W: f32 = 72.0;
/// The rail entries' inset from the panel's left edge.
pub const RAIL_INSET: f32 = 2.0;
/// The rail entry text's left edge inside its entry rect.
pub const RAIL_TEXT_DX: f32 = 6.0;
/// The active rail entry's marker width.
pub const RAIL_MARKER_W: f32 = 2.0;

/// One tabs-band entry height.
pub const TAB_H: f32 = 18.0;
/// The gap above a tabs band's entries.
pub const TAB_INSET: f32 = 3.0;
/// The gap below a tabs band's entries.
pub const TAB_TRAIL: f32 = 2.0;
/// The first tabs-band entry's inset from the panel's left edge.
pub const TAB_X0: f32 = 4.0;
/// The gap between two tabs-band entries.
pub const TAB_GAP: f32 = 2.0;
/// The width a tab entry adds to its label's cell width. The strip's
/// tabs use the same add.
pub const TAB_PAD: f32 = 14.0;
/// A tab label's left edge inside its entry rect. The strip's tabs use
/// the same offset.
pub const TAB_TEXT_DX: f32 = 7.0;
/// A tabs-band label's top inside its entry rect.
pub const TAB_TEXT_DY: f32 = 8.0;
/// The active tabs-band entry's underline height.
pub const TAB_UNDERLINE_H: f32 = 2.0;
/// The strip tab entries' inset from the title bar top. Their height is
/// [`TITLE_H`] less twice this inset.
pub const STRIP_TAB_INSET: f32 = 2.0;

/// One button's height.
pub const BUTTON_H: f32 = 16.0;
/// The width a button adds to its label's cell width.
pub const BUTTON_PAD: f32 = 18.0;
/// A button label's left edge inside its button rect.
pub const BUTTON_TEXT_DX: f32 = 7.0;
/// A button label's top inside its button rect.
pub const BUTTON_TEXT_DY: f32 = 2.0;
/// The gap between two buttons on one row.
pub const BUTTON_GAP: f32 = 6.0;
/// The vertical distance from one button row's top to the next.
pub const BUTTON_ROW_ADVANCE: f32 = 20.0;
/// The gap between the last text row's bottom and the first button row.
pub const BUTTON_TOP_GAP: f32 = 2.0;

/// Body fill: (12, 12, 14) at alpha 0.98. rs2b0t draws the same colour
/// at 0.88 in sRGB space. The panel blends in linear space, so 0.98
/// gives rs2b0t's measured composite over the chat parchment.
pub const BG: [u8; 4] = [12, 12, 14, 250];
/// Title bar fill: (28, 28, 34) at alpha 0.99.
pub const TITLE_BG: [u8; 4] = [28, 28, 34, 252];
/// The 1 px border around the body, the title bar and each button.
/// This is rs2b0t's measured composite, drawn opaque.
pub const BORDER: [u8; 4] = [81, 81, 91, 255];
/// Body text.
pub const FG: [u8; 4] = [205, 211, 218, 255];
/// Dim text: the footer, inactive tabs and rail entries, the toggle
/// glyph, and a disabled control's label.
pub const FG_DIM: [u8; 4] = [138, 145, 154, 255];
/// The strip status text.
pub const STATUS: [u8; 4] = [111, 224, 139, 255];
/// White text: the active tabs-band entry and the active rail entry.
pub const WHITE: [u8; 4] = [255, 255, 255, 255];
/// A button's fill. This is rs2b0t's measured composite, drawn opaque.
pub const WIDGET: [u8; 4] = [47, 47, 54, 255];
/// A hovered or held button's fill, and an active tab's or rail
/// entry's fill. rs2b0t's measured composite, drawn opaque.
pub const WIDGET_HOT: [u8; 4] = [70, 70, 82, 255];
/// The accent a paint with no usable accent text uses: rs2b0t's
/// default `#7ad0ff`.
pub const DEFAULT_ACCENT: [u8; 3] = [122, 208, 255];

/// Parse an accent colour text, `"#rgb"` or `"#rrggbb"`, into sRGB
/// bytes. The parse reads the text once and allocates nothing. Text of
/// any other shape returns `None`.
pub fn parse_accent(text: &str) -> Option<[u8; 3]> {
    let hex = text.strip_prefix('#')?;
    let digit = |byte: u8| (byte as char).to_digit(16).map(|value| value as u8);
    let bytes = hex.as_bytes();
    match bytes {
        [r, g, b] => Some([digit(*r)? * 17, digit(*g)? * 17, digit(*b)? * 17]),
        [r0, r1, g0, g1, b0, b1] => Some([
            digit(*r0)? * 16 + digit(*r1)?,
            digit(*g0)? * 16 + digit(*g1)?,
            digit(*b0)? * 16 + digit(*b1)?,
        ]),
        _ => None,
    }
}

impl PaintView<'_> {
    /// The script's accent colour as sRGB bytes. `None` when the paint
    /// has no accent text, or the text is not `"#rgb"` or `"#rrggbb"`.
    /// Callers that must draw an accent use [`DEFAULT_ACCENT`] then.
    pub fn accent_rgb(&self) -> Option<[u8; 3]> {
        self.accent().and_then(parse_accent)
    }
}

#[cfg(test)]
mod tests {
    use super::parse_accent;

    #[test]
    fn accent_text_parses_short_and_long_hex() {
        assert_eq!(parse_accent("#5be05b"), Some([91, 224, 91]));
        assert_eq!(parse_accent("#7ad0ff"), Some([122, 208, 255]));
        assert_eq!(parse_accent("#5B0"), Some([85, 187, 0]));
        assert_eq!(parse_accent("#abc"), Some([170, 187, 204]));
    }

    #[test]
    fn accent_text_rejects_other_shapes() {
        assert_eq!(parse_accent("5be05b"), None);
        assert_eq!(parse_accent("#5be05"), None);
        assert_eq!(parse_accent("#5be05bb"), None);
        assert_eq!(parse_accent("#gggggg"), None);
        assert_eq!(parse_accent(""), None);
        assert_eq!(parse_accent("#"), None);
    }
}
