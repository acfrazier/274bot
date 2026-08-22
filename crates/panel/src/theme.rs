/// Amber accent color (#FFB000) for the 274 panel.
pub const ACCENT: [f32; 4] = [1.0, 176.0 / 255.0, 0.0, 1.0]; // #FFB000
/// Hover variant of the amber accent (#FFC14D).
pub const ACCENT_HOVER: [f32; 4] = [1.0, 193.0 / 255.0, 77.0 / 255.0, 1.0]; // #FFC14D
/// Panel background (#111).
pub const BG: [f32; 4] = [0.067, 0.067, 0.067, 1.0]; // #111
/// Panel window title.
pub const TITLE: &str = "274bot";

/// Integer UI scale for ImGui chrome + Game Image **display**. Never mutates 765×503.
pub fn integer_ui_scale(dpi: f32) -> f32 {
    dpi.max(1.0).round().max(1.0)
}
