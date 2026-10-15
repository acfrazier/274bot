//! Status pane (spec `2026-09-01-headless-tui-design.md`): the selected
//! slot's key/value rows — state, player, tile, walk, queue, modals, mem —
//! plus the guardian status, the last login error while retrying and the
//! newest operation, then the process meter. The values come from the
//! shared `frontend-core` projection ([`SlotDetail`], [`ResourceView`]), the
//! same the panel's status and resource sections show; this pane only lays
//! them out.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Paragraph, Widget, Wrap};

use frontend_core::resources::{format_background, format_bots};
use frontend_core::{ResourceView, SlotDetail};

/// The status pane widget: key/value rows for the selected slot, plus the
/// operator's picked walk dest and the selected profile's mem mode.
pub struct StatusPane<'a> {
    pub detail: Option<&'a SlotDetail>,
    /// The walk cell: the operator's picked dest (`x z level`) or `—`.
    pub walk: &'a str,
    /// The applied memory mode, with the queued next-login mode if different.
    pub mem: &'a str,
    /// Pending whole-client next-login notice on its own wrapped line.
    pub mem_notice: Option<&'a str>,
    pub resources: Option<&'a ResourceView>,
    pub background_notice: Option<&'a str>,
}

impl<'a> StatusPane<'a> {
    pub fn new(detail: Option<&'a SlotDetail>, walk: &'a str, mem: &'a str) -> Self {
        Self {
            detail,
            walk,
            mem,
            mem_notice: None,
            resources: None,
            background_notice: None,
        }
    }

    pub fn mem_notice(mut self, notice: Option<&'a str>) -> Self {
        self.mem_notice = notice;
        self
    }

    pub fn resources(mut self, view: &'a ResourceView) -> Self {
        self.resources = Some(view);
        self
    }

    pub fn notice(mut self, notice: Option<&'a str>) -> Self {
        self.background_notice = notice;
        self
    }
}

impl Widget for StatusPane<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let block = Block::default().borders(Borders::ALL).title("status");
        let inner = block.inner(area);
        block.render(area, buf);
        // Most important first, so a short pane keeps what explains the
        // bot: its state, why it is retrying, its newest operation and any
        // guardian or welcome hold; related cells share a line so the
        // meter still fits the 120x40 pane.
        let mut lines: Vec<Line> = match self.detail {
            None => vec![
                Line::from("state: no bot selected"),
                Line::from(format!("walk: {}", self.walk)),
            ],
            Some(d) => {
                let mut lines = vec![Line::from(format!("state: {}", d.state()))];
                if let Some(script) = d.script_phase() {
                    lines.push(Line::from(format!("script: {}", script.label())));
                    if let Some(phase) = script.phase() {
                        lines.push(Line::from(format!("phase: {phase}")));
                    }
                    if let Some(reason) = script.reason() {
                        lines.push(Line::from(format!("reason: {reason}")));
                    }
                }
                if let (false, Some(error)) = (d.row().phase().is_error(), d.row().error()) {
                    lines.push(Line::from(format!("last error: {error}")));
                }
                if let Some(op) = d.row().last_op() {
                    lines.push(Line::from(format!("operation: {op}")));
                }
                if let Some(random) = d.random() {
                    lines.push(Line::from(format!("random: {random}")));
                }
                if let Some(welcome) = d.welcome() {
                    lines.push(Line::from(format!("welcome: {welcome}")));
                }
                let player = if d.player().is_empty() {
                    "?"
                } else {
                    d.player()
                };
                let world = match d.row().world() {
                    Some(number) => format!("w{number}"),
                    None => "local".to_string(),
                };
                let queue = match d.row().queue() {
                    Some(place) => place.to_string(),
                    None => "—".to_string(),
                };
                lines.push(Line::from(format!("player: {player} · world: {world}")));
                lines.push(Line::from(format!(
                    "tile: {} {} · walk: {}",
                    d.tile().0,
                    d.tile().1,
                    self.walk
                )));
                if let Some(reason) = d.manual_walk_risk() {
                    lines.push(Line::from(format!("Walk risk: {reason}")));
                }
                lines.push(Line::from(format!(
                    "queue: {queue} · modals: {} · mem: {}",
                    d.modal(),
                    self.mem
                )));
                if let Some(note) = self.mem_notice {
                    lines.push(Line::from(format!("mem: {note}")));
                }
                lines
            }
        };
        if let Some(view) = self.resources {
            let mut bots = format!("bots: {}", format_bots(view.bots(), view.ingame()));
            if view.background() > 0 {
                bots.push_str(" · ");
                bots.push_str(&format_background(view.background()));
            }
            lines.push(Line::from(bots));
            lines.push(Line::from(format!("cpu: {}", view.cpu().text())));
            lines.push(Line::from(format!("ram: {}", view.ram().text())));
            lines.push(Line::from(format!("traffic: {}", view.traffic().text())));
        }
        if let Some(notice) = self.background_notice {
            lines.push(Line::from(format!("notice: {notice}")));
        }
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render(inner, buf);
    }
}

#[cfg(test)]
mod tests {
    use frontend_core::{
        ActionKind, FleetRow, Metric, OpBrief, OperationId, Outcome, Phase, ResourceView,
        SlotDetail,
    };
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use super::StatusPane;

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

    /// The pane shows the projected detail as given: the retained error of
    /// a retrying slot is labelled history, and its newest operation shows.
    #[test]
    fn a_retrying_slot_shows_its_last_error_and_operation() {
        let mut detail = SlotDetail::default();
        let mut row = FleetRow::fixture("alice", Phase::Connecting, None, None);
        row.set_error(Some("code 5: already logged in".into()));
        row.set_last_op(Some(OpBrief::new(
            OperationId(7),
            ActionKind::Login,
            Outcome::Pending,
        )));
        detail.set_row(row);
        detail.set_state("logging in…");
        let text = render(StatusPane::new(Some(&detail), "—", "lowmem"), 60, 14);
        assert!(text.contains("state: logging in…"), "{text:?}");
        assert!(
            text.contains("last error: code 5: already logged in"),
            "{text:?}"
        );
        assert!(text.contains("operation: op#7 Log in accepted"), "{text:?}");
    }

    #[test]
    fn a_current_error_is_the_state_not_a_last_error() {
        let mut detail = SlotDetail::default();
        let mut row = FleetRow::fixture("bob", Phase::LoginError, None, None);
        row.set_error(Some("code 3: invalid username or password".into()));
        detail.set_row(row);
        detail.set_state("login code 3: invalid username or password");
        let text = render(StatusPane::new(Some(&detail), "—", "lowmem"), 60, 12);
        assert!(!text.contains("last error"), "{text:?}");
    }

    #[test]
    fn the_meter_shows_measuring_and_unavailable_values() {
        let view = ResourceView::with_metrics(
            Metric::Measuring,
            Metric::available("64.0 MB process, peak 80.0 MB"),
            Metric::unavailable("no live slots"),
        );
        let text = render(
            StatusPane::new(None, "—", "lowmem").resources(&view),
            60,
            12,
        );
        assert!(text.contains("cpu: measuring…"), "{text:?}");
        assert!(text.contains("ram: 64.0 MB process"), "{text:?}");
        assert!(text.contains("traffic: no live slots"), "{text:?}");
    }

    /// Cells pack onto shared rows, so a 58x14 pane shows a ready bot's
    /// newest operation, its background bots and the whole meter.
    #[test]
    fn a_58x14_pane_shows_the_whole_meter() {
        let mut detail = SlotDetail::default();
        let mut row = FleetRow::fixture("fc3bob", Phase::Ready, None, None);
        row.set_last_op(Some(OpBrief::new(
            OperationId(8),
            ActionKind::Logout,
            Outcome::Completed,
        )));
        detail.set_row(row);
        detail.set_state("ingame scene 2");
        detail.set_player("Fc3bob");
        detail.set_tile((3094, 3106, 0));
        detail.set_modal(3559);
        let mut view = ResourceView::with_metrics(
            Metric::available("0.1 cores (0% of 16)"),
            Metric::available("263.9 MB process, peak 263.9 MB"),
            Metric::available("1.2 KB/s"),
        );
        view.set_counts(3, 2, 2);
        let text = render(
            StatusPane::new(Some(&detail), "—", "lowmem").resources(&view),
            58,
            14,
        );
        assert!(
            text.contains("operation: op#8 Log out completed"),
            "{text:?}"
        );
        assert!(text.contains("traffic: 1.2 KB/s"), "{text:?}");
    }

    /// A 38x6 pane keeps a retrying bot's state and why it is retrying
    /// ahead of the player/tile rows.
    #[test]
    fn a_38x6_pane_keeps_the_retry_reason() {
        let mut detail = SlotDetail::default();
        let mut row = FleetRow::fixture("fc3alice", Phase::Connecting, None, None);
        row.set_error(Some("code 5: Try again in 60 secs".into()));
        detail.set_row(row);
        detail.set_state("logging in…");
        let text = render(StatusPane::new(Some(&detail), "—", "lowmem"), 38, 6);
        assert!(text.contains("state: logging in…"), "{text:?}");
        assert!(text.contains("last error: code 5"), "{text:?}");
    }
    #[test]
    fn stopped_native_queue_refusal_precedes_routine_status_rows() {
        let mut detail = SlotDetail::default();
        detail.set_row(FleetRow::fixture("qs-alice", Phase::Ready, None, None));
        detail.set_state("ingame scene 2");
        let status = script::native::ScriptStatus {
            run: api::selected::RunKey {
                slot: 1,
                run: 2,
                session: 3,
            },
            card: script::CompiledId("Quester"),
            phase: script::native::NativePhase::Blocked,
            active_settings: 1,
            pending_settings: None,
            fields: std::sync::Arc::from([]),
            failure: Some(script::native::ScriptFailure {
                code: "empty-queue".into(),
                message: "no quests selected; review Script prefs and Skip, then Stop/Start".into(),
            }),
        };
        let detail = detail.with_native_status(Some(std::sync::Arc::new(status)));
        let text = render(StatusPane::new(Some(&detail), "—", "lowmem"), 90, 8);
        assert!(text.contains("script: stopped (blocked)"), "{text:?}");
        assert!(text.contains("no quests selected"), "{text:?}");
        assert!(
            text.contains("Script prefs and Skip, then Stop/Start"),
            "{text:?}"
        );
    }
}

#[cfg(test)]
mod walk_risk_tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn manual_refusal_reason_survives_an_unarmed_route() {
        let mut detail = SlotDetail::default();
        detail.set_state("ingame");
        detail.set_manual_walk_risk(Some(std::sync::Arc::from(
            "Route assessment unknown: Poison",
        )));
        let mut terminal = Terminal::new(TestBackend::new(90, 14)).unwrap();
        terminal
            .draw(|frame| {
                frame.render_widget(StatusPane::new(Some(&detail), "—", "lowmem"), frame.area())
            })
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(
            text.contains("Walk risk: Route assessment unknown: Poison"),
            "{text:?}"
        );
    }
}
