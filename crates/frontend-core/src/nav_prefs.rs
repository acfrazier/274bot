#[cfg(any(test, feature = "test-support"))]
use std::io;

use host_play::{local_state::Update, DangerLevel};

use crate::prefs::{PreferenceError, PreferencePatch, UiPreferences};

/// A boolean inside the shared `panel-ui.json` `nav` object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavPreference {
    /// Whether the TUI map paints the content-defined special areas.
    ShowSpecialAreas,
    /// Whether qualifying manual movement pauses the owning script.
    PauseScriptOnManualWalkAbort,
    /// Global permission for every walk to use teleports.
    AllowTeleports,
    /// Global permission for every walk to enter the wilderness.
    AllowWilderness,
    /// Global permission for manual WalkTo to use BankBudget fetch.
    AllowBankFetch,
    /// Global permission override for every walk to enter danger zones.
    AllowDangerZones,
    /// Global permission for every walk to run dialogue steps.
    AllowPrereqDialogue,
    /// Global permission for every walk to start quests.
    AllowQuestStart,
    /// Whether the user dismissed the one-time script-scope notice.
    ScriptScopeNoticeAck,
}

impl NavPreference {
    fn key_and_default(self) -> (&'static str, bool) {
        match self {
            Self::ShowSpecialAreas => ("show_special_areas", false),
            Self::PauseScriptOnManualWalkAbort => ("pause_script_on_manual_walk_abort", true),
            Self::AllowTeleports => ("allow_teleports", false),
            Self::AllowWilderness => ("allow_wilderness", false),
            Self::AllowBankFetch => ("allow_bank_fetch", false),
            Self::AllowDangerZones => (
                "allow_danger_zones",
                DangerLevel::default() == DangerLevel::Always,
            ),
            Self::AllowPrereqDialogue => ("allow_prereq_dialogue", true),
            Self::AllowQuestStart => ("allow_quest_start", true),
            Self::ScriptScopeNoticeAck => ("script_scope_notice_ack", false),
        }
    }

    /// Stable recorder identity for this shared navigation setting.
    pub(crate) const fn setting_identity(self) -> &'static str {
        match self {
            Self::ShowSpecialAreas => "nav.show_special_areas",
            Self::PauseScriptOnManualWalkAbort => "nav.pause_script_on_manual_walk_abort",
            Self::AllowTeleports => "nav.allow_teleports",
            Self::AllowWilderness => "nav.allow_wilderness",
            Self::AllowBankFetch => "nav.allow_bank_fetch",
            Self::AllowDangerZones => "nav.allow_danger_zones",
            Self::AllowPrereqDialogue => "nav.allow_prereq_dialogue",
            Self::AllowQuestStart => "nav.allow_quest_start",
            Self::ScriptScopeNoticeAck => "nav.script_scope_notice_ack",
        }
    }
}
/// Read one shared nested nav boolean through the process service's
/// preferences, with its legacy default. (Changing it is the
/// `SetNavPreference` command.)
pub fn nav_preference(
    preferences: &UiPreferences,
    preference: NavPreference,
) -> Result<bool, PreferenceError> {
    let (key, default) = preference.key_and_default();
    Ok(preferences
        .document()?
        .get("nav")
        .and_then(|value| value.get(key).and_then(serde_json::Value::as_bool))
        .unwrap_or(default))
}

/// Submit one shared nested nav boolean without waiting.
/// The service merges only that member into its latest durable document.
pub(crate) fn write_nav_preference(
    preferences: &UiPreferences,
    preference: NavPreference,
    enabled: bool,
) -> Result<Update, PreferenceError> {
    let (key, _) = preference.key_and_default();
    preferences.begin_patch(PreferencePatch::member(
        &["nav", key],
        Some(serde_json::Value::Bool(enabled)),
    )?)
}

/// Fixture seam: arrange a stored nav boolean as another writer would
/// (front-end tests), durable when it returns.
#[cfg(any(test, feature = "test-support"))]
pub fn write_nav_preference_for_test(
    preferences: &UiPreferences,
    preference: NavPreference,
    enabled: bool,
) -> io::Result<()> {
    crate::prefs::settled_for_test(write_nav_preference(preferences, preference, enabled))
}

#[cfg(test)]
mod tests {
    use super::{nav_preference, write_nav_preference, NavPreference};
    use crate::prefs::settled_for_test;
    use crate::prefs::UiPreferences as Preferences;
    use host_play::local_state::{LocalStateService, StateRoot};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A private scratch state root: where a legacy `panel-ui.json` is
    /// written before any service opens the root.
    fn root(label: &str) -> PathBuf {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "274bot-frontend-nav-prefs-{}-{label}-{id}",
            std::process::id()
        ));
        vault::create_private_dir(&dir).unwrap();
        dir
    }

    /// The preferences of a process service opened on `dir`; the first
    /// open imports a legacy `panel-ui.json` already there.
    fn open(dir: &std::path::Path) -> Preferences {
        Preferences::new(
            LocalStateService::open_process(StateRoot::new(dir.to_path_buf())).preferences(),
        )
    }

    /// Wait until the service published its first readable document.
    fn ready(prefs: &Preferences) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while !prefs.ready() {
            assert!(
                std::time::Instant::now() < deadline,
                "preferences never opened"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// A scratch state root, and the preferences of a service opened on it.
    fn scratch(label: &str) -> (PathBuf, Preferences) {
        let dir = root(label);
        let preferences = open(&dir);
        ready(&preferences);
        (dir, preferences)
    }

    /// A scratch state root holding `legacy` as its `panel-ui.json`, and the
    /// preferences of the service that imported it.
    fn scratch_with_legacy(label: &str, legacy: &[u8]) -> (PathBuf, Preferences) {
        let dir = root(label);
        vault::write_private_file(&dir.join("panel-ui.json"), legacy).unwrap();
        let preferences = open(&dir);
        ready(&preferences);
        (dir, preferences)
    }

    #[test]
    fn legacy_and_default_values_and_cross_surface_writes_preserve_other_preferences() {
        let (dir, prefs) = scratch_with_legacy(
            "roundtrip",
            br#"{"last_focus":"alice","capture":false,"nav":{"show_special_areas":false,"custom":{"keep":7}}}"#,
        );

        assert!(!nav_preference(&prefs, NavPreference::ShowSpecialAreas).unwrap());
        assert!(nav_preference(&prefs, NavPreference::PauseScriptOnManualWalkAbort).unwrap());
        for preference in [
            NavPreference::AllowPrereqDialogue,
            NavPreference::AllowQuestStart,
        ] {
            assert!(
                nav_preference(&prefs, preference).unwrap(),
                "missing {preference:?} defaults on"
            );
            settled_for_test(write_nav_preference(&prefs, preference, false)).unwrap();
            settled_for_test(write_nav_preference(&prefs, preference, true)).unwrap();
        }
        for preference in [
            NavPreference::AllowTeleports,
            NavPreference::AllowWilderness,
            NavPreference::AllowBankFetch,
            NavPreference::ScriptScopeNoticeAck,
        ] {
            assert!(
                !nav_preference(&prefs, preference).unwrap(),
                "missing {preference:?} defaults off"
            );
            settled_for_test(write_nav_preference(&prefs, preference, true)).unwrap();
        }
        assert!(nav_preference(&prefs, NavPreference::AllowDangerZones).unwrap());
        settled_for_test(write_nav_preference(
            &prefs,
            NavPreference::AllowDangerZones,
            true,
        ))
        .unwrap();

        settled_for_test(write_nav_preference(
            &prefs,
            NavPreference::ShowSpecialAreas,
            true,
        ))
        .unwrap();
        settled_for_test(write_nav_preference(
            &prefs,
            NavPreference::PauseScriptOnManualWalkAbort,
            false,
        ))
        .unwrap();

        let saved = prefs.reload().unwrap();
        assert_eq!(saved["last_focus"], "alice");
        assert_eq!(saved["capture"], false);
        assert_eq!(saved["nav"]["custom"]["keep"], 7);
        assert_eq!(saved["nav"]["show_special_areas"], true);
        assert_eq!(saved["nav"]["pause_script_on_manual_walk_abort"], false);
        assert_eq!(saved["nav"]["allow_teleports"], true);
        assert_eq!(saved["nav"]["allow_wilderness"], true);
        assert_eq!(saved["nav"]["allow_bank_fetch"], true);
        assert_eq!(saved["nav"]["allow_danger_zones"], true);
        assert_eq!(saved["nav"]["allow_prereq_dialogue"], true);
        assert_eq!(saved["nav"]["allow_quest_start"], true);
        assert_eq!(saved["nav"]["script_scope_notice_ack"], true);

        let (missing_dir, missing) =
            scratch_with_legacy("legacy", br#"{"last_focus":"bob","nav":{}}"#);
        assert!(nav_preference(&missing, NavPreference::PauseScriptOnManualWalkAbort).unwrap());
        for preference in [
            NavPreference::AllowPrereqDialogue,
            NavPreference::AllowQuestStart,
        ] {
            assert!(
                nav_preference(&missing, preference).unwrap(),
                "an old file without the key keeps {preference:?} on"
            );
        }
        for preference in [
            NavPreference::AllowTeleports,
            NavPreference::AllowWilderness,
            NavPreference::AllowBankFetch,
            NavPreference::ScriptScopeNoticeAck,
        ] {
            assert!(!nav_preference(&missing, preference).unwrap());
        }
        assert!(nav_preference(&missing, NavPreference::AllowDangerZones).unwrap());
        assert!(!nav_preference(&missing, NavPreference::ShowSpecialAreas).unwrap());

        std::fs::remove_dir_all(dir).unwrap();
        std::fs::remove_dir_all(missing_dir).unwrap();
    }

    #[test]
    fn failed_nav_preference_write_is_returned_to_the_frontend() {
        let (dir, _) = scratch("blocked-parent");
        let blocked = dir.join("blocked");
        std::fs::write(&blocked, b"not a directory").unwrap();
        let prefs = Preferences::new(
            LocalStateService::open_process(StateRoot::new(blocked.clone())).preferences(),
        );
        settled_for_test(write_nav_preference(
            &prefs,
            NavPreference::PauseScriptOnManualWalkAbort,
            false,
        ))
        .expect_err("the preference writer must report an inaccessible destination");
        assert_eq!(std::fs::read(&blocked).unwrap(), b"not a directory");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn danger_preference_fails_closed_for_a_refused_preferences_store() {
        let dir = root("refused-danger");
        let legacy = dir.join("panel-ui.json");
        let oversize = vec![b' '; host_play::local_state::MAX_PREFERENCES_BYTES as usize + 1];
        vault::write_private_file(&legacy, &oversize).unwrap();
        let prefs = open(&dir);
        assert!(prefs.reload().is_err());
        assert!(nav_preference(&prefs, NavPreference::AllowDangerZones).is_err());
        assert_eq!(
            std::fs::read(&legacy).unwrap(),
            oversize,
            "a refused source is left untouched"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
