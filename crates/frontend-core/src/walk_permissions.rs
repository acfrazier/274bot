//! Durable walk-global permissions shared by the panel and TUI.
//!
//! Frontends render and admit manual walks from the same projection. Reads
//! fail closed; writes merge only values changed by the caller into the
//! shared `panel-ui.json` transaction.

use std::io;
use std::path::Path;

use host_play::WalkGlobals;
use nav::router::FindOptions;

/// Shared explanation of how the Nav config walk grants apply to scripts.
pub const SCRIPT_SCOPE_NOTICE: &str = "Teleports and wilderness in Nav config now apply to every walk, including scripts. Bank fetch still applies only to manual WalkTo. Danger is new (default off). rs2b0t-compatible scripts always allow wilderness and bank fetch.";

/// Warning shown in place of the one-shot danger control when its global is on.
pub const GLOBAL_DANGER_WARNING: &str = "Global danger-zone override is enabled.";

/// Label for the single-admission danger-zone control.
pub const DANGER_THIS_WALK_LABEL: &str = "Route through danger zones";
/// Shared scope copy for teleports, wilderness, and danger grants.
pub const GLOBAL_PERMISSION_SCOPE: &str = "Global — applies to every walk.";
/// Bank-budget fetching is available to manual WalkTo only.
pub const BANK_FETCH_PERMISSION_SCOPE: &str = "Manual WalkTo only.";

/// Shared labels for the four durable walk permissions: preference id, label.
pub const GLOBAL_PERMISSION_LABELS: [(&str, &str); 4] = [
    ("allow_teleports", "allow teleports"),
    ("allow_wilderness", "allow wilderness"),
    ("allow_bank_fetch", "allow bank fetch"),
    ("allow_danger_zones", DANGER_THIS_WALK_LABEL),
];

/// One durable read of all global walk permissions and the shared notice ack.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WalkGlobalsView {
    pub globals: WalkGlobals,
    pub script_scope_notice_ack: bool,
}

impl WalkGlobalsView {
    /// Read the `nav` object once. Missing or malformed grant data grants
    /// nothing; a missing or malformed notice acknowledgement remains off.
    pub fn read_at(path: &Path) -> Self {
        let Some(nav) = host_play::panel_ui_value_at(path, "nav") else {
            return Self::default();
        };
        let script_scope_notice_ack = nav
            .get("script_scope_notice_ack")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let globals = serde_json::from_value(nav).unwrap_or_default();
        Self {
            globals,
            script_scope_notice_ack,
        }
    }

    /// Manual WalkTo has no script grant; a danger opt-in applies to one walk.
    pub fn manual_options(self, danger_this_walk: bool) -> FindOptions {
        self.globals
            .manual_options(self.danger_this_walk(danger_this_walk))
    }

    /// A global danger grant supersedes, and therefore consumes, the one-shot.
    pub fn danger_this_walk(self, requested: bool) -> bool {
        requested && !self.globals.allow_danger_zones
    }

    /// Resolve the three script permission ids inherited from global grants.
    /// Bank fetch is intentionally manual-WalkTo-only and has no script id.
    pub fn permission_enabled(self, id: &str) -> Option<bool> {
        match id {
            "allow_teleports" | "allowTeleports" => Some(self.globals.allow_teleports),
            "allow_wilderness" | "allowWilderness" => Some(self.globals.allow_wilderness),
            "allow_danger_zones" | "allowDangerZones" => Some(self.globals.allow_danger_zones),
            _ => None,
        }
    }

    /// Persist only globals and notice acknowledgement changed by this edit,
    /// merging them together under the shared panel-preferences transaction.
    pub fn persist_changed_at(path: &Path, before: Self, after: Self) -> io::Result<()> {
        let changed = [
            (
                "allow_teleports",
                before.globals.allow_teleports,
                after.globals.allow_teleports,
            ),
            (
                "allow_wilderness",
                before.globals.allow_wilderness,
                after.globals.allow_wilderness,
            ),
            (
                "allow_bank_fetch",
                before.globals.allow_bank_fetch,
                after.globals.allow_bank_fetch,
            ),
            (
                "allow_danger_zones",
                before.globals.allow_danger_zones,
                after.globals.allow_danger_zones,
            ),
            (
                "script_scope_notice_ack",
                before.script_scope_notice_ack,
                after.script_scope_notice_ack,
            ),
        ];
        if changed.iter().all(|(_, before, after)| before == after) {
            return Ok(());
        }

        host_play::update_panel_ui_at(path, |document, _, _| {
            let root = document.as_object_mut().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "panel-ui.json is not an object")
            })?;
            let nav = root.entry("nav").or_insert_with(|| serde_json::json!({}));
            if !nav.is_object() {
                *nav = serde_json::json!({});
            }
            let nav = nav
                .as_object_mut()
                .expect("a replacement nav value is always an object");
            for (key, before, after) in changed {
                if before != after {
                    nav.insert(key.into(), serde_json::Value::Bool(after));
                }
            }
            Ok(None)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::WalkGlobalsView;
    use host_play::WalkGlobals;
    use nav::zones::ZoneExempt;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct TempStore {
        dir: PathBuf,
        path: PathBuf,
    }

    impl TempStore {
        fn new(label: &str) -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let dir = std::env::temp_dir().join(format!(
                "frontend-core-walk-permissions-{label}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Self {
                path: dir.join("panel-ui.json"),
                dir,
            }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempStore {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn projection_reads_globals_and_fails_closed_for_missing_or_corrupt_files() {
        let store = TempStore::new("read");
        assert_eq!(
            WalkGlobalsView::read_at(store.path()),
            WalkGlobalsView::default()
        );

        std::fs::write(
            store.path(),
            r#"{"nav":{"allow_teleports":true,"allow_wilderness":true,"allow_bank_fetch":true,"allow_danger_zones":true,"script_scope_notice_ack":true}}"#,
        )
        .unwrap();
        let view = WalkGlobalsView::read_at(store.path());
        assert_eq!(
            view.globals,
            WalkGlobals {
                allow_teleports: true,
                allow_wilderness: true,
                allow_bank_fetch: true,
                allow_danger_zones: true,
            }
        );
        assert!(view.script_scope_notice_ack);
        assert_eq!(view.permission_enabled("allow_teleports"), Some(true));
        assert_eq!(view.permission_enabled("allowTeleports"), Some(true));
        assert_eq!(view.permission_enabled("allow_wilderness"), Some(true));
        assert_eq!(view.permission_enabled("allowWilderness"), Some(true));
        assert_eq!(view.permission_enabled("allow_danger_zones"), Some(true));
        assert_eq!(view.permission_enabled("allowDangerZones"), Some(true));
        assert_eq!(view.permission_enabled("allow_bank_fetch"), None);
        assert_eq!(view.permission_enabled("allowBankFetch"), None);
        assert_eq!(view.permission_enabled("unrelated"), None);

        std::fs::write(store.path(), b"{truncated").unwrap();
        assert_eq!(
            WalkGlobalsView::read_at(store.path()),
            WalkGlobalsView::default()
        );
    }

    #[test]
    fn manual_danger_uses_the_global_or_one_shot_but_global_consumes_one_shot() {
        let store = TempStore::new("manual");
        std::fs::write(
            store.path(),
            r#"{"nav":{"allow_teleports":true,"allow_wilderness":true,"allow_bank_fetch":true,"allow_danger_zones":false}}"#,
        )
        .unwrap();
        let view = WalkGlobalsView::read_at(store.path());
        assert!(view.danger_this_walk(true));
        let once = view.manual_options(view.danger_this_walk(true));
        assert!(once.allow_teleports);
        assert!(once.allow_wilderness);
        assert!(once.allow_bank_fetch);
        assert_eq!(once.zones, ZoneExempt::all());
        assert!(!view
            .manual_options(view.danger_this_walk(false))
            .zones
            .is_all());

        std::fs::write(store.path(), r#"{"nav":{"allow_danger_zones":true}}"#).unwrap();
        let global = WalkGlobalsView::read_at(store.path());
        assert!(!global.danger_this_walk(true));
        assert_eq!(
            global.manual_options(global.danger_this_walk(true)).zones,
            ZoneExempt::all()
        );
    }

    #[test]
    fn changed_projection_fields_merge_in_one_durable_update() {
        let store = TempStore::new("write");
        std::fs::write(
            store.path(),
            r#"{"unrelated":{"keep":true},"nav":{"show_nav_path":true,"allow_wilderness":false,"custom":"keep"}}"#,
        )
        .unwrap();
        let before = WalkGlobalsView::read_at(store.path());
        let mut external =
            serde_json::from_slice::<serde_json::Value>(&std::fs::read(store.path()).unwrap())
                .unwrap();
        external["nav"]["allow_wilderness"] = serde_json::Value::Bool(true);
        external["nav"]["show_nav_path"] = serde_json::Value::Bool(false);
        std::fs::write(store.path(), serde_json::to_vec(&external).unwrap()).unwrap();

        let after = WalkGlobalsView {
            globals: WalkGlobals {
                allow_teleports: true,
                ..before.globals
            },
            script_scope_notice_ack: true,
        };
        WalkGlobalsView::persist_changed_at(store.path(), before, after).unwrap();
        let persisted =
            serde_json::from_slice::<serde_json::Value>(&std::fs::read(store.path()).unwrap())
                .unwrap();
        assert_eq!(persisted["unrelated"]["keep"], true);
        assert_eq!(persisted["nav"]["custom"], "keep");
        assert_eq!(persisted["nav"]["allow_teleports"], true);
        assert_eq!(persisted["nav"]["allow_wilderness"], true);
        assert!(persisted["nav"].get("allow_bank_fetch").is_none());
        assert!(persisted["nav"].get("allow_danger_zones").is_none());
        assert_eq!(persisted["nav"]["script_scope_notice_ack"], true);
        assert_eq!(persisted["nav"]["show_nav_path"], false);

        crate::nav_preference_at(
            store.path(),
            crate::NavPreference::AllowTeleports,
            Some(false),
        )
        .unwrap();
        WalkGlobalsView::persist_changed_at(store.path(), after, after).unwrap();
        let after_noop = WalkGlobalsView::read_at(store.path());
        assert!(!after_noop.globals.allow_teleports);
    }
}
