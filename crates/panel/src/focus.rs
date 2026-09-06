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
///
/// The focused slot ignores a per-slot Off: that bit is the **rail**
/// (stress50 members must not grow heads while unfocused). The Game pane's
/// one seat follows focus so a −1 on s00 can still show a working member.
pub fn renderer_for(f: &Focus, name: &str) -> bool {
    if f.focused.as_deref() == Some(name) {
        return f.renderer;
    }
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

/// Opt-in benchmark draw policy from the requested [`host_play::memory::RenderPolicy`].
/// New low-end modes force Game pane, wall membership, cadence knobs and per-slot
/// renderer bits so adverse persisted prefs cannot change the requested cell.
/// Legacy fixed-one / rotating-all paths keep their historical field writes.
#[cfg(feature = "memory-profile")]
pub fn memory_draw_policy(
    f: &mut Focus,
    names: &[String],
    policy: host_play::memory::RenderPolicy,
) {
    use host_play::memory::RenderPolicy;
    f.renderer = true;
    match policy {
        RenderPolicy::RotatingAll => {
            f.only_render_selected = false;
            for name in names {
                f.renderer_by.insert(name.clone(), true);
            }
        }
        RenderPolicy::FixedOne => {
            // Historical --single-renderer / BOT_MEMORY_SINGLE_RENDERER path.
            f.game_pane_open = true;
            f.only_render_selected = true;
            for name in names {
                f.renderer_by.insert(name.clone(), false);
            }
        }
        RenderPolicy::FocusedOne => {
            f.game_pane_open = true;
            f.only_render_selected = true;
            f.focused_50 = true;
            f.sidecar_50 = false;
            f.live_full_rate = false;
            f.wall_open = true;
            f.wall = names.to_vec();
            for name in names {
                f.renderer_by.insert(name.clone(), false);
            }
        }
        RenderPolicy::FocusedPlusBackground => {
            f.game_pane_open = true;
            f.only_render_selected = false;
            f.focused_50 = true;
            f.sidecar_50 = false;
            f.live_full_rate = false;
            f.wall_open = true;
            f.wall = names.to_vec();
            for name in names {
                f.renderer_by.insert(name.clone(), true);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::{draw_for_slot, full_rate_for, should_capture, should_draw, Focus};

    #[cfg(feature = "memory-profile")]
    fn adverse_focus(names: &[String]) -> Focus {
        Focus {
            focused: Some(names[0].clone()),
            renderer: false,
            game_pane_open: false,
            capture: false,
            only_render_selected: false,
            sidecar_50: true,
            live_full_rate: true,
            focused_50: false,
            wall_open: false,
            wall: vec![],
            renderer_by: HashMap::from([(names[0].clone(), false)]),
        }
    }

    #[cfg(feature = "memory-profile")]
    #[test]
    fn memory_single_renderer_keeps_other_thirty_one_heads_off() {
        use host_play::memory::RenderPolicy;
        let names: Vec<_> = (0..32).map(|i| format!("bot{i}")).collect();
        let mut f = Focus {
            focused: Some(names[0].clone()),
            renderer: false,
            game_pane_open: false,
            capture: false,
            only_render_selected: false,
            sidecar_50: false,
            live_full_rate: false,
            focused_50: true,
            wall_open: true,
            wall: names.clone(),
            renderer_by: HashMap::new(),
        };
        super::memory_draw_policy(&mut f, &names, RenderPolicy::FixedOne);
        assert_eq!(names.iter().filter(|n| draw_for_slot(&f, n)).count(), 1);
        assert!(draw_for_slot(&f, &names[0]));
        f.only_render_selected = false;
        assert_eq!(
            names.iter().filter(|n| draw_for_slot(&f, n)).count(),
            1,
            "rail override stays off"
        );
        super::memory_draw_policy(&mut f, &names, RenderPolicy::RotatingAll);
        assert_eq!(
            names.iter().filter(|n| draw_for_slot(&f, n)).count(),
            32,
            "existing all-render mode"
        );
    }

    #[cfg(feature = "memory-profile")]
    #[test]
    fn memory_focused_one_is_deterministic_for_one_and_sixteen() {
        use host_play::memory::RenderPolicy;
        for n in [1usize, 16] {
            let names: Vec<_> = (0..n).map(|i| format!("bot{i}")).collect();
            let mut f = adverse_focus(&names);
            super::memory_draw_policy(&mut f, &names, RenderPolicy::FocusedOne);
            assert!(f.game_pane_open);
            assert!(f.only_render_selected);
            assert!(f.focused_50);
            assert!(!f.sidecar_50);
            assert!(!f.live_full_rate);
            assert_eq!(f.wall, names);
            assert!(f.wall_open);
            assert_eq!(names.iter().filter(|n| draw_for_slot(&f, n)).count(), 1);
            assert!(draw_for_slot(&f, &names[0]));
            assert!(full_rate_for(&f, &names[0]), "focused seat full-rate");
            for name in names.iter().skip(1) {
                assert!(!draw_for_slot(&f, name), "{name} sim-only");
                assert!(!full_rate_for(&f, name));
            }
        }
    }

    #[cfg(feature = "memory-profile")]
    #[test]
    fn memory_focused_plus_background_is_deterministic_for_one_and_sixteen() {
        use host_play::memory::RenderPolicy;
        for n in [1usize, 16] {
            let names: Vec<_> = (0..n).map(|i| format!("bot{i}")).collect();
            let mut f = adverse_focus(&names);
            super::memory_draw_policy(&mut f, &names, RenderPolicy::FocusedPlusBackground);
            assert!(f.game_pane_open);
            assert!(!f.only_render_selected);
            assert!(f.focused_50);
            assert!(!f.sidecar_50);
            assert!(!f.live_full_rate);
            assert!(f.wall_open);
            assert_eq!(f.wall, names);
            assert_eq!(
                names.iter().filter(|n| draw_for_slot(&f, n)).count(),
                n,
                "every requested slot draws"
            );
            assert!(full_rate_for(&f, &names[0]), "focused full-rate");
            for name in names.iter().skip(1) {
                assert!(draw_for_slot(&f, name), "{name} draws");
                assert!(
                    !full_rate_for(&f, name),
                    "{name} stays 1 fps skip-paint cadence"
                );
            }
        }
    }

    #[cfg(feature = "memory-profile")]
    #[test]
    fn memory_policy_switch_between_modes() {
        use host_play::memory::RenderPolicy;
        let names: Vec<_> = (0..16).map(|i| format!("bot{i}")).collect();
        let mut f = adverse_focus(&names);
        super::memory_draw_policy(&mut f, &names, RenderPolicy::FocusedOne);
        assert_eq!(names.iter().filter(|n| draw_for_slot(&f, n)).count(), 1);
        super::memory_draw_policy(&mut f, &names, RenderPolicy::FocusedPlusBackground);
        assert_eq!(names.iter().filter(|n| draw_for_slot(&f, n)).count(), 16);
        assert!(full_rate_for(&f, &names[0]));
        assert!(!full_rate_for(&f, &names[1]));
        super::memory_draw_policy(&mut f, &names, RenderPolicy::FixedOne);
        assert_eq!(names.iter().filter(|n| draw_for_slot(&f, n)).count(), 1);
        super::memory_draw_policy(&mut f, &names, RenderPolicy::RotatingAll);
        assert_eq!(names.iter().filter(|n| draw_for_slot(&f, n)).count(), 16);
    }

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

    /// Per-slot Off is the rail. The Game pane's one head follows focus
    /// so stress50 can click a working member after s00's handshake −1.
    #[test]
    fn focused_slot_draws_even_when_renderer_by_is_off() {
        let mut f = Focus {
            focused: Some("b".into()),
            renderer: true,
            game_pane_open: true,
            capture: false,
            only_render_selected: true,
            sidecar_50: false,
            live_full_rate: false,
            focused_50: false,
            wall_open: true,
            wall: vec!["a".into(), "b".into(), "c".into()],
            renderer_by: HashMap::from([
                ("a".into(), true),
                ("b".into(), false),
                ("c".into(), false),
            ]),
        };
        assert!(
            draw_for_slot(&f, "b"),
            "Game pane follows focus onto a rail-Off profile"
        );
        assert!(
            !draw_for_slot(&f, "a"),
            "only-render-selected: unfocused GPU profile drops the head"
        );
        f.only_render_selected = false;
        assert!(draw_for_slot(&f, "b"));
        assert!(draw_for_slot(&f, "a"));
        assert!(
            !draw_for_slot(&f, "c"),
            "unfocused raster Off still cannot grow a head on render-all"
        );
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
