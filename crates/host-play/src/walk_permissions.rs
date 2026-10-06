//! Permission composition and durable danger levels at host admissions.
use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex};

use nav::router::FindOptions;
use nav::zones::ZoneExempt;
use script::native::{RiskPolicy, WalkBit, WalkOptions, WalkPermissions};

/// Runtime net activation is held until the integrated S2c gate passes.
pub const NET_AVAILABLE: bool = false;

/// Stored global danger-routing level.
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

    fn risk_policy(self, script: WalkPermissions, walk: WalkOptions) -> RiskPolicy {
        if walk.allow_danger_zones == WalkBit::Forbid {
            return RiskPolicy::Avoid;
        }

        if walk
            .allow_danger_zones
            .resolve(self == Self::Always, script.allow_danger_zones)
        {
            return RiskPolicy::Proceed;
        }

        match self {
            Self::Never | Self::Always => RiskPolicy::Avoid,
            Self::WhenSurvivable => RiskPolicy::Inherit,
        }
    }
}

/// Durable global grants published by either frontend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct WalkGlobals {
    pub allow_teleports: bool,
    pub allow_wilderness: bool,
    pub allow_bank_fetch: bool,
    pub allow_danger_zones: bool,
    pub survivable_routing: bool,
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
    /// Return a deny-all value for an unreadable or malformed preference file.
    pub const fn fail_closed() -> Self {
        Self {
            allow_teleports: false,
            allow_wilderness: false,
            allow_bank_fetch: false,
            allow_danger_zones: false,
            survivable_routing: false,
        }
    }

    /// Read all globals in one pass, ignoring unrelated panel preferences.
    /// An absent file is a first-run default; any other read/parse error is
    /// reported so callers can use [`Self::fail_closed`].
    pub fn read_at(path: &Path) -> io::Result<Self> {
        #[derive(Default, serde::Deserialize)]
        #[serde(default)]
        struct Preferences {
            nav: WalkGlobals,
        }
        let file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(error),
        };
        serde_json::from_reader::<_, Preferences>(file)
            .map(|prefs| prefs.nav)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }

    /// Project the stored pair of booleans to the three-level setting.
    pub const fn danger_level(self) -> DangerLevel {
        if self.allow_danger_zones {
            DangerLevel::Always
        } else if self.survivable_routing {
            DangerLevel::WhenSurvivable
        } else {
            DangerLevel::Never
        }
    }

    /// Gate the middle level until the runtime net is available.
    pub const fn effective_danger_level(self) -> DangerLevel {
        match self.danger_level() {
            DangerLevel::WhenSurvivable if !NET_AVAILABLE => DangerLevel::Never,
            level => level,
        }
    }

    /// Set the canonical boolean representation of a danger-routing level.
    pub fn set_danger_level(&mut self, level: DangerLevel) {
        (self.allow_danger_zones, self.survivable_routing) = match level {
            DangerLevel::Never => (false, false),
            DangerLevel::WhenSurvivable => (false, true),
            DangerLevel::Always => (true, false),
        };
    }

    /// Resolve script and walk overrides without modifying scoped named grants.
    pub fn risk_policy(self, script: WalkPermissions, walk: WalkOptions) -> RiskPolicy {
        self.effective_danger_level().risk_policy(script, walk)
    }

    /// Resolve verdict authority separately from router grants. While held,
    /// inherited requests retain precisely the router's pre-S2b outcome;
    /// explicit script and per-walk overrides still use the admission seam.
    pub fn enforces_risk(self, script: WalkPermissions, walk: WalkOptions) -> bool {
        NET_AVAILABLE || script.allow_danger_zones || walk.allow_danger_zones != WalkBit::Inherit
    }

    /// Manual WalkTo has no script grant. Its danger control is one admission only.
    pub fn manual_options(self, danger_this_walk: bool) -> FindOptions {
        FindOptions {
            allow_teleports: self.allow_teleports,
            allow_wilderness: self.allow_wilderness,
            allow_bank_fetch: self.allow_bank_fetch,
            zones: if self.allow_danger_zones || danger_this_walk {
                ZoneExempt::all()
            } else {
                ZoneExempt::NONE
            },
            ..FindOptions::default()
        }
    }

    fn native_options(self, script: WalkPermissions, walk: WalkOptions) -> FindOptions {
        let policy = self.risk_policy(script, walk);
        self.native_options_with_policy(script, walk, policy)
    }

    fn native_options_with_policy(
        self,
        script: WalkPermissions,
        walk: WalkOptions,
        policy: RiskPolicy,
    ) -> FindOptions {
        FindOptions {
            allow_teleports: walk
                .allow_teleports
                .resolve(self.allow_teleports, script.allow_teleports),
            allow_wilderness: walk
                .allow_wilderness
                .resolve(self.allow_wilderness, script.allow_wilderness),
            // Native supply preparation belongs to the card's Provisioner, not BankBudget.
            allow_bank_fetch: false,
            zones: if policy == RiskPolicy::Proceed {
                ZoneExempt::all()
            } else {
                ZoneExempt::NONE
            },
            ..FindOptions::default()
        }
    }
}

pub(crate) fn globals_for(bot: Option<&super::NavBot>) -> WalkGlobals {
    match bot.and_then(|bot| bot.walk_globals_store.as_ref()) {
        Some(path) => WalkGlobals::read_at(path).unwrap_or_else(|_| WalkGlobals::fail_closed()),
        None => bot
            .and_then(|bot| bot.walk_globals.as_ref())
            .map_or_else(WalkGlobals::default, |globals| *globals.lock().unwrap()),
    }
}

/// Record the held-off middle level once per session, never from a tick path.
pub(crate) fn log_runtime_net_gate(slot: &str, globals: WalkGlobals, logged: &mut bool) {
    if globals.danger_level() == DangerLevel::WhenSurvivable
        && globals.effective_danger_level() == DangerLevel::Never
        && !*logged
    {
        *logged = true;
        api::host_log!(
            api::hostlog::Category::NavEvent,
            api::hostlog::Level::Info,
            slot = slot,
            "When survivable is not available yet: acts as Never; inherited routing is unchanged"
        );
    }
}

/// Resolve native search options and risk policy from one durable settings read.
pub(crate) fn native_admission(
    navs: &Arc<Mutex<HashMap<String, super::NavBot>>>,
    name: &str,
    walk: WalkOptions,
) -> (FindOptions, RiskPolicy, bool) {
    let mut all = navs.lock().unwrap();
    let mut bot = all.get_mut(name);
    let globals = globals_for(bot.as_deref());
    if let Some(bot) = bot.as_deref_mut() {
        log_runtime_net_gate(name, globals, &mut bot.runtime_gate_logged);
    }
    let script = bot
        .and_then(|bot| bot.native_permissions)
        .unwrap_or_default();
    let policy = globals.risk_policy(script, walk);
    (
        globals.native_options_with_policy(script, walk, policy),
        policy,
        globals.enforces_risk(script, walk),
    )
}

/// Compiled callers using the existing bool wire (Sherlock's clue machine and
/// watchdog walks) treat false as Inherit. Isolate v1/v2 remain unchanged.
pub(crate) fn compiled_options(
    navs: &Arc<Mutex<HashMap<String, super::NavBot>>>,
    name: &str,
    opts: FindOptions,
) -> FindOptions {
    let mut all = navs.lock().unwrap();
    let Some(bot) = all.get_mut(name) else {
        return opts;
    };
    if bot.compat_v1 {
        return opts;
    }
    let globals = globals_for(Some(bot));
    log_runtime_net_gate(name, globals, &mut bot.runtime_gate_logged);
    let Some(script) = bot.native_permissions else {
        // Legacy bool calls keep teleport/wilderness/bank authority unchanged.
        // Only the host's danger preference is inherited.
        let zones =
            if globals.risk_policy(Default::default(), Default::default()) == RiskPolicy::Proceed {
                ZoneExempt::all()
            } else {
                ZoneExempt::NONE
            };
        return FindOptions {
            zones: opts
                .zones
                .union(zones)
                .expect("all or empty fits named capacity"),
            ..opts
        };
    };
    let bit = |allow| {
        if allow {
            WalkBit::Allow
        } else {
            WalkBit::Inherit
        }
    };
    let resolved = globals.native_options(
        script,
        WalkOptions {
            allow_teleports: bit(opts.allow_teleports),
            allow_wilderness: bit(opts.allow_wilderness),
            allow_danger_zones: WalkBit::Inherit,
        },
    );
    FindOptions {
        zones: opts
            .zones
            .union(resolved.zones)
            .expect("all or empty does not exceed named capacity"),
        allow_teleports: resolved.allow_teleports,
        allow_wilderness: resolved.allow_wilderness,
        allow_bank_fetch: false,
        ..opts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_permission_truth_table_and_manual_fetch_isolation() {
        for (global, script, inherited) in [
            (false, false, false),
            (false, true, true),
            (true, false, true),
            (true, true, true),
        ] {
            let globals = WalkGlobals {
                allow_teleports: global,
                allow_wilderness: global,
                allow_bank_fetch: true,
                allow_danger_zones: global,
                survivable_routing: true,
            };
            let script = WalkPermissions {
                allow_teleports: script,
                allow_wilderness: script,
                allow_danger_zones: script,
            };
            for (bit, expected) in [
                (WalkBit::Inherit, inherited),
                (WalkBit::Allow, true),
                (WalkBit::Forbid, false),
            ] {
                let opts = globals.native_options(
                    script,
                    WalkOptions {
                        allow_teleports: bit,
                        allow_wilderness: bit,
                        allow_danger_zones: bit,
                    },
                );
                assert_eq!(opts.allow_teleports, expected);
                assert_eq!(opts.allow_wilderness, expected);
                assert_eq!(opts.zones.is_all(), expected);
                assert!(!opts.allow_bank_fetch);
            }
            assert!(globals.manual_options(false).allow_bank_fetch);
            assert_eq!(globals.manual_options(false).zones.is_all(), global);
            assert!(globals.manual_options(true).zones.is_all());
            assert_eq!(globals.manual_options(false).zones.is_all(), global);
        }
    }

    #[test]
    fn durable_globals_are_reread_at_native_and_manual_admissions() {
        let _iso = script::IsolatedEnv::enter("walk-globals-reread");
        let path = super::super::panel_ui_path();
        let bot = super::super::NavBot {
            walk_globals_store: Some(Arc::new(path.clone())),
            native_permissions: Some(WalkPermissions::default()),
            ..Default::default()
        };
        let navs = Arc::new(Mutex::new(HashMap::from([("alice".into(), bot)])));
        for enabled in [true, false, true] {
            super::super::persist_panel_ui_value_at(
                &path,
                "nav",
                serde_json::json!({
                    "allow_teleports": enabled,
                    "allow_wilderness": enabled,
                    "allow_bank_fetch": enabled,
                    "allow_danger_zones": enabled,
                }),
            )
            .unwrap();
            let (native, policy, _) = native_admission(&navs, "alice", WalkOptions::default());
            assert_eq!(
                policy,
                if enabled {
                    RiskPolicy::Proceed
                } else {
                    RiskPolicy::Avoid
                }
            );
            let compiled = compiled_options(&navs, "alice", FindOptions::default());
            let manual = WalkGlobals::read_at(&path).unwrap().manual_options(false);
            for options in [native, compiled, manual] {
                assert_eq!(options.allow_teleports, enabled);
                assert_eq!(options.allow_wilderness, enabled);
                assert_eq!(options.zones.is_all(), enabled);
            }
            assert!(!native.allow_bank_fetch && !compiled.allow_bank_fetch);
            assert_eq!(manual.allow_bank_fetch, enabled);
        }
    }

    #[test]
    fn isolate_booleans_do_not_inherit_native_globals() {
        let navs = Arc::new(Mutex::new(HashMap::from([(
            "isolate".into(),
            super::super::NavBot {
                walk_globals: Some(Arc::new(Mutex::new(WalkGlobals {
                    allow_teleports: true,
                    allow_wilderness: true,
                    allow_bank_fetch: true,
                    allow_danger_zones: true,
                    survivable_routing: true,
                }))),
                ..Default::default()
            },
        )])));
        for allow_bank_fetch in [false, true] {
            let explicit = FindOptions {
                allow_bank_fetch,
                ..Default::default()
            };
            let compiled = compiled_options(&navs, "isolate", explicit);
            assert_eq!(compiled.allow_teleports, explicit.allow_teleports);
            assert_eq!(compiled.allow_wilderness, explicit.allow_wilderness);
            assert_eq!(compiled.allow_bank_fetch, explicit.allow_bank_fetch);
            assert!(
                compiled.zones.is_all(),
                "S2b applies shared risk settings independently of frozen v1 H booleans"
            );
        }
    }
    #[test]
    fn walk_globals_round_trip_default_absent_old_file_and_fail_closed_error() {
        let dir =
            std::env::temp_dir().join(format!("host-play-walk-globals-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("panel-ui.json");

        let defaults = WalkGlobals::default();
        assert!(!defaults.allow_teleports);
        assert!(!defaults.allow_wilderness);
        assert!(!defaults.allow_bank_fetch);
        assert!(!defaults.allow_danger_zones);
        assert!(defaults.survivable_routing);
        assert_eq!(defaults.danger_level(), DangerLevel::WhenSurvivable);
        assert_eq!(defaults.effective_danger_level(), DangerLevel::Never);
        assert_eq!(
            serde_json::from_value::<WalkGlobals>(serde_json::to_value(defaults).unwrap()).unwrap(),
            defaults
        );
        assert_eq!(WalkGlobals::read_at(&path).unwrap(), defaults);

        std::fs::write(&path, r#"{"nav":{"allow_danger_zones":false}}"#).unwrap();
        let migrated = WalkGlobals::read_at(&path).unwrap();
        assert_eq!(migrated.danger_level(), DangerLevel::WhenSurvivable);
        assert!(migrated.survivable_routing);
        assert_eq!(migrated.effective_danger_level(), DangerLevel::Never);

        std::fs::write(&path, b"{malformed").unwrap();
        assert!(WalkGlobals::read_at(&path).is_err());
        assert_eq!(
            WalkGlobals::fail_closed(),
            WalkGlobals {
                allow_teleports: false,
                allow_wilderness: false,
                allow_bank_fetch: false,
                allow_danger_zones: false,
                survivable_routing: false,
            }
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn danger_policy_exhaustively_resolves_levels_script_bits_and_walk_bits() {
        for stored_level in [
            DangerLevel::Never,
            DangerLevel::WhenSurvivable,
            DangerLevel::Always,
        ] {
            let mut globals = WalkGlobals::fail_closed();
            globals.set_danger_level(stored_level);
            assert_eq!(globals.danger_level(), stored_level);
            let effective_level = if stored_level == DangerLevel::WhenSurvivable && !NET_AVAILABLE {
                DangerLevel::Never
            } else {
                stored_level
            };
            assert_eq!(
                globals.effective_danger_level(),
                effective_level,
                "gate for {stored_level:?}"
            );
            for script_grant in [false, true] {
                let script = WalkPermissions {
                    allow_danger_zones: script_grant,
                    ..WalkPermissions::default()
                };
                for walk_bit in [WalkBit::Inherit, WalkBit::Allow, WalkBit::Forbid] {
                    let walk = WalkOptions {
                        allow_danger_zones: walk_bit,
                        ..WalkOptions::default()
                    };
                    let expected = if walk_bit == WalkBit::Forbid {
                        RiskPolicy::Avoid
                    } else if walk_bit.resolve(effective_level == DangerLevel::Always, script_grant)
                    {
                        RiskPolicy::Proceed
                    } else {
                        match effective_level {
                            DangerLevel::Never | DangerLevel::Always => RiskPolicy::Avoid,
                            DangerLevel::WhenSurvivable => RiskPolicy::Inherit,
                        }
                    };
                    assert_eq!(
                        globals.risk_policy(script, walk),
                        expected,
                        "stored={stored_level:?}, effective={effective_level:?}, script={script_grant}, walk={walk_bit:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn complete_danger_precedence_table_is_tested_independently_of_net_activation() {
        for level in [
            DangerLevel::Never,
            DangerLevel::WhenSurvivable,
            DangerLevel::Always,
        ] {
            for script_grant in [false, true] {
                let script = WalkPermissions {
                    allow_danger_zones: script_grant,
                    ..WalkPermissions::default()
                };
                for walk_bit in [WalkBit::Inherit, WalkBit::Allow, WalkBit::Forbid] {
                    let walk = WalkOptions {
                        allow_danger_zones: walk_bit,
                        ..WalkOptions::default()
                    };
                    let expected = match (level, script_grant, walk_bit) {
                        (_, _, WalkBit::Forbid) => RiskPolicy::Avoid,
                        (_, _, WalkBit::Allow) | (_, true, WalkBit::Inherit) => RiskPolicy::Proceed,
                        (DangerLevel::Never, false, WalkBit::Inherit) => RiskPolicy::Avoid,
                        (DangerLevel::WhenSurvivable, false, WalkBit::Inherit) => {
                            RiskPolicy::Inherit
                        }
                        (DangerLevel::Always, false, WalkBit::Inherit) => RiskPolicy::Proceed,
                    };
                    assert_eq!(
                        level.risk_policy(script, walk),
                        expected,
                        "level={level:?}, script={script_grant}, walk={walk_bit:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn inherited_avoid_does_not_remove_named_zone_grants() {
        use nav::zones::ZoneKey;

        let named = ZoneExempt::named(&[ZoneKey::Zone(7)]).unwrap();
        let globals = WalkGlobals {
            survivable_routing: false,
            ..WalkGlobals::default()
        };
        let navs = Arc::new(Mutex::new(HashMap::from([(
            "alice".into(),
            super::super::NavBot {
                native_permissions: Some(WalkPermissions::default()),
                walk_globals: Some(Arc::new(Mutex::new(globals))),
                ..Default::default()
            },
        )])));
        let resolved = compiled_options(
            &navs,
            "alice",
            FindOptions {
                zones: named,
                ..FindOptions::default()
            },
        );
        assert_eq!(resolved.zones, named);
        assert!(!resolved.zones.is_all());
    }
}

#[cfg(test)]
mod held_gate_tests {
    use super::*;

    #[test]
    fn inherited_and_explicit_enforcement_are_distinct_while_held() {
        const { assert!(!NET_AVAILABLE) };
        for level in [
            DangerLevel::Never,
            DangerLevel::WhenSurvivable,
            DangerLevel::Always,
        ] {
            let mut globals = WalkGlobals::default();
            globals.set_danger_level(level);
            assert!(!globals.enforces_risk(WalkPermissions::default(), WalkOptions::default()));
            for bit in [WalkBit::Allow, WalkBit::Forbid] {
                assert!(globals.enforces_risk(
                    WalkPermissions::default(),
                    WalkOptions {
                        allow_danger_zones: bit,
                        ..Default::default()
                    },
                ));
            }
            assert!(globals.enforces_risk(
                WalkPermissions {
                    allow_danger_zones: true,
                    ..Default::default()
                },
                WalkOptions::default(),
            ));
        }
    }

    #[test]
    fn repeated_native_compiled_and_manual_admissions_share_one_session_note() {
        let navs = Arc::new(Mutex::new(HashMap::from([
            ("alice".to_owned(), super::super::NavBot::default()),
            ("bob".to_owned(), super::super::NavBot::default()),
        ])));
        assert!(!navs.lock().unwrap()["alice"].runtime_gate_logged);
        native_admission(&navs, "alice", WalkOptions::default());
        assert!(navs.lock().unwrap()["alice"].runtime_gate_logged);
        assert!(!navs.lock().unwrap()["bob"].runtime_gate_logged);
        for _ in 0..3 {
            compiled_options(&navs, "alice", FindOptions::default());
            native_admission(&navs, "alice", WalkOptions::default());
            let mut all = navs.lock().unwrap();
            log_runtime_net_gate(
                "alice",
                WalkGlobals::default(),
                &mut all.get_mut("alice").unwrap().runtime_gate_logged,
            );
            assert!(all["alice"].runtime_gate_logged);
        }
    }
}
