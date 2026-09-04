//! Persisted panel UI prefs (`~/.274bot/panel-ui.json`): last focused
//! profile and per-profile collapsed section maps.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::nav_settings::NavSettings;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PanelUiState {
    pub last_focus: Option<String>,
    #[serde(default)]
    pub collapsed: HashMap<String, HashMap<String, bool>>,
    #[serde(default)]
    pub nav: NavSettings,
    /// Per-member rail blit override. Absent = focused folded, others open.
    #[serde(default)]
    pub rail_preview: HashMap<String, bool>,
    /// Global none/GPU/CPU for every slot (General config).
    #[serde(default)]
    pub raster: vault::RasterMode,
    /// Global lowmem for every slot. Default true (headless default).
    #[serde(default = "default_true")]
    pub lowmem: bool,
    /// Strip collapsing-header order. Empty = [`crate::chrome::HEADING_ORDER`].
    #[serde(default)]
    pub section_order: Vec<String>,
    /// Script Browse category chip order. Unknown categories append at open.
    #[serde(default)]
    pub script_category_order: Vec<String>,
    /// Last directory visited in the out-of-tree Load file browser.
    #[serde(default)]
    pub script_load_last_dir: Option<PathBuf>,
    /// Last directory visited in the catalog-import folder dialog.
    #[serde(default)]
    pub script_catalog_last_dir: Option<PathBuf>,
    /// Read-only parameters preview in the rail (default off).
    #[serde(default)]
    pub show_parameters_rail: bool,
    /// Global capture input pref (General config). Default on.
    #[serde(default = "default_true")]
    pub capture: bool,
    /// Panel strip section visibility. Absent = on (parameters uses
    /// [`Self::show_parameters_rail`]).
    #[serde(default)]
    pub panel_sections: HashMap<String, bool>,
    /// General config collapsible rows closed. Absent = open.
    #[serde(default)]
    pub config_collapsed: HashMap<String, bool>,
    /// Panel CRT palette (named theme consts). Absent = amber defaults.
    #[serde(default)]
    pub chrome: crate::theme::ChromeColors,
}

/// Panel subsection ids in General config (parameters shares
/// [`PanelUiState::show_parameters_rail`]).
pub const PANEL_SECTION_IDS: &[&str] =
    &["status", "profile", "script", "debug", "log", "parameters"];

/// Whether a panel strip heading should draw in [`crate::app::panel_window`].
pub fn panel_section_visible(state: &PanelUiState, id: &str) -> bool {
    if id == "parameters" {
        return state.show_parameters_rail;
    }
    state.panel_sections.get(id).copied().unwrap_or(true)
}

/// Write a panel strip heading visibility bit (parameters →
/// `show_parameters_rail`).
pub fn set_panel_section_visible(state: &mut PanelUiState, id: &str, visible: bool) {
    if id == "parameters" {
        state.show_parameters_rail = visible;
    } else {
        state.panel_sections.insert(id.to_string(), visible);
    }
}

fn default_true() -> bool {
    true
}

impl Default for PanelUiState {
    fn default() -> Self {
        Self {
            last_focus: None,
            collapsed: HashMap::new(),
            nav: NavSettings::default(),
            rail_preview: HashMap::new(),
            raster: vault::RasterMode::Gpu,
            lowmem: true,
            section_order: Vec::new(),
            script_category_order: Vec::new(),
            script_load_last_dir: None,
            script_catalog_last_dir: None,
            show_parameters_rail: false,
            capture: true,
            panel_sections: HashMap::new(),
            config_collapsed: HashMap::new(),
            chrome: crate::theme::ChromeColors::default(),
        }
    }
}

/// Default closed (collapsed) when no persisted entry: script + parameters only.
pub fn default_section_closed(id: &str) -> bool {
    id == "script" || id == "parameters"
}

/// Prefer `last` when it is still in `names`; otherwise the first name.
pub fn pick_focus(names: &[String], last: Option<&str>) -> Option<String> {
    if let Some(l) = last {
        if names.iter().any(|n| n == l) {
            return Some(l.to_string());
        }
    }
    names.first().cloned()
}

/// `~/.274bot/panel-ui.json` (same HOME rule as the vault path).
pub fn path() -> PathBuf {
    script::bot_file("panel-ui.json")
}

pub fn load() -> PanelUiState {
    #[cfg(test)]
    {
        // Per-test-thread isolation so parallel `select` calls do not race
        // on a shared temp file or the operator's real prefs.
        TEST_STATE.with(|s| s.borrow().clone())
    }
    #[cfg(not(test))]
    load_at(&path())
}

pub fn save(state: &PanelUiState) {
    #[cfg(test)]
    {
        TEST_STATE.with(|s| *s.borrow_mut() = state.clone());
    }
    #[cfg(not(test))]
    save_at(&path(), state);
}

pub fn load_at(p: &Path) -> PanelUiState {
    match std::fs::read(p) {
        Ok(data) => serde_json::from_slice(&data).unwrap_or_default(),
        Err(_) => PanelUiState::default(),
    }
}

pub fn save_at(p: &Path, state: &PanelUiState) {
    if let Ok(data) = serde_json::to_vec_pretty(state) {
        let _ = vault::write_private_file(p, &data);
    }
}

#[cfg(test)]
thread_local! {
    static TEST_STATE: std::cell::RefCell<PanelUiState> =
        std::cell::RefCell::new(PanelUiState::default());
}

#[cfg(test)]
mod tests {
    use super::{load, load_at, path, pick_focus, save, save_at, NavSettings, PanelUiState};
    use std::collections::HashMap;

    #[test]
    fn panel_ui_roundtrip_keeps_nav() {
        let mut s = PanelUiState::default();
        s.nav.show_nav_path = true;
        s.nav.color_path = "#AABBCC".into();
        s.nav.allow_bank_fetch = true;
        let bytes = serde_json::to_vec(&s).unwrap();
        let back: PanelUiState = serde_json::from_slice(&bytes).unwrap();
        assert!(back.nav.show_nav_path);
        assert_eq!(back.nav.color_path, "#AABBCC");
        assert!(
            back.nav.allow_bank_fetch,
            "the BankBudget flag round-trips like the other nav toggles"
        );
    }

    #[test]
    fn panel_ui_without_nav_key_is_defaults() {
        let back: PanelUiState =
            serde_json::from_str(r#"{"last_focus":null,"collapsed":{}}"#).unwrap();
        assert_eq!(back.nav, NavSettings::default());
    }

    #[test]
    fn old_nav_object_without_allow_wilderness_keeps_focus_and_colors() {
        // A pre-Task-1 prefs file carries a `nav` object with no
        // `allow_wilderness` key. It must load with the new field defaulted
        // (false) instead of failing deserialize and resetting the whole
        // `PanelUiState` (`load_at` falls back to `PanelUiState::default()`,
        // wiping focus / collapsed / colors).
        let dir =
            std::env::temp_dir().join(format!("274bot-panel-ui-old-nav-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("panel-ui.json");
        std::fs::write(
            &p,
            r##"{
  "last_focus": "alice",
  "collapsed": {"bob": {"nav": true}},
  "nav": {
    "allow_teleports": false,
    "show_nav_path": true,
    "hop_labels": true,
    "hop_label_px": 11,
    "color_path": "#AABBCC",
    "color_transport": "#00FF00",
    "color_click": "#FFFFFF",
    "color_text": "#FFFFFF",
    "collision_fill": false,
    "nsew_labels": false,
    "client_trail": false,
    "color_collision": "#0080FF",
    "color_client": "#00D4FF",
    "color_client_run_alt": "#FFFF00",
    "component_flood": false
  }
}"##,
        )
        .unwrap();
        let back = load_at(&p);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(
            back.last_focus.as_deref(),
            Some("alice"),
            "focus must survive an old nav object"
        );
        assert!(back.collapsed["bob"]["nav"]);
        assert!(
            !back.nav.allow_wilderness,
            "missing allow_wilderness defaults false"
        );
        assert!(
            !back.nav.allow_bank_fetch,
            "missing allow_bank_fetch defaults false"
        );
        assert!(back.nav.show_nav_path, "present fields keep their values");
        assert_eq!(back.nav.color_path, "#AABBCC");
    }

    #[test]
    fn pick_focus_prefers_last_when_present() {
        let names = vec!["a".into(), "b".into()];
        assert_eq!(pick_focus(&names, Some("b")).as_deref(), Some("b"));
        assert_eq!(pick_focus(&names, Some("z")).as_deref(), Some("a"));
        assert_eq!(pick_focus(&names, None).as_deref(), Some("a"));
        assert_eq!(pick_focus(&[], Some("a")), None);
    }

    #[test]
    fn focus_first_prefers_last_focus() {
        // Helper used by Session::focus_first_profile.
        let names = vec!["a".into(), "b".into()];
        assert_eq!(pick_focus(&names, Some("b")).as_deref(), Some("b"));
    }

    #[test]
    fn load_save_roundtrip_last_focus() {
        let dir = std::env::temp_dir().join(format!(
            "274bot-panel-ui-roundtrip-{}-{}",
            std::process::id(),
            "rt"
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("panel-ui.json");
        let _ = std::fs::remove_file(&p);

        let mut state = PanelUiState {
            last_focus: Some("bob".into()),
            ..Default::default()
        };
        state
            .collapsed
            .insert("bob".into(), HashMap::from([("nav".into(), true)]));
        save_at(&p, &state);

        let loaded = load_at(&p);
        assert_eq!(loaded.last_focus.as_deref(), Some("bob"));
        assert!(loaded.collapsed["bob"]["nav"]);
    }

    #[test]
    fn load_missing_file_is_default() {
        let p = std::env::temp_dir().join(format!(
            "274bot-panel-ui-missing-{}-{}.json",
            std::process::id(),
            "x"
        ));
        let _ = std::fs::remove_file(&p);
        let loaded = load_at(&p);
        assert!(loaded.last_focus.is_none());
        assert!(loaded.collapsed.is_empty());
    }

    #[test]
    fn save_load_roundtrip_via_default_api() {
        let state = PanelUiState {
            last_focus: Some("carol".into()),
            ..Default::default()
        };
        save(&state);
        assert_eq!(load().last_focus.as_deref(), Some("carol"));
    }

    #[test]
    fn path_uses_home_274bot() {
        let p = path();
        assert!(p.ends_with("panel-ui.json"));
        assert!(
            p.to_string_lossy().contains(".274bot"),
            "path should sit under .274bot, got {}",
            p.display()
        );
    }

    #[test]
    fn script_category_order_persist_roundtrip() {
        let dir =
            std::env::temp_dir().join(format!("274bot-panel-ui-cat-order-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("panel-ui.json");
        let state = PanelUiState {
            script_category_order: vec!["Prayer".into(), "Combat".into(), "Skilling".into()],
            ..Default::default()
        };
        save_at(&p, &state);
        let loaded = load_at(&p);
        assert_eq!(
            loaded.script_category_order,
            vec!["Prayer", "Combat", "Skilling"]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn script_catalog_last_dir_persist_roundtrip() {
        let dir =
            std::env::temp_dir().join(format!("274bot-panel-ui-cat-dir-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("panel-ui.json");
        let state = PanelUiState {
            script_catalog_last_dir: Some(std::path::PathBuf::from("/tmp/rs2b0t")),
            script_load_last_dir: Some(std::path::PathBuf::from("/tmp/scripts")),
            ..Default::default()
        };
        save_at(&p, &state);
        let loaded = load_at(&p);
        assert_eq!(
            loaded.script_catalog_last_dir,
            Some(std::path::PathBuf::from("/tmp/rs2b0t"))
        );
        assert_eq!(
            loaded.script_load_last_dir,
            Some(std::path::PathBuf::from("/tmp/scripts"))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn default_section_closed_only_script_and_parameters() {
        use super::default_section_closed;
        assert!(default_section_closed("script"));
        assert!(default_section_closed("parameters"));
        assert!(!default_section_closed("profile"));
        assert!(!default_section_closed("credentials"));
        assert!(!default_section_closed("status"));
        assert!(!default_section_closed("log"));
        assert!(!default_section_closed("rendering"));
        assert!(!default_section_closed("input"));
        assert!(!default_section_closed("debug"));
    }

    #[test]
    fn capture_default_on() {
        assert!(PanelUiState::default().capture);
        let back: PanelUiState =
            serde_json::from_str(r#"{"last_focus":null,"collapsed":{}}"#).unwrap();
        assert!(back.capture, "missing capture key defaults on");
    }

    #[test]
    fn panel_section_visible_defaults_on_except_parameters() {
        use super::{panel_section_visible, set_panel_section_visible, PANEL_SECTION_IDS};
        let s = PanelUiState::default();
        for id in PANEL_SECTION_IDS {
            if *id == "parameters" {
                assert!(!panel_section_visible(&s, id));
            } else {
                assert!(panel_section_visible(&s, id), "{id} defaults on");
            }
        }
        let mut s = PanelUiState::default();
        set_panel_section_visible(&mut s, "status", false);
        assert!(!panel_section_visible(&s, "status"));
        set_panel_section_visible(&mut s, "parameters", true);
        assert!(panel_section_visible(&s, "parameters"));
        assert!(s.show_parameters_rail);
    }

    #[test]
    fn capture_persist_roundtrip() {
        let dir =
            std::env::temp_dir().join(format!("274bot-panel-ui-capture-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("panel-ui.json");
        let state = PanelUiState {
            capture: false,
            ..Default::default()
        };
        save_at(&p, &state);
        assert!(!load_at(&p).capture);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn chrome_defaults_match_theme_consts() {
        use crate::theme::{rgba_to_hex, ACCENT, ACCENT_HOVER, BG, BG_DEEP, BORDER, ERROR, FRAME, GREEN, HOVER_FILL, ACTIVE_FILL, TEXT, TEXT_DIM, WARN};
        let c = PanelUiState::default().chrome;
        assert_eq!(c.accent, rgba_to_hex(ACCENT));
        assert_eq!(c.accent_hover, rgba_to_hex(ACCENT_HOVER));
        assert_eq!(c.bg, rgba_to_hex(BG));
        assert_eq!(c.bg_deep, rgba_to_hex(BG_DEEP));
        assert_eq!(c.text, rgba_to_hex(TEXT));
        assert_eq!(c.text_dim, rgba_to_hex(TEXT_DIM));
        assert_eq!(c.frame, rgba_to_hex(FRAME));
        assert_eq!(c.hover_fill, rgba_to_hex(HOVER_FILL));
        assert_eq!(c.active_fill, rgba_to_hex(ACTIVE_FILL));
        assert_eq!(c.border, rgba_to_hex(BORDER));
        assert_eq!(c.warn, rgba_to_hex(WARN));
        assert_eq!(c.error, rgba_to_hex(ERROR));
        assert_eq!(c.green, rgba_to_hex(GREEN));
    }

    #[test]
    fn old_prefs_without_chrome_keep_theme_defaults() {
        let back: PanelUiState =
            serde_json::from_str(r#"{"last_focus":null,"collapsed":{}}"#).unwrap();
        assert_eq!(back.chrome, crate::theme::ChromeColors::default());
    }
}
