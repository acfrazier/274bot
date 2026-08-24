/// A chrome inventory entry: one left-nav section, whether it is wired to a
/// live handler, and which campaign is expected to implement it.
pub struct Section {
    pub id: &'static str,
    pub wired: bool,
    pub campaign_hint: Option<&'static str>,
}

/// Static inventory of rs2b0t chrome sections.
pub fn sections() -> &'static [Section] {
    &[
        Section {
            id: "profile",
            wired: true,
            campaign_hint: None,
        },
        Section {
            id: "credentials",
            wired: true,
            campaign_hint: None,
        },
        Section {
            id: "script",
            wired: true,
            campaign_hint: None,
        },
        Section {
            id: "parameters",
            wired: false,
            campaign_hint: Some("campaign 5"),
        },
        Section {
            id: "status",
            wired: true,
            campaign_hint: None,
        },
        Section {
            id: "log",
            wired: true,
            campaign_hint: None,
        },
        Section {
            id: "rendering",
            wired: true,
            campaign_hint: None,
        },
        Section {
            id: "input",
            wired: true,
            campaign_hint: None,
        },
    ]
}

/// Gap between equal-width buttons (rs2b0t `gap: 6px`).
pub const BUTTON_GAP: f32 = 6.0;
/// Below this, a row of equal buttons stacks full-width instead of overflowing.
pub const MIN_BUTTON: f32 = 72.0;

/// Width of each of `count` equal buttons in an `avail` strip, with gaps.
/// The row never exceeds `avail` (the 330px panel must not grow a horizontal
/// scrollbar).
pub fn equal_button_width(avail: f32, count: usize) -> f32 {
    let n = count.max(1) as f32;
    let avail = avail.max(1.0);
    ((avail - BUTTON_GAP * (n - 1.0)) / n).max(1.0)
}

/// `(button_width, stack)`: stack full-width when equal cells would be too
/// narrow for the label to exist without forcing a horizontal scroll.
pub fn button_row_layout(avail: f32, count: usize) -> (f32, bool) {
    let w = equal_button_width(avail, count);
    if w < MIN_BUTTON {
        (avail.max(1.0), true)
    } else {
        (w, false)
    }
}

/// Title-row tooltips for the wired MultiBox toggle: `on` = the sidecar
/// rail/grid is up, `off` = closing it does not log anyone out.
pub fn multibox_tooltip(on: bool) -> &'static str {
    if on {
        "hide rail — slots keep running"
    } else {
        "sidecar wall"
    }
}

/// Hover copy for TV-mode controls that would spawn extra rasters
/// (per-profile renderer, render-all, grid). Channel-head is one tube.
pub const GPU_LATER: &str = "gpu later or opti yourself";

/// Global/Nav/Loadouts are controls inside parameters, still listed.
/// MultiBox was a title-row mock before campaign 4; it is now wired.
/// Script chrome (Browse…/Start/Pause/Stop) is wired as of campaign 5.
pub const MOCK_BUTTONS: &[&str] = &["Global settings", "Nav settings", "Loadouts"];

pub const SCRIPT_ROW: &[&str] = &["Start", "Pause", "Stop"];
pub const PARAM_ROW: &[&str] = &["Global settings", "Nav settings", "Loadouts"];

#[cfg(test)]
mod tests {
    use crate::chrome::{
        button_row_layout, equal_button_width, multibox_tooltip, sections, BUTTON_GAP, GPU_LATER,
        MIN_BUTTON, MOCK_BUTTONS, PARAM_ROW, SCRIPT_ROW,
    };
    use crate::theme::{apply_amber, integer_ui_scale, ACCENT, PANEL_WIDTH};

    #[test]
    fn sections_contains_all_section_ids() {
        let ids: Vec<&str> = sections().iter().map(|s| s.id).collect();
        for id in [
            "profile",
            "credentials",
            "script",
            "parameters",
            "status",
            "log",
            "rendering",
            "input",
        ] {
            assert!(ids.contains(&id), "missing section id {id:?}");
        }
    }

    #[test]
    fn accent_is_amber_not_green() {
        assert_eq!(ACCENT[0], 1.0);
        const {
            assert!(ACCENT[1] > 0.6, "amber G must exceed 0.6");
        }
        const {
            assert!(ACCENT[0] > ACCENT[1], "amber is red-dominant, never green");
        }
    }

    #[test]
    fn multibox_tooltip_matches_the_plan_copy() {
        assert_eq!(multibox_tooltip(true), "hide rail — slots keep running");
        assert_eq!(multibox_tooltip(false), "sidecar wall");
    }

    #[test]
    fn gpu_later_is_the_tv_mode_hover() {
        assert_eq!(GPU_LATER, "gpu later or opti yourself");
    }

    #[test]
    fn multibox_is_wired_not_a_mock() {
        assert!(
            !MOCK_BUTTONS.contains(&"MultiBox"),
            "MultiBox is a live toggle; only parameter chrome stays mocked"
        );
        for b in ["Global settings", "Nav settings", "Loadouts"] {
            assert!(
                MOCK_BUTTONS.contains(&b),
                "param mock {b:?} must still be listed"
            );
        }
        for b in ["Browse…", "Start", "Pause", "Stop"] {
            assert!(
                !MOCK_BUTTONS.contains(&b),
                "script {b:?} is wired, not a mock"
            );
        }
    }

    #[test]
    fn browse_start_stop_are_not_mocks() {
        assert!(!MOCK_BUTTONS.contains(&"Browse…"));
        assert!(!MOCK_BUTTONS.contains(&"Start"));
        assert!(!MOCK_BUTTONS.contains(&"Pause"));
        assert!(!MOCK_BUTTONS.contains(&"Stop"));
    }

    #[test]
    fn script_section_is_wired() {
        assert!(
            sections().iter().any(|s| s.id == "script" && s.wired),
            "the script section is un-mocked this task"
        );
    }

    #[test]
    fn integer_ui_scale_retina_is_2_and_never_below_1() {
        assert_eq!(integer_ui_scale(2.0), 2.0);
        assert_eq!(integer_ui_scale(1.0), 1.0);
        assert_eq!(integer_ui_scale(1.75), 2.0);
        assert_eq!(integer_ui_scale(0.5), 1.0);
    }

    #[test]
    fn equal_button_width_stays_inside_panel() {
        // 330 minus 10px padding each side, the rs2b0t content box.
        let avail = PANEL_WIDTH - 20.0;
        for n in 1..=4 {
            let w = equal_button_width(avail, n);
            let total = w * n as f32 + BUTTON_GAP * (n.saturating_sub(1) as f32);
            assert!(
                total <= avail + 0.01,
                "row of {n} buttons {total} overflowed {avail}"
            );
        }
        let (w, stack) = button_row_layout(40.0, 3);
        assert!(stack);
        assert_eq!(w, 40.0);
        let (_, stack) = button_row_layout(avail, 3);
        assert!(!stack, "Start/Pause/Stop fit the 330px strip");
        const {
            assert!(MIN_BUTTON > 0.0);
        }
        assert_eq!(SCRIPT_ROW.len(), 3);
        assert_eq!(PARAM_ROW.len(), 3);
    }

    #[test]
    fn credentials_grid_is_two_by_two() {
        let (w, stack) = button_row_layout(330.0, 2);
        assert!(!stack);
        assert!(w > 100.0);
    }

    #[test]
    fn apply_amber_replaces_imgui_blue_title() {
        let _guard = crate::IMGUI_CTX_TEST_GUARD.lock().unwrap();
        let mut ctx = dear_imgui_rs::Context::create();
        apply_amber(ctx.style_mut());
        let title = ctx.style().color(dear_imgui_rs::StyleColor::TitleBgActive);
        // Default imgui TitleBgActive is the blue-gray chrome. Amber CRT is
        // dark and red-dominant, never that blue.
        assert!(title[2] < 0.15, "title bar must not be imgui blue");
        assert!(title[0] > title[2]);
        let bg = ctx.style().color(dear_imgui_rs::StyleColor::WindowBg);
        assert!(bg[0] < 0.1 && bg[1] < 0.1 && bg[2] < 0.1);
        let hover = ctx.style().color(dear_imgui_rs::StyleColor::ButtonHovered);
        assert!(
            hover[1] < 0.5,
            "hover fill must stay dark so #ddd text is readable on amber"
        );
        let check_bg = ctx
            .style()
            .color(dear_imgui_rs::StyleColor::CheckboxSelectedBg);
        assert!(
            check_bg[2] < 0.2,
            "checkbox fill must not be default imgui blue"
        );
        let frame_hover = ctx.style().color(dear_imgui_rs::StyleColor::FrameBgHovered);
        assert!(frame_hover[2] < 0.2, "frame hover must not be imgui blue");
    }
}
