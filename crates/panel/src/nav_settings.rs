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
    pub allow_teleports: bool,
    pub allow_wilderness: bool,
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
}

impl Default for NavSettings {
    /// rs2b0t Path-paint defaults: path red, transport green, click/text
    /// white, collision reserved blue, client cyan, run-alt yellow.
    fn default() -> Self {
        Self {
            allow_teleports: false,
            allow_wilderness: false,
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
        }
    }
}

/// Live harness overlay: when `live_force_layers`, force the paint-layer
/// toggles on for this session without writing prefs. Teleports and
/// colours still come from `saved`.
pub fn effective(saved: &NavSettings, live_force_layers: bool) -> NavSettings {
    let mut e = saved.clone();
    if live_force_layers {
        e.show_nav_path = true;
        e.collision_fill = true;
        e.nsew_labels = true;
        e.client_trail = true;
        e.component_flood = true;
    }
    e
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
    fn defaults_match_rs2b0t_path_paint() {
        let d = NavSettings::default();
        assert!(!d.allow_teleports);
        assert!(!d.allow_wilderness);
        assert!(!d.show_nav_path);
        assert_eq!(d.color_path, "#FF0000");
        assert_eq!(d.color_transport, "#00FF00");
        assert_eq!(d.color_click, "#FFFFFF");
        assert_eq!(d.color_text, "#FFFFFF");
        assert_eq!(d.color_collision, "#0080FF");
        assert_eq!(d.color_client, "#00D4FF");
        assert_eq!(d.color_client_run_alt, "#FFFF00");
        assert!(!d.collision_fill && !d.client_trail && !d.component_flood);
    }

    #[test]
    fn live_force_layers_does_not_change_saved_teles_or_colours() {
        let saved = NavSettings::default();
        let e = effective(&saved, true);
        assert!(
            e.show_nav_path && e.collision_fill && e.nsew_labels && e.client_trail && e.component_flood
        );
        assert!(!e.allow_teleports);
        assert!(!e.allow_wilderness);
        assert_eq!(e.color_path, "#FF0000");
    }

    #[test]
    fn effective_without_live_force_is_saved_unchanged() {
        let mut saved = NavSettings::default();
        saved.show_nav_path = true;
        saved.color_path = "#AABBCC".into();
        let e = effective(&saved, false);
        assert_eq!(e, saved);
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
