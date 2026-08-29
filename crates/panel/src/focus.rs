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
    /// Sidecar-50 pref: wall/grid members render at 50 fps instead of the
    /// 1 fps watch cadence.
    pub sidecar_50: bool,
    /// Ephemeral live overlay: every drawing slot at 50 fps, focused
    /// included. Not sidecar-50. Not persisted.
    pub live_full_rate: bool,
    /// Game-pane (focused) slot at 50 fps. Does not follow that client
    /// onto the rail — rail cadence is [`Focus::sidecar_50`] only.
    pub focused_50: bool,
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

/// Whether this slot runs at the 50 fps frame cadence instead of the
/// 1 fps watch cadence. Capture does **not** raise fps. Focused 50 fps
/// is the Game pane only; sidecar-50 is every drawing rail/grid member.
pub fn full_rate_for(f: &Focus, name: &str) -> bool {
    if !draw_for_slot(f, name) {
        return false;
    }
    if f.live_full_rate {
        return true;
    }
    if f.focused.as_deref() == Some(name) {
        return f.focused_50;
    }
    f.sidecar_50
}

/// Capture (the focused bot's capture checkbox) additionally requires draw.
pub fn should_capture(f: &Focus) -> bool {
    should_draw(f) && f.capture
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::{draw_for_slot, full_rate_for, should_capture, should_draw, Focus};

    #[test]
    fn draw_requires_focus_pane_and_renderer() {
        let mut f = Focus {
            focused: Some("test".into()),
            renderer: true,
            game_pane_open: true,
            capture: false,
            only_render_selected: true,
            sidecar_50: false,
            live_full_rate: false,
            focused_50: false,
            wall_open: false,
            wall: vec![],
            renderer_by: HashMap::new(),
        };
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
        let f = Focus {
            focused: Some("a".into()),
            renderer: true,
            game_pane_open: true,
            capture: true,
            only_render_selected: true,
            sidecar_50: false,
            live_full_rate: false,
            focused_50: false,
            wall_open: false,
            wall: vec![],
            renderer_by: HashMap::new(),
        };
        assert!(should_capture(&f));
        let f = Focus {
            focused: Some("a".into()),
            renderer: false,
            game_pane_open: true,
            capture: true,
            only_render_selected: true,
            sidecar_50: false,
            live_full_rate: false,
            focused_50: false,
            wall_open: false,
            wall: vec![],
            renderer_by: HashMap::new(),
        };
        assert!(!should_capture(&f));
    }

    #[test]
    fn draw_for_slot_requires_this_slot_to_be_focused() {
        let f = Focus {
            focused: Some("a".into()),
            renderer: true,
            game_pane_open: true,
            capture: false,
            only_render_selected: true,
            sidecar_50: false,
            live_full_rate: false,
            focused_50: false,
            wall_open: false,
            wall: vec![],
            renderer_by: HashMap::new(),
        };
        assert!(draw_for_slot(&f, "a"));
        assert!(!draw_for_slot(&f, "b"));
        assert!(!draw_for_slot(&f, ""));
        // No focus: no slot draws.
        let f = Focus {
            focused: None,
            renderer: true,
            game_pane_open: true,
            capture: false,
            only_render_selected: true,
            sidecar_50: false,
            live_full_rate: false,
            focused_50: false,
            wall_open: false,
            wall: vec![],
            renderer_by: HashMap::new(),
        };
        assert!(!draw_for_slot(&f, "a"));
        // Renderer off: the focused slot does not draw either.
        let f = Focus {
            focused: Some("a".into()),
            renderer: false,
            game_pane_open: true,
            capture: false,
            only_render_selected: true,
            sidecar_50: false,
            live_full_rate: false,
            focused_50: false,
            wall_open: false,
            wall: vec![],
            renderer_by: HashMap::new(),
        };
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
            sidecar_50: false,
            live_full_rate: false,
            focused_50: false,
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

    #[test]
    fn full_rate_for_pref_raises_drawing_members_only() {
        let mut f = Focus {
            focused: Some("a".into()),
            renderer: true,
            game_pane_open: true,
            capture: false,
            only_render_selected: false,
            sidecar_50: true,
            live_full_rate: false,
            focused_50: false,
            wall_open: true,
            wall: vec!["a".into(), "b".into()],
            renderer_by: HashMap::from([("a".into(), true), ("b".into(), true)]),
        };
        // Sidecar is rail/grid members only; focused 50 fps is its own knob.
        assert!(full_rate_for(&f, "b"), "drawing member runs at 50 fps");
        assert!(!full_rate_for(&f, "a"), "focused slot keeps its own path");
        // Pref off keeps the 1 fps watch cadence.
        f.sidecar_50 = false;
        assert!(!full_rate_for(&f, "b"));
        // Collapsed rail (only render selected): members do not draw, so
        // the pref cannot raise them.
        f.sidecar_50 = true;
        f.only_render_selected = true;
        assert!(!full_rate_for(&f, "b"));
        f.only_render_selected = false;
        // Wall closed or per-slot renderer off: no draw, no raise.
        f.wall_open = false;
        assert!(!full_rate_for(&f, "b"));
        f.wall_open = true;
        f.renderer_by.insert("b".into(), false);
        assert!(!full_rate_for(&f, "b"));
    }

    #[test]
    fn live_full_rate_raises_every_drawing_slot_including_focused() {
        let mut f = Focus {
            focused: Some("a".into()),
            renderer: true,
            game_pane_open: true,
            capture: false,
            only_render_selected: false,
            sidecar_50: false,
            live_full_rate: true,
            focused_50: false,
            wall_open: true,
            wall: vec!["a".into(), "b".into()],
            renderer_by: HashMap::from([("a".into(), true), ("b".into(), true)]),
        };
        assert!(
            full_rate_for(&f, "a"),
            "focused slot is 50 fps without capture"
        );
        assert!(
            full_rate_for(&f, "b"),
            "drawing member is 50 fps without sidecar_50"
        );
        f.live_full_rate = false;
        assert!(!full_rate_for(&f, "a"));
        assert!(!full_rate_for(&f, "b"));
        f.sidecar_50 = true;
        assert!(
            !full_rate_for(&f, "a"),
            "sidecar still does not raise focused"
        );
        assert!(full_rate_for(&f, "b"));
    }

    #[test]
    fn focused_50_raises_only_the_game_pane_slot() {
        let mut f = Focus {
            focused: Some("a".into()),
            renderer: true,
            game_pane_open: true,
            capture: true,
            only_render_selected: false,
            sidecar_50: false,
            live_full_rate: false,
            focused_50: true,
            wall_open: true,
            wall: vec!["a".into(), "b".into()],
            renderer_by: HashMap::from([("a".into(), true), ("b".into(), true)]),
        };
        assert!(full_rate_for(&f, "a"), "Game pane is 50 fps");
        assert!(
            !full_rate_for(&f, "b"),
            "capture/focused 50 fps must not raise the rail"
        );
        f.focused = Some("b".into());
        assert!(
            !full_rate_for(&f, "a"),
            "a on the rail is watch unless sidecar"
        );
        assert!(
            full_rate_for(&f, "b"),
            "b in the Game pane takes focused 50"
        );
        f.focused_50 = false;
        assert!(!full_rate_for(&f, "b"), "capture alone does not raise fps");
    }
}
