//! Queue-card overlay for the Game Image.
//!
//! The armed route's remaining tiles are painted by the client's 3D renderer
//! and on the pack map, so this overlay draws only the queue card belonging
//! to the bot shown in each Image. The place itself comes from the shared
//! fleet row ([`QueuePlace`]), and so does the head's reason for waiting;
//! only the rs2b0t card copy lives here.

use std::collections::HashMap;
use std::time::Instant;

use dear_imgui_rs::Ui;
use frontend_core::QueuePlace;

use crate::theme::{scale_px, ACCENT};

/// Fixed card title, matching rs2b0t.
const QUEUE_CARD_TITLE: &str = "AUTO-LOGIN QUEUE";

/// Amber queue-card lines for one slot's FIFO place: title, `k of n`,
/// bots ahead, and at the head the host's reason for waiting (`wait`,
/// from [`QueuePlace::write_wait`]). Empty when the slot is not queued, so
/// the card disappears the moment the grant lands (`logging in…`).
fn queue_card_lines(queue: Option<QueuePlace>, wait: &str) -> Vec<String> {
    let Some(place) = queue else {
        return Vec::new();
    };
    let ahead = match place.ahead() {
        1 => "1 bot in front".to_string(),
        n => format!("{n} bots in front"),
    };
    let mut lines = vec![QUEUE_CARD_TITLE.to_string(), place.to_string(), ahead];
    if !wait.is_empty() {
        lines.push(wait.to_string());
    }
    lines
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
    line_h += scale_px(ui, QUEUE_CARD_LINE_GAP);
    let content_h = line_h * lines.len() as f32;
    let box_w = content_w + scale_px(ui, QUEUE_CARD_PAD * 2.0);
    let box_h = content_h + scale_px(ui, QUEUE_CARD_PAD * 2.0);
    (content_w, line_h, box_w, box_h)
}

/// Draw the queue card as a dark amber-bordered block over the Image.
/// `min` is the Image's top-left corner. Width/height come from the active
/// font measurement so left and right padding stay equal.
fn draw_queue_card(ui: &Ui, min: [f32; 2], lines: &[String]) {
    let (_content_w, line_h, box_w, box_h) = queue_card_metrics(ui, lines);
    let outer = scale_px(ui, QUEUE_CARD_OUTER);
    let pad = scale_px(ui, QUEUE_CARD_PAD);
    let box_min = [(min[0] + outer).round(), (min[1] + outer).round()];
    let box_max = [(box_min[0] + box_w).round(), (box_min[1] + box_h).round()];
    let text_origin = [box_min[0] + pad, box_min[1] + pad];
    let dl = ui.get_window_draw_list();
    dl.add_rect(box_min, box_max, [0.0, 0.0, 0.0, 0.6])
        .filled(true)
        .build();
    dl.add_rect(box_min, box_max, ACCENT)
        .thickness(scale_px(ui, 1.0))
        .build();
    for (i, line) in lines.iter().enumerate() {
        dl.add_text(
            [
                (text_origin[0]).round(),
                (text_origin[1] + i as f32 * line_h).round(),
            ],
            ACCENT,
            line,
        );
    }
}

/// One Image's queue card. Its text is cached on the FIFO place and the
/// head's wait text (whose countdown changes once a second), so steady
/// frames reuse it and only a change rebuilds it. The card must appear
/// immediately on enqueue, so this is not a timer.
#[derive(Default)]
struct QueueCard {
    queue: Option<QueuePlace>,
    wait: String,
    wait_scratch: String,
    lines: Vec<String>,
}

impl QueueCard {
    /// Bring the card up to `queue` at `now`; `true` when its lines were
    /// rebuilt.
    fn update(&mut self, queue: Option<QueuePlace>, now: Instant) -> bool {
        self.wait_scratch.clear();
        if let Some(place) = queue {
            place.write_wait(&mut self.wait_scratch, now);
        }
        if queue == self.queue && self.wait_scratch == self.wait {
            return false;
        }
        self.queue = queue;
        std::mem::swap(&mut self.wait, &mut self.wait_scratch);
        self.lines = queue_card_lines(queue, &self.wait);
        true
    }

    /// No-op when the bot is not queued, so the card disappears the moment
    /// the grant lands.
    fn draw(&self, ui: &Ui, min: [f32; 2]) {
        if !self.lines.is_empty() {
            draw_queue_card(ui, min, &self.lines);
        }
    }
}

/// Queue cards over the Grid's Images: one cached card per queued bot,
/// dropped when that bot leaves the queue.
#[derive(Default)]
pub struct QueueCards {
    cards: HashMap<String, QueueCard>,
}

impl QueueCards {
    /// Draw `name`'s queue card over its Image (`min` is the Image's
    /// top-left corner).
    pub fn frame(&mut self, ui: &Ui, name: &str, queue: Option<QueuePlace>, min: [f32; 2]) {
        if let Some(card) = self.update(name, queue, Instant::now()) {
            card.draw(ui, min);
        }
    }

    /// `name`'s card brought up to `queue`, or `None` (and no card kept)
    /// when it is not queued.
    fn update(
        &mut self,
        name: &str,
        queue: Option<QueuePlace>,
        now: Instant,
    ) -> Option<&QueueCard> {
        if queue.is_none() {
            self.cards.remove(name);
            return None;
        }
        if !self.cards.contains_key(name) {
            self.cards.insert(name.to_owned(), QueueCard::default());
        }
        let card = self.cards.get_mut(name).expect("inserted above");
        card.update(queue, now);
        Some(card)
    }
}

/// Cached queue-card state for the bot shown by this overlay.
pub struct PathOverlay {
    card: QueueCard,
}

impl PathOverlay {
    pub fn new() -> Self {
        Self {
            card: QueueCard::default(),
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
    pub fn frame(&mut self, ui: &Ui, queue: Option<QueuePlace>, min: [f32; 2], _size: [f32; 2]) {
        self.card.update(queue, Instant::now());
        self.card.draw(ui, min);
    }
}

impl Default for PathOverlay {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use frontend_core::QueuePlace;

    use super::{
        draw_queue_card, queue_card_lines, queue_card_metrics, PathOverlay, QueueCard, QueueCards,
        QUEUE_CARD_OUTER, QUEUE_CARD_PAD,
    };
    use crate::session::Session;

    fn place(position: u32, total: u32) -> Option<QueuePlace> {
        Some(QueuePlace::new(position, total))
    }

    #[test]
    fn queue_card_lines_match_rs2b0t_copy() {
        assert_eq!(queue_card_lines(None, ""), Vec::<String>::new());
        let lines = queue_card_lines(place(1, 2), "");
        assert_eq!(lines[0], "AUTO-LOGIN QUEUE");
        assert_eq!(lines[1], "1 of 2");
        assert_eq!(lines[2], "0 bots in front");
        assert_eq!(lines.len(), 3);
        let lines = queue_card_lines(place(2, 2), "");
        assert_eq!(lines[2], "1 bot in front");
    }

    /// The head's card says why "0 bots in front" is still waiting, and
    /// its countdown moves without a new place.
    #[test]
    fn head_card_names_its_blocker_and_counts_down() {
        let _guard = crate::test_support::imgui_context_guard();
        let mut ctx = dear_imgui_rs::Context::create();
        ctx.prepare_frame(
            dear_imgui_rs::FramePrepareOptions::new([900.0, 700.0], 1.0 / 60.0)
                .renderer_has_textures(),
        );
        let ui = ctx.frame();
        let head = |blocker| {
            QueuePlace::of(&host_play::SlotStatus {
                queue_position: 1,
                queue_total: 4,
                queue_blocker: Some(blocker),
                ..host_play::SlotStatus::default()
            })
        };
        let mut overlay = PathOverlay::new();
        ui.window("##overlay-head-test").build(|| {
            overlay.frame(
                ui,
                head(host::login_queue::AdmitBlocker::Handshake),
                [10.0, 10.0],
                [90.0, 90.0],
            );
            assert_eq!(overlay.card.lines[2], "0 bots in front");
            assert_eq!(overlay.card.lines[3], "another bot is logging in");
            let cooldown = head(host::login_queue::AdmitBlocker::ServerHold {
                until: std::time::Instant::now() + std::time::Duration::from_secs(20),
            });
            overlay.frame(ui, cooldown, [10.0, 10.0], [90.0, 90.0]);
            assert_eq!(
                overlay.card.lines[3],
                "server: too many login attempts, next try in 20 s"
            );
        });
        ctx.render();
    }

    /// Grid cards are cached per bot: steady frames rebuild nothing, a
    /// countdown that moves rebuilds that bot's lines, and a bot that leaves
    /// the queue drops its card.
    #[test]
    fn grid_cards_rebuild_only_when_their_text_changes() {
        use std::time::{Duration, Instant};

        let start = Instant::now();
        let head = QueuePlace::of(&host_play::SlotStatus {
            queue_position: 1,
            queue_total: 3,
            queue_blocker: Some(host::login_queue::AdmitBlocker::ServerHold {
                until: start + Duration::from_secs(20),
            }),
            ..host_play::SlotStatus::default()
        });
        let behind = place(2, 3);
        let mut cards = QueueCards::default();
        let lines = |cards: &mut QueueCards, name, queue, now| {
            let card = cards.update(name, queue, now).expect("queued");
            (card.lines.as_ptr(), card.lines.clone())
        };
        let (head_ptr, head_lines) = lines(&mut cards, "alice", head, start);
        let (behind_ptr, _) = lines(&mut cards, "bob", behind, start);
        assert_eq!(
            head_lines[3],
            "server: too many login attempts, next try in 20 s"
        );

        let mut card = QueueCard::default();
        assert!(card.update(head, start), "the first frame builds the card");
        for frame in 1..60u64 {
            let now = start + Duration::from_millis(16 * frame);
            assert!(
                !card.update(head, now),
                "steady frame {frame} rebuilt the card"
            );
            assert_eq!(lines(&mut cards, "alice", head, now).0, head_ptr);
            assert_eq!(lines(&mut cards, "bob", behind, now).0, behind_ptr);
        }

        let tick = start + Duration::from_secs(1);
        assert!(card.update(head, tick), "the countdown moved");
        let (_, ticked) = lines(&mut cards, "alice", head, tick);
        assert_eq!(
            ticked[3],
            "server: too many login attempts, next try in 19 s"
        );
        assert_eq!(lines(&mut cards, "bob", behind, tick).0, behind_ptr);

        assert!(cards.update("alice", None, tick).is_none());
        assert!(
            !cards.cards.contains_key("alice"),
            "a granted bot drops its card"
        );
        assert!(cards.cards.contains_key("bob"));
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
        assert!(overlay.card.lines.is_empty(), "no queue -> no card");
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
        let lines = queue_card_lines(place(1, 2), "");
        let (content_w, line_h, box_w, box_h) = queue_card_metrics(ui, &lines);
        assert!(content_w > 0.0, "title must measure wider than zero");
        let pad = crate::theme::scale_px(ui, QUEUE_CARD_PAD * 2.0);
        assert!(
            (box_w - (content_w + pad)).abs() < 0.01,
            "box width is content plus equal left/right pad"
        );
        assert!(
            (box_h - (line_h * lines.len() as f32 + pad)).abs() < 0.01,
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
        let short = queue_card_lines(place(1, 2), "");
        let long = queue_card_lines(place(12, 49), "");
        let (w_short, _, box_short, _) = queue_card_metrics(ui, &short);
        let (w_long, _, box_long, _) = queue_card_metrics(ui, &long);
        let pad = crate::theme::scale_px(ui, QUEUE_CARD_PAD * 2.0);
        // Title still dominates both; pads stay equal either way.
        assert!((box_short - (w_short + pad)).abs() < 0.01);
        assert!((box_long - (w_long + pad)).abs() < 0.01);
        assert!(
            box_long + 0.01 >= box_short,
            "wider k-of-n must not shrink the measured box"
        );
        ctx.render();
    }
}
