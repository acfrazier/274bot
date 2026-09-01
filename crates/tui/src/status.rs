//! Status pane (spec `2026-09-01-headless-tui-design.md`): the focused
//! slot's panel-style key/value rows — state, player, tile, walk, queue,
//! modals, mem — plus the guardian's [`host_play::RandomStatus`]. One
//! bot's rows, not a concatenated line.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Paragraph, Widget, Wrap};

use api::RandomKind;
use host_play::SlotStatus;

/// The state cell: `ingame scene N`, a login error, `logging in…`, or
/// `waiting`.
pub fn state_text(s: &SlotStatus) -> String {
    if s.ingame {
        format!("ingame scene {}", s.scene_state)
    } else if let Some(err) = &s.error {
        format!("login {err}")
    } else if s.login_started.is_some() {
        "logging in…".to_string()
    } else {
        "waiting".to_string()
    }
}

/// The queue cell: `k of n`, or `—` when not queued (the same -1
/// sentinel as the walk fields).
pub fn queue_text(s: &SlotStatus) -> String {
    if s.queue_position > 0 && s.queue_total > 0 {
        format!("{} of {}", s.queue_position, s.queue_total)
    } else {
        "—".into()
    }
}

/// Status-row names for the guardian kinds (kebab-case, the same names
/// the panel's status section uses).
pub fn random_kind_name(kind: RandomKind) -> &'static str {
    match kind {
        RandomKind::Dialog => "dialog",
        RandomKind::Pick => "pick",
        RandomKind::Evade => "evade",
        RandomKind::Maze => "maze",
        RandomKind::Mime => "mime",
        RandomKind::Box => "box",
        RandomKind::Lamp => "lamp",
        RandomKind::Hazard => "hazard",
        RandomKind::LostTool => "lost-tool",
        RandomKind::LostGear => "lost-gear",
    }
}

/// The random status-row value: `dialog: mysterious old man`, plus
/// `(hold)` while the slot freezes on the event and `(off)` when the
/// profile toggle is off (toggle-off still detects + publishes). `None`
/// when nothing is detected — the row is skipped then.
pub fn random_status_text(s: &SlotStatus) -> Option<String> {
    let kind = s.random.kind?;
    let mut text = format!(
        "{}: {}",
        random_kind_name(kind),
        s.random.name.as_deref().unwrap_or("?")
    );
    if s.random.hold {
        text.push_str(" (hold)");
    }
    if !s.random.toggle {
        text.push_str(" (off)");
    }
    Some(text)
}

/// The status pane widget: key/value rows for the focused slot, plus the
/// operator's picked walk dest and the focused profile's mem mode.
pub struct StatusPane<'a> {
    pub slot: Option<&'a SlotStatus>,
    /// The walk cell: the operator's picked dest (`x z level`) or `—`.
    pub walk: &'a str,
    /// The mem cell: `lowmem` / `highmem`.
    pub mem: &'a str,
}

impl<'a> StatusPane<'a> {
    pub fn new(slot: Option<&'a SlotStatus>, walk: &'a str, mem: &'a str) -> Self {
        Self { slot, walk, mem }
    }
}

impl Widget for StatusPane<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let block = Block::default().borders(Borders::ALL).title("status");
        let inner = block.inner(area);
        block.render(area, buf);
        let lines: Vec<Line> = match self.slot {
            None => vec![
                Line::from("state: no slots"),
                Line::from("player: —"),
                Line::from(format!("walk: {}", self.walk)),
            ],
            Some(s) => {
                let mut lines = vec![
                    Line::from(format!("state: {}", state_text(s))),
                    Line::from(format!(
                        "player: {}",
                        if s.player.is_empty() {
                            "?"
                        } else {
                            s.player.as_str()
                        }
                    )),
                    Line::from(format!("tile: {} {}", s.tile_x, s.tile_z)),
                    Line::from(format!("walk: {}", self.walk)),
                    Line::from(format!("queue: {}", queue_text(s))),
                    Line::from(format!("modals: {}", s.main_modal_id)),
                    Line::from(format!("mem: {}", self.mem)),
                ];
                if let Some(random) = random_status_text(s) {
                    lines.push(Line::from(format!("random: {random}")));
                }
                lines
            }
        };
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render(inner, buf);
    }
}

#[cfg(test)]
mod tests {
    use host_play::{RandomStatus, SlotStatus};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use api::RandomKind;

    use super::{queue_text, random_status_text, state_text, StatusPane};

    fn status(ingame: bool, scene_state: i32) -> SlotStatus {
        SlotStatus {
            username: "test".into(),
            ingame,
            scene_state,
            tile_x: 10,
            tile_z: 11,
            ..SlotStatus::default()
        }
    }

    fn render(pane: StatusPane<'_>, w: u16, h: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal
            .draw(|frame| frame.render_widget(pane, frame.area()))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn status_shows_ingame_scene_2_and_player() {
        let s = status(true, 2);
        assert_eq!(state_text(&s), "ingame scene 2");
        let text = render(StatusPane::new(Some(&s), "10 11 0", "lowmem"), 40, 12);
        assert!(text.contains("ingame scene 2"), "state row: {text:?}");
        assert!(text.contains("tile: 10 11"), "tile row: {text:?}");
        assert!(text.contains("walk: 10 11 0"), "walk row: {text:?}");
    }

    #[test]
    fn status_shows_the_random_status_name_and_hold() {
        let mut s = status(true, 2);
        s.random = RandomStatus {
            kind: Some(RandomKind::Dialog),
            name: Some("mysterious old man".into()),
            hold: true,
            toggle: true,
            ..RandomStatus::default()
        };
        assert_eq!(
            random_status_text(&s).as_deref(),
            Some("dialog: mysterious old man (hold)")
        );
        let text = render(StatusPane::new(Some(&s), "—", "lowmem"), 40, 12);
        assert!(
            text.contains("mysterious old man"),
            "random row paints: {text:?}"
        );
        assert!(text.contains("(hold)"), "hold suffix: {text:?}");
    }

    #[test]
    fn random_toggle_off_still_detects_and_labels_off() {
        let mut s = status(true, 2);
        s.random = RandomStatus {
            kind: Some(RandomKind::Lamp),
            name: Some("genie".into()),
            hold: false,
            toggle: false,
            ..RandomStatus::default()
        };
        assert_eq!(
            random_status_text(&s).as_deref(),
            Some("lamp: genie (off)"),
            "toggle-off detects and publishes with an (off) suffix"
        );
    }

    #[test]
    fn queue_text_formats_k_of_n_and_dash() {
        let mut s = status(true, 2);
        assert_eq!(queue_text(&s), "—");
        s.queue_position = 2;
        s.queue_total = 3;
        assert_eq!(queue_text(&s), "2 of 3");
    }

    #[test]
    fn empty_pane_says_no_slots() {
        let text = render(StatusPane::new(None, "—", "lowmem"), 40, 8);
        assert!(text.contains("no slots"), "empty status: {text:?}");
    }
}
