//! `TuiApp`: view model for the headless panel. A real `Play` wires in
//! Task 10; for now the app holds a fake/view model and draws chrome
//! around it.

use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

/// Headless panel view model.
pub struct TuiApp {
    title: String,
}

impl TuiApp {
    /// New app showing `title`.
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
        }
    }

    /// The window title line.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Render the chrome into `frame`. Kept minimal until Play wiring
    /// (Task 10) adds map / chat / status panes.
    pub fn draw(&self, frame: &mut Frame<'_>) {
        let area = frame.area();
        let title = Paragraph::new(Line::from(self.title.as_str())).wrap(Wrap { trim: false });
        frame.render_widget(title, area);
    }
}

#[cfg(test)]
mod tests {
    use super::TuiApp;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    /// The TestBackend draws without a real terminal — the TUI is
    /// raster-off, so this is the only backend CI needs.
    #[test]
    fn draws_title_containing_274bot() {
        let app = TuiApp::new("274bot headless");
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();

        let buf = terminal.backend().buffer();
        let text: String = buf.content().iter().map(|cell| cell.symbol()).collect();
        assert!(
            text.contains("274bot"),
            "buffer does not contain 274bot: {text:?}"
        );
    }
}
