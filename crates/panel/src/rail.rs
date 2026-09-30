//! Sidecar rail chrome: window geometry, the tile size, and the colour of
//! each member's status dot (its [`Light`] comes from the shared fleet row).

use dear_imgui_rs::Ui;
use frontend_core::Light;

/// Width of the MultiBox sidecar rail (rs2b0t's 264px strip).
pub const RAIL_W: f32 = 264.0;
/// Cap-body tile draw size inside the rail (rs2b0t ~236×155).
pub const TILE_W: f32 = 236.0;
pub const TILE_H: f32 = 155.0;
/// Default OS window without the rail (game + 330 chrome).
pub const BASE_WINDOW_W: f32 = 1120.0;
/// Default OS window height.
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

/// Rail split so the sidecar stays [`RAIL_W`] px at `window_w`.
pub fn rail_split_ratio(window_w: f32) -> f32 {
    (RAIL_W / window_w.max(1.0)).clamp(0.05, 0.85)
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
pub(crate) fn draw_status_dot(ui: &Ui, light: Light, width: f32) {
    const DOT_CENTERS: [[f32; 2]; 5] = [
        [0.28, 0.28],
        [0.72, 0.28],
        [0.50, 0.50],
        [0.28, 0.72],
        [0.72, 0.72],
    ];

    let line_h = ui.text_line_height();
    let [x, y] = ui.cursor_screen_pos();
    let half = line_h * 0.11;
    let colour = light_rgb(light);
    {
        let draw_list = ui.get_window_draw_list();
        for [cx, cy] in DOT_CENTERS {
            let center = [x + width * cx, y + line_h * cy];
            draw_list
                .add_rect(
                    [center[0] - half, center[1] - half],
                    [center[0] + half, center[1] + half],
                    colour,
                )
                .filled(true)
                .build();
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
        TILE_H, TILE_W,
    };
    use frontend_core::Light;

    #[test]
    fn rail_constants_match_the_plan() {
        assert_eq!(RAIL_W, 264.0);
        assert_eq!(TILE_W, 236.0);
        assert_eq!(TILE_H, 155.0);
        assert_eq!(crate::theme::RAIL_WINDOW, "274bot-rail");
    }

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
        let r = rail_split_ratio(2000.0);
        assert!((r * 2000.0 - RAIL_W).abs() < 0.01);
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
    fn status_dot_draws_five_light_colored_squares_inside_one_line() {
        let _guard = crate::test_support::imgui_context_guard();
        let mut ctx = dear_imgui_rs::Context::create();
        let _ = ctx.font_atlas_mut().build();
        ctx.io_mut().set_display_size([128.0, 96.0]);
        ctx.io_mut().set_delta_time(1.0 / 60.0);

        let light = Light::Yellow;
        let width = 18.0;
        let mut row = None;
        {
            let ui = ctx.frame();
            ui.window("##status-dot-test")
                .position([0.0, 0.0], dear_imgui_rs::Condition::Always)
                .size([80.0, 48.0], dear_imgui_rs::Condition::Always)
                .flags(
                    dear_imgui_rs::WindowFlags::NO_TITLE_BAR
                        | dear_imgui_rs::WindowFlags::NO_RESIZE
                        | dear_imgui_rs::WindowFlags::NO_MOVE
                        | dear_imgui_rs::WindowFlags::NO_SAVED_SETTINGS
                        | dear_imgui_rs::WindowFlags::NO_BACKGROUND,
                )
                .build(|| {
                    row = Some((ui.cursor_screen_pos(), ui.text_line_height()));
                    super::draw_status_dot(ui, light, width);
                });
        }
        let (origin, line_h) = row.expect("the status-dot test window was drawn");
        let draw_data = ctx.render();
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
            assert!(
                ((max_x - min_x) - (max_y - min_y)).abs() < 0.01,
                "each marker rectangle is square"
            );
        }
    }
}
