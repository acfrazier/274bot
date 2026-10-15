//! Persisted panel UI prefs (`~/.274bot/panel-ui.json`): last focused
//! profile and per-profile collapsed section maps.
//!
//! The file is the operator preferences the process local-state service
//! owns ([`Preferences`]): the panel reads and writes it only through the
//! session's handle, never by path.

use std::collections::HashMap;
use std::io;
use std::path::PathBuf;

use crate::nav_settings::NavSettings;

use frontend_core::log::{Level, Source};
use host_play::local_state::Preferences;
use serde_json::{Map, Value};

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
    /// The operator pressed "Don't show again" on the render-all warning.
    /// Absent = ask every time. "Only render focused" itself is never saved.
    #[serde(default)]
    pub render_all_warning_ack: bool,
    /// "sidecar 50 fps": unfocused wall members at 50 fps. Absent = off.
    #[serde(default)]
    pub sidecar_50: bool,
    /// "focused 50 fps": the Game pane's bot at 50 fps. Absent = on, as it
    /// was before the choice was saved.
    #[serde(default = "default_true")]
    pub focused_50: bool,
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
            render_all_warning_ack: false,
            sidecar_50: false,
            focused_50: true,
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

/// The preferences the session reads. Under `cfg(test)` a per-thread
/// stand-in, so parallel `select` calls do not race on a shared file or the
/// operator's real prefs; [`load_from`] always reads the store.
pub fn load(preferences: &Preferences) -> PanelUiState {
    #[cfg(test)]
    {
        let _ = preferences;
        TEST_STATE.with(|s| s.borrow().clone())
    }
    #[cfg(not(test))]
    load_from(preferences)
}

/// Save the session's preferences (the per-thread stand-in under
/// `cfg(test)`, like [`load`]). A refused write is logged, not returned.
pub fn save(preferences: &Preferences, state: &PanelUiState) {
    #[cfg(test)]
    {
        let _ = preferences;
        TEST_STATE.with(|s| *s.borrow_mut() = state.clone());
    }
    #[cfg(not(test))]
    save_to(preferences, state);
}

/// The panel preferences `preferences` holds: defaults when the file is
/// absent, when the service refused it (unreadable), or when it does not
/// read as panel preferences.
pub fn load_from(preferences: &Preferences) -> PanelUiState {
    let Ok(document) = preferences.document() else {
        return PanelUiState::default();
    };
    match serde_json::from_value::<PanelUiState>(Value::Object((*document).clone())) {
        Ok(mut state) => {
            state.debug_panel.normalize();
            state
        }
        Err(_) => PanelUiState::default(),
    }
}

fn prefs_log(level: Level, message: String) {
    eprintln!("panel: {message}");
    frontend_core::log::global().process_line(Source::Host, level, &message);
}

const MERGED_NAV_KEYS: &[&str] = &[
    "allow_teleports",
    "allow_wilderness",
    "allow_bank_fetch",
    "allow_danger_zones",
    "survivable_routing",
    "script_scope_notice_ack",
    "survivable_routing_notice_ack",
];

/// Merge the panel's own keys (`replacement`, the serialized state) into the
/// document read now: every other key stays as the document holds it, and
/// the walk permission keys of `nav` ([`MERGED_NAV_KEYS`]) stay as the
/// document holds them. A document that does not read as panel preferences
/// is refused whole, and nothing is written over it.
fn merge_saved_panel_state(
    document: &mut Map<String, Value>,
    replacement: &Value,
) -> io::Result<()> {
    if let Err(error) = serde_json::from_value::<PanelUiState>(Value::Object(document.clone())) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "the stored preferences do not read as panel preferences ({error}); \
                 nothing was written"
            ),
        ));
    }

    let replacement = replacement
        .as_object()
        .expect("serialized PanelUiState is an object");
    let previous_nav = document.get("nav").and_then(Value::as_object);
    let mut merged_nav = previous_nav.cloned().unwrap_or_default();
    merged_nav.extend(
        replacement
            .get("nav")
            .and_then(Value::as_object)
            .expect("serialized PanelUiState always has a nav object")
            .clone(),
    );
    for key in MERGED_NAV_KEYS {
        let default = *key == "survivable_routing";
        let enabled = previous_nav
            .and_then(|nav| nav.get(*key))
            .and_then(Value::as_bool)
            .unwrap_or(default);
        merged_nav.insert((*key).into(), Value::Bool(enabled));
    }
    document.extend(replacement.clone());
    document.insert("nav".into(), Value::Object(merged_nav));
    Ok(())
}

/// Write `state` into `preferences`, the outcome logged (see
/// [`save_to_checked`]) and not returned.
pub fn save_to(preferences: &Preferences, state: &PanelUiState) {
    let _ = save_to_checked(preferences, state);
}

/// Persist panel preferences and return write failures to the caller that
/// needs to display an inline notice (the per-thread stand-in under
/// `cfg(test)`, like [`load`]).
pub fn save_checked(preferences: &Preferences, state: &PanelUiState) -> io::Result<()> {
    #[cfg(test)]
    {
        let _ = preferences;
        TEST_STATE.with(|s| *s.borrow_mut() = state.clone());
        Ok(())
    }
    #[cfg(not(test))]
    {
        save_to_checked(preferences, state)
    }
}

/// Persist the panel's own keys into `preferences` through the same
/// serialized read/merge/write transaction as shared nested preference
/// updates. A file the service refused, or one that does not read as panel
/// preferences, is not written: the error is returned and logged once.
pub fn save_to_checked(preferences: &Preferences, state: &PanelUiState) -> io::Result<()> {
    let refused = |error: &dyn std::fmt::Display| {
        prefs_log(
            Level::Error,
            format!(
                "refused to save panel preferences {}: {error}",
                preferences.path().display()
            ),
        );
    };
    let replacement = match serde_json::to_value(state) {
        Ok(replacement) => replacement,
        Err(error) => {
            refused(&error);
            return Err(io::Error::new(io::ErrorKind::InvalidData, error));
        }
    };
    match preferences.update(|document| merge_saved_panel_state(document, &replacement)) {
        Ok(()) => Ok(()),
        Err(error) => {
            refused(&error);
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
    use super::{load, load_from, save, save_to, save_to_checked, NavSettings, PanelUiState};
    use crate::test_support::{scratch_service, TestDir};
    use host_play::local_state::Preferences;
    use std::collections::HashMap;

    /// A scratch state root and the preferences of a service opened on it:
    /// the service stops before the directory goes.
    fn scratch(label: &str) -> (TestDir, Preferences) {
        let dir = TestDir::new(label);
        let preferences = scratch_service(&dir).preferences();
        (dir, preferences)
    }

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
        // `PanelUiState` (`load_from` falls back to `PanelUiState::default()`,
        // wiping focus / collapsed / colors).
        let (_dir, prefs) = scratch("ui-old-nav");
        std::fs::write(
            prefs.path(),
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
        prefs.reload().unwrap();
        let back = load_from(&prefs);
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
        assert!(
            back.nav.survivable_routing,
            "an old nav object migrates to When survivable"
        );
    }

    #[test]
    fn panel_and_tui_nav_writes_round_trip_without_clobbering_each_other() {
        let (_dir, prefs) = scratch("ui-nav-cross-surface");
        let state = PanelUiState {
            last_focus: Some("alice".into()),
            capture: false,
            nav: NavSettings {
                pause_script_on_manual_walk_abort: false,
                ..NavSettings::default()
            },
            ..PanelUiState::default()
        };
        save_to(&prefs, &state);

        frontend_core::nav_preference(
            &prefs,
            frontend_core::NavPreference::ShowSpecialAreas,
            Some(true),
        )
        .unwrap();
        let loaded = load_from(&prefs);
        assert_eq!(loaded.last_focus.as_deref(), Some("alice"));
        assert!(!loaded.capture);
        assert!(
            !loaded.nav.pause_script_on_manual_walk_abort,
            "TUI's nested nav write preserves the panel's saved pause toggle"
        );
        assert!(loaded.nav.show_special_areas);

        frontend_core::nav_preference(
            &prefs,
            frontend_core::NavPreference::PauseScriptOnManualWalkAbort,
            Some(true),
        )
        .unwrap();
        let loaded = load_from(&prefs);
        assert!(loaded.nav.pause_script_on_manual_walk_abort);
        assert!(
            loaded.nav.show_special_areas,
            "panel's typed save preserves TUI's special-area preference"
        );
    }

    #[test]
    fn overlapping_panel_save_and_single_key_nav_writer_keep_both_edits() {
        let (dir, prefs) = scratch("ui-nav-overlap");
        // The single-key writers are a second service on the same files: a
        // peer frontend's writes, as far as the panel's preferences can tell.
        let peer = scratch_service(&dir).preferences();
        save_to_checked(
            &prefs,
            &PanelUiState {
                last_focus: Some("before".into()),
                ..PanelUiState::default()
            },
        )
        .unwrap();
        peer.set("unrelated", serde_json::json!({"keep": 7}))
            .unwrap();
        peer.set(
            "quester_paths",
            serde_json::json!({"enabled": true, "folder": "checkout/quester/paths/289"}),
        )
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
            "survivable_routing": false,
            "script_scope_notice_ack": false,
            "survivable_routing_notice_ack": true,
            "show_nav_path": true
        });
        // Reproduce the stale panel snapshot deterministically before the
        // concurrent stress loop: a single-key writer lands, then the panel
        // saves its older in-memory state.
        peer.set("nav", nav.clone()).unwrap();
        let after_writer: serde_json::Value =
            serde_json::from_slice(&std::fs::read(prefs.path()).unwrap()).unwrap();
        assert_eq!(after_writer["last_focus"], "before");
        assert_eq!(after_writer["nav"]["allow_teleports"], true);
        assert_eq!(after_writer["nav"]["allow_wilderness"], false);
        assert_eq!(after_writer["nav"]["show_nav_path"], true);

        save_to_checked(&prefs, &panel_state).unwrap();
        let after_panel_save: serde_json::Value =
            serde_json::from_slice(&std::fs::read(prefs.path()).unwrap()).unwrap();
        assert_eq!(after_panel_save["last_focus"], "panel-save");
        assert_eq!(after_panel_save["capture"], true);
        assert_eq!(after_panel_save["nav"]["show_nav_path"], true);
        assert_eq!(after_panel_save["nav"]["allow_teleports"], true);
        assert_eq!(after_panel_save["nav"]["allow_wilderness"], false);
        assert_eq!(after_panel_save["nav"]["allow_bank_fetch"], false);
        assert_eq!(after_panel_save["nav"]["allow_danger_zones"], false);
        assert_eq!(after_panel_save["nav"]["survivable_routing"], false);
        assert_eq!(after_panel_save["nav"]["script_scope_notice_ack"], false);
        assert_eq!(
            after_panel_save["nav"]["survivable_routing_notice_ack"],
            true
        );
        assert_eq!(after_panel_save["quester_paths"]["enabled"], true);
        assert_eq!(
            after_panel_save["quester_paths"]["folder"],
            "checkout/quester/paths/289"
        );
        let nav = serde_json::json!({
            "allow_teleports": false,
            "allow_wilderness": true,
            "allow_bank_fetch": true,
            "allow_danger_zones": true,
            "survivable_routing": true,
            "script_scope_notice_ack": true,
            "survivable_routing_notice_ack": false,
            "show_nav_path": true
        });
        peer.set("nav", nav.clone()).unwrap();
        let after_second_writer: serde_json::Value =
            serde_json::from_slice(&std::fs::read(prefs.path()).unwrap()).unwrap();
        assert_eq!(after_second_writer["last_focus"], "panel-save");
        assert_eq!(after_second_writer["capture"], true);
        assert_eq!(after_second_writer["nav"]["show_nav_path"], true);
        assert_eq!(after_second_writer["nav"]["allow_danger_zones"], true);
        assert_eq!(after_second_writer["nav"]["survivable_routing"], true);
        assert_eq!(after_second_writer["nav"]["script_scope_notice_ack"], true);
        assert_eq!(
            after_second_writer["nav"]["survivable_routing_notice_ack"],
            false
        );
        // The panel's preferences see the peer's write once they re-read.
        prefs.reload().unwrap();
        let mut refreshed_nav = panel_state.nav.clone();
        refreshed_nav.refresh_walk_globals(&prefs).unwrap();
        assert!(!refreshed_nav.allow_teleports);
        assert!(refreshed_nav.allow_wilderness);
        assert!(refreshed_nav.allow_bank_fetch);
        assert!(refreshed_nav.allow_danger_zones);
        assert!(refreshed_nav.survivable_routing);
        assert!(refreshed_nav.script_scope_notice_ack);
        assert!(!refreshed_nav.survivable_routing_notice_ack);

        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let panel_prefs = prefs.clone();
        let panel_barrier = std::sync::Arc::clone(&barrier);
        let panel = std::thread::spawn(move || {
            for _ in 0..32 {
                panel_barrier.wait();
                save_to_checked(&panel_prefs, &panel_state).unwrap();
            }
        });
        let nav_barrier = std::sync::Arc::clone(&barrier);
        let writer = std::thread::spawn(move || {
            for _ in 0..32 {
                nav_barrier.wait();
                peer.set("nav", nav.clone()).unwrap();
            }
        });
        panel.join().unwrap();
        writer.join().unwrap();

        let document: serde_json::Value =
            serde_json::from_slice(&std::fs::read(prefs.path()).unwrap()).unwrap();
        assert_eq!(document["last_focus"], "panel-save");
        assert_eq!(document["capture"], true);
        assert_eq!(document["nav"]["show_nav_path"], true);
        assert_eq!(document["nav"]["allow_teleports"], false);
        assert_eq!(document["nav"]["allow_wilderness"], true);
        assert_eq!(document["nav"]["allow_bank_fetch"], true);
        assert_eq!(document["nav"]["allow_danger_zones"], true);
        assert_eq!(document["nav"]["survivable_routing"], true);
        assert_eq!(document["nav"]["script_scope_notice_ack"], true);
        assert_eq!(document["nav"]["survivable_routing_notice_ack"], false);
        assert_eq!(document["unrelated"]["keep"], 7);
    }

    #[test]
    fn load_save_roundtrip_last_focus() {
        let (_dir, prefs) = scratch("ui-roundtrip");

        let mut state = PanelUiState {
            last_focus: Some("bob".into()),
            ..Default::default()
        };
        state
            .collapsed
            .insert("bob".into(), HashMap::from([("nav".into(), true)]));
        state.debug_panel.recents = vec!["~give".into()];
        state.debug_panel.favorites = vec!["~reset".into()];
        save_to(&prefs, &state);

        let loaded = load_from(&prefs);
        assert_eq!(loaded.last_focus.as_deref(), Some("bob"));
        assert!(loaded.collapsed["bob"]["nav"]);
        assert_eq!(loaded.debug_panel.recents, vec!["~give"]);
        assert_eq!(loaded.debug_panel.favorites, vec!["~reset"]);
    }

    #[test]
    fn load_missing_file_is_default() {
        let (_dir, prefs) = scratch("ui-missing");
        let loaded = load_from(&prefs);
        assert!(loaded.last_focus.is_none());
        assert!(loaded.collapsed.is_empty());
    }

    /// A corrupt preferences file (not JSON) is refused by reads, so the
    /// panel starts from defaults; the panel's next save moves it aside
    /// byte for byte to `panel-ui.json.corrupt-<time>` and lands in fresh
    /// preferences (T5.2 r4, the recovery the old shared writer made); no
    /// `.new` side file.
    #[test]
    fn corrupt_prefs_are_moved_aside_and_the_save_lands_in_fresh_preferences() {
        let (dir, prefs) = scratch("ui-corrupt");
        let corrupt = b"{ not valid json";
        std::fs::write(prefs.path(), corrupt).unwrap();
        prefs
            .reload()
            .expect_err("a corrupt preferences file is refused by reads");
        assert_eq!(load_from(&prefs), PanelUiState::default());
        assert_eq!(std::fs::read(prefs.path()).unwrap(), corrupt);
        let state = PanelUiState {
            last_focus: Some("alice".into()),
            ..Default::default()
        };
        save_to_checked(&prefs, &state).unwrap();
        let backups: Vec<_> = std::fs::read_dir(&*dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("panel-ui.json.corrupt-")
            })
            .collect();
        assert_eq!(backups.len(), 1, "{backups:?}");
        assert_eq!(std::fs::read(&backups[0]).unwrap(), corrupt);
        assert!(!dir.join("panel-ui.json.new").exists());
        assert_eq!(load_from(&prefs).last_focus.as_deref(), Some("alice"));
    }

    /// A preferences file refused for the file itself (here: larger than
    /// the bound) is never written over or moved: no save lands, no side
    /// file takes the panel's state, and the panel starts from defaults.
    #[test]
    fn save_refuses_oversize_prefs_and_leaves_the_file_untouched() {
        let (dir, prefs) = scratch("ui-oversize");
        let oversize = vec![b' '; host_play::local_state::MAX_PREFERENCES_BYTES as usize + 1];
        std::fs::write(prefs.path(), &oversize).unwrap();
        prefs
            .reload()
            .expect_err("an oversize preferences file is refused");
        let state = PanelUiState {
            last_focus: Some("alice".into()),
            ..Default::default()
        };
        save_to_checked(&prefs, &state).expect_err("a refused file takes no save");
        save_to(&prefs, &state);
        assert!(std::fs::read(prefs.path()).unwrap() == oversize);
        let siblings: Vec<_> = std::fs::read_dir(&*dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with("panel-ui.json") && name != "panel-ui.json")
            .collect();
        assert!(
            siblings.iter().all(|name| name.ends_with(".lock")),
            "no `.new` side file and no `.corrupt-` backup: {siblings:?}"
        );
        assert_eq!(load_from(&prefs), PanelUiState::default());
    }

    /// A document that is valid JSON but not panel preferences is refused
    /// the same way: the save errors and the file keeps its bytes.
    #[test]
    fn save_refuses_a_document_that_does_not_read_as_panel_preferences() {
        let (dir, prefs) = scratch("ui-not-panel-prefs");
        let foreign = br#"{"last_focus":7,"peer_marker":"kept"}"#;
        std::fs::write(prefs.path(), foreign).unwrap();
        prefs.reload().unwrap();
        let state = PanelUiState {
            last_focus: Some("alice".into()),
            ..Default::default()
        };
        let error =
            save_to_checked(&prefs, &state).expect_err("a foreign document takes no panel save");
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert!(
            error.to_string().contains("panel preferences"),
            "the error says why: {error}"
        );
        assert_eq!(std::fs::read(prefs.path()).unwrap(), foreign);
        assert!(
            !dir.join("panel-ui.json.new").exists(),
            "the panel's state is not redirected to a side file"
        );
        assert_eq!(load_from(&prefs), PanelUiState::default());
    }

    /// The per-thread stand-in behind `load`/`save` never touches the store.
    #[test]
    fn save_load_roundtrip_via_default_api() {
        let (_dir, prefs) = scratch("ui-default-api");
        let state = PanelUiState {
            last_focus: Some("carol".into()),
            ..Default::default()
        };
        save(&prefs, &state);
        assert_eq!(load(&prefs).last_focus.as_deref(), Some("carol"));
        assert!(
            !prefs.path().exists(),
            "the stand-in does not write the preferences file"
        );
    }

    /// The composition root opens the service where the vault already lives:
    /// `<home>/.274bot`, with the preferences in `panel-ui.json` there.
    #[test]
    fn session_preferences_sit_under_home_274bot() {
        let session = crate::session::Session::new();
        let p = session.preferences.path();
        assert!(p.ends_with(".274bot/panel-ui.json"), "got {}", p.display());
        assert!(
            p.starts_with(script::bot_home()),
            "the file sits under the thread's home, got {}",
            p.display()
        );
    }

    #[test]
    fn script_category_order_persist_roundtrip() {
        let (_dir, prefs) = scratch("ui-cat-order");
        let state = PanelUiState {
            script_category_order: vec!["Prayer".into(), "Combat".into(), "Skilling".into()],
            ..Default::default()
        };
        save_to(&prefs, &state);
        let loaded = load_from(&prefs);
        assert_eq!(
            loaded.script_category_order,
            vec!["Prayer", "Combat", "Skilling"]
        );
    }

    #[test]
    fn script_catalog_last_dir_persist_roundtrip() {
        let (_dir, prefs) = scratch("ui-cat-dir");
        let state = PanelUiState {
            script_catalog_last_dir: Some(std::path::PathBuf::from("/tmp/rs2b0t")),
            script_load_last_dir: Some(std::path::PathBuf::from("/tmp/scripts")),
            ..Default::default()
        };
        save_to(&prefs, &state);
        let loaded = load_from(&prefs);
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

    /// An old file has none of the new keys: the render-all warning is still
    /// asked, sidecar 50 fps is off and focused 50 fps stays on, as before the
    /// choices were saved.
    #[test]
    fn missing_render_ack_and_50_fps_keys_behave_as_before() {
        let old: PanelUiState =
            serde_json::from_str(r#"{"last_focus":null,"collapsed":{}}"#).unwrap();
        assert!(!old.render_all_warning_ack);
        assert!(!old.sidecar_50);
        assert!(old.focused_50);
    }

    /// The acknowledgement and both 50 fps choices survive a save and a
    /// reload through the preferences file, next to the keys other features
    /// wrote.
    #[test]
    fn render_ack_and_50_fps_choices_round_trip_through_the_file() {
        let (_dir, prefs) = scratch("ui-render-ack");
        let state = PanelUiState {
            render_all_warning_ack: true,
            sidecar_50: true,
            focused_50: false,
            capture: false,
            ..Default::default()
        };
        save_to(&prefs, &state);
        let back = load_from(&prefs);
        assert!(back.render_all_warning_ack);
        assert!(back.sidecar_50);
        assert!(!back.focused_50);
        assert!(!back.capture, "other keys are kept");
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(prefs.path()).unwrap()).unwrap();
        assert_eq!(value["render_all_warning_ack"], true);
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
        let (_dir, prefs) = scratch("ui-capture");
        let state = PanelUiState {
            capture: false,
            ..Default::default()
        };
        save_to(&prefs, &state);
        assert!(!load_from(&prefs).capture);
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
        let (_dir, prefs) = scratch("ui-map-bake");
        let state = PanelUiState {
            map_bake: MapBakeChoice::Always,
            ..Default::default()
        };
        save_to(&prefs, &state);
        assert_eq!(load_from(&prefs).map_bake, MapBakeChoice::Always);
    }
}
