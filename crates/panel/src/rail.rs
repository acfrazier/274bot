//! Sidecar rail chrome: window geometry, the tile size, and the colour of
//! each member's status dot (its [`Light`] comes from the shared fleet row).

use dear_imgui_rs::Ui;
use frontend_core::Light;

/// Base logical width of the MultiBox sidecar at 100% scale.
pub const RAIL_W: f32 = 264.0;
/// Base logical tile draw size inside the rail, at 100% scale.
pub const TILE_W: f32 = 236.0;
pub const TILE_H: f32 = 155.0;
/// Default OS inner width without the rail (game + panel), in logical px.
pub const BASE_WINDOW_W: f32 = 1120.0;
/// Default OS inner height in logical px.
pub const BASE_WINDOW_H: f32 = 580.0;

/// Minimum inner size so the native 765×503 blit is not covered by the
/// 330px panel (and the 264px rail when open).
pub fn os_window_size(rail_open: bool) -> (f32, f32) {
    (
        BASE_WINDOW_W + if rail_open { RAIL_W } else { 0.0 },
        BASE_WINDOW_H,
    )
}

/// Next OS inner size when the rail opens or closes. Opening grows to
/// at least the rail-open minimum. Closing subtracts [`RAIL_W`] (the
/// MultiBox strip on the 274bot panel) and never goes below the
/// closed minimum — a window the operator stretched keeps the extra.
pub fn next_os_window_size(
    current: (f32, f32),
    rail_was_open: bool,
    rail_open: bool,
) -> (f32, f32) {
    let (need_w, need_h) = os_window_size(rail_open);
    if rail_was_open && !rail_open {
        ((current.0 - RAIL_W).max(need_w), current.1.max(need_h))
    } else {
        (current.0.max(need_w), current.1.max(need_h))
    }
}

/// Rail split so the sidecar stays [`RAIL_W`] × `scale` physical px.
pub fn rail_split_ratio(window_w: f32, scale: f32) -> f32 {
    (RAIL_W * scale / window_w.max(1.0)).clamp(0.05, 0.85)
}

/// Remove glyph (U+2717), drawn in `theme::ERROR` red.
pub const REMOVE_GLYPH: &str = "\u{2717}";
/// Fold the rail blit (squash the head). Operator may swap; see spec.
pub const FOLD_GLYPH: &str = "\u{2582}";
/// Unfold the rail blit (raise the head).
pub const UNFOLD_GLYPH: &str = "\u{2585}";

/// The cap dot's fill colour for a row's light (amber CRT palette).
pub fn light_rgb(light: Light) -> [f32; 4] {
    match light {
        Light::Grey => crate::theme::TEXT_DIM,
        Light::Red => crate::theme::ERROR,
        Light::Yellow => crate::theme::ACCENT,
        Light::Green => crate::theme::GREEN,
    }
}
/// Draw five filled squares in the U+2059 pattern for a member's light.
/// The marker consumes one text line and does not depend on font coverage.
/// Its size and origin are snapped in framebuffer pixels at any scale.
pub(crate) fn draw_status_dot(ui: &Ui, light: Light, width: f32) {
    const DOT_CENTERS: [[f32; 2]; 5] = [
        [0.28, 0.28],
        [0.72, 0.28],
        [0.50, 0.50],
        [0.28, 0.72],
        [0.72, 0.72],
    ];

    const DOT_SIZE: f32 = 3.0;
    let scale = crate::theme::scale_px(ui, 1.0);
    let logical_width = width / scale;
    let size = (DOT_SIZE * scale).round().max(1.0);
    let [x, y] = ui.cursor_screen_pos();
    let line_h = ui.text_line_height();
    let colour = light_rgb(light);
    {
        let draw_list = ui.get_window_draw_list();
        for [cx, cy] in DOT_CENTERS {
            let center = [logical_width * cx, crate::theme::PANEL_FONT_SIZE * cy];
            // Snap the base geometry before scaling its origin; a shared
            // physical size keeps fractional-DPI dots square.
            let logical_min = [
                (center[0] - DOT_SIZE * 0.5).round(),
                (center[1] - DOT_SIZE * 0.5).round(),
            ];
            let min = [
                (x + logical_min[0] * scale).round(),
                (y + logical_min[1] * scale).round(),
            ];
            let max = [min[0] + size, min[1] + size];
            draw_list.add_rect(min, max, colour).filled(true).build();
        }
    }
    ui.dummy([width, line_h]);
}

/// Whether this rail/grid tile shows its blit. Sidecar + `only_selected` stays
/// cap-only; grid + `only_selected` keeps the focused blit only (the grid *is*
/// the Game pane). Default: grid keeps every blit; the sidecar folds the
/// focused member (the Game pane already shows it). `is_focused` is whether
/// `name` is the focused member (callers read it under the focus lock).
pub fn rail_preview_open(
    name: &str,
    is_focused: bool,
    only_selected: bool,
    grid: bool,
    preview: &std::collections::HashMap<String, bool>,
) -> bool {
    if only_selected && !(grid && is_focused) {
        return false;
    }
    preview.get(name).copied().unwrap_or(grid || !is_focused)
}

#[cfg(test)]
mod tests {
    use super::{
        os_window_size, rail_preview_open, rail_split_ratio, BASE_WINDOW_H, BASE_WINDOW_W, RAIL_W,
    };
    use frontend_core::Light;

    #[test]
    fn rail_preview_defaults_fold_focused() {
        let empty = std::collections::HashMap::new();
        assert!(
            !rail_preview_open("a", true, false, false, &empty),
            "sidecar folds the focused blit by default"
        );
        assert!(
            rail_preview_open("b", false, false, false, &empty),
            "other members show a blit by default"
        );
        assert!(!rail_preview_open("b", false, true, false, &empty));
        assert!(
            rail_preview_open("a", true, false, true, &empty),
            "grid keeps the focused blit — there is no separate Game pane"
        );
        let mut on = std::collections::HashMap::new();
        on.insert("a".into(), true);
        assert!(rail_preview_open("a", true, false, false, &on));
    }

    #[test]
    fn rail_preview_only_selected_grid_keeps_focused_suppresses_others() {
        let empty = std::collections::HashMap::new();
        assert!(
            rail_preview_open("a", true, true, true, &empty),
            "grid + only_selected must show the focused cell blit"
        );
        assert!(
            !rail_preview_open("b", false, true, true, &empty),
            "grid + only_selected must suppress non-focused cells"
        );
        assert!(
            !rail_preview_open("a", true, true, false, &empty),
            "sidecar + only_selected stays cap-only even for focused"
        );
        let mut folded = std::collections::HashMap::new();
        folded.insert("a".into(), false);
        assert!(
            !rail_preview_open("a", true, true, true, &folded),
            "manual fold on focused grid cell is honored when only_selected"
        );
        folded.insert("a".into(), true);
        assert!(
            rail_preview_open("a", true, true, true, &folded),
            "manual unfold on focused grid cell is honored when only_selected"
        );
        folded.insert("b".into(), true);
        assert!(
            !rail_preview_open("b", false, true, true, &folded),
            "only_selected overrides a manual unfold on a non-focused grid cell"
        );
    }

    #[test]
    fn os_window_grows_by_rail_width_and_keeps_height() {
        assert_eq!(os_window_size(false), (BASE_WINDOW_W, BASE_WINDOW_H));
        assert_eq!(
            os_window_size(true),
            (BASE_WINDOW_W + RAIL_W, BASE_WINDOW_H)
        );
        for scale in [1.0, 1.25, 1.5, 1.75, 2.0] {
            let width = 2000.0 * scale;
            let ratio = rail_split_ratio(width, scale);
            assert!(
                (ratio * width - RAIL_W * scale).abs() < 0.01,
                "rail stays {} physical px at {scale}× in a {width}px window",
                RAIL_W * scale
            );
        }
    }

    #[test]
    fn next_os_window_size_shrinks_when_the_rail_closes() {
        let base = os_window_size(false);
        let rail = os_window_size(true);
        assert_eq!(
            super::next_os_window_size(base, false, true),
            rail,
            "MultiBox on grows by the strip"
        );
        assert_eq!(
            super::next_os_window_size(rail, true, false),
            base,
            "MultiBox off on the 274bot panel re-shrinks by the strip"
        );
        let stretched = (rail.0 + 80.0, rail.1);
        assert_eq!(
            super::next_os_window_size(stretched, true, false),
            (base.0 + 80.0, base.1),
            "an operator-stretched window keeps the extra after the strip closes"
        );
        assert_eq!(
            super::next_os_window_size(stretched, false, false),
            stretched,
            "already-closed must not keep subtracting RAIL_W every frame"
        );
    }

    #[test]
    fn status_dot_draws_five_pixel_aligned_squares_at_every_ui_scale() {
        let _guard = crate::test_support::imgui_context_guard();
        let light = Light::Yellow;

        for scale in [1.0, 1.25, 1.5, 1.75, 2.0] {
            let mut ctx = dear_imgui_rs::Context::create();
            crate::app::apply_ui_scale(ctx.style_mut(), scale);
            let _ = ctx.font_atlas_mut().build();
            ctx.io_mut().set_display_size([128.0 * scale, 96.0 * scale]);
            ctx.io_mut().set_display_framebuffer_scale([1.0, 1.0]);
            ctx.io_mut().set_delta_time(1.0 / 60.0);

            let mut row = None;
            {
                let ui = ctx.frame();
                ui.window("##status-dot-test")
                    .position([0.0, 0.0], dear_imgui_rs::Condition::Always)
                    .size(
                        [80.0 * scale, 48.0 * scale],
                        dear_imgui_rs::Condition::Always,
                    )
                    .flags(
                        dear_imgui_rs::WindowFlags::NO_TITLE_BAR
                            | dear_imgui_rs::WindowFlags::NO_RESIZE
                            | dear_imgui_rs::WindowFlags::NO_MOVE
                            | dear_imgui_rs::WindowFlags::NO_SAVED_SETTINGS
                            | dear_imgui_rs::WindowFlags::NO_BACKGROUND,
                    )
                    .build(|| {
                        row = Some((ui.cursor_screen_pos(), ui.text_line_height()));
                        super::draw_status_dot(ui, light, crate::theme::scale_px(ui, 18.0));
                    });
            }
            let (origin, line_h) = row.expect("the status-dot test window was drawn");
            let draw_data = ctx.render();
            assert_eq!(draw_data.framebuffer_scale(), [1.0, 1.0]);
            let vertices: Vec<_> = draw_data
                .draw_lists()
                .flat_map(|list| list.vtx_buffer().iter())
                .collect();
            let indices = draw_data
                .draw_lists()
                .map(|list| list.idx_buffer().len())
                .sum::<usize>();
            assert_eq!(vertices.len(), 5 * 4, "five rectangle quads are drawn");
            assert_eq!(indices, 5 * 6, "five rectangles have six indices each");

            let expected_colour = super::light_rgb(light);
            let mut physical_sizes = Vec::with_capacity(5);
            for rectangle in vertices.as_chunks::<4>().0 {
                assert!(
                    rectangle.iter().all(|vertex| {
                        let actual = dear_imgui_rs::Color::from_imgui_u32(vertex.col).to_array();
                        actual
                            .into_iter()
                            .zip(expected_colour)
                            .all(|(actual, expected)| (actual - expected).abs() <= 1.0 / 255.0)
                    }),
                    "every square uses Light::rgb"
                );
                let min_y = rectangle
                    .iter()
                    .map(|vertex| vertex.pos[1])
                    .fold(f32::INFINITY, f32::min);
                let max_y = rectangle
                    .iter()
                    .map(|vertex| vertex.pos[1])
                    .fold(f32::NEG_INFINITY, f32::max);
                assert!(
                    min_y >= origin[1] && max_y <= origin[1] + line_h,
                    "square y range {min_y}..{max_y} stays inside line {origin_y}..{}",
                    origin[1] + line_h,
                    origin_y = origin[1]
                );
                let min_x = rectangle
                    .iter()
                    .map(|vertex| vertex.pos[0])
                    .fold(f32::INFINITY, f32::min);
                let max_x = rectangle
                    .iter()
                    .map(|vertex| vertex.pos[0])
                    .fold(f32::NEG_INFINITY, f32::max);
                let physical_bounds = [min_x, min_y, max_x, max_y];
                assert!(
                    physical_bounds
                        .iter()
                        .all(|coordinate| (*coordinate - coordinate.round()).abs() < 0.001),
                    "all square origins and edges must be integer physical pixels at {scale}×: {physical_bounds:?}"
                );
                let physical_width = max_x - min_x;
                let physical_height = max_y - min_y;
                assert!(
                    (physical_width - physical_height).abs() < 0.001,
                    "each square must stay square in physical pixels at {scale}×: {physical_width}×{physical_height}"
                );
                assert!(
                    (physical_width - physical_width.round()).abs() < 0.001,
                    "square size must be an integer physical width at {scale}×: {physical_width}"
                );
                physical_sizes.push(physical_width.round() as i32);
            }
            assert!(
                physical_sizes.iter().all(|size| *size == physical_sizes[0]),
                "all five squares must have equal physical sizes at {scale}×: {physical_sizes:?}"
            );
        }
    }
}
