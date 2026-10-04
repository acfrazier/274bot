//! Permission composition at host admissions. Navigation retains boolean options.
use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex};

use nav::router::FindOptions;
use nav::zones::ZoneExempt;
use script::native::{WalkBit, WalkOptions, WalkPermissions};

/// Durable global grants published by either frontend. Missing keys are off.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(default)]
pub struct WalkGlobals {
    pub allow_teleports: bool,
    pub allow_wilderness: bool,
    pub allow_bank_fetch: bool,
    pub allow_danger_zones: bool,
}

impl WalkGlobals {
    /// Read all four globals in one pass, ignoring unrelated panel preferences.
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
        FindOptions {
            allow_teleports: walk
                .allow_teleports
                .resolve(self.allow_teleports, script.allow_teleports),
            allow_wilderness: walk
                .allow_wilderness
                .resolve(self.allow_wilderness, script.allow_wilderness),
            // Native supply preparation belongs to the card's Provisioner, not BankBudget.
            allow_bank_fetch: false,
            zones: if walk
                .allow_danger_zones
                .resolve(self.allow_danger_zones, script.allow_danger_zones)
            {
                ZoneExempt::all()
            } else {
                ZoneExempt::NONE
            },
            ..FindOptions::default()
        }
    }
}

fn globals_for(bot: Option<&super::NavBot>) -> WalkGlobals {
    match bot.and_then(|bot| bot.walk_globals_store.as_ref()) {
        Some(path) => WalkGlobals::read_at(path).unwrap_or_default(),
        None => bot
            .and_then(|bot| bot.walk_globals.as_ref())
            .map_or_else(WalkGlobals::default, |globals| *globals.lock().unwrap()),
    }
}

pub(crate) fn native_options(
    navs: &Arc<Mutex<HashMap<String, super::NavBot>>>,
    name: &str,
    walk: WalkOptions,
) -> FindOptions {
    let all = navs.lock().unwrap();
    let bot = all.get(name);
    let globals = globals_for(bot);
    let script = bot
        .and_then(|bot| bot.native_permissions)
        .unwrap_or_default();
    globals.native_options(script, walk)
}

/// Compiled callers using the existing bool wire (Sherlock's clue machine and
/// watchdog walks) treat false as Inherit. Isolate v1/v2 remain unchanged.
pub(crate) fn compiled_options(
    navs: &Arc<Mutex<HashMap<String, super::NavBot>>>,
    name: &str,
    opts: FindOptions,
) -> FindOptions {
    let all = navs.lock().unwrap();
    let Some(bot) = all.get(name) else {
        return opts;
    };
    let Some(script) = bot.native_permissions else {
        return opts;
    };
    let globals = globals_for(Some(bot));
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
                allow_danger_zones: global,
                allow_bank_fetch: true,
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
            let native = native_options(&navs, "alice", WalkOptions::default());
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
                }))),
                ..Default::default()
            },
        )])));
        for allow_bank_fetch in [false, true] {
            let explicit = FindOptions {
                allow_bank_fetch,
                ..Default::default()
            };
            assert_eq!(compiled_options(&navs, "isolate", explicit), explicit);
        }
    }
}
