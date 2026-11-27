//! Experimental Luau activation in the existing shared preferences document.

use crate::prefs::{PreferenceError, PreferencePatch, UiPreferences, Update};

/// The setting label shown by both frontends.
pub const LABEL: &str = "Use experimental Luau script runtime";
/// The setting's explanation.
pub const HELP: &str =
    "Shows Luau scripts in the script picker. Experimental: scripts may not behave as expected.";
/// The first-enable warning title.
pub const WARNING_TITLE: &str = "Experimental Luau scripts";
/// The operator-approved first-enable warning.
pub const WARNING_BODY: &str = "Luau script support is experimental. It may not behave as expected, and a later release can change it.\n\nA Luau script can use only the 274bot script API. It cannot read your files, use the network or start programs. 274bot stops a script that uses too much memory or does not yield, and the card shows the reason.\n\nA script controls your character. Run only scripts that you wrote or that you trust.";
/// The warning shown before disabling an active setting.
pub const DISABLE_BODY: &str = "This will hide all of your imported Luau scripts from the card picker. Nothing will be deleted. Continue?";
/// A saved selection's refusal while Luau is off.
pub const DISABLED_REFUSAL: &str = "Luau scripts are turned off. Turn on 'Use experimental Luau script runtime' in Settings to run this script.";
/// A saved selection's refusal when this build has no Luau runtime.
pub const UNAVAILABLE_REFUSAL: &str = "This build does not include the Luau script runtime.";
/// The persisted switch, absent by default.
pub const ENABLED_KEY: &str = "luau_runtime_enabled";
/// The stored acknowledgement of the first-enable warning.
pub const ACK_KEY: &str = "luau_runtime_ack";

/// Whether the warning has a durable acknowledgement.
pub fn acknowledged(preferences: &UiPreferences) -> bool {
    preferences.document().is_ok_and(|document| {
        document.get(ACK_KEY).and_then(serde_json::Value::as_bool) == Some(true)
    })
}

/// Whether Luau may load files or start a run. Cold, malformed and unavailable
/// preferences fail closed.
pub fn enabled(preferences: &UiPreferences) -> bool {
    cfg!(feature = "luau")
        && preferences.document().is_ok_and(|document| {
            document.get(ACK_KEY).and_then(serde_json::Value::as_bool) == Some(true)
                && document
                    .get(ENABLED_KEY)
                    .and_then(serde_json::Value::as_bool)
                    == Some(true)
        })
}

pub(crate) fn write_enabled(
    preferences: &UiPreferences,
    enabled: bool,
) -> Result<Update, PreferenceError> {
    preferences.begin_patch(PreferencePatch::set(ENABLED_KEY, enabled.into())?)
}

pub(crate) fn write_acknowledgement(
    preferences: &UiPreferences,
) -> Result<Update, PreferenceError> {
    let mut patch = PreferencePatch::set(ACK_KEY, true.into())?;
    patch.set_member(&[ENABLED_KEY], Some(true.into()))?;
    preferences.begin_patch(patch)
}

/// A confirmation that has no effect until its command is durable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LuauPrompt {
    /// The operator has not acknowledged the experimental warning.
    FirstEnable,
    /// Hiding imported cards requires confirmation.
    Disable,
}

/// The command a frontend submits after the shared view accepts a decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LuauAction {
    /// Enable a previously acknowledged runtime.
    Enable,
    /// Persist the first warning acknowledgement and enable atomically.
    AcknowledgeAndEnable,
    /// Hide cards without removing references or settings.
    Disable,
}

/// Shared settings-row and confirmation state. The value always follows
/// durable preferences, never an uncommitted click.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LuauSettingView {
    enabled: bool,
    acknowledged: bool,
    prompt: Option<LuauPrompt>,
}

impl LuauSettingView {
    /// Whether this build contains the runtime.
    pub const fn available(&self) -> bool {
        cfg!(feature = "luau")
    }

    /// The durable enabled value.
    pub const fn enabled(&self) -> bool {
        self.enabled
    }

    /// The durable warning acknowledgement.
    pub const fn acknowledged(&self) -> bool {
        self.acknowledged
    }

    /// The pending confirmation, including a confirmed save still pending.
    pub const fn prompt(&self) -> Option<LuauPrompt> {
        self.prompt
    }

    /// Adopt the latest durable value without submitting any write.
    pub fn follow(&mut self, preferences: &UiPreferences) {
        self.enabled = enabled(preferences);
        self.acknowledged = acknowledged(preferences);
        if matches!(
            (self.prompt, self.enabled),
            (Some(LuauPrompt::FirstEnable), true) | (Some(LuauPrompt::Disable), false)
        ) {
            self.prompt = None;
        }
    }

    /// Request a switch change. A first enable or disable opens its warning.
    pub fn request(&mut self, enabled: bool) -> Option<LuauAction> {
        if !self.available() || enabled == self.enabled {
            return None;
        }
        if enabled && self.acknowledged {
            return Some(LuauAction::Enable);
        }
        self.prompt = Some(if enabled {
            LuauPrompt::FirstEnable
        } else {
            LuauPrompt::Disable
        });
        None
    }

    /// Confirm the warning. Keep it visible until its write is durable.
    pub fn confirm(&mut self) -> Option<LuauAction> {
        match self.prompt? {
            LuauPrompt::FirstEnable => Some(LuauAction::AcknowledgeAndEnable),
            LuauPrompt::Disable => Some(LuauAction::Disable),
        }
    }

    /// Cancel a warning without changing durable state.
    pub fn cancel(&mut self) {
        self.prompt = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "luau")]
    use crate::prefs::settled_for_test;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn preferences(label: &str) -> (std::path::PathBuf, UiPreferences) {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "274bot-luau-prefs-{}-{label}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        vault::create_private_dir(&root).unwrap();
        let prefs = UiPreferences::open_for_test(root.clone());
        prefs.reload().unwrap();
        (root, prefs)
    }

    #[cfg(feature = "luau")]
    #[test]
    fn activation_requires_confirmation_and_preserves_imports_when_disabled() {
        let (root, prefs) = preferences("activation");
        prefs
            .set("luau_imports", serde_json::json!(["scripts/alice.luau"]))
            .unwrap();
        prefs
            .set(
                "script_settings",
                serde_json::json!({"alice": {"count": 7}}),
            )
            .unwrap();
        let before = prefs.document().unwrap();
        let mut view = LuauSettingView::default();
        view.follow(&prefs);
        assert!(!view.enabled());
        assert_eq!(view.request(true), None);
        assert_eq!(view.prompt(), Some(LuauPrompt::FirstEnable));
        view.cancel();
        assert_eq!(view.prompt(), None);
        assert!(!acknowledged(&prefs));
        assert_eq!(prefs.document().unwrap(), before);

        view.request(true);
        assert_eq!(view.confirm(), Some(LuauAction::AcknowledgeAndEnable));
        assert!(
            !view.enabled(),
            "confirmation does not anticipate durability"
        );
        settled_for_test(write_acknowledgement(&prefs)).unwrap();
        view.follow(&prefs);
        assert!(view.enabled());
        assert_eq!(view.prompt(), None);
        assert!(acknowledged(&prefs));
        assert_eq!(view.request(false), None);
        assert_eq!(view.prompt(), Some(LuauPrompt::Disable));
        view.cancel();
        assert!(view.enabled());
        view.request(false);
        assert_eq!(view.confirm(), Some(LuauAction::Disable));
        settled_for_test(write_enabled(&prefs, false)).unwrap();
        view.follow(&prefs);
        assert!(!view.enabled());
        assert!(acknowledged(&prefs));
        let after = prefs.reload().unwrap();
        assert_eq!(after["luau_imports"], before["luau_imports"]);
        assert_eq!(after["script_settings"], before["script_settings"]);
        assert_eq!(view.request(true), Some(LuauAction::Enable));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_or_incomplete_opt_in_never_enables_the_runtime() {
        let (root, prefs) = preferences("fail-closed");
        assert!(!enabled(&prefs));
        prefs.set(ENABLED_KEY, true.into()).unwrap();
        assert!(!enabled(&prefs), "the warning must also be acknowledged");
        prefs.set(ACK_KEY, "true".into()).unwrap();
        assert!(!enabled(&prefs), "malformed acknowledgement is not consent");
        prefs.set(ACK_KEY, true.into()).unwrap();
        assert_eq!(enabled(&prefs), cfg!(feature = "luau"));
        let mut view = LuauSettingView::default();
        view.follow(&prefs);
        assert_eq!(view.enabled(), cfg!(feature = "luau"));
        if !cfg!(feature = "luau") {
            assert_eq!(view.request(true), None);
            assert_eq!(view.confirm(), None);
            assert_eq!(view.prompt(), None);
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
