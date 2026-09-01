//! Queue-card overlay for the Game Image.
//!
//! The armed route's remaining tiles are painted by the client's 3D
//! renderer and on the pack map, so this overlay draws only the focused
//! slot's queue card over the Image.

use dear_imgui_rs::Ui;

use crate::queue_card::{queue_ahead_label, queue_k_of_n, QUEUE_CARD_TITLE};
use crate::session::Session;
use crate::theme::ACCENT;

/// Amber queue-card lines for a focused FIFO place: title, `k of n`,
/// ahead label. Empty when the slot is not queued, so the card disappears
/// the moment the grant lands (`logging in…`).
fn queue_card_lines(queue: Option<(i32, i32)>) -> Vec<String> {
    match queue {
        Some((k, n)) => vec![
            QUEUE_CARD_TITLE.to_string(),
            queue_k_of_n(k, n).unwrap_or_default(),
            queue_ahead_label(k.max(1) as u32),
        ],
        None => Vec::new(),
    }
}

/// Draw the queue card as a dark amber-bordered block over the Image.
/// `min` is the Image's top-left corner. Width is an estimate (no font
/// measurement at this layer), so the block only needs to be legible, not
/// pixel-perfect.
fn draw_queue_card(ui: &Ui, min: [f32; 2], lines: &[String]) {
    const PAD: f32 = 8.0;
    const LINE_H: f32 = 15.0;
    let top = [min[0] + PAD, min[1] + PAD];
    let width = lines
        .iter()
        .map(|l| l.len() as f32 * 7.0)
        .fold(PAD * 2.0, f32::max);
    let height = PAD * 2.0 + LINE_H * (lines.len() as f32 - 1.0) + 13.0;
    let dl = ui.get_window_draw_list();
    dl.add_rect(
        [top[0] - PAD, top[1] - PAD],
        [top[0] + width, top[1] + height],
        [0.0, 0.0, 0.0, 0.6],
    )
    .filled(true)
    .build();
    dl.add_rect(
        [top[0] - PAD, top[1] - PAD],
        [top[0] + width, top[1] + height],
        ACCENT,
    )
    .thickness(1.0)
    .build();
    for (i, line) in lines.iter().enumerate() {
        dl.add_text([top[0], top[1] + i as f32 * LINE_H], ACCENT, line);
    }
}

/// Queue card over the focused slot's cell (MultiBox grid mode). No-op
/// when the focused slot is not queued, so the card disappears the moment
/// the grant lands.
pub fn draw_focused_queue_card(ui: &Ui, session: &Session, min: [f32; 2]) {
    let lines = queue_card_lines(session.queue_place());
    if !lines.is_empty() {
        draw_queue_card(ui, min, &lines);
    }
}

/// Cached queue-card state for the focused slot. The card must appear
/// immediately on enqueue, so it is cached on the FIFO tuple, not a
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

    /// Draw the focused slot's queue card over the Image. `min` is the
    /// Image widget's top-left corner; `size` is unused now that the
    /// polyline is gone.
    pub fn frame(&mut self, ui: &Ui, session: &Session, min: [f32; 2], _size: [f32; 2]) {
        let queue = session.queue_place();
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
    use api::snapshot::WorldTile;
    use nav::collision::WorldCollision;
    use nav::tile::Tile;
    use nav::transport::TransportGraph;
    use nav::world::NavWorld;

    use super::{queue_card_lines, PathOverlay};
    use crate::session::Session;

    /// A `w`×`h` all-walkable level-0 world at (0,0).
    fn open_world(w: usize, h: usize) -> NavWorld {
        NavWorld::from_parts(
            WorldCollision {
                origin: WorldTile {
                    x: 0,
                    z: 0,
                    level: 0,
                },
                width: w,
                height: h,
                walk: vec![0u8; w * h],
                blocked: vec![0u64; (w * h).div_ceil(64)],
                flags: None,
            },
            TransportGraph::default(),
            Vec::new(),
        )
    }

    #[test]
    fn overlay_does_not_stroke_a_path_polyline() {
        let _guard = crate::IMGUI_CTX_TEST_GUARD.lock().unwrap();
        let mut ctx = dear_imgui_rs::Context::create();
        ctx.prepare_frame(
            dear_imgui_rs::FramePrepareOptions::new([900.0, 700.0], 1.0 / 60.0)
                .renderer_has_textures(),
        );
        let ui = ctx.frame();
        let mut s = Session::new();
        s.focus.lock().unwrap().focused = Some("alice".into());
        let world = open_world(3, 3);
        s.arm_walk_on(
            &world,
            Tile {
                x: 0,
                z: 0,
                level: 0,
            },
            Tile {
                x: 2,
                z: 2,
                level: 0,
            },
        );
        let mut overlay = PathOverlay::new();
        ui.window("##overlay-test").build(|| {
            overlay.frame(ui, &s, [10.0, 10.0], [90.0, 90.0]);
        });
        ctx.render();
        assert!(
            overlay.points().is_empty(),
            "polyline is gone; 3D paints the path"
        );
    }

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
    fn overlay_draws_queue_card_when_focused_slot_is_queued() {
        let _guard = crate::IMGUI_CTX_TEST_GUARD.lock().unwrap();
        let mut ctx = dear_imgui_rs::Context::create();
        ctx.prepare_frame(
            dear_imgui_rs::FramePrepareOptions::new([900.0, 700.0], 1.0 / 60.0)
                .renderer_has_textures(),
        );
        let ui = ctx.frame();
        let mut s = Session::new();
        s.focus.lock().unwrap().focused = Some("alice".into());
        s.statuses.push(host_play::SlotStatus {
            username: "alice".into(),
            queue_position: 1,
            queue_total: 2,
            ..host_play::SlotStatus::default()
        });
        let mut overlay = PathOverlay::new();
        ui.window("##overlay-queue-test").build(|| {
            overlay.frame(ui, &s, [10.0, 10.0], [90.0, 90.0]);
        });
        ctx.render();
        assert_eq!(
            overlay.queue_lines,
            vec![
                "AUTO-LOGIN QUEUE".to_string(),
                "1 of 2".to_string(),
                "0 bots in front".to_string()
            ]
        );
    }

    #[test]
    fn overlay_queue_card_follows_fifo_head_when_focus_already_granted() {
        let _guard = crate::IMGUI_CTX_TEST_GUARD.lock().unwrap();
        let mut ctx = dear_imgui_rs::Context::create();
        ctx.prepare_frame(
            dear_imgui_rs::FramePrepareOptions::new([900.0, 700.0], 1.0 / 60.0)
                .renderer_has_textures(),
        );
        let ui = ctx.frame();
        let mut s = Session::new();
        s.focus.lock().unwrap().focused = Some("s00".into());
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
            overlay.frame(ui, &s, [10.0, 10.0], [90.0, 90.0]);
        });
        ctx.render();
        assert_eq!(
            overlay.queue_lines[1], "1 of 49",
            "after the focused slot grants, the card steps k of n for the next queued member"
        );
    }

    #[test]
    fn overlay_skips_queue_card_when_not_queued() {
        let _guard = crate::IMGUI_CTX_TEST_GUARD.lock().unwrap();
        let mut ctx = dear_imgui_rs::Context::create();
        ctx.prepare_frame(
            dear_imgui_rs::FramePrepareOptions::new([900.0, 700.0], 1.0 / 60.0)
                .renderer_has_textures(),
        );
        let ui = ctx.frame();
        let s = Session::new();
        s.focus.lock().unwrap().focused = Some("alice".into());
        let mut overlay = PathOverlay::new();
        ui.window("##overlay-queue-test").build(|| {
            overlay.frame(ui, &s, [10.0, 10.0], [90.0, 90.0]);
        });
        ctx.render();
        assert!(overlay.queue_lines.is_empty(), "no queue -> no card");
    }
}
