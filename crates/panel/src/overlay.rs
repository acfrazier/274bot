//! Queue-card overlay for the Game Image.
//!
//! The armed route's remaining tiles are painted by the client's 3D renderer
//! and on the pack map, so this overlay draws only the queue card belonging
//! to the bot shown in each Image.

use dear_imgui_rs::Ui;

use crate::queue_card::{queue_ahead_label, queue_k_of_n, QUEUE_CARD_TITLE};
use crate::theme::ACCENT;

/// Amber queue-card lines for one slot's FIFO place: title, `k of n`,
/// ahead label. Empty when the slot is not queued, so the card disappears
/// the moment the grant lands (`logging in…`).
fn queue_card_lines(queue: Option<(i32, i32)>) -> Vec<String> {
    let Some((position, total)) = queue else {
        return Vec::new();
    };
    let Some(place) = queue_k_of_n(position, total) else {
        return Vec::new();
    };
    vec![
        QUEUE_CARD_TITLE.to_string(),
        place,
        queue_ahead_label(position as u32),
    ]
}

/// Horizontal/vertical pad inside the queue card border (equal L/R and T/B).
const QUEUE_CARD_PAD: f32 = 8.0;
/// Inset of the card's outer border from the Image top-left.
const QUEUE_CARD_OUTER: f32 = 8.0;
/// Extra leading between measured lines (on top of the active font size).
const QUEUE_CARD_LINE_GAP: f32 = 2.0;

/// Measured content bounds for the queue card at the active font/UI scale.
/// Returns `(content_w, line_h, box_w, box_h)`. Empty `lines` yields zeros.
fn queue_card_metrics(ui: &Ui, lines: &[String]) -> (f32, f32, f32, f32) {
    if lines.is_empty() {
        return (0.0, 0.0, 0.0, 0.0);
    }
    let font = ui.current_font();
    let font_sz = ui.current_font_size().max(1.0);
    let mut content_w = 0.0_f32;
    let mut line_h = font_sz;
    for line in lines {
        let sz = font.calc_text_size(font_sz, f32::MAX, 0.0, line);
        content_w = content_w.max(sz[0]);
        line_h = line_h.max(sz[1].max(font_sz));
    }
    line_h += QUEUE_CARD_LINE_GAP;
    let content_h = line_h * lines.len() as f32;
    let box_w = content_w + QUEUE_CARD_PAD * 2.0;
    let box_h = content_h + QUEUE_CARD_PAD * 2.0;
    (content_w, line_h, box_w, box_h)
}

/// Draw the queue card as a dark amber-bordered block over the Image.
/// `min` is the Image's top-left corner. Width/height come from the active
/// font measurement so left and right padding stay equal.
fn draw_queue_card(ui: &Ui, min: [f32; 2], lines: &[String]) {
    let (_content_w, line_h, box_w, box_h) = queue_card_metrics(ui, lines);
    let box_min = [min[0] + QUEUE_CARD_OUTER, min[1] + QUEUE_CARD_OUTER];
    let box_max = [box_min[0] + box_w, box_min[1] + box_h];
    let text_origin = [box_min[0] + QUEUE_CARD_PAD, box_min[1] + QUEUE_CARD_PAD];
    let dl = ui.get_window_draw_list();
    dl.add_rect(box_min, box_max, [0.0, 0.0, 0.0, 0.6])
        .filled(true)
        .build();
    dl.add_rect(box_min, box_max, ACCENT).thickness(1.0).build();
    for (i, line) in lines.iter().enumerate() {
        dl.add_text(
            [text_origin[0], text_origin[1] + i as f32 * line_h],
            ACCENT,
            line,
        );
    }
}

/// Queue card over one bot's Image. No-op when that bot is not queued, so
/// the card disappears the moment the grant lands.
pub fn draw_queue_card_for(ui: &Ui, queue: Option<(i32, i32)>, min: [f32; 2]) {
    let lines = queue_card_lines(queue);
    if !lines.is_empty() {
        draw_queue_card(ui, min, &lines);
    }
}

/// Cached queue-card state for the bot shown by this overlay. The card must
/// appear immediately on enqueue, so it is cached on the FIFO tuple, not a
/// timer.
pub struct PathOverlay {
    queue: Option<(i32, i32)>,
    queue_lines: Vec<String>,
}

impl PathOverlay {
    pub fn new() -> Self {
        Self {
            queue: None,
            queue_lines: Vec::new(),
        }
    }

    /// Always empty: the path polyline is gone — the client paints the
    /// route in 3D and the pack map shows the tiles.
    pub fn points(&self) -> &[[f32; 2]] {
        &[]
    }

    /// Draw the displayed bot's queue card over the Image. `min` is the
    /// Image widget's top-left corner; `size` is unused now that the
    /// polyline is gone.
    pub fn frame(&mut self, ui: &Ui, queue: Option<(i32, i32)>, min: [f32; 2], _size: [f32; 2]) {
        if queue != self.queue {
            self.queue = queue;
            self.queue_lines = queue_card_lines(queue);
        }
        if !self.queue_lines.is_empty() {
            draw_queue_card(ui, min, &self.queue_lines);
        }
    }
}

impl Default for PathOverlay {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        draw_queue_card, queue_card_lines, queue_card_metrics, PathOverlay, QUEUE_CARD_OUTER,
        QUEUE_CARD_PAD,
    };
    use crate::session::Session;

    #[test]
    fn queue_card_lines_match_rs2b0t_copy() {
        assert_eq!(queue_card_lines(None), Vec::<String>::new());
        let lines = queue_card_lines(Some((1, 2)));
        assert_eq!(lines[0], "AUTO-LOGIN QUEUE");
        assert_eq!(lines[1], "1 of 2");
        assert_eq!(lines[2], "0 bots in front");
        let lines = queue_card_lines(Some((2, 2)));
        assert_eq!(lines[2], "1 bot in front");
    }

    #[test]
    fn invalid_queue_tuple_does_not_paint_a_card() {
        assert!(queue_card_lines(Some((3, 0))).is_empty());
        assert!(queue_card_lines(Some((3, 2))).is_empty());
    }

    #[test]
    fn each_bot_view_keeps_its_own_queue_place_when_focus_changes() {
        let _guard = crate::test_support::imgui_context_guard();
        let mut ctx = dear_imgui_rs::Context::create();
        ctx.prepare_frame(
            dear_imgui_rs::FramePrepareOptions::new([900.0, 700.0], 1.0 / 60.0)
                .renderer_has_textures(),
        );
        let ui = ctx.frame();
        let mut s = Session::new();
        s.statuses.push(host_play::SlotStatus {
            username: "alice".into(),
            queue_position: 1,
            queue_total: 3,
            ..host_play::SlotStatus::default()
        });
        s.statuses.push(host_play::SlotStatus {
            username: "bob".into(),
            queue_position: 2,
            queue_total: 3,
            ..host_play::SlotStatus::default()
        });
        s.set_focus_for_test("alice");
        let mut alice = PathOverlay::new();
        let mut bob = PathOverlay::new();
        ui.window("##overlay-alice").build(|| {
            alice.frame(ui, s.queue_for("alice"), [10.0, 10.0], [90.0, 90.0]);
        });
        ui.window("##overlay-bob").build(|| {
            bob.frame(ui, s.queue_for("bob"), [110.0, 10.0], [90.0, 90.0]);
        });
        s.set_focus_for_test("bob");
        ui.window("##overlay-alice-after-focus").build(|| {
            alice.frame(ui, s.queue_for("alice"), [10.0, 110.0], [90.0, 90.0]);
        });
        ui.window("##overlay-bob-after-focus").build(|| {
            bob.frame(ui, s.queue_for("bob"), [110.0, 110.0], [90.0, 90.0]);
        });
        ctx.render();
        assert_eq!(alice.queue_lines[1], "1 of 3");
        assert_eq!(bob.queue_lines[1], "2 of 3");
    }

    #[test]
    fn slot_without_queue_does_not_borrow_another_slots_card() {
        let _guard = crate::test_support::imgui_context_guard();
        let mut ctx = dear_imgui_rs::Context::create();
        ctx.prepare_frame(
            dear_imgui_rs::FramePrepareOptions::new([900.0, 700.0], 1.0 / 60.0)
                .renderer_has_textures(),
        );
        let ui = ctx.frame();
        let mut s = Session::new();
        s.set_focus_for_test("s00");
        s.statuses.push(host_play::SlotStatus {
            username: "s00".into(),
            ..host_play::SlotStatus::default()
        });
        s.statuses.push(host_play::SlotStatus {
            username: "s01".into(),
            queue_position: 1,
            queue_total: 49,
            ..host_play::SlotStatus::default()
        });
        let mut overlay = PathOverlay::new();
        ui.window("##overlay-queue-fifo-test").build(|| {
            overlay.frame(ui, s.queue_for("s00"), [10.0, 10.0], [90.0, 90.0]);
        });
        ctx.render();
        assert!(overlay.queue_lines.is_empty());
    }

    #[test]
    fn overlay_skips_queue_card_when_not_queued() {
        let _guard = crate::test_support::imgui_context_guard();
        let mut ctx = dear_imgui_rs::Context::create();
        ctx.prepare_frame(
            dear_imgui_rs::FramePrepareOptions::new([900.0, 700.0], 1.0 / 60.0)
                .renderer_has_textures(),
        );
        let ui = ctx.frame();
        let mut s = Session::new();
        s.set_focus_for_test("alice");
        let mut overlay = PathOverlay::new();
        ui.window("##overlay-queue-test").build(|| {
            overlay.frame(ui, None, [10.0, 10.0], [90.0, 90.0]);
        });
        ctx.render();
        assert!(overlay.queue_lines.is_empty(), "no queue -> no card");
    }

    #[test]
    fn queue_card_metrics_use_measured_width_with_equal_padding() {
        let _guard = crate::test_support::imgui_context_guard();
        let mut ctx = dear_imgui_rs::Context::create();
        ctx.prepare_frame(
            dear_imgui_rs::FramePrepareOptions::new([900.0, 700.0], 1.0 / 60.0)
                .renderer_has_textures(),
        );
        let ui = ctx.frame();
        let lines = queue_card_lines(Some((1, 2)));
        let (content_w, line_h, box_w, box_h) = queue_card_metrics(ui, &lines);
        assert!(content_w > 0.0, "title must measure wider than zero");
        assert!(
            (box_w - (content_w + QUEUE_CARD_PAD * 2.0)).abs() < 0.01,
            "box width is content plus equal left/right pad"
        );
        assert!(
            (box_h - (line_h * lines.len() as f32 + QUEUE_CARD_PAD * 2.0)).abs() < 0.01,
            "box height is lines plus equal top/bottom pad"
        );
        // Longest line is the title; char*7 estimate is not the source of width.
        let title_est = lines[0].len() as f32 * 7.0;
        let font = ui.current_font();
        let measured = font.calc_text_size(ui.current_font_size(), f32::MAX, 0.0, &lines[0])[0];
        assert!(
            (content_w - measured).abs() < 0.01,
            "content width tracks font measurement, not a fixed glyph estimate"
        );
        assert!(
            (content_w - title_est).abs() > 0.5 || measured > 0.0,
            "measurement path is active ({content_w} vs est {title_est})"
        );
        ui.window("##queue-card-metrics").build(|| {
            draw_queue_card(ui, [10.0, 10.0], &lines);
        });
        let _ = QUEUE_CARD_OUTER;
        ctx.render();
    }

    #[test]
    fn queue_card_metrics_track_longer_k_of_n_counts() {
        let _guard = crate::test_support::imgui_context_guard();
        let mut ctx = dear_imgui_rs::Context::create();
        ctx.prepare_frame(
            dear_imgui_rs::FramePrepareOptions::new([900.0, 700.0], 1.0 / 60.0)
                .renderer_has_textures(),
        );
        let ui = ctx.frame();
        let short = queue_card_lines(Some((1, 2)));
        let long = queue_card_lines(Some((12, 49)));
        let (w_short, _, box_short, _) = queue_card_metrics(ui, &short);
        let (w_long, _, box_long, _) = queue_card_metrics(ui, &long);
        // Title still dominates both; pads stay equal either way.
        assert!((box_short - (w_short + QUEUE_CARD_PAD * 2.0)).abs() < 0.01);
        assert!((box_long - (w_long + QUEUE_CARD_PAD * 2.0)).abs() < 0.01);
        assert!(
            box_long + 0.01 >= box_short,
            "wider k-of-n must not shrink the measured box"
        );
        ctx.render();
    }
}
