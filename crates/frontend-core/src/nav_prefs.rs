use std::io;
use std::path::Path;

/// A boolean inside the shared `panel-ui.json` `nav` object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavPreference {
    /// Whether the TUI map paints the content-defined special areas.
    ShowSpecialAreas,
    /// Whether qualifying manual movement pauses the owning script.
    PauseScriptOnManualWalkAbort,
}

impl NavPreference {
    const fn key_and_default(self) -> (&'static str, bool) {
        match self {
            Self::ShowSpecialAreas => ("show_special_areas", false),
            Self::PauseScriptOnManualWalkAbort => ("pause_script_on_manual_walk_abort", true),
        }
    }
}

/// Read or update one shared nested nav boolean. The read preserves legacy
/// defaults; the write preserves every other `nav` and root preference.
pub fn nav_preference_at(
    path: &Path,
    preference: NavPreference,
    update: Option<bool>,
) -> io::Result<bool> {
    let (key, default) = preference.key_and_default();
    if let Some(enabled) = update {
        let mut nav = host_play::panel_ui_value_at(path, "nav")
            .filter(|value| value.is_object())
            .unwrap_or_else(|| serde_json::json!({}));
        nav.as_object_mut()
            .expect("a replacement nav object is always an object")
            .insert(key.into(), serde_json::Value::Bool(enabled));
        host_play::persist_panel_ui_value_at(path, "nav", nav)?;
        Ok(enabled)
    } else {
        Ok(host_play::panel_ui_value_at(path, "nav")
            .and_then(|value| value.get(key).and_then(serde_json::Value::as_bool))
            .unwrap_or(default))
    }
}

#[cfg(test)]
mod tests {
    use super::{nav_preference_at, NavPreference};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn temp_path(label: &str) -> PathBuf {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "274bot-frontend-nav-prefs-{}-{label}-{id}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("panel-ui.json")
    }

    #[test]
    fn legacy_and_default_values_and_cross_surface_writes_preserve_other_preferences() {
        let path = temp_path("roundtrip");
        std::fs::write(
            &path,
            r#"{"last_focus":"alice","capture":false,"nav":{"show_special_areas":false,"custom":{"keep":7}}}"#,
        )
        .unwrap();

        assert!(!nav_preference_at(&path, NavPreference::ShowSpecialAreas, None).unwrap());
        assert!(
            nav_preference_at(&path, NavPreference::PauseScriptOnManualWalkAbort, None).unwrap()
        );

        nav_preference_at(&path, NavPreference::ShowSpecialAreas, Some(true)).unwrap();
        nav_preference_at(
            &path,
            NavPreference::PauseScriptOnManualWalkAbort,
            Some(false),
        )
        .unwrap();

        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved["last_focus"], "alice");
        assert_eq!(saved["capture"], false);
        assert_eq!(saved["nav"]["custom"]["keep"], 7);
        assert_eq!(saved["nav"]["show_special_areas"], true);
        assert_eq!(saved["nav"]["pause_script_on_manual_walk_abort"], false);

        let missing = temp_path("legacy");
        std::fs::write(&missing, r#"{"last_focus":"bob","nav":{}}"#).unwrap();
        assert!(
            nav_preference_at(&missing, NavPreference::PauseScriptOnManualWalkAbort, None).unwrap()
        );
        assert!(!nav_preference_at(&missing, NavPreference::ShowSpecialAreas, None).unwrap());
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
        std::fs::remove_dir_all(missing.parent().unwrap()).unwrap();
    }

    #[test]
    fn failed_nav_preference_write_is_returned_to_the_frontend() {
        let root_file = temp_path("blocked-parent");
        let root = root_file.parent().unwrap().to_path_buf();
        let blocked = root.join("blocked");
        std::fs::write(&blocked, b"not a directory").unwrap();
        let path = blocked.join("panel-ui.json");
        nav_preference_at(
            &path,
            NavPreference::PauseScriptOnManualWalkAbort,
            Some(false),
        )
        .expect_err("the preference writer must report an inaccessible destination");
        assert_eq!(std::fs::read(&blocked).unwrap(), b"not a directory");
        std::fs::remove_dir_all(root).unwrap();
    }
}
