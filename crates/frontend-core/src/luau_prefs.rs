//! The opt-in experimental Luau script-runtime preference shared by panel and TUI.
//!
//! Runtime dispatch still owns the final admission check: a saved selection
//! must not run unless this durable setting is on.

use host_play::local_state::Update;

use crate::prefs::{PreferenceError, PreferencePatch, UiPreferences};

/// The global settings-row label.
pub const LUAU_RUNTIME_LABEL: &str = "Use experimental Luau script runtime";
/// The global settings-row help line.
pub const LUAU_RUNTIME_NOTE: &str =
    "Shows Luau scripts in the script picker. Experimental: scripts may not behave as expected.";
/// The first-enable confirmation title.
pub const LUAU_RUNTIME_WARNING_TITLE: &str = "Experimental Luau scripts";
/// The approved first-enable warning text, excluding its Enable/Cancel buttons.
pub const LUAU_RUNTIME_FIRST_ENABLE_WARNING: &str = "Luau script support is experimental. It may not behave as expected, and a later release can change it.\n\nA Luau script can use only the 274bot script API. It cannot read your files, use the network or start programs. 274bot stops a script that uses too much memory or does not yield, and the card shows the reason.\n\nA script controls your character. Run only scripts that you wrote or that you trust.";
/// The disable confirmation title.
pub const LUAU_RUNTIME_DISABLE_TITLE: &str = "Disable experimental Luau scripts?";
/// The approved disable confirmation text.
pub const LUAU_RUNTIME_DISABLE_CONFIRMATION: &str = "This will hide all of your imported Luau scripts from the card picker. Nothing will be deleted. Continue?";
/// The refusal shown when a saved Luau selection is not admitted.
pub const LUAU_START_REFUSAL: &str = "Luau scripts are turned off. Turn on 'Use experimental Luau script runtime' in Settings to run this script.";

const LUAU_RUNTIME_KEY: &str = "experimental_luau_runtime";
pub(crate) const LUAU_RUNTIME_NOTICE_ACK_KEY: &str = "experimental_luau_runtime_notice_ack";

/// The durable runtime switch. An absent or malformed member is off.
pub(crate) fn luau_runtime_enabled(
    preferences: &UiPreferences,
) -> Result<bool, PreferenceError> {
    Ok(preferences
        .document()?
        .get(LUAU_RUNTIME_KEY)
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false))
}

/// Whether the one-time first-enable notice has been durably acknowledged.
pub(crate) fn luau_runtime_notice_acknowledged(
    preferences: &UiPreferences,
) -> Result<bool, PreferenceError> {
    Ok(preferences
        .document()?
        .get(LUAU_RUNTIME_NOTICE_ACK_KEY)
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false))
}

/// Persist only the shared runtime switch.
pub(crate) fn write_luau_runtime(
    preferences: &UiPreferences,
    enabled: bool,
) -> Result<Update, PreferenceError> {
    preferences.begin_patch(PreferencePatch::member(
        &[LUAU_RUNTIME_KEY],
        Some(serde_json::Value::Bool(enabled)),
    )?)
}

/// Fail closed while the saved value is absent, unreadable or off.
pub(crate) const fn luau_start_refusal(enabled: Option<bool>) -> Option<&'static str> {
    if matches!(enabled, Some(true)) {
        None
    } else {
        Some(LUAU_START_REFUSAL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    fn scratch(label: &str) -> (PathBuf, UiPreferences) {
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "274bot-frontend-luau-prefs-{}-{label}-{id}",
            std::process::id()
        ));
        vault::create_private_dir(&dir).unwrap();
        vault::write_private_file(
            &dir.join("panel-ui.json"),
            br#"{"last_focus":"alice","nav":{"allow_teleports":true}}"#,
        )
        .unwrap();
        let preferences = UiPreferences::open_for_test(dir.clone());
        preferences.reload().unwrap();
        (dir, preferences)
    }

    #[test]
    fn defaults_off_and_saves_only_the_runtime_setting() {
        let _audit = crate::pvp_prefs::TEST_AUDIT_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (dir, preferences) = scratch("roundtrip");
        assert_eq!(luau_runtime_enabled(&preferences).ok(), Some(false));
        assert_eq!(
            luau_runtime_notice_acknowledged(&preferences).ok(),
            Some(false)
        );

        crate::prefs::settled_for_test(
            preferences.begin_set(LUAU_RUNTIME_NOTICE_ACK_KEY, true.into()),
        )
        .unwrap();
        assert_eq!(
            luau_runtime_notice_acknowledged(&preferences).ok(),
            Some(true)
        );
        assert_eq!(luau_runtime_enabled(&preferences).ok(), Some(false));

        crate::prefs::settled_for_test(write_luau_runtime(&preferences, true)).unwrap();
        assert_eq!(luau_runtime_enabled(&preferences).ok(), Some(true));
        crate::prefs::settled_for_test(write_luau_runtime(&preferences, false)).unwrap();
        let saved = preferences.reload().unwrap();
        assert_eq!(saved["experimental_luau_runtime"], false);
        assert_eq!(saved[LUAU_RUNTIME_NOTICE_ACK_KEY], true);
        assert_eq!(saved["last_focus"], "alice");
        assert_eq!(saved["nav"]["allow_teleports"], true);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn luau_start_admission_fails_closed() {
        assert_eq!(luau_start_refusal(None), Some(LUAU_START_REFUSAL));
        assert_eq!(luau_start_refusal(Some(false)), Some(LUAU_START_REFUSAL));
        assert_eq!(luau_start_refusal(Some(true)), None);
    }
}
