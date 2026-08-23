/// Panel focus policy: which bot the panel is locked onto and whether the
/// game renderer / capture should run for it.
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Focus {
    pub focused: Option<String>,
    /// "game renderer" checkbox.
    pub renderer: bool,
    pub game_pane_open: bool,
    /// This focused bot's capture checkbox.
    pub capture: bool,
    /// When false, wall members also draw (wall policy below).
    pub only_render_selected: bool,
    /// Whether the wall draw policy is active.
    pub wall_open: bool,
    /// Wall members eligible to draw when `only_render_selected` is off.
    pub wall: Vec<String>,
    /// Per-slot renderer overrides; absent names fall back to `renderer`.
    pub renderer_by: HashMap<String, bool>,
}

/// Whether this slot has the renderer enabled: per-slot override if present,
/// else the focused bot's `renderer` checkbox.
pub fn renderer_for(f: &Focus, name: &str) -> bool {
    f.renderer_by.get(name).copied().unwrap_or(f.renderer)
}

/// The game renderer draws only when a bot is focused, its pane is open,
/// and the renderer is enabled.
pub fn should_draw(f: &Focus) -> bool {
    match f.focused.as_deref() {
        Some(name) => draw_for_slot(f, name),
        None => false,
    }
}

/// Whether this specific slot draws: the renderer is on and the slot is
/// either the focused one or, when the wall policy allows it, a wall member.
/// Unfocused non-wall slots must stay `set_draw(false)`.
pub fn draw_for_slot(f: &Focus, name: &str) -> bool {
    if !f.game_pane_open || !renderer_for(f, name) {
        return false;
    }
    if f.focused.as_deref() == Some(name) {
        return true;
    }
    !f.only_render_selected && f.wall_open && f.wall.iter().any(|n| n == name)
}

/// Capture (the focused bot's capture checkbox) additionally requires draw.
pub fn should_capture(f: &Focus) -> bool {
    should_draw(f) && f.capture
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::{Focus, draw_for_slot, should_capture, should_draw};

    #[test]
    fn draw_requires_focus_pane_and_renderer() {
        let mut f = Focus { focused: Some("test".into()), renderer: true, game_pane_open: true, capture: false, only_render_selected: true, wall_open: false, wall: vec![], renderer_by: HashMap::new() };
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
        let f = Focus { focused: Some("a".into()), renderer: true, game_pane_open: true, capture: true, only_render_selected: true, wall_open: false, wall: vec![], renderer_by: HashMap::new() };
        assert!(should_capture(&f));
        let f = Focus { focused: Some("a".into()), renderer: false, game_pane_open: true, capture: true, only_render_selected: true, wall_open: false, wall: vec![], renderer_by: HashMap::new() };
        assert!(!should_capture(&f));
    }

    #[test]
    fn draw_for_slot_requires_this_slot_to_be_focused() {
        let f = Focus { focused: Some("a".into()), renderer: true, game_pane_open: true, capture: false, only_render_selected: true, wall_open: false, wall: vec![], renderer_by: HashMap::new() };
        assert!(draw_for_slot(&f, "a"));
        assert!(!draw_for_slot(&f, "b"));
        assert!(!draw_for_slot(&f, ""));
        // No focus: no slot draws.
        let f = Focus { focused: None, renderer: true, game_pane_open: true, capture: false, only_render_selected: true, wall_open: false, wall: vec![], renderer_by: HashMap::new() };
        assert!(!draw_for_slot(&f, "a"));
        // Renderer off: the focused slot does not draw either.
        let f = Focus { focused: Some("a".into()), renderer: false, game_pane_open: true, capture: false, only_render_selected: true, wall_open: false, wall: vec![], renderer_by: HashMap::new() };
        assert!(!draw_for_slot(&f, "a"));
    }

    #[test]
    fn only_render_selected_off_paints_wall_members() {
        let mut f = Focus {
            focused: Some("a".into()),
            renderer: true,
            game_pane_open: true,
            capture: false,
            only_render_selected: true,
            wall_open: true,
            wall: vec!["a".into(), "b".into()],
            renderer_by: HashMap::from([("a".into(), true), ("b".into(), true)]),
        };
        assert!(draw_for_slot(&f, "a"));
        assert!(!draw_for_slot(&f, "b"));
        f.only_render_selected = false;
        assert!(draw_for_slot(&f, "b"));
        f.wall_open = false;
        assert!(!draw_for_slot(&f, "b"));
        f.wall_open = true;
        f.renderer_by.insert("b".into(), false);
        assert!(!draw_for_slot(&f, "b"));
    }
}
