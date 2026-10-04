//! Persisted panel UI prefs (`~/.274bot/panel-ui.json`): last focused
//! profile and per-profile collapsed section maps.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::nav_settings::NavSettings;

use frontend_core::log::{Level, Source};

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
    /// Server revision selected before the process profile is bound.
    /// Old preference files migrate to revision 274.
    #[serde(default = "default_server_revision")]
    pub server_revision: u16,
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
    /// Operator acknowledged that other profiles keep running in the background.
    #[serde(default)]
    pub background_bots_ack: bool,
    /// Panel CRT palette (named theme consts). Absent = amber defaults.
    #[serde(default)]
    pub chrome: crate::theme::ChromeColors,
    /// Remembered answer to the local WalkTo terrain bake warning, shared
    /// with the TUI. Absent (0.1.8.1) or unknown = ask.
    #[serde(default)]
    pub map_bake: frontend_core::MapBakeChoice,
    /// Write a per-session log file under `~/.274bot/logs/` (shared with the
    /// TUI as `frontend_core::log_file::SESSION_LOG_KEY`). Absent = off.
    #[serde(default)]
    pub session_log_file: bool,
    /// Fleet window status-column toggles that differ from their defaults
    /// (see `fleet_columns`).
    #[serde(default)]
    pub fleet_columns: HashMap<String, bool>,
    /// Debug command recents and favorites. Argument buffers stay ephemeral
    /// because the selected content pin may change command shapes.
    #[serde(default)]
    pub debug_panel: crate::debug_panel::DebugPanelPrefs,
}

/// Panel subsection ids in General config (parameters shares
/// [`PanelUiState::show_parameters_rail`]).
pub const PANEL_SECTION_IDS: &[&str] = &[
    "status",
    "resource",
    "profile",
    "script",
    "debug",
    "log",
    "parameters",
];

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

fn default_server_revision() -> u16 {
    274
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
            server_revision: default_server_revision(),
            section_order: Vec::new(),
            script_category_order: Vec::new(),
            script_load_last_dir: None,
            script_catalog_last_dir: None,
            show_parameters_rail: false,
            capture: true,
            panel_sections: HashMap::new(),
            config_collapsed: HashMap::new(),
            background_bots_ack: false,
            chrome: crate::theme::ChromeColors::default(),
            map_bake: frontend_core::MapBakeChoice::Ask,
            session_log_file: false,
            fleet_columns: HashMap::new(),
            debug_panel: crate::debug_panel::DebugPanelPrefs::default(),
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
        Ok(data) => match serde_json::from_slice::<PanelUiState>(&data) {
            Ok(mut state) => {
                state.debug_panel.normalize();
                state
            }
            Err(_) => PanelUiState::default(),
        },
        Err(_) => PanelUiState::default(),
    }
}

fn prefs_log(level: Level, message: String) {
    eprintln!("panel: {message}");
    frontend_core::log::global().process_line(Source::Host, level, &message);
}

fn replacement_path(p: &Path) -> PathBuf {
    let mut name = p.as_os_str().to_os_string();
    name.push(".new");
    PathBuf::from(name)
}
const MERGED_NAV_KEYS: &[&str] = &[
    "allow_teleports",
    "allow_wilderness",
    "allow_bank_fetch",
    "allow_danger_zones",
    "script_scope_notice_ack",
];

fn merge_saved_panel_state(
    document: &mut serde_json::Value,
    file_exists: bool,
    corrupt_json: bool,
    replacement: &serde_json::Value,
    path: &Path,
) -> std::io::Result<Option<PathBuf>> {
    if file_exists
        && (corrupt_json || serde_json::from_value::<PanelUiState>(document.clone()).is_err())
    {
        let sibling = replacement_path(path);
        *document = replacement.clone();
        prefs_log(
            Level::Warn,
            format!(
                "preserved invalid panel preferences {}; wrote current preferences to {}",
                path.display(),
                sibling.display()
            ),
        );
        return Ok(Some(sibling));
    }

    let previous_nav = document.get("nav").and_then(serde_json::Value::as_object);
    let replacement_nav = replacement
        .get("nav")
        .and_then(serde_json::Value::as_object)
        .expect("serialized PanelUiState always has a nav object")
        .clone();
    let mut merged_nav = previous_nav.cloned().unwrap_or_default();
    merged_nav.extend(replacement_nav);
    for key in MERGED_NAV_KEYS {
        let enabled = previous_nav
            .and_then(|nav| nav.get(*key))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        merged_nav.insert((*key).into(), serde_json::Value::Bool(enabled));
    }
    let mut replacement = replacement.clone();
    replacement
        .as_object_mut()
        .expect("serialized PanelUiState is always an object")
        .insert("nav".into(), serde_json::Value::Object(merged_nav));
    document
        .as_object_mut()
        .expect("valid panel preferences are an object")
        .extend(
            replacement
                .as_object()
                .expect("serialized PanelUiState is an object")
                .clone(),
        );
    Ok(None)
}

pub fn save_at(p: &Path, state: &PanelUiState) {
    let _ = save_at_checked(p, state);
}

/// Persist panel preferences and return write failures to the caller that
/// needs to display an inline notice.
pub fn save_checked(state: &PanelUiState) -> std::io::Result<()> {
    #[cfg(test)]
    {
        TEST_STATE.with(|s| *s.borrow_mut() = state.clone());
        Ok(())
    }
    #[cfg(not(test))]
    {
        save_at_checked(&path(), state)
    }
}

/// Persist preferences at an explicit path through the same serialized
/// read/merge/write transaction as shared nested preference updates.
pub fn save_at_checked(p: &Path, state: &PanelUiState) -> std::io::Result<()> {
    let replacement = match serde_json::to_value(state) {
        Ok(replacement) => replacement,
        Err(error) => {
            prefs_log(
                Level::Error,
                format!("refused to save panel preferences {}: {error}", p.display()),
            );
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, error));
        }
    };
    match host_play::update_panel_ui_at(p, |document, file_exists, corrupt_json| {
        merge_saved_panel_state(document, file_exists, corrupt_json, &replacement, p)
    }) {
        Ok(()) => Ok(()),
        Err(error) => {
            prefs_log(
                Level::Error,
                format!("refused to save panel preferences {}: {error}", p.display()),
            );
            Err(error)
        }
    }
}

#[cfg(test)]
thread_local! {
    static TEST_STATE: std::cell::RefCell<PanelUiState> =
        std::cell::RefCell::new(PanelUiState::default());
}

#[cfg(test)]
mod tests {
    use super::{
        load, load_at, path, pick_focus, save, save_at, save_at_checked, NavSettings, PanelUiState,
    };
    use crate::test_support::TestDir;
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
        let dir = TestDir::new("ui-old-nav");
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
        assert!(
            back.nav.pause_script_on_manual_walk_abort,
            "missing manual-walk pause key defaults on"
        );
        assert!(back.nav.show_nav_path, "present fields keep their values");
        assert_eq!(back.nav.color_path, "#AABBCC");
    }

    #[test]
    fn panel_and_tui_nav_writes_round_trip_without_clobbering_each_other() {
        let dir = TestDir::new("ui-nav-cross-surface");
        let path = dir.join("panel-ui.json");
        let state = PanelUiState {
            last_focus: Some("alice".into()),
            capture: false,
            nav: NavSettings {
                pause_script_on_manual_walk_abort: false,
                ..NavSettings::default()
            },
            ..PanelUiState::default()
        };
        save_at(&path, &state);

        frontend_core::nav_preference_at(
            &path,
            frontend_core::NavPreference::ShowSpecialAreas,
            Some(true),
        )
        .unwrap();
        let loaded = load_at(&path);
        assert_eq!(loaded.last_focus.as_deref(), Some("alice"));
        assert!(!loaded.capture);
        assert!(
            !loaded.nav.pause_script_on_manual_walk_abort,
            "TUI's nested nav write preserves the panel's saved pause toggle"
        );
        assert!(loaded.nav.show_special_areas);

        frontend_core::nav_preference_at(
            &path,
            frontend_core::NavPreference::PauseScriptOnManualWalkAbort,
            Some(true),
        )
        .unwrap();
        let loaded = load_at(&path);
        assert!(loaded.nav.pause_script_on_manual_walk_abort);
        assert!(
            loaded.nav.show_special_areas,
            "panel's typed save preserves TUI's special-area preference"
        );
    }

    #[test]
    fn overlapping_panel_save_and_single_key_nav_writer_keep_both_edits() {
        let dir = TestDir::new("ui-nav-overlap");
        let path = dir.join("panel-ui.json");
        save_at_checked(
            &path,
            &PanelUiState {
                last_focus: Some("before".into()),
                ..PanelUiState::default()
            },
        )
        .unwrap();
        host_play::persist_panel_ui_value_at(&path, "unrelated", serde_json::json!({"keep": 7}))
            .unwrap();
        let panel_state = PanelUiState {
            last_focus: Some("panel-save".into()),
            capture: true,
            nav: NavSettings {
                show_nav_path: true,
                ..NavSettings::default()
            },
            ..PanelUiState::default()
        };
        let nav = serde_json::json!({
            "allow_teleports": true,
            "allow_wilderness": false,
            "allow_bank_fetch": false,
            "allow_danger_zones": false,
            "script_scope_notice_ack": false,
            "show_nav_path": true
        });
        // Reproduce the stale panel snapshot deterministically before the
        // concurrent stress loop: a single-key writer lands, then the panel
        // saves its older in-memory state.
        host_play::persist_panel_ui_value_at(&path, "nav", nav.clone()).unwrap();
        let after_writer: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(after_writer["last_focus"], "before");
        assert_eq!(after_writer["nav"]["allow_teleports"], true);
        assert_eq!(after_writer["nav"]["allow_wilderness"], false);
        assert_eq!(after_writer["nav"]["show_nav_path"], true);

        save_at_checked(&path, &panel_state).unwrap();
        let after_panel_save: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(after_panel_save["last_focus"], "panel-save");
        assert_eq!(after_panel_save["capture"], true);
        assert_eq!(after_panel_save["nav"]["show_nav_path"], true);
        assert_eq!(after_panel_save["nav"]["allow_teleports"], true);
        assert_eq!(after_panel_save["nav"]["allow_wilderness"], false);
        assert_eq!(after_panel_save["nav"]["allow_bank_fetch"], false);
        assert_eq!(after_panel_save["nav"]["allow_danger_zones"], false);
        assert_eq!(after_panel_save["nav"]["script_scope_notice_ack"], false);
        let nav = serde_json::json!({
            "allow_teleports": false,
            "allow_wilderness": true,
            "allow_bank_fetch": true,
            "allow_danger_zones": true,
            "script_scope_notice_ack": true,
            "show_nav_path": true
        });
        host_play::persist_panel_ui_value_at(&path, "nav", nav.clone()).unwrap();
        let after_second_writer: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(after_second_writer["last_focus"], "panel-save");
        assert_eq!(after_second_writer["capture"], true);
        assert_eq!(after_second_writer["nav"]["show_nav_path"], true);
        assert_eq!(after_second_writer["nav"]["allow_teleports"], false);
        assert_eq!(after_second_writer["nav"]["allow_wilderness"], true);
        assert_eq!(after_second_writer["nav"]["allow_bank_fetch"], true);
        assert_eq!(after_second_writer["nav"]["allow_danger_zones"], true);
        assert_eq!(after_second_writer["nav"]["script_scope_notice_ack"], true);
        let mut refreshed_nav = panel_state.nav.clone();
        refreshed_nav.refresh_walk_globals_at(&path).unwrap();
        assert!(!refreshed_nav.allow_teleports);
        assert!(refreshed_nav.allow_wilderness);
        assert!(refreshed_nav.allow_bank_fetch);
        assert!(refreshed_nav.allow_danger_zones);
        assert!(refreshed_nav.script_scope_notice_ack);

        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let panel_path = path.clone();
        let panel_barrier = std::sync::Arc::clone(&barrier);
        let panel = std::thread::spawn(move || {
            for _ in 0..32 {
                panel_barrier.wait();
                save_at_checked(&panel_path, &panel_state).unwrap();
            }
        });
        let nav_path = path.clone();
        let nav_barrier = std::sync::Arc::clone(&barrier);
        let writer = std::thread::spawn(move || {
            for _ in 0..32 {
                nav_barrier.wait();
                host_play::persist_panel_ui_value_at(&nav_path, "nav", nav.clone()).unwrap();
            }
        });
        panel.join().unwrap();
        writer.join().unwrap();

        let document: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(document["last_focus"], "panel-save");
        assert_eq!(document["capture"], true);
        assert_eq!(document["nav"]["show_nav_path"], true);
        assert_eq!(document["nav"]["allow_teleports"], false);
        assert_eq!(document["nav"]["allow_wilderness"], true);
        assert_eq!(document["nav"]["allow_bank_fetch"], true);
        assert_eq!(document["nav"]["allow_danger_zones"], true);
        assert_eq!(document["nav"]["script_scope_notice_ack"], true);
        assert_eq!(document["unrelated"]["keep"], 7);
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
        let dir = TestDir::new("ui-roundtrip");
        let p = dir.join("panel-ui.json");

        let mut state = PanelUiState {
            last_focus: Some("bob".into()),
            ..Default::default()
        };
        state
            .collapsed
            .insert("bob".into(), HashMap::from([("nav".into(), true)]));
        state.debug_panel.recents = vec!["~give".into()];
        state.debug_panel.favorites = vec!["~reset".into()];
        save_at(&p, &state);

        let loaded = load_at(&p);
        assert_eq!(loaded.last_focus.as_deref(), Some("bob"));
        assert!(loaded.collapsed["bob"]["nav"]);
        assert_eq!(loaded.debug_panel.recents, vec!["~give"]);
        assert_eq!(loaded.debug_panel.favorites, vec!["~reset"]);
    }

    #[test]
    fn load_missing_file_is_default() {
        let dir = TestDir::new("ui-missing");
        let p = dir.join("panel-ui.json");
        let loaded = load_at(&p);
        assert!(loaded.last_focus.is_none());
        assert!(loaded.collapsed.is_empty());
    }

    #[test]
    fn save_preserves_corrupt_prefs_and_writes_actual_state_to_new_sibling() {
        let dir = TestDir::new("ui-corrupt");
        let p = dir.join("panel-ui.json");
        let corrupt = b"{ not valid json";
        std::fs::write(&p, corrupt).unwrap();
        let state = PanelUiState {
            last_focus: Some("alice".into()),
            ..Default::default()
        };
        save_at(&p, &state);
        assert_eq!(std::fs::read(&p).unwrap(), corrupt);
        let repaired = super::replacement_path(&p);
        assert_eq!(load_at(&repaired).last_focus.as_deref(), Some("alice"));
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
        let dir = TestDir::new("ui-cat-order");
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
    }

    #[test]
    fn script_catalog_last_dir_persist_roundtrip() {
        let dir = TestDir::new("ui-cat-dir");
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
    }

    #[test]
    fn default_section_closed_only_script_and_parameters() {
        use super::default_section_closed;
        assert!(default_section_closed("script"));
        assert!(default_section_closed("parameters"));
        assert!(!default_section_closed("profile"));
        assert!(!default_section_closed("credentials"));
        assert!(!default_section_closed("status"));
        assert!(!default_section_closed("resource"));
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
    fn background_bots_ack_defaults_off() {
        let back: PanelUiState =
            serde_json::from_str(r#"{"last_focus":null,"collapsed":{}}"#).unwrap();
        assert!(
            !back.background_bots_ack,
            "missing background ack defaults to show the notice"
        );
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
        let dir = TestDir::new("ui-capture");
        let p = dir.join("panel-ui.json");
        let state = PanelUiState {
            capture: false,
            ..Default::default()
        };
        save_at(&p, &state);
        assert!(!load_at(&p).capture);
    }

    #[test]
    fn session_log_file_is_off_for_old_prefs_and_survives_a_panel_save() {
        let old: PanelUiState =
            serde_json::from_str(r#"{"last_focus":null,"collapsed":{}}"#).unwrap();
        assert!(!old.session_log_file, "absent key: no session file");
        // The TUI writes the shared key; a later panel save must keep it.
        let on: PanelUiState = serde_json::from_str(&format!(
            r#"{{"last_focus":null,"collapsed":{{}},"{}":true}}"#,
            frontend_core::log_file::SESSION_LOG_KEY
        ))
        .unwrap();
        assert!(on.session_log_file);
        let bytes = serde_json::to_vec(&on).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value[frontend_core::log_file::SESSION_LOG_KEY], true);
    }

    #[test]
    fn old_prefs_without_chrome_keep_theme_defaults() {
        let back: PanelUiState =
            serde_json::from_str(r#"{"last_focus":null,"collapsed":{}}"#).unwrap();
        assert_eq!(back.chrome, crate::theme::ChromeColors::default());
    }

    #[test]
    fn old_prefs_default_revision_to_274_and_289_roundtrips() {
        let old: PanelUiState =
            serde_json::from_str(r#"{"last_focus":null,"collapsed":{}}"#).unwrap();
        assert_eq!(old.server_revision, 274);

        let state = PanelUiState {
            server_revision: 289,
            ..Default::default()
        };
        let bytes = serde_json::to_vec(&state).unwrap();
        let back: PanelUiState = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(back.server_revision, 289);
    }

    #[test]
    fn map_bake_choice_defaults_to_ask_and_a_bad_value_keeps_the_other_prefs() {
        use frontend_core::MapBakeChoice;
        let old: PanelUiState =
            serde_json::from_str(r#"{"last_focus":"alice","collapsed":{}}"#).unwrap();
        assert_eq!(old.map_bake, MapBakeChoice::Ask, "0.1.8.1 file asks");
        let odd: PanelUiState =
            serde_json::from_str(r#"{"last_focus":"alice","map_bake":7,"capture":false}"#).unwrap();
        assert_eq!(odd.map_bake, MapBakeChoice::Ask);
        assert_eq!(odd.last_focus.as_deref(), Some("alice"));
        assert!(!odd.capture, "an unreadable choice does not reset the file");
        let dir = TestDir::new("ui-map-bake");
        let p = dir.join("panel-ui.json");
        let state = PanelUiState {
            map_bake: MapBakeChoice::Always,
            ..Default::default()
        };
        save_at(&p, &state);
        assert_eq!(load_at(&p).map_bake, MapBakeChoice::Always);
    }
}
