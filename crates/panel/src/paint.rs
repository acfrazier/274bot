//! Script-paint overlay for the Game Image.
//!
//! Drawn on the Game window draw list over the client's chatbox rect —
//! never a second ImGui window (those dock into the Game node and vanish)
//! and never pixels on the 765×503 game texture. The title row collapses
//! to title-only (rs2b0t `paint:collapsed`, view-local).

use dear_imgui_rs::{MouseButton, Ui};
use script::shim::ScriptPaint;

use crate::game_view::{APPLET_H, APPLET_W};
use crate::theme::{ACCENT, BG_DEEP, TEXT};

/// Applet-space chatbox rect `(x, y, w, h)` the client reserves for game
/// chat on the 765×503 stage.
pub const CHATBOX: [f32; 4] = [8.0, 345.0, 506.0, 150.0];

/// Map the applet-space chatbox onto the Game Image's display rect
/// (`min` = the Image widget's top-left corner, `size` = its display
/// size, which is the native 765×503 in single-bot mode).
pub fn chatbox_rect(min: [f32; 2], size: [f32; 2]) -> [f32; 4] {
    let sx = size[0] / APPLET_W as f32;
    let sy = size[1] / APPLET_H as f32;
    [
        min[0] + CHATBOX[0] * sx,
        min[1] + CHATBOX[1] * sy,
        CHATBOX[2] * sx,
        CHATBOX[3] * sy,
    ]
}

/// Cached script-paint overlay for the focused slot.
pub struct PaintOverlay {
    /// `true` = title-only (rs2b0t `paint:collapsed`). View-local: reset
    /// when the paint goes away.
    collapsed: bool,
    /// The paint's title + rows submitted this frame (empty while
    /// collapsed or when no paint is showing). The GPU-less tests assert
    /// on this mirror of the window text.
    lines: Vec<String>,
}

impl PaintOverlay {
    pub fn new() -> Self {
        Self {
            collapsed: false,
            lines: Vec::new(),
        }
    }

    /// Draw the focused slot's paint over the Game Image. `min`/`size`
    /// are the Image widget's display rect. No-op without a paint.
    pub fn frame(&mut self, ui: &Ui, paint: Option<&ScriptPaint>, min: [f32; 2], size: [f32; 2]) {
        self.lines.clear();
        let Some(paint) = paint.filter(|p| p.title.is_some() || !p.lines.is_empty()) else {
            self.collapsed = false;
            return;
        };
        let [x, y, w, h] = chatbox_rect(min, size);
        let row_h = ui.frame_height().max(16.0);
        let height = if self.collapsed { row_h } else { h };
        if ui.is_mouse_hovering_rect([x, y], [x + w, y + row_h])
            && ui.is_mouse_clicked(MouseButton::Left)
        {
            self.collapsed = !self.collapsed;
        }
        let dl = ui.get_window_draw_list();
        dl.add_rect(
            [x, y],
            [x + w, y + height],
            [BG_DEEP[0], BG_DEEP[1], BG_DEEP[2], 0.92],
        )
        .filled(true)
        .build();
        dl.add_rect([x, y], [x + w, y + height], ACCENT)
            .thickness(1.0)
            .build();
        let glyph = if self.collapsed { "+" } else { "–" };
        let header = match &paint.title {
            Some(t) => format!("{glyph} {t}"),
            None => glyph.to_string(),
        };
        dl.add_text([x + 6.0, y + 2.0], ACCENT, &header);
        if !self.collapsed {
            if let Some(title) = &paint.title {
                self.lines.push(title.clone());
            }
            let mut ty = y + row_h;
            for row in &paint.lines {
                self.lines.push(row.clone());
                if ty + 14.0 <= y + height {
                    dl.add_text([x + 6.0, ty], TEXT, row);
                    ty += 14.0;
                }
            }
        }
    }
}

impl Default for PaintOverlay {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use dear_imgui_rs::FramePrepareOptions;
    use script::shim::ScriptPaint;

    use super::{chatbox_rect, PaintOverlay, CHATBOX};

    fn paint(title: Option<&str>, lines: &[&str]) -> ScriptPaint {
        ScriptPaint {
            title: title.map(str::to_string),
            accent: None,
            lines: lines.iter().map(|l| l.to_string()).collect(),
        }
    }

    fn prepare_frame(ctx: &mut dear_imgui_rs::Context) {
        ctx.prepare_frame(
            FramePrepareOptions::new([900.0, 700.0], 1.0 / 60.0).renderer_has_textures(),
        );
    }

    #[test]
    fn chatbox_rect_maps_the_game_image_to_the_client_chatbox() {
        // Native 765×503 blit: the chatbox keeps its applet-space spot.
        assert_eq!(
            chatbox_rect([10.0, 20.0], [765.0, 503.0]),
            [18.0, 365.0, 506.0, 150.0]
        );
        // A scaled blit (grid mode) scales the chatbox with the image.
        let [x, y, w, h] = chatbox_rect([0.0, 0.0], [382.5, 251.5]);
        assert!((x - CHATBOX[0] * 0.5).abs() < 0.01);
        assert!((y - CHATBOX[1] * 0.5).abs() < 0.01);
        assert!((w - CHATBOX[2] * 0.5).abs() < 0.01);
        assert!((h - CHATBOX[3] * 0.5).abs() < 0.01);
    }

    #[test]
    fn paint_window_shows_title_and_lines_over_the_chatbox() {
        let _guard = crate::IMGUI_CTX_TEST_GUARD.lock().unwrap();
        let mut ctx = dear_imgui_rs::Context::create();
        let mut overlay = PaintOverlay::new();
        let p = paint(Some("BoneBurier"), &["a row", "second row"]);
        prepare_frame(&mut ctx);
        {
            let ui = ctx.frame();
            let _ = ui
                .window("Game")
                .position([0.0, 0.0], dear_imgui_rs::Condition::Always)
                .size([900.0, 700.0], dear_imgui_rs::Condition::Always)
                .build(|| {
                    overlay.frame(ui, Some(&p), [10.0, 20.0], [765.0, 503.0]);
                });
        }
        ctx.render();
        assert_eq!(
            overlay.lines,
            vec![
                "BoneBurier".to_string(),
                "a row".to_string(),
                "second row".to_string()
            ],
            "the paint title and rows are in the overlay text"
        );
    }

    #[test]
    fn paint_window_hides_without_a_paint() {
        let _guard = crate::IMGUI_CTX_TEST_GUARD.lock().unwrap();
        let mut ctx = dear_imgui_rs::Context::create();
        let mut overlay = PaintOverlay::new();
        prepare_frame(&mut ctx);
        {
            let ui = ctx.frame();
            overlay.frame(ui, None, [10.0, 20.0], [765.0, 503.0]);
        }
        ctx.render();
        assert!(overlay.lines.is_empty(), "no paint -> no overlay text");
    }

    /// One frame with the overlay up; `mouse` + `left_down` simulate the
    /// pointer for the `–`/`+` title-row click.
    fn paint_click_frame(
        ctx: &mut dear_imgui_rs::Context,
        overlay: &mut PaintOverlay,
        p: &ScriptPaint,
        mouse: Option<[f32; 2]>,
        left_down: bool,
    ) {
        prepare_frame(ctx);
        if let Some(m) = mouse {
            ctx.io_mut().add_mouse_pos_event(m);
        }
        ctx.io_mut()
            .add_mouse_button_event(dear_imgui_rs::MouseButton::Left, left_down);
        {
            let ui = ctx.frame();
            let _ = ui
                .window("Game")
                .position([0.0, 0.0], dear_imgui_rs::Condition::Always)
                .size([900.0, 700.0], dear_imgui_rs::Condition::Always)
                .build(|| {
                    overlay.frame(ui, Some(p), [10.0, 20.0], [765.0, 503.0]);
                });
        }
        ctx.render();
    }

    #[test]
    fn collapse_click_hides_body_and_shrinks_to_title_height() {
        let _guard = crate::IMGUI_CTX_TEST_GUARD.lock().unwrap();
        let mut ctx = dear_imgui_rs::Context::create();
        let mut overlay = PaintOverlay::new();
        let p = paint(Some("BoneBurier"), &["a row", "second row"]);
        // The title row spans the chatbox top strip: hover it, then press.
        let title = [30.0, 380.0];
        paint_click_frame(&mut ctx, &mut overlay, &p, Some(title), false);
        assert!(!overlay.collapsed, "expanded by default");
        paint_click_frame(&mut ctx, &mut overlay, &p, Some(title), true);
        assert!(overlay.collapsed, "`–` collapses to title-only");
        paint_click_frame(&mut ctx, &mut overlay, &p, Some(title), false);
        assert!(
            overlay.lines.is_empty(),
            "collapsed: body rows are not shown"
        );
        assert!(overlay.collapsed);
        // The `+` bar expands again.
        paint_click_frame(&mut ctx, &mut overlay, &p, Some(title), true);
        assert!(!overlay.collapsed, "`+` expands back");
        assert_eq!(
            overlay.lines,
            vec![
                "BoneBurier".to_string(),
                "a row".to_string(),
                "second row".to_string()
            ]
        );
    }
}
