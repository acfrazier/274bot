//! Durable walk-global permissions shared by the panel and TUI.
//!
//! Frontends render and admit manual walks from the same projection. Reads
//! fail closed; writes merge only values changed by the caller into the
//! shared preferences, through the process local-state service.

pub use host_play::DangerLevel;
use host_play::WalkGlobals as HostWalkGlobals;
use nav_planner::router::FindOptions;

/// Durable movement grants shared by the panel and TUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct WalkGlobals {
    allow_teleports: bool,
    allow_wilderness: bool,
    allow_bank_fetch: bool,
    allow_danger_zones: bool,
    survivable_routing: bool,
    allow_prereq_dialogue: bool,
    allow_quest_start: bool,
}

impl Default for WalkGlobals {
    fn default() -> Self {
        Self {
            allow_teleports: false,
            allow_wilderness: false,
            allow_bank_fetch: false,
            allow_danger_zones: false,
            survivable_routing: false,
            allow_prereq_dialogue: true,
            allow_quest_start: true,
        }
    }
}

impl WalkGlobals {
    pub const fn new(
        allow_teleports: bool,
        allow_wilderness: bool,
        allow_bank_fetch: bool,
        allow_danger_zones: bool,
        allow_prereq_dialogue: bool,
        allow_quest_start: bool,
    ) -> Self {
        Self {
            allow_teleports,
            allow_wilderness,
            allow_bank_fetch,
            allow_danger_zones,
            survivable_routing: false,
            allow_prereq_dialogue,
            allow_quest_start,
        }
    }

    pub(crate) fn from_host(globals: HostWalkGlobals) -> Self {
        Self::new(
            globals.allow_teleports,
            globals.allow_wilderness,
            globals.allow_bank_fetch,
            globals.allow_danger_zones,
            globals.allow_prereq_dialogue,
            globals.allow_quest_start,
        )
        .with_survivable_routing(globals.survivable_routing)
    }

    pub(crate) const fn to_host(self) -> HostWalkGlobals {
        HostWalkGlobals {
            allow_teleports: self.allow_teleports,
            allow_wilderness: self.allow_wilderness,
            allow_bank_fetch: self.allow_bank_fetch,
            allow_danger_zones: self.allow_danger_zones,
            survivable_routing: self.survivable_routing,
            allow_prereq_dialogue: self.allow_prereq_dialogue,
            allow_quest_start: self.allow_quest_start,
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
    pub const fn allow_prereq_dialogue(self) -> bool {
        self.allow_prereq_dialogue
    }
    pub const fn allow_quest_start(self) -> bool {
        self.allow_quest_start
    }

    pub const fn fail_closed() -> Self {
        Self {
            allow_teleports: false,
            allow_wilderness: false,
            allow_bank_fetch: false,
            allow_danger_zones: false,
            allow_prereq_dialogue: false,
            allow_quest_start: false,
            survivable_routing: false,
        }
    }

    pub const fn danger_level(self) -> DangerLevel {
        self.to_host().danger_level()
    }

    /// These globals at `level`, including both stored danger-routing bits.
    pub const fn with_danger_level(self, level: DangerLevel) -> Self {
        Self {
            allow_danger_zones: matches!(level, DangerLevel::Always),
            survivable_routing: matches!(level, DangerLevel::WhenSurvivable),
            ..self
        }
    }

    /// Set the private stored middle-level bit without changing other globals.
    pub const fn with_survivable_routing(self, enabled: bool) -> Self {
        Self {
            survivable_routing: enabled,
            ..self
        }
    }

    pub fn manual_options(self, danger_this_walk: bool) -> FindOptions {
        self.to_host().manual_options(danger_this_walk)
    }
}

/// Shared explanation of how the Nav config walk grants apply to scripts.
pub const SCRIPT_SCOPE_NOTICE: &str = "Teleports and wilderness in Nav config now apply to every walk, including scripts. Bank fetch still applies only to manual WalkTo. rs2b0t-compatible scripts always allow wilderness and bank fetch.";

/// What each danger-routing level does, shared by the panel tooltip and the
/// TUI settings line.
pub const fn danger_routing_tooltip(level: DangerLevel) -> &'static str {
    match level {
        DangerLevel::Never => NEVER_DANGER_NOTE,
        DangerLevel::WhenSurvivable => WHEN_SURVIVABLE_DANGER_NOTE,
        DangerLevel::Always => GLOBAL_DANGER_WARNING,
    }
}

/// The default level's explanation.
pub const NEVER_DANGER_NOTE: &str = "Danger-zone routes are refused unless the walk or its script asks to cross. A script's request is checked against a death-probability bound of 1%.";

/// Explanation for the held middle level.
pub const WHEN_SURVIVABLE_DANGER_NOTE: &str = "Currently behaves exactly like Never while the runtime safety net is held. When the net is available, this level uses it.";
/// Warning shown for the global Always override and in place of the one-shot control.
pub const GLOBAL_DANGER_WARNING: &str = "Global danger-zone override is enabled. Walks proceed without the survival check. The model's death-probability bound is information only.";

/// Label for the single-admission danger-zone control.
pub const DANGER_THIS_WALK_LABEL: &str = "Route through danger zones";
/// One note above the routing grants, replacing the per-row scope suffixes.
pub const ROUTING_SCOPE_NOTE: &str = "Settings are global except where otherwise specified. rs2b0t-compatible scripts always allow wilderness and bank fetch.";
/// Bank-budget fetching is available to manual WalkTo only.
pub const BANK_FETCH_PERMISSION_SCOPE: &str = "Manual WalkTo only.";

/// Danger-routing levels in the order the drop-down lists them.
pub const DANGER_ROUTING_LEVELS: [DangerLevel; 3] = [
    DangerLevel::Never,
    DangerLevel::WhenSurvivable,
    DangerLevel::Always,
];

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

/// Shared labels for the durable prerequisite grants: preference id, label.
pub const PREREQ_PERMISSION_LABELS: [(&str, &str); 2] = [
    ("allow_prereq_dialogue", "Allow dialogue steps on walks"),
    ("allow_quest_start", "Allow starting quests on walks"),
];

/// How the prerequisite grants compose: the global switch or the script's
/// own grant allows the step on a walk, and only a per-request Forbid
/// narrows it.
pub const PREREQ_SCOPE_NOTE: &str = "Dialogue and quest-start grants are additive: the global switch or the script's own grant allows the step, and only a per-request Forbid narrows it.";

/// One durable read of all global walk permissions and their notice acknowledgements.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WalkGlobalsView {
    globals: WalkGlobals,
    script_scope_notice_ack: bool,
}

impl WalkGlobalsView {
    pub const fn new(globals: WalkGlobals, script_scope_notice_ack: bool) -> Self {
        Self {
            globals,
            script_scope_notice_ack,
        }
    }

    pub const fn globals(self) -> WalkGlobals {
        self.globals
    }
    pub const fn script_scope_notice_ack(self) -> bool {
        self.script_scope_notice_ack
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
        )
    }

    fn fail_closed() -> Self {
        Self::new(WalkGlobals::fail_closed(), false)
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
            "allow_prereq_dialogue" | "allowPrereqDialogue" => {
                Some(self.globals.allow_prereq_dialogue)
            }
            "allow_quest_start" | "allowQuestStart" => Some(self.globals.allow_quest_start),
            _ => None,
        }
    }

    /// Submit one danger-level choice as one intent. Both stored danger bits
    /// are updated in the same write.
    pub(crate) fn persist_danger(
        preferences: &crate::prefs::UiPreferences,
        level: DangerLevel,
    ) -> Result<host_play::local_state::Update, crate::prefs::PreferenceError> {
        let allow = matches!(level, DangerLevel::Always);
        let survivable = matches!(level, DangerLevel::WhenSurvivable);
        let mut patch = crate::prefs::PreferencePatch::default();
        patch.set_member(&["nav", "allow_danger_zones"], Some(allow.into()))?;
        patch.set_member(&["nav", "survivable_routing"], Some(survivable.into()))?;
        preferences.begin_patch(patch)
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
    fn projection_reads_migrates_and_fails_closed() {
        let store = TempStore::new("read");
        let absent = WalkGlobalsView::read(store.prefs());
        assert_eq!(absent, WalkGlobalsView::default());
        assert_eq!(absent.danger_level(), DangerLevel::Never);

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
                allow_prereq_dialogue: true,
                allow_quest_start: true,
                survivable_routing: false,
            }
        );
        assert_eq!(view.danger_level(), DangerLevel::Always);
        assert!(view.script_scope_notice_ack);
        assert_eq!(view.permission_enabled("allow_teleports"), Some(true));
        assert_eq!(view.permission_enabled("allowTeleports"), Some(true));
        assert_eq!(view.permission_enabled("allow_wilderness"), Some(true));
        assert_eq!(view.permission_enabled("allowWilderness"), Some(true));
        assert_eq!(view.permission_enabled("allow_danger_zones"), Some(true));
        assert_eq!(view.permission_enabled("allowDangerZones"), Some(true));
        assert_eq!(view.permission_enabled("allow_prereq_dialogue"), Some(true));
        assert_eq!(view.permission_enabled("allowPrereqDialogue"), Some(true));
        assert_eq!(view.permission_enabled("allow_quest_start"), Some(true));
        assert_eq!(view.permission_enabled("allowQuestStart"), Some(true));
        assert_eq!(view.permission_enabled("allow_bank_fetch"), None);
        assert_eq!(view.permission_enabled("allowBankFetch"), None);
        assert_eq!(view.permission_enabled("unrelated"), None);

        vault::write_private_file(
            store.path(),
            br#"{"nav":{"allow_danger_zones":false,"script_scope_notice_ack":true}}"#,
        )
        .unwrap();
        let migrated = WalkGlobalsView::read(store.prefs());
        assert_eq!(migrated.danger_level(), DangerLevel::Never);
        assert!(
            migrated.globals.allow_prereq_dialogue,
            "an old file without the prereq keys keeps dialogue steps on"
        );
        assert!(
            migrated.globals.allow_quest_start,
            "an old file without the prereq keys keeps quest starts on"
        );
        assert!(migrated.script_scope_notice_ack);

        vault::write_private_file(store.path(), b"{truncated").unwrap();
        let malformed = WalkGlobalsView::read(store.prefs());
        assert_eq!(malformed.globals, WalkGlobals::fail_closed());
        assert_eq!(malformed.danger_level(), DangerLevel::Never);
        assert!(!malformed.globals.allow_prereq_dialogue);
        assert!(!malformed.globals.allow_quest_start);
        assert!(!malformed.script_scope_notice_ack);
    }

    /// A saved middle-level pair remains selected without a read rewrite.
    #[test]
    fn saved_middle_level_loads_without_a_rewrite() {
        let store = TempStore::new("middle");
        let middle = br#"{"nav":{"allow_danger_zones":false,"survivable_routing":true,"survivable_routing_notice_ack":true,"allow_teleports":true}}"#;
        vault::write_private_file(store.path(), middle).unwrap();
        let view = WalkGlobalsView::read(store.prefs());
        assert_eq!(view.danger_level(), DangerLevel::WhenSurvivable);
        assert!(!view.globals.allow_danger_zones);
        assert!(view.globals.survivable_routing);
        assert!(view.globals.allow_teleports);
        assert_eq!(std::fs::read(store.path()).unwrap(), middle);

        // Always takes precedence when both stored bits are true.
        vault::write_private_file(
            store.path(),
            br#"{"nav":{"allow_danger_zones":true,"survivable_routing":true}}"#,
        )
        .unwrap();
        assert_eq!(
            WalkGlobalsView::read(store.prefs()).danger_level(),
            DangerLevel::Always
        );
    }

    #[test]
    fn prereq_grants_default_on_and_round_trip_through_the_nav_object() {
        let store = TempStore::new("prereq");
        let defaults = WalkGlobalsView::read(store.prefs());
        assert!(defaults.globals.allow_prereq_dialogue);
        assert!(defaults.globals.allow_quest_start);

        crate::prefs::settled_for_test(crate::nav_prefs::write_nav_preference(
            store.prefs(),
            crate::NavPreference::AllowPrereqDialogue,
            false,
        ))
        .unwrap();
        crate::prefs::settled_for_test(crate::nav_prefs::write_nav_preference(
            store.prefs(),
            crate::NavPreference::AllowQuestStart,
            false,
        ))
        .unwrap();
        let denied = WalkGlobalsView::read(store.prefs());
        assert!(!denied.globals.allow_prereq_dialogue);
        assert!(!denied.globals.allow_quest_start);
        let persisted =
            serde_json::from_slice::<serde_json::Value>(&std::fs::read(store.path()).unwrap())
                .unwrap();
        assert_eq!(persisted["nav"]["allow_prereq_dialogue"], false);
        assert_eq!(persisted["nav"]["allow_quest_start"], false);

        crate::prefs::settled_for_test(crate::nav_prefs::write_nav_preference(
            store.prefs(),
            crate::NavPreference::AllowPrereqDialogue,
            true,
        ))
        .unwrap();
        crate::prefs::settled_for_test(crate::nav_prefs::write_nav_preference(
            store.prefs(),
            crate::NavPreference::AllowQuestStart,
            true,
        ))
        .unwrap();
        let allowed = WalkGlobalsView::read(store.prefs());
        assert!(allowed.globals.allow_prereq_dialogue);
        assert!(allowed.globals.allow_quest_start);
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
    fn danger_level_merges_both_bits_in_one_durable_update() {
        let store = TempStore::new("write");
        vault::write_private_file(
            store.path(),
            br#"{"unrelated":{"keep":true},"nav":{"show_nav_path":true,"allow_wilderness":false,"allow_danger_zones":false,"survivable_routing":true,"custom":"keep"}}"#,
        )
        .unwrap();
        let _before = WalkGlobalsView::read(store.prefs());
        let mut external =
            serde_json::from_slice::<serde_json::Value>(&std::fs::read(store.path()).unwrap())
                .unwrap();
        external["nav"]["allow_wilderness"] = serde_json::Value::Bool(true);
        external["nav"]["show_nav_path"] = serde_json::Value::Bool(false);
        vault::write_private_file(store.path(), &serde_json::to_vec(&external).unwrap()).unwrap();

        crate::prefs::settled_for_test(WalkGlobalsView::persist_danger(
            store.prefs(),
            DangerLevel::WhenSurvivable,
        ))
        .unwrap();
        let middle =
            serde_json::from_slice::<serde_json::Value>(&std::fs::read(store.path()).unwrap())
                .unwrap();
        assert_eq!(middle["unrelated"]["keep"], true);
        assert_eq!(middle["nav"]["custom"], "keep");
        assert_eq!(middle["nav"]["allow_wilderness"], true);
        assert!(middle["nav"].get("allow_bank_fetch").is_none());
        assert_eq!(middle["nav"]["allow_danger_zones"], false);
        assert_eq!(middle["nav"]["survivable_routing"], true);
        assert_eq!(middle["nav"]["show_nav_path"], false);
        assert_eq!(
            WalkGlobalsView::read(store.prefs()).danger_level(),
            DangerLevel::WhenSurvivable
        );

        crate::prefs::settled_for_test(WalkGlobalsView::persist_danger(
            store.prefs(),
            DangerLevel::Always,
        ))
        .unwrap();
        let always =
            serde_json::from_slice::<serde_json::Value>(&std::fs::read(store.path()).unwrap())
                .unwrap();
        assert_eq!(always["nav"]["allow_danger_zones"], true);
        assert_eq!(always["nav"]["survivable_routing"], false);

        crate::prefs::settled_for_test(crate::nav_prefs::write_nav_preference(
            store.prefs(),
            crate::NavPreference::AllowTeleports,
            false,
        ))
        .unwrap();
        crate::prefs::settled_for_test(WalkGlobalsView::persist_danger(
            store.prefs(),
            DangerLevel::Never,
        ))
        .unwrap();
        let after = WalkGlobalsView::read(store.prefs());
        assert!(!after.globals.allow_teleports);
        assert_eq!(after.danger_level(), DangerLevel::Never);
        assert!(!after.globals.survivable_routing);
    }

    #[test]
    fn three_levels_round_trip_with_shared_copy() {
        assert_eq!(
            super::DANGER_ROUTING_LEVELS,
            [
                DangerLevel::Never,
                DangerLevel::WhenSurvivable,
                DangerLevel::Always
            ]
        );
        assert_eq!(WalkGlobals::default().danger_level(), DangerLevel::Never);
        for level in super::DANGER_ROUTING_LEVELS {
            let globals = WalkGlobals::default().with_danger_level(level);
            assert_eq!(globals.danger_level(), level);
            assert_eq!(
                globals.survivable_routing(),
                level == DangerLevel::WhenSurvivable
            );
            assert_eq!(level.next().next().next(), level);
        }
        assert!(super::danger_routing_tooltip(DangerLevel::Never).contains("1%"));
        assert!(super::danger_routing_tooltip(DangerLevel::WhenSurvivable)
            .contains("exactly like Never"));
        assert!(super::danger_routing_tooltip(DangerLevel::WhenSurvivable)
            .contains("runtime safety net"));
        assert_eq!(
            super::danger_routing_tooltip(DangerLevel::Always),
            super::GLOBAL_DANGER_WARNING
        );
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
