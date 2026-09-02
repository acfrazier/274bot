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
            id: "script",
            wired: true,
            campaign_hint: None,
        },
        Section {
            id: "debug",
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
    button_row_layout_min(avail, count, MIN_BUTTON)
}

/// Like [`button_row_layout`], with an explicit minimum cell width.
pub fn button_row_layout_min(avail: f32, count: usize, min: f32) -> (f32, bool) {
    let w = equal_button_width(avail, count);
    if w < min {
        (avail.max(1.0), true)
    } else {
        (w, false)
    }
}

/// Wrapping equal-width cells: `(width, same_line)`. A leftover last row
/// (one button) takes the full `avail`, matching WalkTo under Log in/Logout.
pub fn button_cells(avail: f32, count: usize) -> Vec<(f32, bool)> {
    button_cells_min(avail, count, MIN_BUTTON)
}

/// Like [`button_cells`], wrapping when a cell would be narrower than `min`.
pub fn button_cells_min(avail: f32, count: usize, min: f32) -> Vec<(f32, bool)> {
    let count = count.max(1);
    let avail = avail.max(1.0);
    let min = min.max(1.0);
    let mut per_row = 1usize;
    for n in 1..=count {
        if equal_button_width(avail, n) + 0.01 >= min {
            per_row = n;
        }
    }
    (0..count)
        .map(|i| {
            let row = i / per_row;
            let last_row = (count - 1) / per_row;
            let n_this = if row == last_row {
                count - row * per_row
            } else {
                per_row
            };
            (equal_button_width(avail, n_this), i % per_row != 0)
        })
        .collect()
}

/// Longest General-config label needs more than [`MIN_BUTTON`] or the
/// 330px 3-up row clips "General config".
pub const CONFIG_MIN: f32 = 124.0;

/// Title-row tooltips for the wired MultiBox toggle: `on` = the sidecar
/// rail/grid is up, `off` = closing it does not log anyone out.
pub fn multibox_tooltip(on: bool) -> &'static str {
    if on {
        "hide rail — slots keep running"
    } else {
        "sidecar wall"
    }
}

/// Default strip heading order. Drag a collapsing header to reorder;
/// unknown/missing ids are filled from this list.
pub const HEADING_ORDER: &[&str] = &["status", "profile", "script", "parameters", "debug", "log"];

/// Merge a saved order with [`HEADING_ORDER`]: drop unknown ids, append
/// any missing defaults (so old prefs still show new headings).
pub fn resolve_heading_order(saved: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for id in saved {
        if HEADING_ORDER.contains(&id.as_str()) && !out.iter().any(|x| x == id) {
            out.push(id.clone());
        }
    }
    for id in HEADING_ORDER {
        if !out.iter().any(|x| x == *id) {
            out.push((*id).to_string());
        }
    }
    out
}

/// Move `from` in front of `onto`. No-op if either id is missing.
pub fn move_heading(order: &mut Vec<String>, from: &str, onto: &str) {
    if from == onto {
        return;
    }
    let Some(i) = order.iter().position(|x| x == from) else {
        return;
    };
    order.remove(i);
    let at = order.iter().position(|x| x == onto).unwrap_or(order.len());
    order.insert(at, from.to_string());
}

/// Under WalkTo, above the reorderable headings: General config, Nav
/// config, and Loadouts are live windows.
pub const MOCK_BUTTONS: &[&str] = &[];

pub const SCRIPT_ROW: &[&str] = &["Start", "Pause", "Stop"];
/// Under WalkTo, not credentials and not parameters.
pub const CONFIG_ROW: &[&str] = &["General config", "Nav config", "Loadouts"];

#[cfg(test)]
mod tests {
    use crate::chrome::{
        button_cells, button_cells_min, button_row_layout, equal_button_width, move_heading,
        multibox_tooltip, resolve_heading_order, sections, BUTTON_GAP, CONFIG_MIN, CONFIG_ROW,
        HEADING_ORDER, MIN_BUTTON, MOCK_BUTTONS, SCRIPT_ROW,
    };
    use crate::theme::{apply_amber, integer_ui_scale, ACCENT, PANEL_WIDTH};

    #[test]
    fn heading_order_defaults_and_merges() {
        assert_eq!(
            resolve_heading_order(&[]),
            HEADING_ORDER
                .iter()
                .map(|s| (*s).to_string())
                .collect::<Vec<_>>()
        );
        let saved = vec!["log".into(), "nope".into(), "status".into()];
        assert_eq!(
            resolve_heading_order(&saved),
            ["log", "status", "profile", "script", "parameters", "debug"]
        );
        let mut order = resolve_heading_order(&[]);
        move_heading(&mut order, "profile", "status");
        assert_eq!(order[0], "profile");
        assert_eq!(order[1], "status");
        move_heading(&mut order, "profile", "profile");
        assert_eq!(order[0], "profile");
    }

    #[test]
    fn sections_contains_all_section_ids() {
        let ids: Vec<&str> = sections().iter().map(|s| s.id).collect();
        for id in [
            "profile",
            "script",
            "parameters",
            "status",
            "log",
            "rendering",
            "input",
            "debug",
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
    fn multibox_is_wired_not_a_mock() {
        assert!(
            !MOCK_BUTTONS.contains(&"MultiBox"),
            "MultiBox is a live toggle; only parameter chrome stays mocked"
        );
        assert!(
            !MOCK_BUTTONS.contains(&"Loadouts"),
            "Loadouts is wired, not a mock"
        );
        assert!(
            !MOCK_BUTTONS.contains(&"General config"),
            "General config opens slot render + global cadence"
        );
        for b in ["Browse…", "Start", "Pause", "Stop"] {
            assert!(
                !MOCK_BUTTONS.contains(&b),
                "script {b:?} is wired, not a mock"
            );
        }
    }

    #[test]
    fn walkto_row_wires_slot_and_nav_and_loadouts() {
        assert_eq!(CONFIG_ROW, ["General config", "Nav config", "Loadouts"]);
        assert!(
            !MOCK_BUTTONS.contains(&"Nav config"),
            "Nav config is its own window"
        );
        assert!(!MOCK_BUTTONS.contains(&"General config"));
        assert!(
            !MOCK_BUTTONS.contains(&"Loadouts"),
            "Loadouts opens its own window"
        );
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
        // Debug 4-up at a scrollbar-trimmed strip would stack under
        // MIN_BUTTON; packed rows use equal_button_width anyway.
        let packed = equal_button_width(280.0, 4);
        assert!(packed < MIN_BUTTON);
        assert!(packed > 40.0);
        const {
            assert!(MIN_BUTTON > 0.0);
        }
        assert_eq!(SCRIPT_ROW.len(), 3);
        assert_eq!(CONFIG_ROW.len(), 3);
        let script = button_cells(avail, 3);
        assert_eq!(script.len(), 3);
        assert!(script[1].1 && script[2].1, "Start/Pause/Stop stay one row");
        let cfg = button_cells_min(avail, 3, CONFIG_MIN);
        assert!(
            !cfg[1].1 || !cfg[2].1,
            "General config wraps instead of clipping"
        );
        assert!(
            cfg[2].0 > cfg[0].0,
            "leftover config button is full width like WalkTo"
        );
    }

    #[test]
    fn login_logout_row_is_two_buttons() {
        let (w, stack) = button_row_layout(330.0, 2);
        assert!(!stack);
        assert!(w > 100.0);
        let cells = button_cells(330.0, 2);
        assert_eq!(cells.len(), 2);
        assert!(cells[1].1);
        assert!((cells[0].0 - cells[1].0).abs() < 0.01);
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
        assert_eq!(
            ctx.style().window_menu_button_position(),
            dear_imgui_rs::Direction::None,
            "tab-bar corner menu is off"
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
