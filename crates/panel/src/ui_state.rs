//! Persisted panel UI prefs (`~/.274bot/panel-ui.json`): last focused
//! profile and per-profile collapsed section maps.
//!
//! The file is the operator preferences the process local-state service
//! owns ([`Preferences`]): the panel reads and writes it only through the
//! session's handle, never by path.

use std::collections::HashMap;
#[cfg(test)]
use std::io;
use std::path::PathBuf;

use crate::nav_settings::NavSettings;

#[cfg(test)]
use frontend_core::log::Level;
use frontend_core::UiPreferences as Preferences;
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
    pub raster: frontend_core::command::RasterMode,
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
    /// The fleet toolbar is open (not folded to its header) in the rail and
    /// the strip's fleet section. Absent = open. Apart from the tile folds.
    #[serde(default = "default_true")]
    pub fleet_toolbar_open: bool,
    /// Debug command recents and favorites. Argument buffers stay ephemeral
    /// because the selected content pin may change command shapes.
    #[serde(default)]
    pub debug_panel: crate::debug_panel::DebugPanelPrefs,
}

/// Panel subsection ids in General config (parameters shares
/// [`PanelUiState::show_parameters_rail`]).
pub const PANEL_SECTION_IDS: &[&str] = &[
    "status",
    "fleet",
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
            raster: frontend_core::command::RasterMode::Gpu,
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
            fleet_toolbar_open: true,
            debug_panel: crate::debug_panel::DebugPanelPrefs::default(),
        }
    }
}

/// Default closed (collapsed) when no persisted entry: script + parameters only.
pub fn default_section_closed(id: &str) -> bool {
    id == "script" || id == "parameters"
}

/// The preferences the session reads. Under `cfg(test)` a per-thread
/// stand-in for the panel's own keys, so parallel `select` calls do not race
/// on a shared file or the operator's real prefs. The keys only the core
/// writes (`CORE_OWNED_KEYS`) still come from the store once their command
/// has written them, as they do outside tests; [`load_from`] always reads
/// the store.
pub fn load(preferences: &Preferences) -> PanelUiState {
    let Ok(document) = preferences.document() else {
        #[cfg(test)]
        return TEST_STATE.with(|state| state.borrow().clone());
        #[cfg(not(test))]
        return PanelUiState::default();
    };
    load_snapshot(&document)
}

pub(crate) fn load_snapshot(document: &Map<String, Value>) -> PanelUiState {
    #[cfg(test)]
    {
        if DOCUMENT_READER.with(std::cell::Cell::get) {
            return from_document(document);
        }
        let stand_in = TEST_STATE.with(|state| state.borrow().clone());
        with_core_owned_keys(stand_in, document)
    }
    #[cfg(not(test))]
    from_document(document)
}

/// Test seam: while the returned guard lives, this test thread reads the
/// panel's own members from the preferences document, as the product does,
/// instead of the per-thread stand-in (so another process's saved value
/// reaches it).
#[cfg(test)]
pub(crate) fn read_documents_for_test() -> DocumentReader {
    DOCUMENT_READER.with(|reader| reader.set(true));
    DocumentReader
}

/// Ends [`read_documents_for_test`] when dropped (test threads are reused).
#[cfg(test)]
pub(crate) struct DocumentReader;

#[cfg(test)]
impl Drop for DocumentReader {
    fn drop(&mut self) {
        DOCUMENT_READER.with(|reader| reader.set(false));
    }
}

/// `state` with the [`CORE_OWNED_KEYS`] the store holds: their command rows
/// write the store, never the stand-in, so a key the store holds wins over
/// the stand-in's (seeded) value, and a key it lacks keeps it.
#[cfg(test)]
fn with_core_owned_keys(state: PanelUiState, document: &Map<String, Value>) -> PanelUiState {
    let mut merged = serde_json::to_value(&state).expect("PanelUiState serializes");
    let keys = merged
        .as_object_mut()
        .expect("serialized PanelUiState is an object");
    for key in CORE_OWNED_KEYS {
        if let Some(stored) = document.get(*key) {
            keys.insert((*key).into(), stored.clone());
        }
    }
    serde_json::from_value(merged).unwrap_or(state)
}

/// Seed the per-thread fixture for the panel's own preferences.
#[cfg(test)]
pub fn save(_preferences: &Preferences, state: &PanelUiState) {
    TEST_STATE.with(|s| *s.borrow_mut() = state.clone());
}

/// Mirror one committed panel preference in the test-only snapshot.
#[cfg(test)]
pub(crate) fn commit_test_member(path: &[String], value: &Option<Value>) {
    TEST_STATE.with(|state| {
        let Ok(mut stored) = serde_json::to_value(state.borrow().clone()) else {
            return;
        };
        let Some(root) = stored.as_object_mut() else {
            return;
        };
        apply_json_member(root, path, value);
        let Ok(mut updated) = serde_json::from_value::<PanelUiState>(stored) else {
            return;
        };
        updated.debug_panel.normalize();
        *state.borrow_mut() = updated;
    });
}

/// The panel preferences `preferences` holds: defaults when the file is
/// absent, when the service refused it (unreadable), or when it does not
/// read as panel preferences.
#[cfg(test)]
pub fn load_from(preferences: &Preferences) -> PanelUiState {
    let Ok(document) = preferences.document() else {
        return PanelUiState::default();
    };
    from_document(&document)
}

pub(crate) fn from_document(document: &Map<String, Value>) -> PanelUiState {
    match serde_json::from_value::<PanelUiState>(Value::Object(document.clone())) {
        Ok(mut state) => {
            state.debug_panel.normalize();
            state
        }
        Err(_) => PanelUiState::default(),
    }
}

/// `state` with the previews of the requests still pending laid over it,
/// oldest first, so the newest request of a member decides its value.
pub(crate) fn apply_pending_members<'a>(
    mut state: PanelUiState,
    members: impl IntoIterator<Item = (&'a [String], &'a Option<Value>)>,
) -> PanelUiState {
    let Ok(mut serialized) = serde_json::to_value(&state) else {
        return state;
    };
    let Some(root) = serialized.as_object_mut() else {
        return state;
    };
    for (path, value) in members {
        apply_json_member(root, path, value);
    }
    if let Ok(mut updated) = serde_json::from_value::<PanelUiState>(serialized) {
        updated.debug_panel.normalize();
        state = updated;
    }
    state
}

/// Whether the member at `path` is a remembered launch choice: the server
/// revision and the Game-pane raster and low-memory modes. Such a choice
/// takes effect only when its own write is durable (its owner applies it
/// then), so a pending preview never lays over it.
pub(crate) fn is_launch_choice(path: &[String]) -> bool {
    matches!(
        path.first().map(String::as_str),
        Some("raster" | "lowmem" | "server_revision")
    )
}

/// The panel view of the preferences after a change: the latest durable
/// `document` with the previews of the requests still `pending` laid over
/// it (oldest first, so the newest of a member wins). What the panel does
/// not read from the document stays as `current` has it: the saved choices
/// the core owns (draw switches, bake choice, notices, session file, walk
/// grants, the special-areas overlay and the manual-walk pause) and the
/// server revision (its owner follows it). The remembered raster and memory
/// mode follow the document while no bot is focused (`bot_focused` false);
/// with a bot focused the controls show that bot's own saved modes, so they
/// stay. A member this does not name follows the document, so a new
/// panel-owned member is rebased by default.
pub(crate) fn rebased<'a>(
    current: &PanelUiState,
    document: &Map<String, Value>,
    pending: impl IntoIterator<Item = (&'a [String], &'a Option<Value>)>,
    bot_focused: bool,
) -> PanelUiState {
    let next = apply_pending_members(
        load_snapshot(document),
        pending
            .into_iter()
            .filter(|(path, _)| !is_launch_choice(path)),
    );
    let nav = NavSettings {
        allow_teleports: current.nav.allow_teleports,
        allow_wilderness: current.nav.allow_wilderness,
        allow_bank_fetch: current.nav.allow_bank_fetch,
        allow_danger_zones: current.nav.allow_danger_zones,
        survivable_routing: current.nav.survivable_routing,
        script_scope_notice_ack: current.nav.script_scope_notice_ack,
        survivable_routing_notice_ack: current.nav.survivable_routing_notice_ack,
        show_special_areas: current.nav.show_special_areas,
        pause_script_on_manual_walk_abort: current.nav.pause_script_on_manual_walk_abort,
        ..next.nav
    };
    let (raster, lowmem) = if bot_focused {
        (current.raster, current.lowmem)
    } else {
        (next.raster, next.lowmem)
    };
    PanelUiState {
        capture: current.capture,
        sidecar_50: current.sidecar_50,
        focused_50: current.focused_50,
        map_bake: current.map_bake,
        session_log_file: current.session_log_file,
        background_bots_ack: current.background_bots_ack,
        render_all_warning_ack: current.render_all_warning_ack,
        raster,
        lowmem,
        server_revision: current.server_revision,
        nav,
        ..next
    }
}

fn apply_json_member(root: &mut Map<String, Value>, path: &[String], value: &Option<Value>) {
    let Some((leaf, parents)) = path.split_last() else {
        return;
    };
    let mut object = root;
    for parent in parents {
        let entry = object
            .entry(parent.clone())
            .or_insert_with(|| Value::Object(Map::new()));
        if !entry.is_object() {
            *entry = Value::Object(Map::new());
        }
        object = entry
            .as_object_mut()
            .expect("a preference path parent was made an object");
    }
    match value {
        Some(value) => {
            object.insert(leaf.clone(), value.clone());
        }
        None => {
            object.remove(leaf);
        }
    }
}

#[cfg(test)]
fn prefs_log(_level: Level, message: String) {
    api::host_log!(
        stderr;
        api::hostlog::Category::Lifecycle,
        api::hostlog::Level::Error,
        "panel: {message}"
    );
}

/// The `nav` keys the core writes (walk permissions, notices, the manual
/// movement pause, the special-area paint): a panel save keeps the
/// document's value, or the key's default when the document has none.
#[cfg(test)]
const MERGED_NAV_KEYS: &[(&str, bool)] = &[
    ("allow_teleports", false),
    ("allow_wilderness", false),
    ("allow_bank_fetch", false),
    ("allow_danger_zones", false),
    ("survivable_routing", true),
    ("script_scope_notice_ack", false),
    ("survivable_routing_notice_ack", false),
    ("pause_script_on_manual_walk_abort", true),
    ("show_special_areas", false),
];

/// Top-level operator preferences only the core writes, through their
/// commands (`SetDraw`, `AcknowledgeNotice`, `SetMapBakeChoice`,
/// `SetSessionLog`): a panel save never writes them; the document keeps
/// what it holds.
#[cfg(test)]
const CORE_OWNED_KEYS: &[&str] = &[
    "capture",
    "sidecar_50",
    "focused_50",
    "render_all_warning_ack",
    "background_bots_ack",
    "map_bake",
    "session_log_file",
];

/// Merge the panel's own keys (`replacement`, the serialized state) into the
/// document read now: every other key stays as the document holds it, and
/// the walk permission keys of `nav` ([`MERGED_NAV_KEYS`]) stay as the
/// document holds them. A document that does not read as panel preferences
/// is refused whole, and nothing is written over it.
#[cfg(test)]
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
    for (key, default) in MERGED_NAV_KEYS {
        let enabled = previous_nav
            .and_then(|nav| nav.get(*key))
            .and_then(Value::as_bool)
            .unwrap_or(*default);
        merged_nav.insert((*key).into(), Value::Bool(enabled));
    }
    let kept: Vec<(&str, Option<Value>)> = CORE_OWNED_KEYS
        .iter()
        .map(|key| (*key, document.get(*key).cloned()))
        .collect();
    document.extend(replacement.clone());
    document.insert("nav".into(), Value::Object(merged_nav));
    for (key, value) in kept {
        match value {
            Some(value) => {
                document.insert(key.into(), value);
            }
            None => {
                document.remove(key);
            }
        }
    }
    Ok(())
}

/// Write `state` into `preferences`, the outcome logged (see
/// [`save_to_checked`]) and not returned.
#[cfg(test)]
pub fn save_to(preferences: &Preferences, state: &PanelUiState) {
    let _ = save_to_checked(preferences, state);
}

/// Seed the per-thread panel fixture through the checked fixture API.
#[cfg(test)]
pub fn save_checked(_preferences: &Preferences, state: &PanelUiState) -> io::Result<()> {
    TEST_STATE.with(|s| *s.borrow_mut() = state.clone());
    Ok(())
}

/// Persist the panel's own keys into `preferences` through the same
/// serialized read/merge/write transaction as shared nested preference
/// updates. A file the service refused, or one that does not read as panel
/// preferences, is not written: the error is returned and logged once.
#[cfg(test)]
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
    match preferences.update(move |document| merge_saved_panel_state(document, &replacement)) {
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
    static DOCUMENT_READER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
mod tests {
    use super::{load, load_from, save, save_to, save_to_checked, NavSettings, PanelUiState};
    use crate::test_support::{patiently, scratch_service, set_patiently, TestDir};
    use frontend_core::UiPreferences as Preferences;
    use std::collections::HashMap;

    /// A scratch state root and the preferences of a service opened on it:
    /// the service stops before the directory goes.
    fn scratch(label: &str) -> (TestDir, Preferences) {
        let dir = TestDir::new(label);
        let preferences = frontend_core::UiPreferences::of_service_for_test(&scratch_service(&dir));
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
        crate::test_support::write_private_fixture(
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
        // "capture input" belongs to its SetDraw row and the manual-walk
        // pause to its SetNavPreference row: the core writes both, and no
        // panel save does. The panel's own state below still holds them on
        // (the defaults), and the document keeps what the rows wrote.
        set_patiently(&prefs, "capture", serde_json::json!(false));
        patiently(|| {
            frontend_core::nav_prefs::write_nav_preference_for_test(
                &prefs,
                frontend_core::NavPreference::PauseScriptOnManualWalkAbort,
                false,
            )
            .map_err(|error| error.to_string())
        })
        .unwrap();
        let state = PanelUiState {
            last_focus: Some("alice".into()),
            nav: NavSettings {
                show_nav_path: true,
                ..NavSettings::default()
            },
            ..PanelUiState::default()
        };
        save_to(&prefs, &state);

        frontend_core::nav_prefs::write_nav_preference_for_test(
            &prefs,
            frontend_core::NavPreference::ShowSpecialAreas,
            true,
        )
        .map(|()| true)
        .unwrap();
        let loaded = load_from(&prefs);
        assert_eq!(loaded.last_focus.as_deref(), Some("alice"));
        assert!(!loaded.capture);
        assert!(
            loaded.nav.show_nav_path,
            "TUI's nested nav write preserves the panel's saved nav toggle"
        );
        assert!(
            !loaded.nav.pause_script_on_manual_walk_abort,
            "TUI's nested nav write preserves the pause toggle its row saved"
        );
        assert!(loaded.nav.show_special_areas);

        frontend_core::nav_prefs::write_nav_preference_for_test(
            &prefs,
            frontend_core::NavPreference::PauseScriptOnManualWalkAbort,
            true,
        )
        .map(|()| true)
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
        let peer = frontend_core::UiPreferences::of_service_for_test(&scratch_service(&dir));
        patiently(|| {
            save_to_checked(
                &prefs,
                &PanelUiState {
                    last_focus: Some("before".into()),
                    ..PanelUiState::default()
                },
            )
            .map_err(|error| error.to_string())
        })
        .unwrap();
        // "capture input" belongs to its SetDraw row: the core writes it and
        // a panel save keeps what the document holds. The panel's snapshot
        // below still holds it off, and must not win.
        set_patiently(&prefs, "capture", serde_json::json!(true));
        set_patiently(&peer, "unrelated", serde_json::json!({"keep": 7}));
        set_patiently(
            &peer,
            "quester_paths",
            serde_json::json!({"enabled": true, "folder": "checkout/quester/paths/289"}),
        );
        let panel_state = PanelUiState {
            last_focus: Some("panel-save".into()),
            capture: false,
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
        set_patiently(&peer, "nav", nav.clone());
        let after_writer: serde_json::Value =
            serde_json::from_slice(&std::fs::read(prefs.path()).unwrap()).unwrap();
        assert_eq!(after_writer["last_focus"], "before");
        assert_eq!(after_writer["nav"]["allow_teleports"], true);
        assert_eq!(after_writer["nav"]["allow_wilderness"], false);
        assert_eq!(after_writer["nav"]["show_nav_path"], true);

        patiently(|| save_to_checked(&prefs, &panel_state).map_err(|error| error.to_string()))
            .unwrap();
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
        set_patiently(&peer, "nav", nav.clone());
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
        let panel_prefs = prefs.share();
        let panel_barrier = std::sync::Arc::clone(&barrier);
        let panel = std::thread::spawn(move || {
            for _ in 0..32 {
                panel_barrier.wait();
                patiently(|| {
                    save_to_checked(&panel_prefs, &panel_state).map_err(|error| error.to_string())
                })
                .unwrap();
            }
        });
        let nav_barrier = std::sync::Arc::clone(&barrier);
        let writer = std::thread::spawn(move || {
            for _ in 0..32 {
                nav_barrier.wait();
                set_patiently(&peer, "nav", nav.clone());
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
        crate::test_support::write_private_fixture(prefs.path(), corrupt).unwrap();
        prefs
            .reload()
            .expect_err("a corrupt preferences file is refused by reads");
        assert_eq!(load_from(&prefs), PanelUiState::default());
        assert_eq!(std::fs::read(prefs.path()).unwrap(), corrupt);
        let state = PanelUiState {
            last_focus: Some("alice".into()),
            ..Default::default()
        };
        patiently(|| save_to_checked(&prefs, &state).map_err(|error| error.to_string())).unwrap();
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
        crate::test_support::write_private_fixture(prefs.path(), foreign).unwrap();
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
            p.starts_with(frontend_core::scripts::paths::bot_home()),
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

    /// The acknowledgement and both 50 fps choices survive a panel save and
    /// a reload through the preferences file, next to the keys other
    /// features wrote. Their command rows write them (`AcknowledgeNotice`,
    /// `SetDraw`) with the same preference writer, and the panel's own save,
    /// here of its default state, never does.
    #[test]
    fn render_ack_and_50_fps_choices_round_trip_through_the_file() {
        let (_dir, prefs) = scratch("ui-render-ack");
        set_patiently(&prefs, "render_all_warning_ack", serde_json::json!(true));
        set_patiently(&prefs, "sidecar_50", serde_json::json!(true));
        set_patiently(&prefs, "focused_50", serde_json::json!(false));
        set_patiently(&prefs, "capture", serde_json::json!(false));
        save_to(&prefs, &PanelUiState::default());
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

    /// "capture input" is written by its `SetDraw` row (one key through the
    /// preference writer), kept by the panel's own save, and read back.
    #[test]
    fn capture_persist_roundtrip() {
        let (_dir, prefs) = scratch("ui-capture");
        set_patiently(&prefs, "capture", serde_json::json!(false));
        save_to(&prefs, &PanelUiState::default());
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
        // `SetMapBakeChoice` writes the choice with this writer; the panel's
        // own save, here of its default state, keeps it.
        patiently(|| {
            frontend_core::map_bake::persist_map_bake_choice_for_test(&prefs, MapBakeChoice::Always)
                .map_err(|error| error.to_string())
        })
        .unwrap();
        save_to(&prefs, &PanelUiState::default());
        assert_eq!(load_from(&prefs).map_bake, MapBakeChoice::Always);
    }
}
