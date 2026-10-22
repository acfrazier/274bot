use std::io;

use host_play::local_state::Update;

use crate::prefs::UiPreferences;

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
    /// Whether the host admits survivable crossings after its runtime net is available.
    SurvivableRouting,
    /// Whether the user dismissed the one-time script-scope notice.
    ScriptScopeNoticeAck,
    /// Whether the user dismissed the danger-routing migration notice.
    SurvivableRoutingNoticeAck,
}

impl NavPreference {
    const fn key_and_default(self) -> (&'static str, bool) {
        match self {
            Self::ShowSpecialAreas => ("show_special_areas", false),
            Self::PauseScriptOnManualWalkAbort => ("pause_script_on_manual_walk_abort", true),
            Self::AllowTeleports => ("allow_teleports", false),
            Self::AllowWilderness => ("allow_wilderness", false),
            Self::AllowBankFetch => ("allow_bank_fetch", false),
            Self::AllowDangerZones => ("allow_danger_zones", false),
            Self::SurvivableRouting => ("survivable_routing", true),
            Self::ScriptScopeNoticeAck => ("script_scope_notice_ack", false),
            Self::SurvivableRoutingNoticeAck => ("survivable_routing_notice_ack", false),
        }
    }
}

/// Read one shared nested nav boolean through the process service's
/// preferences, with its legacy default. (Changing it is the
/// `SetNavPreference` command.)
pub fn nav_preference(preferences: &UiPreferences, preference: NavPreference) -> io::Result<bool> {
    let (key, default) = preference.key_and_default();
    if preference == NavPreference::SurvivableRouting {
        return Ok(host_play::WalkGlobals::read(preferences.inner())
            .unwrap_or_else(|_| host_play::WalkGlobals::fail_closed())
            .survivable_routing);
    }
    Ok(preferences
        .value("nav")
        .and_then(|value| value.get(key).and_then(serde_json::Value::as_bool))
        .unwrap_or(default))
}

/// Write one shared nested nav boolean: only that key is merged, against
/// the file as the service reads it then. A write still queued when the
/// bounded wait ends comes back [`Update::Queued`] for its command to
/// settle from.
pub(crate) fn write_nav_preference(
    preferences: &UiPreferences,
    preference: NavPreference,
    enabled: bool,
) -> io::Result<Update> {
    let (key, _) = preference.key_and_default();
    preferences.begin_update(move |document| {
        let mut nav = document
            .get("nav")
            .filter(|value| value.is_object())
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        nav.as_object_mut()
            .expect("a replacement nav object is always an object")
            .insert(key.into(), serde_json::Value::Bool(enabled));
        document.insert("nav".into(), nav);
        Ok(())
    })
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

    /// A scratch state root, and the preferences of a service opened on it.
    fn scratch(label: &str) -> (PathBuf, Preferences) {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "274bot-frontend-nav-prefs-{}-{label}-{id}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let preferences = Preferences::new(
            LocalStateService::open_process(StateRoot::new(dir.clone())).preferences(),
        );
        (dir, preferences)
    }

    #[test]
    fn legacy_and_default_values_and_cross_surface_writes_preserve_other_preferences() {
        let (dir, prefs) = scratch("roundtrip");
        vault::write_private_file(
            &prefs.path(),
            br#"{"last_focus":"alice","capture":false,"nav":{"show_special_areas":false,"custom":{"keep":7}}}"#,
        )
        .unwrap();
        prefs.reload().unwrap();

        assert!(!nav_preference(&prefs, NavPreference::ShowSpecialAreas).unwrap());
        assert!(nav_preference(&prefs, NavPreference::PauseScriptOnManualWalkAbort).unwrap());
        assert!(
            nav_preference(&prefs, NavPreference::SurvivableRouting).unwrap(),
            "the new preference defaults on when absent from a valid old file"
        );
        for preference in [
            NavPreference::AllowTeleports,
            NavPreference::AllowWilderness,
            NavPreference::AllowBankFetch,
            NavPreference::AllowDangerZones,
            NavPreference::ScriptScopeNoticeAck,
            NavPreference::SurvivableRoutingNoticeAck,
        ] {
            assert!(
                !nav_preference(&prefs, preference).unwrap(),
                "missing {preference:?} defaults off"
            );
            settled_for_test(write_nav_preference(&prefs, preference, true)).unwrap();
        }

        settled_for_test(write_nav_preference(
            &prefs,
            NavPreference::SurvivableRouting,
            false,
        ))
        .unwrap();
        settled_for_test(write_nav_preference(
            &prefs,
            NavPreference::SurvivableRouting,
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

        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(prefs.path()).unwrap()).unwrap();
        assert_eq!(saved["last_focus"], "alice");
        assert_eq!(saved["capture"], false);
        assert_eq!(saved["nav"]["custom"]["keep"], 7);
        assert_eq!(saved["nav"]["show_special_areas"], true);
        assert_eq!(saved["nav"]["pause_script_on_manual_walk_abort"], false);
        assert_eq!(saved["nav"]["allow_teleports"], true);
        assert_eq!(saved["nav"]["allow_wilderness"], true);
        assert_eq!(saved["nav"]["allow_bank_fetch"], true);
        assert_eq!(saved["nav"]["allow_danger_zones"], true);
        assert_eq!(saved["nav"]["survivable_routing"], true);
        assert_eq!(saved["nav"]["script_scope_notice_ack"], true);
        assert_eq!(saved["nav"]["survivable_routing_notice_ack"], true);

        let (missing_dir, missing) = scratch("legacy");
        vault::write_private_file(&missing.path(), br#"{"last_focus":"bob","nav":{}}"#).unwrap();
        missing.reload().unwrap();
        assert!(nav_preference(&missing, NavPreference::PauseScriptOnManualWalkAbort).unwrap());
        assert!(
            nav_preference(&missing, NavPreference::SurvivableRouting).unwrap(),
            "an old file without the key migrates to the survivable middle level"
        );
        for preference in [
            NavPreference::AllowTeleports,
            NavPreference::AllowWilderness,
            NavPreference::AllowBankFetch,
            NavPreference::AllowDangerZones,
            NavPreference::ScriptScopeNoticeAck,
            NavPreference::SurvivableRoutingNoticeAck,
        ] {
            assert!(!nav_preference(&missing, preference).unwrap());
        }
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
    fn survivable_preference_fails_closed_for_malformed_files() {
        let (dir, prefs) = scratch("malformed-survivable");
        vault::write_private_file(&prefs.path(), b"{malformed").unwrap();
        assert!(!nav_preference(&prefs, NavPreference::SurvivableRouting).unwrap());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
