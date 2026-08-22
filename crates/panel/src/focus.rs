/// Panel focus policy: which bot the panel is locked onto and whether the
/// game renderer / capture should run for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Focus {
    pub focused: Option<String>,
    /// "game renderer" checkbox.
    pub renderer: bool,
    pub game_pane_open: bool,
    /// This focused bot's capture checkbox.
    pub capture: bool,
}

/// The game renderer draws only when a bot is focused, its pane is open,
/// and the renderer is enabled.
pub fn should_draw(f: &Focus) -> bool {
    f.focused.is_some() && f.game_pane_open && f.renderer
}

/// Whether this specific slot draws: the renderer is on and the slot is the
/// focused one. Unfocused slots must stay `set_draw(false)`.
pub fn draw_for_slot(f: &Focus, name: &str) -> bool {
    should_draw(f) && f.focused.as_deref() == Some(name)
}

/// Capture (the focused bot's capture checkbox) additionally requires draw.
pub fn should_capture(f: &Focus) -> bool {
    should_draw(f) && f.capture
}

#[cfg(test)]
mod tests {
    use super::{Focus, draw_for_slot, should_capture, should_draw};

    #[test]
    fn draw_requires_focus_pane_and_renderer() {
        let mut f = Focus { focused: Some("test".into()), renderer: true, game_pane_open: true, capture: false };
        assert!(should_draw(&f));
        assert!(!should_capture(&f));
        f.renderer = false;
        assert!(!should_draw(&f));
        f.renderer = true;
        f.game_pane_open = false;
        assert!(!should_draw(&f));
    }

    #[test]
    fn capture_implies_draw() {
        let f = Focus { focused: Some("a".into()), renderer: true, game_pane_open: true, capture: true };
        assert!(should_capture(&f));
        let f = Focus { focused: Some("a".into()), renderer: false, game_pane_open: true, capture: true };
        assert!(!should_capture(&f));
    }

    #[test]
    fn draw_for_slot_requires_this_slot_to_be_focused() {
        let f = Focus { focused: Some("a".into()), renderer: true, game_pane_open: true, capture: false };
        assert!(draw_for_slot(&f, "a"));
        assert!(!draw_for_slot(&f, "b"));
        assert!(!draw_for_slot(&f, ""));
        // No focus: no slot draws.
        let f = Focus { focused: None, renderer: true, game_pane_open: true, capture: false };
        assert!(!draw_for_slot(&f, "a"));
        // Renderer off: the focused slot does not draw either.
        let f = Focus { focused: Some("a".into()), renderer: false, game_pane_open: true, capture: false };
        assert!(!draw_for_slot(&f, "a"));
    }
}
