//! The rs2b0t profile import overlay: type the export file's path, read the
//! preview (nothing is written yet), choose what happens to profiles already
//! in the vault, confirm, read the report, and optionally delete the export
//! file. The flow itself is `frontend_core::rs2b0t_import::ProfileImport`,
//! owned by the binary; this overlay only holds the text it shows and turns
//! keys into [`ImportAction`]s.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use frontend_core::rs2b0t_import::{ConflictPolicy, Stage, PROFILE_FILE_NAME};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget, Wrap};

/// What the overlay shows. The binary rebuilds it from the flow after each
/// action and while the import's writes settle. `lines` and `error` are
/// the flow's own, which it redacts as it builds them, so the derived
/// `Debug` holds no export password.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportView {
    pub stage: Stage,
    /// The path being typed (Path stage).
    pub path: String,
    /// The preview or the report.
    pub lines: Vec<String>,
    pub error: Option<String>,
    pub policy: ConflictPolicy,
    /// Accounts the 289 client cannot log in with (the preview's count).
    pub unusable: usize,
    /// Whether those accounts are imported anyway.
    pub include_unusable: bool,
    /// Every write of the confirmed import has settled.
    pub settled: bool,
    pub can_delete: bool,
    pub delete_asked: bool,
    pub scroll: u16,
}

impl Default for ImportView {
    fn default() -> Self {
        Self {
            stage: Stage::Path,
            path: String::new(),
            lines: Vec::new(),
            error: None,
            policy: ConflictPolicy::Skip,
            unusable: 0,
            include_unusable: false,
            settled: false,
            can_delete: false,
            delete_asked: false,
            scroll: 0,
        }
    }
}

/// One step of the import, for the binary to run on its flow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportAction {
    /// Read and preview the file at this path.
    Preview(String),
    Policy(ConflictPolicy),
    /// Import accounts the 289 client cannot log in with too (or not).
    IncludeUnusable(bool),
    /// Write the previewed import.
    Confirm,
    /// Back to the path, dropping the parsed file.
    Back,
    AskDelete,
    ConfirmDelete,
    CancelDelete,
    Close,
}

fn is_text(key: &KeyEvent) -> bool {
    !key.modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER)
}

impl ImportView {
    pub fn hints(&self) -> &'static str {
        match self.stage {
            Stage::Path => "type the file path · Enter preview · Esc close",
            Stage::Preview if self.unusable > 0 && self.include_unusable => {
                "o skip/overwrite existing · u leave unusable accounts out · Enter/y import · Up/Down scroll · Esc back"
            }
            Stage::Preview if self.unusable > 0 => {
                "o skip/overwrite existing · u import unusable accounts too · Enter/y import · Up/Down scroll · Esc back"
            }
            Stage::Preview => {
                "o skip/overwrite existing · Enter/y import · Up/Down scroll · Esc back"
            }
            Stage::Report if self.delete_asked => "y delete the export file · n keep it",
            Stage::Report if self.can_delete => {
                "d delete the export file… · Up/Down scroll · Esc close"
            }
            Stage::Report => "Up/Down scroll · Esc close",
        }
    }

    /// One key: `None` keeps the overlay open with no action.
    pub fn on_key(&mut self, key: KeyEvent) -> Option<ImportAction> {
        match self.stage {
            Stage::Path => match key.code {
                KeyCode::Esc => Some(ImportAction::Close),
                KeyCode::Enter => Some(ImportAction::Preview(self.path.clone())),
                KeyCode::Backspace => {
                    self.path.pop();
                    None
                }
                KeyCode::Char(c) if is_text(&key) => {
                    self.path.push(c);
                    None
                }
                _ => None,
            },
            Stage::Preview => match key.code {
                KeyCode::Esc | KeyCode::Char('b') => Some(ImportAction::Back),
                KeyCode::Enter | KeyCode::Char('y') => Some(ImportAction::Confirm),
                KeyCode::Char('o') => Some(ImportAction::Policy(match self.policy {
                    ConflictPolicy::Skip => ConflictPolicy::Overwrite,
                    ConflictPolicy::Overwrite => ConflictPolicy::Skip,
                })),
                KeyCode::Char('u') if self.unusable > 0 => {
                    Some(ImportAction::IncludeUnusable(!self.include_unusable))
                }
                _ => {
                    self.scroll_key(key.code);
                    None
                }
            },
            Stage::Report if self.delete_asked => match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                    Some(ImportAction::ConfirmDelete)
                }
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    Some(ImportAction::CancelDelete)
                }
                _ => None,
            },
            Stage::Report => match key.code {
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => Some(ImportAction::Close),
                KeyCode::Char('d') if self.can_delete => Some(ImportAction::AskDelete),
                _ => {
                    self.scroll_key(key.code);
                    None
                }
            },
        }
    }

    fn scroll_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Up | KeyCode::Char('k') => self.scroll_by_lines(-1),
            KeyCode::Down | KeyCode::Char('j') => self.scroll_by_lines(1),
            KeyCode::PageUp => self.scroll_by_lines(-10),
            KeyCode::PageDown => self.scroll_by_lines(10),
            KeyCode::Home => self.scroll = 0,
            _ => {}
        }
    }

    /// The wheel: three lines a notch.
    pub fn scroll_by(&mut self, delta: i32) {
        self.scroll_by_lines(delta.saturating_mul(3));
    }

    /// Scroll, stopping with the last line still at the top.
    fn scroll_by_lines(&mut self, delta: i32) {
        let last = u16::try_from(self.lines.len().saturating_sub(1)).unwrap_or(u16::MAX);
        let next = i32::from(self.scroll).saturating_add(delta);
        self.scroll = u16::try_from(next.max(0)).unwrap_or(u16::MAX).min(last);
    }

    /// Draw over `area`; returns the popup rectangle.
    pub fn render(&self, area: Rect, buf: &mut Buffer) -> Rect {
        let popup = centered(
            area,
            area.width.saturating_sub(6).max(30),
            area.height.saturating_sub(4).max(8),
        );
        Clear.render(popup, buf);
        let title = match self.stage {
            Stage::Path => " Import rs2b0t profiles ",
            Stage::Preview => " Import rs2b0t profiles · preview (nothing written yet) ",
            Stage::Report if self.settled => " Import rs2b0t profiles · report ",
            Stage::Report => " Import rs2b0t profiles · saving… ",
        };
        let block = Block::default().borders(Borders::ALL).title(title);
        let inner = block.inner(popup);
        block.render(popup, buf);
        if inner.height < 2 {
            return popup;
        }
        let body = Rect::new(inner.x, inner.y, inner.width, inner.height - 1);
        let mut lines: Vec<Line> = Vec::new();
        if let Some(error) = &self.error {
            lines.push(Line::styled(
                error.clone(),
                Style::default().add_modifier(Modifier::BOLD),
            ));
            lines.push(Line::from(""));
        }
        match self.stage {
            Stage::Path => {
                lines.push(Line::from(format!(
                    "Path of the {PROFILE_FILE_NAME} file rs2b0t exported (Settings → export):"
                )));
                lines.push(Line::styled(
                    format!("> {}_", self.path),
                    Style::default().add_modifier(Modifier::BOLD),
                ));
                lines.push(Line::from(""));
                lines.push(Line::from(
                    "The preview shows every account and setting before anything is written.",
                ));
                lines.push(Line::from(
                    "That file holds your account passwords in plain text: keep it private.",
                ));
            }
            Stage::Preview | Stage::Report => {
                lines.extend(self.lines.iter().map(|line| Line::from(line.clone())));
            }
        }
        let scroll = if self.stage == Stage::Path {
            0
        } else {
            self.scroll
        };
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0))
            .render(body, buf);
        let footer = if self.delete_asked {
            "Delete the export file now? It cannot be undone. [y delete] [n keep]"
        } else {
            self.hints()
        };
        buf.set_stringn(
            inner.x,
            inner.y + inner.height - 1,
            footer,
            usize::from(inner.width),
            Style::default().add_modifier(Modifier::BOLD),
        );
        popup
    }
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn path_stage_types_and_previews() {
        let mut view = ImportView::default();
        for c in "a.json".chars() {
            assert_eq!(view.on_key(key(KeyCode::Char(c))), None);
        }
        view.on_key(key(KeyCode::Backspace));
        assert_eq!(view.path, "a.jso");
        assert_eq!(
            view.on_key(key(KeyCode::Enter)),
            Some(ImportAction::Preview("a.jso".into()))
        );
        assert_eq!(view.on_key(key(KeyCode::Esc)), Some(ImportAction::Close));
    }

    #[test]
    fn preview_toggles_policy_confirms_or_goes_back() {
        let mut view = ImportView {
            stage: Stage::Preview,
            lines: vec!["a".into(), "b".into(), "c".into()],
            ..ImportView::default()
        };
        assert_eq!(
            view.on_key(key(KeyCode::Char('o'))),
            Some(ImportAction::Policy(ConflictPolicy::Overwrite))
        );
        // No unusable account: nothing to include.
        assert_eq!(view.on_key(key(KeyCode::Char('u'))), None);
        view.unusable = 1;
        assert_eq!(
            view.on_key(key(KeyCode::Char('u'))),
            Some(ImportAction::IncludeUnusable(true))
        );
        view.include_unusable = true;
        assert_eq!(
            view.on_key(key(KeyCode::Char('u'))),
            Some(ImportAction::IncludeUnusable(false))
        );
        assert_eq!(
            view.on_key(key(KeyCode::Enter)),
            Some(ImportAction::Confirm)
        );
        assert_eq!(view.on_key(key(KeyCode::Esc)), Some(ImportAction::Back));
        view.on_key(key(KeyCode::Down));
        assert_eq!(view.scroll, 1);
        // Never past the last line.
        view.on_key(key(KeyCode::PageDown));
        assert_eq!(view.scroll, 2);
        view.scroll_by(-5);
        assert_eq!(view.scroll, 0);
    }

    #[test]
    fn report_deletes_only_after_an_explicit_yes() {
        let mut view = ImportView {
            stage: Stage::Report,
            settled: true,
            ..ImportView::default()
        };
        // Not offered (a failure, or the file is gone).
        assert_eq!(view.on_key(key(KeyCode::Char('d'))), None);
        view.can_delete = true;
        assert_eq!(
            view.on_key(key(KeyCode::Char('d'))),
            Some(ImportAction::AskDelete)
        );
        view.delete_asked = true;
        assert_eq!(view.on_key(key(KeyCode::Char('x'))), None);
        assert_eq!(
            view.on_key(key(KeyCode::Char('n'))),
            Some(ImportAction::CancelDelete)
        );
        assert_eq!(
            view.on_key(key(KeyCode::Char('y'))),
            Some(ImportAction::ConfirmDelete)
        );
    }
}
