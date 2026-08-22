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
        Section { id: "profile", wired: true, campaign_hint: None },
        Section { id: "credentials", wired: true, campaign_hint: None },
        Section { id: "script", wired: false, campaign_hint: Some("campaign 5") },
        Section { id: "parameters", wired: false, campaign_hint: Some("campaign 5") },
        Section { id: "status", wired: true, campaign_hint: None },
        Section { id: "log", wired: true, campaign_hint: None },
        Section { id: "rendering", wired: true, campaign_hint: None },
        Section { id: "input", wired: true, campaign_hint: None },
    ]
}

/// Global/Nav/Loadouts/MultiBox are controls inside parameters/title, still listed.
pub const MOCK_BUTTONS: &[&str] = &[
    "Browse…",
    "Start",
    "Pause",
    "Stop",
    "Global settings",
    "Nav settings",
    "Loadouts",
    "MultiBox",
];

#[cfg(test)]
mod tests {
    use crate::chrome::{MOCK_BUTTONS, sections};
    use crate::theme::{ACCENT, integer_ui_scale};

    #[test]
    fn sections_contains_all_section_ids() {
        let ids: Vec<&str> = sections().iter().map(|s| s.id).collect();
        for id in ["profile", "credentials", "script", "parameters", "status", "log", "rendering", "input"] {
            assert!(ids.contains(&id), "missing section id {id:?}");
        }
    }

    #[test]
    fn accent_is_amber_not_green() {
        assert_eq!(ACCENT[0], 1.0);
        assert!(ACCENT[1] > 0.6, "amber G must exceed 0.6");
        assert!(ACCENT[0] > ACCENT[1], "amber is red-dominant, never green");
    }

    #[test]
    fn mock_buttons_include_panel_controls() {
        for b in ["Global settings", "Nav settings", "Loadouts", "MultiBox"] {
            assert!(MOCK_BUTTONS.contains(&b), "missing mock button {b:?}");
        }
    }

    #[test]
    fn integer_ui_scale_retina_is_2_and_never_below_1() {
        assert_eq!(integer_ui_scale(2.0), 2.0);
        assert_eq!(integer_ui_scale(1.0), 1.0);
        assert_eq!(integer_ui_scale(1.75), 2.0);
        assert_eq!(integer_ui_scale(0.5), 1.0);
    }
}
