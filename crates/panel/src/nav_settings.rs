/// Persisted nav-debug settings (`PanelUiState.nav`, rs2b0t Path-paint
/// defaults) plus the live-harness overlay that forces paint layers on
/// for a run without writing prefs.
///
/// `#[serde(default)]`: a prefs file written before a field existed (e.g.
/// `allow_wilderness`) still loads, with missing fields filled from
/// [`Default`] — otherwise `load_at` would fail the whole `PanelUiState`
/// deserialize and wipe focus/collapsed/colors.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct NavSettings {
    /// Durable global permission for every walk to use teleports.
    pub allow_teleports: bool,
    /// Durable global permission for every walk to enter the wilderness.
    pub allow_wilderness: bool,
    /// Durable global permission for manual WalkTo to use BankBudget fetch.
    pub allow_bank_fetch: bool,
    /// Durable global override for every walk to route through danger zones.
    pub allow_danger_zones: bool,
    /// Allow admission of survivable crossings once the runtime net is available.
    pub survivable_routing: bool,
    /// The shared one-time script-scope explanation was dismissed.
    pub script_scope_notice_ack: bool,
    /// The one-time danger-routing migration explanation was dismissed.
    pub survivable_routing_notice_ack: bool,
    pub show_nav_path: bool,
    pub hop_labels: bool,
    /// 11px default; the settings UI clamps writes to 8..=28.
    pub hop_label_px: i32,
    pub color_path: String,
    pub color_transport: String,
    pub color_click: String,
    pub color_text: String,
    pub collision_fill: bool,
    pub nsew_labels: bool,
    pub client_trail: bool,
    pub color_collision: String,
    pub color_client: String,
    pub color_client_run_alt: String,
    pub component_flood: bool,
    /// Persisted WalkTo map tint for content-defined special areas.
    pub show_special_areas: bool,
    /// Ease orbit yaw toward the remaining path (rs2b0t `navCameraFollow`).
    pub camera_follow: bool,
    /// Pause the owning script after qualifying manual movement cancels its walk.
    pub pause_script_on_manual_walk_abort: bool,
}

impl Default for NavSettings {
    /// rs2b0t Path-paint defaults: path red, transport green, click/text
    /// white, collision reserved blue, client cyan, run-alt yellow.
    fn default() -> Self {
        Self {
            allow_teleports: false,
            allow_wilderness: false,
            allow_bank_fetch: false,
            allow_danger_zones: false,
            survivable_routing: true,
            script_scope_notice_ack: false,
            survivable_routing_notice_ack: false,
            show_nav_path: false,
            hop_labels: true,
            hop_label_px: 11,
            color_path: "#FF0000".into(),
            color_transport: "#00FF00".into(),
            color_click: "#FFFFFF".into(),
            color_text: "#FFFFFF".into(),
            collision_fill: false,
            nsew_labels: false,
            client_trail: false,
            color_collision: "#0080FF".into(),
            color_client: "#00D4FF".into(),
            color_client_run_alt: "#FFFF00".into(),
            component_flood: false,
            show_special_areas: false,
            camera_follow: false,
            pause_script_on_manual_walk_abort: true,
        }
    }
}

impl NavSettings {
    pub fn walk_globals(&self) -> host_play::WalkGlobals {
        host_play::WalkGlobals {
            allow_teleports: self.allow_teleports,
            allow_wilderness: self.allow_wilderness,
            allow_bank_fetch: self.allow_bank_fetch,
            allow_danger_zones: self.allow_danger_zones,
            survivable_routing: self.survivable_routing,
        }
    }

    pub fn danger_level(&self) -> frontend_core::DangerLevel {
        self.walk_globals().danger_level()
    }

    pub fn set_danger_level(&mut self, level: frontend_core::DangerLevel) {
        let mut globals = self.walk_globals();
        globals.set_danger_level(level);
        self.allow_danger_zones = globals.allow_danger_zones;
        self.survivable_routing = globals.survivable_routing;
    }

    pub fn refresh_walk_globals_at(&mut self, path: &std::path::Path) -> std::io::Result<()> {
        let view = frontend_core::WalkGlobalsView::read_at(path);
        self.allow_teleports = view.globals.allow_teleports;
        self.allow_wilderness = view.globals.allow_wilderness;
        self.allow_bank_fetch = view.globals.allow_bank_fetch;
        self.allow_danger_zones = view.globals.allow_danger_zones;
        self.survivable_routing = view.globals.survivable_routing;
        self.script_scope_notice_ack = view.script_scope_notice_ack;
        self.survivable_routing_notice_ack = view.survivable_routing_notice_ack;
        Ok(())
    }
}
/// Live harness overlay: when `live_force_layers`, force the paint-layer
/// toggles on for this session without writing prefs. Teleports and
/// colours still come from `saved`. Prefer a full [`NavSettings`] overlay
/// from the scenario bag (`Session::nav_overlay`) over this bool.
pub fn effective(saved: &NavSettings, live_force_layers: bool) -> NavSettings {
    let mut e = saved.clone();
    if live_force_layers {
        e.show_nav_path = true;
        e.collision_fill = true;
        e.client_trail = true;
        e.hop_labels = true;
        e.camera_follow = true;
    }
    e
}

/// Apply a session-only headed live paint choice without changing routing.
pub fn apply_paint_override(saved: &NavSettings, choice: Option<bool>) -> NavSettings {
    let Some(enabled) = choice else {
        return saved.clone();
    };
    if enabled {
        return effective(saved, true);
    }
    let mut out = saved.clone();
    out.show_nav_path = false;
    out.hop_labels = false;
    out.collision_fill = false;
    out.nsew_labels = false;
    out.client_trail = false;
    out.component_flood = false;
    out.camera_follow = false;
    out
}

/// Map a scenario's session-only nav bag onto panel settings. Colours stay
/// the rs2b0t Path-paint defaults (the scenario does not author HTML).
pub fn from_scenario(n: &scenario::ScenarioNav, saved: &NavSettings) -> NavSettings {
    NavSettings {
        allow_teleports: n.allow_teleports,
        allow_wilderness: n.allow_wilderness,
        allow_bank_fetch: false,
        show_nav_path: n.show_nav_path,
        hop_labels: n.hop_labels,
        hop_label_px: n.hop_label_px,
        collision_fill: n.collision_fill,
        nsew_labels: n.nsew_labels,
        client_trail: n.client_trail,
        component_flood: n.component_flood,
        camera_follow: n.camera_follow,
        pause_script_on_manual_walk_abort: saved.pause_script_on_manual_walk_abort,
        ..NavSettings::default()
    }
}

/// `#RGB` / `#RRGGBB` (leading `#` optional) to RGB bytes; any other
/// input falls back to `fallback`.
pub fn parse_html_color(raw: &str, fallback: [u8; 3]) -> [u8; 3] {
    let hex = raw.trim().trim_start_matches('#');
    let nibble = |b: u8| match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    };
    let bytes = hex.as_bytes();
    match bytes.len() {
        3 => {
            let mut out = [0u8; 3];
            for (i, b) in bytes.iter().enumerate() {
                match nibble(*b) {
                    Some(d) => out[i] = d * 17, // doubled digit: 0xF -> 0xFF
                    None => return fallback,
                }
            }
            out
        }
        6 => {
            let mut out = [0u8; 3];
            for (i, pair) in bytes.chunks(2).enumerate() {
                match (nibble(pair[0]), nibble(pair[1])) {
                    (Some(hi), Some(lo)) => out[i] = hi * 16 + lo,
                    _ => return fallback,
                }
            }
            out
        }
        _ => fallback,
    }
}

#[cfg(test)]
mod tests {
    use super::{effective, parse_html_color, NavSettings};

    #[test]
    fn html_hex_roundtrips_from_bytes() {
        assert_eq!(format!("#{:02X}{:02X}{:02X}", 0xff, 0x00, 0x00), "#FF0000");
        assert_eq!(parse_html_color("#00D4FF", [0, 0, 0]), [0x00, 0xd4, 0xff]);
    }

    #[test]
    fn live_force_layers_does_not_change_saved_teles_or_colours() {
        let saved = NavSettings::default();
        let e = effective(&saved, true);
        assert!(
            e.show_nav_path
                && e.collision_fill
                && e.hop_labels
                && e.client_trail
                && e.camera_follow
                && !e.nsew_labels
                && !e.component_flood
        );
        assert!(!e.allow_teleports);
        assert!(!e.allow_wilderness);
        assert!(!e.allow_bank_fetch);
        assert_eq!(e.color_path, "#FF0000");
    }

    /// The BankBudget flag round-trips: serializing an on flag and loading
    /// it back keeps the value (the same persist shape as the wildy/teles
    /// flags).
    #[test]
    fn allow_bank_fetch_round_trips_through_serde() {
        let s = NavSettings {
            allow_bank_fetch: true,
            show_special_areas: true,
            ..Default::default()
        };
        let bytes = serde_json::to_vec(&s).unwrap();
        let back: NavSettings = serde_json::from_slice(&bytes).unwrap();
        assert!(back.allow_bank_fetch);
        assert_eq!(back, s);
    }

    #[test]
    fn danger_level_round_trips_through_both_stored_booleans_and_migrates_old_nav() {
        use frontend_core::DangerLevel;

        assert_eq!(
            NavSettings::default().danger_level(),
            DangerLevel::WhenSurvivable
        );
        let old: NavSettings = serde_json::from_str(r#"{"allow_danger_zones":false}"#).unwrap();
        assert!(old.survivable_routing);
        assert_eq!(old.danger_level(), DangerLevel::WhenSurvivable);
        assert!(!old.survivable_routing_notice_ack);

        for level in [
            DangerLevel::Never,
            DangerLevel::WhenSurvivable,
            DangerLevel::Always,
        ] {
            let mut settings = NavSettings::default();
            settings.set_danger_level(level);
            assert_eq!(settings.danger_level(), level);
            let saved = serde_json::to_vec(&settings).unwrap();
            let loaded: NavSettings = serde_json::from_slice(&saved).unwrap();
            assert_eq!(loaded.danger_level(), level);
            assert_eq!(loaded, settings);
        }
    }
    #[test]
    fn effective_without_live_force_is_saved_unchanged() {
        let saved = NavSettings {
            show_nav_path: true,
            color_path: "#AABBCC".into(),
            ..Default::default()
        };
        let e = effective(&saved, false);
        assert_eq!(e, saved);
    }

    #[test]
    fn manual_walk_pause_defaults_on_for_legacy_nav_preferences() {
        assert!(NavSettings::default().pause_script_on_manual_walk_abort);
        let legacy: NavSettings = serde_json::from_str(r#"{"allow_teleports":true}"#).unwrap();
        assert!(legacy.pause_script_on_manual_walk_abort);
    }

    #[test]
    fn saved_pause_off_survives_scenario_and_paint_overlays() {
        let saved = NavSettings {
            pause_script_on_manual_walk_abort: false,
            ..NavSettings::default()
        };
        let scenario_nav = scenario::nav_test_paints();
        let scenario_overlay = super::from_scenario(&scenario_nav, &saved);
        assert!(!scenario_overlay.pause_script_on_manual_walk_abort);
        assert!(!effective(&saved, true).pause_script_on_manual_walk_abort);
        for choice in [None, Some(true), Some(false)] {
            assert!(
                !super::apply_paint_override(&saved, choice).pause_script_on_manual_walk_abort,
                "paint choice {choice:?} must not replace the saved pause preference"
            );
        }
    }

    #[test]
    fn parse_html_color_expands_short_and_long_forms() {
        assert_eq!(parse_html_color("#F00", [0; 3]), [255, 0, 0]);
        assert_eq!(parse_html_color("#00D4FF", [0; 3]), [0, 212, 255]);
        assert_eq!(parse_html_color("aabbcc", [0; 3]), [170, 187, 204]);
        assert_eq!(parse_html_color("red", [1, 2, 3]), [1, 2, 3]);
        assert_eq!(parse_html_color("#12345", [1, 2, 3]), [1, 2, 3]);
        assert_eq!(parse_html_color("#GGGGGG", [1, 2, 3]), [1, 2, 3]);
    }
}
