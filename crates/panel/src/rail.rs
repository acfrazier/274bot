//! Sidecar rail chrome: window geometry, the tile size, and the traffic
//! light that colours each member's cap dot.

/// Width of the MultiBox sidecar rail (rs2b0t's 264px strip).
pub const RAIL_W: f32 = 264.0;
/// Cap-body tile draw size inside the rail (rs2b0t ~236×155).
pub const TILE_W: f32 = 236.0;
pub const TILE_H: f32 = 155.0;
/// Default OS window without the rail (game + 330 chrome).
pub const BASE_WINDOW_W: f32 = 1120.0;
/// Default OS window height.
pub const BASE_WINDOW_H: f32 = 580.0;

/// OS inner size: rail-open grows width by [`RAIL_W`] so the Game pane
/// stays the same; grid / MultiBox-off restore the base.
pub fn os_window_size(rail_open: bool) -> (f32, f32) {
    (
        BASE_WINDOW_W + if rail_open { RAIL_W } else { 0.0 },
        BASE_WINDOW_H,
    )
}

/// Status dot glyph (U+2059), colored by [`Light::rgb`].
pub const STATUS_GLYPH: &str = "\u{2059}";
/// Remove glyph (U+2717), drawn in `theme::ERROR` red.
pub const REMOVE_GLYPH: &str = "\u{2717}";

/// Cap dot state: error red wins, then not-ingame grey (logged out),
/// then running green, else idle yellow. A FIFO-queued login slot is not
/// ingame, so it is grey.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Light {
    /// Unknown / logged out — not ingame and no error.
    Grey,
    /// Login or runtime error.
    Red,
    /// Idle — ingame and nothing running (paused/stopping scripts, the
    /// run orb).
    Yellow,
    /// Running — ingame and a script is Running or nav is queued.
    Green,
}

impl Light {
    /// The cap dot's fill color (amber CRT palette).
    pub fn rgb(&self) -> [f32; 4] {
        match self {
            Light::Grey => crate::theme::TEXT_DIM,
            Light::Red => crate::theme::ERROR,
            Light::Yellow => crate::theme::ACCENT,
            Light::Green => crate::theme::GREEN,
        }
    }

    /// Short status label for the cap head title (`"{name}: {brief}"`).
    pub fn brief(&self) -> &'static str {
        match self {
            Light::Grey => "logged out",
            Light::Red => "error",
            Light::Yellow => "idle",
            Light::Green => "running",
        }
    }
}

/// The cap head title: member name plus the light's brief status.
pub fn cap_title(name: &str, light: Light) -> String {
    format!("{name}: {}", light.brief())
}

/// Map a slot's status to its tile's traffic light: error red wins, then
/// not-ingame → grey, then running → green, else idle yellow.
pub fn traffic_light(ingame: bool, error: bool, running: bool) -> Light {
    if error {
        Light::Red
    } else if !ingame {
        Light::Grey
    } else if running {
        Light::Green
    } else {
        Light::Yellow
    }
}

#[cfg(test)]
mod tests {
    use super::{
        cap_title, os_window_size, traffic_light, Light, BASE_WINDOW_H, BASE_WINDOW_W, RAIL_W,
        REMOVE_GLYPH, STATUS_GLYPH, TILE_H, TILE_W,
    };

    #[test]
    fn rail_constants_match_the_plan() {
        assert_eq!(RAIL_W, 264.0);
        assert_eq!(TILE_W, 236.0);
        assert_eq!(TILE_H, 155.0);
        assert_eq!(crate::theme::RAIL_WINDOW, "274bot-rail");
    }

    #[test]
    fn os_window_grows_by_rail_width_and_keeps_height() {
        assert_eq!(os_window_size(false), (BASE_WINDOW_W, BASE_WINDOW_H));
        assert_eq!(
            os_window_size(true),
            (BASE_WINDOW_W + RAIL_W, BASE_WINDOW_H)
        );
    }

    #[test]
    fn traffic_light_maps_all_four_states() {
        // Unknown / logged out: not ingame, no error.
        assert_eq!(traffic_light(false, false, false), Light::Grey);
        // A FIFO-queued login slot is not ingame, so it is grey, not running.
        assert_eq!(traffic_light(false, false, true), Light::Grey);
        // Error red wins over ingame and running.
        assert_eq!(traffic_light(false, true, false), Light::Red);
        assert_eq!(traffic_light(true, true, true), Light::Red);
        assert_eq!(traffic_light(false, true, true), Light::Red);
        // Idle yellow: ingame and nothing running.
        assert_eq!(traffic_light(true, false, false), Light::Yellow);
        // Running green: ingame and (script running or nav queued).
        assert_eq!(traffic_light(true, false, true), Light::Green);
    }

    #[test]
    fn idle_is_not_running() {
        // Paused / Stopping scripts and the run orb are not running.
        assert_ne!(
            traffic_light(true, false, false),
            traffic_light(true, false, true)
        );
        // A queued-login (title screen) slot is not running either.
        assert_eq!(traffic_light(false, false, true), Light::Grey);
    }

    #[test]
    fn brief_labels_each_state() {
        assert_eq!(Light::Grey.brief(), "logged out");
        assert_eq!(Light::Red.brief(), "error");
        assert_eq!(Light::Yellow.brief(), "idle");
        assert_eq!(Light::Green.brief(), "running");
    }

    #[test]
    fn cap_title_renders_name_and_brief() {
        assert_eq!(cap_title("bob", Light::Grey), "bob: logged out");
        assert_eq!(cap_title("bob", Light::Red), "bob: error");
        assert_eq!(cap_title("bob", Light::Yellow), "bob: idle");
        assert_eq!(cap_title("bob", Light::Green), "bob: running");
    }

    #[test]
    fn cap_glyphs_are_the_spec_code_points() {
        assert_eq!(STATUS_GLYPH, "\u{2059}");
        assert_eq!(REMOVE_GLYPH, "\u{2717}");
    }
}
