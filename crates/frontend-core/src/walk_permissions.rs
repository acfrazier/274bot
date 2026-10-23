//! Durable walk-global permissions shared by the panel and TUI.
//!
//! Frontends render and admit manual walks from the same projection. Reads
//! fail closed; writes merge only values changed by the caller into the
//! shared preferences, through the process local-state service.

use std::io;

use host_play::WalkGlobals as HostWalkGlobals;
use nav_planner::router::FindOptions;

/// Three durable danger-route policies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DangerLevel {
    Never,
    WhenSurvivable,
    Always,
}

impl DangerLevel {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Never => "Never",
            Self::WhenSurvivable => "When survivable",
            Self::Always => "Always",
        }
    }

    pub const fn next(self) -> Self {
        match self {
            Self::Never => Self::WhenSurvivable,
            Self::WhenSurvivable => Self::Always,
            Self::Always => Self::Never,
        }
    }
}

/// Durable movement grants shared by the panel and TUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct WalkGlobals {
    allow_teleports: bool,
    allow_wilderness: bool,
    allow_bank_fetch: bool,
    allow_danger_zones: bool,
    survivable_routing: bool,
}

impl Default for WalkGlobals {
    fn default() -> Self {
        Self {
            allow_teleports: false,
            allow_wilderness: false,
            allow_bank_fetch: false,
            allow_danger_zones: false,
            survivable_routing: true,
        }
    }
}

impl WalkGlobals {
    pub const fn new(
        allow_teleports: bool,
        allow_wilderness: bool,
        allow_bank_fetch: bool,
        allow_danger_zones: bool,
        survivable_routing: bool,
    ) -> Self {
        Self {
            allow_teleports,
            allow_wilderness,
            allow_bank_fetch,
            allow_danger_zones,
            survivable_routing,
        }
    }

    pub(crate) fn from_host(globals: HostWalkGlobals) -> Self {
        Self::new(
            globals.allow_teleports,
            globals.allow_wilderness,
            globals.allow_bank_fetch,
            globals.allow_danger_zones,
            globals.survivable_routing,
        )
    }

    pub(crate) fn to_host(self) -> HostWalkGlobals {
        HostWalkGlobals {
            allow_teleports: self.allow_teleports,
            allow_wilderness: self.allow_wilderness,
            allow_bank_fetch: self.allow_bank_fetch,
            allow_danger_zones: self.allow_danger_zones,
            survivable_routing: self.survivable_routing,
        }
    }

    pub const fn allow_teleports(self) -> bool {
        self.allow_teleports
    }
    pub const fn allow_wilderness(self) -> bool {
        self.allow_wilderness
    }
    pub const fn allow_bank_fetch(self) -> bool {
        self.allow_bank_fetch
    }
    pub const fn allow_danger_zones(self) -> bool {
        self.allow_danger_zones
    }
    pub const fn survivable_routing(self) -> bool {
        self.survivable_routing
    }

    pub const fn fail_closed() -> Self {
        Self {
            allow_teleports: false,
            allow_wilderness: false,
            allow_bank_fetch: false,
            allow_danger_zones: false,
            survivable_routing: false,
        }
    }

    pub const fn danger_level(self) -> DangerLevel {
        if self.allow_danger_zones {
            DangerLevel::Always
        } else if self.survivable_routing {
            DangerLevel::WhenSurvivable
        } else {
            DangerLevel::Never
        }
    }

    pub fn effective_danger_level(self) -> DangerLevel {
        match self.danger_level() {
            DangerLevel::WhenSurvivable if !host_play::NET_AVAILABLE => DangerLevel::Never,
            level => level,
        }
    }

    /// These globals at `level` (the one mapping of a danger level onto the
    /// two stored booleans).
    pub const fn with_danger_level(self, level: DangerLevel) -> Self {
        let (allow_danger_zones, survivable_routing) = match level {
            DangerLevel::Never => (false, false),
            DangerLevel::WhenSurvivable => (false, true),
            DangerLevel::Always => (true, false),
        };
        Self {
            allow_danger_zones,
            survivable_routing,
            ..self
        }
    }

    pub(crate) fn set_danger_level(&mut self, level: DangerLevel) {
        *self = self.with_danger_level(level);
    }

    pub fn manual_options(self, danger_this_walk: bool) -> FindOptions {
        self.to_host().manual_options(danger_this_walk)
    }
}

/// Shared explanation of how the Nav config walk grants apply to scripts.
pub const SCRIPT_SCOPE_NOTICE: &str = "Teleports and wilderness in Nav config now apply to every walk, including scripts. Bank fetch still applies only to manual WalkTo. rs2b0t-compatible scripts always allow wilderness and bank fetch.";

/// The explanatory copy for the `When survivable` danger-routing level.
pub const SURVIVABLE_ROUTING_TOOLTIP: &str = "The host applies the active survivability policy to danger routes. Scripts and the map control can still override.";

/// Explanatory copy while the middle level is held behind the unavailable
/// runtime net.
pub const SURVIVABLE_ROUTING_HELD_TOOLTIP: &str = "Stored 'When survivable' is not available yet. Effective level: Never; this setting does not change route admission. Global, one-walk, and script overrides still apply.";

/// Return only copy that matches the currently active runtime gate.
pub const fn survivable_routing_tooltip() -> &'static str {
    if host_play::NET_AVAILABLE {
        SURVIVABLE_ROUTING_TOOLTIP
    } else {
        SURVIVABLE_ROUTING_HELD_TOOLTIP
    }
}

/// The short migration notice once survivable routing is active.
pub const SURVIVABLE_ROUTING_NOTICE: &str =
    "Danger routing now has three levels and defaults to 'When survivable'.";

/// Migration notice while the middle level is held behind the runtime net.
pub const SURVIVABLE_ROUTING_HELD_NOTICE: &str = "Danger routing now has three levels and defaults to 'When survivable'. The stored 'When survivable' setting is not available yet; its effective level is Never and it does not change route admission. Global, one-walk, and script overrides still apply. rs2b0t-compatible scripts keep rs2b0t's own behaviour.";

/// Return a migration notice that reflects whether the held level is usable.
pub const fn survivable_routing_notice() -> &'static str {
    if host_play::NET_AVAILABLE {
        SURVIVABLE_ROUTING_NOTICE
    } else {
        SURVIVABLE_ROUTING_HELD_NOTICE
    }
}

/// Warning shown for the global Always override and in place of the one-shot control.
pub const GLOBAL_DANGER_WARNING: &str = "Global danger-zone override is enabled.";

/// Label for the single-admission danger-zone control.
pub const DANGER_THIS_WALK_LABEL: &str = "Route through danger zones";
/// One note above the routing grants, replacing the per-row scope suffixes.
pub const ROUTING_SCOPE_NOTE: &str = "Settings are global except where otherwise specified. rs2b0t-compatible scripts always allow wilderness and bank fetch.";
/// Bank-budget fetching is available to manual WalkTo only.
pub const BANK_FETCH_PERMISSION_SCOPE: &str = "Manual WalkTo only.";

/// Label for the stored middle level while the runtime net is unavailable.
pub const WHEN_SURVIVABLE_HELD_LABEL: &str = "When survivable (not available yet: acts as Never)";

/// Note under the Danger routing drop-down while the middle level is held.
pub const WHEN_SURVIVABLE_HELD_NOTE: &str = "Not available yet: acts as Never.";
/// Danger-routing levels in the order the drop-down lists them.
pub const DANGER_ROUTING_LEVELS: [DangerLevel; 3] = [
    DangerLevel::Never,
    DangerLevel::WhenSurvivable,
    DangerLevel::Always,
];

/// True while the stored middle level cannot admit routes because the runtime net is unavailable.
pub const fn danger_routing_held(level: DangerLevel) -> bool {
    matches!(level, DangerLevel::WhenSurvivable) && !host_play::NET_AVAILABLE
}

/// Display the saved danger level, identifying the gated middle level without
/// changing the canonical preference value or its cycling order.
pub const fn danger_routing_label(level: DangerLevel) -> &'static str {
    match level {
        DangerLevel::WhenSurvivable if !host_play::NET_AVAILABLE => WHEN_SURVIVABLE_HELD_LABEL,
        level => level.label(),
    }
}

/// The panel and TUI follow gates share this decision: either the ordinary
/// hold or host movement ownership freezes the armed manual walk.
pub const fn manual_walk_may_follow(hold: bool, host_move_owned: bool) -> bool {
    !hold && !host_move_owned
}

/// Shared labels for the durable walk settings: preference id, label.
pub const GLOBAL_PERMISSION_LABELS: [(&str, &str); 4] = [
    ("allow_teleports", "allow teleports"),
    ("allow_wilderness", "allow wilderness"),
    ("allow_bank_fetch", "allow bank fetch"),
    ("allow_danger_zones", "Danger routing"),
];

/// One durable read of all global walk permissions and their notice acknowledgements.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WalkGlobalsView {
    globals: WalkGlobals,
    script_scope_notice_ack: bool,
    survivable_routing_notice_ack: bool,
}

impl WalkGlobalsView {
    pub const fn new(
        globals: WalkGlobals,
        script_scope_notice_ack: bool,
        survivable_routing_notice_ack: bool,
    ) -> Self {
        Self {
            globals,
            script_scope_notice_ack,
            survivable_routing_notice_ack,
        }
    }

    pub const fn globals(self) -> WalkGlobals {
        self.globals
    }
    pub const fn script_scope_notice_ack(self) -> bool {
        self.script_scope_notice_ack
    }
    pub const fn survivable_routing_notice_ack(self) -> bool {
        self.survivable_routing_notice_ack
    }

    /// Read the shared preferences once. A missing file is a first-run
    /// default; a refused file or a malformed `nav` denies every grant.
    pub fn read(preferences: &crate::prefs::UiPreferences) -> Self {
        let Ok(document) = preferences.document() else {
            return Self::fail_closed();
        };
        let Some(nav) = document.get("nav") else {
            return Self::default();
        };
        let Some(nav_object) = nav.as_object() else {
            return Self::fail_closed();
        };
        let globals = match serde_json::from_value::<HostWalkGlobals>(serde_json::Value::Object(
            nav_object.clone(),
        )) {
            Ok(globals) => WalkGlobals::from_host(globals),
            Err(_) => return Self::fail_closed(),
        };
        Self::new(
            globals,
            nav.get("script_scope_notice_ack")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
            nav.get("survivable_routing_notice_ack")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
        )
    }

    fn fail_closed() -> Self {
        Self::new(WalkGlobals::fail_closed(), false, false)
    }

    pub const fn danger_level(self) -> DangerLevel {
        self.globals.danger_level()
    }

    pub fn manual_options(self, danger_this_walk: bool) -> FindOptions {
        self.globals
            .manual_options(self.danger_this_walk(danger_this_walk))
    }

    pub fn danger_this_walk(self, requested: bool) -> bool {
        requested && !self.globals.allow_danger_zones
    }

    pub fn permission_enabled(self, id: &str) -> Option<bool> {
        match id {
            "allow_teleports" | "allowTeleports" => Some(self.globals.allow_teleports),
            "allow_wilderness" | "allowWilderness" => Some(self.globals.allow_wilderness),
            "allow_danger_zones" | "allowDangerZones" => Some(self.globals.allow_danger_zones),
            _ => None,
        }
    }

    /// Write the fields `after` changes from `before`, merged into the
    /// file's `nav` object as the service reads it then. A write still
    /// queued when the bounded wait ends comes back
    /// [`host_play::local_state::Update::Queued`].
    pub(crate) fn persist_changed(
        preferences: &crate::prefs::UiPreferences,
        before: Self,
        after: Self,
    ) -> io::Result<host_play::local_state::Update> {
        let before_globals = before.globals;
        let after_globals = after.globals;
        let changed = [
            (
                "allow_teleports",
                before_globals.allow_teleports,
                after_globals.allow_teleports,
            ),
            (
                "allow_wilderness",
                before_globals.allow_wilderness,
                after_globals.allow_wilderness,
            ),
            (
                "allow_bank_fetch",
                before_globals.allow_bank_fetch,
                after_globals.allow_bank_fetch,
            ),
            (
                "allow_danger_zones",
                before_globals.allow_danger_zones,
                after_globals.allow_danger_zones,
            ),
            (
                "survivable_routing",
                before_globals.survivable_routing,
                after_globals.survivable_routing,
            ),
            (
                "script_scope_notice_ack",
                before.script_scope_notice_ack,
                after.script_scope_notice_ack,
            ),
            (
                "survivable_routing_notice_ack",
                before.survivable_routing_notice_ack,
                after.survivable_routing_notice_ack,
            ),
        ];
        if changed.iter().all(|(_, before, after)| before == after) {
            return Ok(host_play::local_state::Update::Durable);
        }

        preferences.begin_update(move |document| {
            let nav = document
                .entry("nav")
                .or_insert_with(|| serde_json::json!({}));
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
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{DangerLevel, WalkGlobals, WalkGlobalsView};
    use crate::prefs::UiPreferences as Preferences;
    use host_play::local_state::{LocalStateService, StateRoot};
    use nav_planner::zones::ZoneExempt;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct TempStore {
        dir: PathBuf,
        path: PathBuf,
        preferences: Preferences,
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
            let preferences = Preferences::new(
                LocalStateService::open_process(StateRoot::new(dir.clone())).preferences(),
            );
            Self {
                path: preferences.path(),
                dir,
                preferences,
            }
        }

        /// The preferences, read again (a file the test just wrote is seen).
        fn prefs(&self) -> &Preferences {
            let _ = self.preferences.reload();
            &self.preferences
        }

        /// The file, for writes another writer makes.
        fn path(&self) -> &std::path::Path {
            &self.path
        }
    }

    impl Drop for TempStore {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn projection_reads_migrates_and_fails_closed_without_reusing_the_old_notice_ack() {
        let store = TempStore::new("read");
        let absent = WalkGlobalsView::read(store.prefs());
        assert_eq!(absent, WalkGlobalsView::default());
        assert_eq!(absent.danger_level(), DangerLevel::WhenSurvivable);
        assert!(!absent.survivable_routing_notice_ack);

        vault::write_private_file(
            store.path(),
            br#"{"nav":{"allow_teleports":true,"allow_wilderness":true,"allow_bank_fetch":true,"allow_danger_zones":true,"survivable_routing":false,"script_scope_notice_ack":true,"survivable_routing_notice_ack":true}}"#,
        )
        .unwrap();
        let view = WalkGlobalsView::read(store.prefs());
        assert_eq!(
            view.globals,
            WalkGlobals {
                allow_teleports: true,
                allow_wilderness: true,
                allow_bank_fetch: true,
                allow_danger_zones: true,
                survivable_routing: false,
            }
        );
        assert_eq!(view.danger_level(), DangerLevel::Always);
        assert!(view.script_scope_notice_ack);
        assert!(view.survivable_routing_notice_ack);
        assert_eq!(view.permission_enabled("allow_teleports"), Some(true));
        assert_eq!(view.permission_enabled("allowTeleports"), Some(true));
        assert_eq!(view.permission_enabled("allow_wilderness"), Some(true));
        assert_eq!(view.permission_enabled("allowWilderness"), Some(true));
        assert_eq!(view.permission_enabled("allow_danger_zones"), Some(true));
        assert_eq!(view.permission_enabled("allowDangerZones"), Some(true));
        assert_eq!(view.permission_enabled("allow_bank_fetch"), None);
        assert_eq!(view.permission_enabled("allowBankFetch"), None);
        assert_eq!(view.permission_enabled("unrelated"), None);

        vault::write_private_file(
            store.path(),
            br#"{"nav":{"allow_danger_zones":false,"script_scope_notice_ack":true}}"#,
        )
        .unwrap();
        let migrated = WalkGlobalsView::read(store.prefs());
        assert_eq!(migrated.danger_level(), DangerLevel::WhenSurvivable);
        assert!(migrated.globals.survivable_routing);
        assert_eq!(
            migrated.globals.effective_danger_level(),
            DangerLevel::Never
        );
        assert!(migrated.script_scope_notice_ack);
        assert!(!migrated.survivable_routing_notice_ack);

        vault::write_private_file(store.path(), b"{truncated").unwrap();
        let malformed = WalkGlobalsView::read(store.prefs());
        assert_eq!(malformed.globals, WalkGlobals::fail_closed());
        assert!(!malformed.globals.survivable_routing);
        assert!(!malformed.script_scope_notice_ack);
        assert!(!malformed.survivable_routing_notice_ack);
    }

    #[test]
    fn manual_danger_uses_the_global_or_one_shot_but_global_consumes_one_shot() {
        let store = TempStore::new("manual");
        vault::write_private_file(
            store.path(),
            br#"{"nav":{"allow_teleports":true,"allow_wilderness":true,"allow_bank_fetch":true,"allow_danger_zones":false}}"#,
        )
        .unwrap();
        let view = WalkGlobalsView::read(store.prefs());
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

        vault::write_private_file(store.path(), br#"{"nav":{"allow_danger_zones":true}}"#).unwrap();
        let global = WalkGlobalsView::read(store.prefs());
        assert!(!global.danger_this_walk(true));
        assert_eq!(
            global.manual_options(global.danger_this_walk(true)).zones,
            ZoneExempt::all()
        );
    }

    #[test]
    fn changed_projection_fields_merge_in_one_durable_update() {
        let store = TempStore::new("write");
        vault::write_private_file(
            store.path(),
            br#"{"unrelated":{"keep":true},"nav":{"show_nav_path":true,"allow_wilderness":false,"allow_danger_zones":true,"survivable_routing":false,"custom":"keep"}}"#,
        )
        .unwrap();
        let before = WalkGlobalsView::read(store.prefs());
        let mut external =
            serde_json::from_slice::<serde_json::Value>(&std::fs::read(store.path()).unwrap())
                .unwrap();
        external["nav"]["allow_wilderness"] = serde_json::Value::Bool(true);
        external["nav"]["show_nav_path"] = serde_json::Value::Bool(false);
        vault::write_private_file(store.path(), &serde_json::to_vec(&external).unwrap()).unwrap();

        let mut after = WalkGlobalsView {
            globals: WalkGlobals {
                allow_teleports: true,
                ..before.globals
            },
            script_scope_notice_ack: true,
            survivable_routing_notice_ack: true,
        };
        after.globals.set_danger_level(DangerLevel::WhenSurvivable);
        crate::prefs::settled_for_test(WalkGlobalsView::persist_changed(
            store.prefs(),
            before,
            after,
        ))
        .unwrap();
        let persisted =
            serde_json::from_slice::<serde_json::Value>(&std::fs::read(store.path()).unwrap())
                .unwrap();
        assert_eq!(persisted["unrelated"]["keep"], true);
        assert_eq!(persisted["nav"]["custom"], "keep");
        assert_eq!(persisted["nav"]["allow_teleports"], true);
        assert_eq!(persisted["nav"]["allow_wilderness"], true);
        assert!(persisted["nav"].get("allow_bank_fetch").is_none());
        assert_eq!(persisted["nav"]["allow_danger_zones"], false);
        assert_eq!(persisted["nav"]["survivable_routing"], true);
        assert_eq!(persisted["nav"]["script_scope_notice_ack"], true);
        assert_eq!(persisted["nav"]["survivable_routing_notice_ack"], true);
        assert_eq!(persisted["nav"]["show_nav_path"], false);

        crate::prefs::settled_for_test(crate::nav_prefs::write_nav_preference(
            store.prefs(),
            crate::NavPreference::AllowTeleports,
            false,
        ))
        .unwrap();
        crate::prefs::settled_for_test(WalkGlobalsView::persist_changed(
            store.prefs(),
            after,
            after,
        ))
        .unwrap();
        let after_noop = WalkGlobalsView::read(store.prefs());
        assert!(!after_noop.globals.allow_teleports);
        assert_eq!(after_noop.danger_level(), DangerLevel::WhenSurvivable);
        assert_eq!(
            after_noop.globals.effective_danger_level(),
            DangerLevel::Never
        );
    }

    #[test]
    fn held_survivable_copy_names_the_effective_level_without_unshipped_claims() {
        let globals = WalkGlobals::default();
        assert_eq!(globals.danger_level(), DangerLevel::WhenSurvivable);
        assert_eq!(globals.effective_danger_level(), DangerLevel::Never);
        assert_eq!(
            super::danger_routing_label(globals.danger_level()),
            super::WHEN_SURVIVABLE_HELD_LABEL
        );
        assert!(super::survivable_routing_tooltip().contains("Effective level: Never"));
        assert!(super::survivable_routing_notice().contains("effective level is Never"));
        for copy in [
            super::survivable_routing_tooltip(),
            super::survivable_routing_notice(),
        ] {
            assert!(!copy.contains("eating"));
            assert!(!copy.contains("praying"));
            assert!(!copy.contains("turn back"));
            assert!(!copy.contains("rebake"));
            assert!(!copy.contains("274V17"));
        }
    }

    #[test]
    fn manual_walk_follow_waits_for_both_production_gates() {
        for hold in [false, true] {
            for host_move_owned in [false, true] {
                assert_eq!(
                    super::manual_walk_may_follow(hold, host_move_owned),
                    !(hold || host_move_owned),
                    "hold={hold}, host_move_owned={host_move_owned}"
                );
            }
        }
    }
}
