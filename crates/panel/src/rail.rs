//! Sidecar rail chrome: window geometry, the tile size, and the traffic
//! light that colours each member's cap dot.

use dear_imgui_rs::{DrawSegmentCount, Ui};

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

/// The status dot's box width (the space the old `U+2059` glyph
/// occupied); the quincunx is drawn inside it via the draw list.
pub const DOT_W: f32 = 16.0;

/// Remove glyph (U+00D7, the multiplication sign, in Latin-1), drawn in
/// `theme::ERROR` red. U+2717 is not in dear-imgui's default Latin-1
/// font and rendered as `?`.
pub const REMOVE_GLYPH: &str = "\u{00D7}";

/// The five quincunx centers for a status dot centered at `center` with
/// `spread`: the center plus four corners. Pure geometry so the draw
/// path is unit-testable.
pub fn quincunx_centers(center: [f32; 2], spread: f32) -> [[f32; 2]; 5] {
    [
        center,
        [center[0] - spread, center[1] - spread],
        [center[0] + spread, center[1] - spread],
        [center[0] - spread, center[1] + spread],
        [center[0] + spread, center[1] + spread],
    ]
}

/// Draw the five-dot status quincunx at the current cursor in `color`
/// and reserve the box the text glyph occupied ([`DOT_W`] wide, one text
/// line tall), then `same_line` back onto it — the exact
/// `text + same_line` flow the old `U+2059` glyph used. Replaces the
/// glyph, which dear-imgui's default Latin-1 font cannot render, with
/// five filled circles in the same box.
pub fn status_quincunx(ui: &Ui, color: [f32; 4]) {
    let pos = ui.cursor_pos();
    let center = [pos[0] + DOT_W * 0.5, pos[1] + ui.text_line_height() * 0.5];
    const RADIUS: f32 = 2.0;
    const SPREAD: f32 = 3.0;
    let draw_list = ui.get_window_draw_list();
    for c in quincunx_centers(center, SPREAD) {
        draw_list
            .add_circle(c, RADIUS, color)
            .filled(true)
            .num_segments(DrawSegmentCount::new(8).expect("8 segments in range"))
            .build();
    }
    ui.dummy([DOT_W, ui.text_line_height()]);
    ui.same_line();
}

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
        cap_title, os_window_size, quincunx_centers, status_quincunx, traffic_light, Light,
        REMOVE_GLYPH, BASE_WINDOW_H, BASE_WINDOW_W, DOT_W, RAIL_W, TILE_H, TILE_W,
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
    fn cap_glyphs_render_in_latin1() {
        // The remove glyph is U+00D7 (multiplication sign), in dear-imgui's
        // default Latin-1 font; U+2717 and U+2059 are not.
        assert_eq!(REMOVE_GLYPH, "\u{00D7}");
    }

    #[test]
    fn quincunx_centers_are_center_plus_four_corners() {
        let centers = quincunx_centers([10.0, 20.0], 3.0);
        assert_eq!(centers.len(), 5);
        assert_eq!(centers[0], [10.0, 20.0]);
        assert_eq!(centers[1], [7.0, 17.0]);
        assert_eq!(centers[2], [13.0, 17.0]);
        assert_eq!(centers[3], [7.0, 23.0]);
        assert_eq!(centers[4], [13.0, 23.0]);
    }

    #[test]
    fn status_quincunx_draws_and_advances_the_cursor_by_dot_w() {
        let _guard = crate::IMGUI_CTX_TEST_GUARD.lock().unwrap();
        let mut ctx = dear_imgui_rs::Context::create();
        // A bare context frame needs the font atlas path set up; the
        // renderer-has-textures flag mirrors what the wgpu renderer sets.
        ctx.prepare_frame(
            dear_imgui_rs::FramePrepareOptions::new([200.0, 200.0], 1.0 / 60.0)
                .renderer_has_textures(),
        );
        let ui = ctx.frame();
        let mut delta = [0.0f32; 2];
        ui.window("##274bot-quincunx-test")
            .size([100.0, 50.0], dear_imgui_rs::Condition::Always)
            .build(|| {
                let before = ui.cursor_pos();
                status_quincunx(&ui, [1.0, 0.0, 0.0, 1.0]);
                let after = ui.cursor_pos();
                delta = [after[0] - before[0], after[1] - before[1]];
            });
        // The dummy reserves DOT_W and same_line() brings the cursor back
        // onto that line plus the default item spacing.
        assert!(
            (delta[0] - DOT_W - 8.0).abs() < 0.01,
            "the quincunx reserves the DOT_W box (moved x by {delta:?})"
        );
        assert!(
            delta[1].abs() < 0.01,
            "same_line keeps the next item on the dot's row (y delta {delta:?})"
        );
    }
}
