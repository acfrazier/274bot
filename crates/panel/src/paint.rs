//! Script-paint overlay for the Game Image.
//!
//! The focused slot's `ScriptPaint` frame renders in an ImGui window
//! parked over the client's chatbox rect — an overlay window, never
//! pixels on the 765×503 game texture. The `–` title row collapses to
//! title-only; `+` expands (rs2b0t `paint:collapsed`, view-local, not a
//! script setting). Outside the overlay rect, clicks fall through to the
//! game unchanged.

use dear_imgui_rs::{Condition, MouseButton, Ui, WindowFlags};
use script::shim::ScriptPaint;

use crate::game_view::{APPLET_H, APPLET_W};

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

/// Stable window id (hidden name; the `–`/`+` title row is the toggle).
const PAINT_WINDOW: &str = "##274-script-paint";
/// Pinned overlay: no decoration, no move/resize, no scroll. The window
/// covers exactly the chatbox rect, so input outside it is untouched.
fn paint_flags() -> WindowFlags {
    WindowFlags::NO_TITLE_BAR
        | WindowFlags::NO_MOVE
        | WindowFlags::NO_RESIZE
        | WindowFlags::NO_SCROLLBAR
        | WindowFlags::NO_SCROLL_WITH_MOUSE
        | WindowFlags::NO_BRING_TO_FRONT_ON_FOCUS
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
        let row_h = ui.frame_height();
        // The title row doubles as the collapse toggle, so the click target
        // is the full header strip (rect hit-test + the raw click flag —
        // same GPU-less pattern as the picker canvas).
        let title_row = [x, y, x + w, y + row_h];
        let height = if self.collapsed { row_h } else { h };
        ui.window(PAINT_WINDOW)
            .flags(paint_flags())
            .position([x, y], Condition::Always)
            .size([w, height], Condition::Always)
            .build(|| {
                if ui.is_mouse_hovering_rect(
                    [title_row[0], title_row[1]],
                    [title_row[2], title_row[3]],
                ) && ui.is_mouse_clicked(MouseButton::Left)
                {
                    self.collapsed = !self.collapsed;
                }
                let glyph = if self.collapsed { "+" } else { "–" };
                ui.text(glyph);
                if !self.collapsed {
                    // Paint text: title then rows (the rs2b0t paint shape).
                    if let Some(title) = &paint.title {
                        ui.same_line();
                        ui.text(title);
                        self.lines.push(title.clone());
                    }
                    for row in &paint.lines {
                        ui.text(row);
                        self.lines.push(row.clone());
                    }
                } else if let Some(title) = &paint.title {
                    // Title-only: the collapse bar keeps the paint title.
                    ui.same_line();
                    ui.text(title);
                }
            });
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

    use super::{chatbox_rect, PaintOverlay, CHATBOX, PAINT_WINDOW};

    fn paint(title: Option<&str>, lines: &[&str]) -> ScriptPaint {
        ScriptPaint {
            title: title.map(str::to_string),
            accent: None,
            lines: lines.iter().map(|l| l.to_string()).collect(),
        }
    }

    /// The paint window's `(pos, size)` read straight from the ImGui
    /// window entry (GPU-less: no renderer, just the context state).
    fn window_rect(name: &str) -> ([f32; 2], [f32; 2]) {
        let cname = std::ffi::CString::new(name).unwrap();
        let win = unsafe { dear_imgui_rs::sys::igFindWindowByName(cname.as_ptr()) };
        assert!(!win.is_null(), "paint window was not created");
        let win = unsafe { &*win };
        ([win.Pos.x, win.Pos.y], [win.Size.x, win.Size.y])
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
            overlay.frame(ui, Some(&p), [10.0, 20.0], [765.0, 503.0]);
        }
        ctx.render();
        assert_eq!(
            overlay.lines,
            vec![
                "BoneBurier".to_string(),
                "a row".to_string(),
                "second row".to_string()
            ],
            "the paint title and rows are in the window text"
        );
        let (pos, size) = window_rect(PAINT_WINDOW);
        assert_eq!(pos, [18.0, 365.0], "overlay sits over the chatbox origin");
        assert!(
            (size[0] - 506.0).abs() < 2.0,
            "overlay spans the chatbox width"
        );
        assert!(
            (size[1] - 150.0).abs() < 2.0,
            "overlay spans the chatbox height"
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
        assert!(overlay.lines.is_empty(), "no paint -> no window text");
        let cname = std::ffi::CString::new(PAINT_WINDOW).unwrap();
        assert!(
            unsafe { dear_imgui_rs::sys::igFindWindowByName(cname.as_ptr()) }.is_null(),
            "no paint -> no overlay window (click-through unchanged)"
        );
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
            overlay.frame(ui, Some(p), [10.0, 20.0], [765.0, 503.0]);
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
        let (pos, size) = window_rect(PAINT_WINDOW);
        assert_eq!(pos, [18.0, 365.0], "collapsed bar stays over the chatbox");
        assert!(
            size[1] < 60.0,
            "collapsed rect is title-height, got {}",
            size[1]
        );
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
