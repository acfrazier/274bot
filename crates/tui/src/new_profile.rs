//! The New profile overlay: the shared profile form's blank row
//! (`frontend_core::ProfileEditor`, opened by `OpenNewProfileForm`),
//! rendered for the terminal. The editor is owned by the binary; this
//! view only holds what the overlay shows (the typed username, the
//! password's length and the form's save notice) and which field takes
//! typing. Keys go to the binary as `AppAction::NewProfileKey`, which
//! edits the editor's draft and saves it through `CreateProfile`, so
//! the profile logic stays in frontend-core, shared with the panel.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget, Wrap};

/// Which field of the form takes typing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NewProfileField {
    #[default]
    Username,
    Password,
}

impl NewProfileField {
    /// The other field (Tab and the arrow keys move between them).
    pub fn other(self) -> Self {
        match self {
            Self::Username => Self::Password,
            Self::Password => Self::Username,
        }
    }
}

/// What the overlay shows. The binary rebuilds it from the editor after
/// each key and after each pump while the form's save settles. The
/// password itself is never copied here: only its length, so the
/// derived `Debug` holds no password.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NewProfileView {
    pub username: String,
    pub password_len: usize,
    pub field: NewProfileField,
    /// The form's save refusal or failure, shown over
    /// `frontend_core::NOTHING_SAVED`.
    pub error: Option<String>,
    /// The form's save is still being written.
    pub saving: bool,
}

impl NewProfileView {
    pub fn hints(&self) -> &'static str {
        if self.saving {
            "saving the profile…"
        } else {
            "Tab next field · Enter save · Esc close"
        }
    }

    /// Draw over `area`; returns the popup rectangle. The labels are
    /// built to fit the popup's width, so no label truncates: the
    /// username field keeps its last cells on screen and the password
    /// mask is capped, with its length named when it is longer.
    pub fn render(&self, area: Rect, buf: &mut Buffer) -> Rect {
        let popup = centered(area, 56, 11);
        Clear.render(popup, buf);
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" New profile ");
        let inner = block.inner(popup);
        block.render(popup, buf);
        if inner.height < 2 || inner.width < 12 {
            return popup;
        }
        let width = usize::from(inner.width);
        let field_width = width.saturating_sub(11);
        let username = tail(&self.username, field_width);
        let mask = "*".repeat(self.password_len.min(field_width));
        let mut password = mask;
        if self.password_len > field_width {
            password = format!("{} ({})", "*".repeat(field_width), self.password_len);
            password = tail(&password, field_width + 1);
        }
        let bold = Style::default().add_modifier(Modifier::BOLD);
        let mut lines = vec![
            Line::from("Add a game account to the vault."),
            Line::from("Type the account's own username and password."),
            Line::from(""),
            Line::styled(
                format!(
                    "{}Username: {}_",
                    marker(self.field == NewProfileField::Username),
                    username
                ),
                if self.field == NewProfileField::Username {
                    bold
                } else {
                    Style::default()
                },
            ),
            Line::styled(
                format!(
                    "{}Password: {}_",
                    marker(self.field == NewProfileField::Password),
                    password
                ),
                if self.field == NewProfileField::Password {
                    bold
                } else {
                    Style::default()
                },
            ),
            Line::from(""),
        ];
        if let Some(error) = &self.error {
            lines.push(Line::styled(error.clone(), Style::default().fg(Color::Red)));
            lines.push(Line::from(frontend_core::NOTHING_SAVED));
        } else if self.saving {
            lines.push(Line::from("Saving…"));
        } else {
            lines.push(Line::styled("[Save Enter]  [Close Esc]", bold));
        }
        Paragraph::new(lines).wrap(Wrap { trim: false }).render(
            Rect::new(inner.x, inner.y, inner.width, inner.height - 1),
            buf,
        );
        buf.set_stringn(
            inner.x,
            inner.y + inner.height - 1,
            self.hints(),
            width,
            bold,
        );
        popup
    }
}

fn marker(active: bool) -> &'static str {
    if active {
        "> "
    } else {
        "  "
    }
}

/// The last `width` characters of `text`, so the end of a typed field
/// (where the cursor sits) stays on screen.
fn tail(text: &str, width: usize) -> String {
    let count = text.chars().count();
    if count <= width {
        return text.to_string();
    }
    text.chars().skip(count - width).collect()
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

    fn drawn(view: &NewProfileView, width: u16, height: u16) -> Vec<String> {
        let area = Rect::new(0, 0, width, height);
        let mut buf = Buffer::empty(area);
        view.render(area, &mut buf);
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect()
    }

    /// The form's full labels and its buttons fit at both terminal
    /// sizes, and the password shows as a mask of its length.
    #[test]
    fn labels_fit_at_both_sizes_and_the_password_is_masked() {
        let view = NewProfileView {
            username: "alice".into(),
            password_len: 7,
            field: NewProfileField::Password,
            error: None,
            saving: false,
        };
        for (width, height) in [(120, 40), (80, 24)] {
            let rows = drawn(&view, width, height);
            let screen = rows.join("\n");
            for label in [
                "New profile",
                "Add a game account to the vault.",
                "Type the account's own username and password.",
                "Username: alice_",
                "Password: *******_",
                "[Save Enter]",
                "[Close Esc]",
                "Tab next field · Enter save · Esc close",
            ] {
                assert!(
                    screen.contains(label),
                    "{width}x{height}: {label}\n{screen}"
                );
            }
        }
    }

    /// A refused save shows its reason in the form, with
    /// `nothing was saved.` under it (the shared form's words).
    #[test]
    fn a_refused_save_shows_its_reason() {
        let view = NewProfileView {
            error: Some("credentials: username required".into()),
            ..NewProfileView::default()
        };
        let rows = drawn(&view, 80, 24);
        let screen = rows.join("\n");
        assert!(
            screen.contains("credentials: username required"),
            "{screen}"
        );
        assert!(screen.contains("nothing was saved."), "{screen}");
    }
}
