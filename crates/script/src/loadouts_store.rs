//! Operator loadouts: `{ name, worn, carry }` persisted at
//! `~/.274bot/loadouts.json` (0o600). `optionsFrom: 'loadouts'` fills
//! setting combos from [`LoadoutsStore::names`].

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use vault::write_private_file;

/// One named equipment/inventory preset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Loadout {
    pub name: String,
    pub worn: Vec<String>,
    pub carry: Vec<String>,
}

/// Default operator loadouts path (`~/.274bot/loadouts.json`).
pub fn default_loadouts_path() -> PathBuf {
    crate::bot_file("loadouts.json")
}

/// Persisted loadout list.
pub struct LoadoutsStore {
    path: PathBuf,
    loadouts: Vec<Loadout>,
    dirty: bool,
}

impl LoadoutsStore {
    pub fn at(path: PathBuf) -> Self {
        let loadouts = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();
        LoadoutsStore {
            path,
            loadouts,
            dirty: false,
        }
    }

    pub fn with_default_path() -> Self {
        Self::at(default_loadouts_path())
    }

    pub fn loadouts(&self) -> &[Loadout] {
        &self.loadouts
    }

    pub fn names(&self) -> Vec<String> {
        self.loadouts.iter().map(|l| l.name.clone()).collect()
    }

    pub fn get(&self, name: &str) -> Option<&Loadout> {
        self.loadouts.iter().find(|l| l.name == name)
    }

    pub fn upsert(&mut self, loadout: Loadout) {
        self.dirty = true;
        if let Some(existing) = self.loadouts.iter_mut().find(|l| l.name == loadout.name) {
            *existing = loadout;
        } else {
            self.loadouts.push(loadout);
        }
    }

    /// Replace the loadout at `index`, including renames (does not key on name).
    pub fn replace_at(&mut self, index: usize, loadout: Loadout) -> bool {
        if index >= self.loadouts.len() {
            return false;
        }
        self.dirty = true;
        self.loadouts[index] = loadout;
        true
    }

    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.loadouts.len();
        self.loadouts.retain(|l| l.name != name);
        if self.loadouts.len() != before {
            self.dirty = true;
            true
        } else {
            false
        }
    }

    pub fn save(&mut self) -> Result<(), String> {
        if !self.dirty {
            return Ok(());
        }
        let raw = serde_json::to_string_pretty(&self.loadouts)
            .map_err(|e| format!("loadouts encode: {e}"))?;
        write_private_file(&self.path, raw.as_bytes()).map_err(|e| e.to_string())?;
        self.dirty = false;
        Ok(())
    }
}

/// Combo options for a setting: inline `options` win; `optionsFrom: 'loadouts'`
/// pulls names from the store.
pub fn resolve_setting_options(
    def: &crate::rs2b0t_registry::SettingDef,
    loadouts: &LoadoutsStore,
) -> Vec<String> {
    if !def.options.is_empty() {
        return def.options.clone();
    }
    if def.options_from.as_deref() == Some("loadouts") {
        return loadouts.names();
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::rs2b0t_registry::SettingDef;

    static TMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn tmp_path() -> PathBuf {
        let n = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "274bot-loadouts-test-{n}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("loadouts.json")
    }

    #[test]
    fn store_round_trips_at_private_mode() {
        let path = tmp_path();
        {
            let mut store = LoadoutsStore::at(path.clone());
            store.upsert(Loadout {
                name: "melee".into(),
                worn: vec!["helm".into(), "plate".into()],
                carry: vec!["food".into()],
            });
            store.upsert(Loadout {
                name: "range".into(),
                worn: vec!["coif".into()],
                carry: vec![],
            });
            store.save().expect("save loadouts");
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "loadouts.json must be 0o600");
        }

        let store = LoadoutsStore::at(path);
        assert_eq!(
            store.names(),
            vec!["melee".to_string(), "range".to_string()]
        );
        let melee = store.get("melee").expect("melee loadout");
        assert_eq!(melee.worn, vec!["helm", "plate"]);
        assert_eq!(melee.carry, vec!["food"]);
    }

    #[test]
    fn resolve_setting_options_lists_loadout_names() {
        let path = tmp_path();
        let mut store = LoadoutsStore::at(path);
        store.upsert(Loadout {
            name: "fish".into(),
            worn: vec![],
            carry: vec!["net".into()],
        });
        store.upsert(Loadout {
            name: "mine".into(),
            worn: vec![],
            carry: vec![],
        });
        let def = SettingDef {
            id: "loadout".into(),
            ty: "string".into(),
            default: None,
            label: Some("Loadout".into()),
            min: None,
            max: None,
            step: None,
            options: Vec::new(),
            option_labels: Vec::new(),
            group: None,
            show_if: None,
            options_from: Some("loadouts".into()),
            csv_toggle: None,
            help: None,
        };
        let opts = resolve_setting_options(&def, &store);
        assert_eq!(opts, vec!["fish", "mine"]);
    }

    #[test]
    fn replace_at_renames_without_duplicating() {
        let path = tmp_path();
        let mut store = LoadoutsStore::at(path);
        store.upsert(Loadout {
            name: "melee".into(),
            worn: vec!["helm".into()],
            carry: vec![],
        });
        store.upsert(Loadout {
            name: "range".into(),
            worn: vec![],
            carry: vec![],
        });
        assert!(store.replace_at(
            0,
            Loadout {
                name: "melee2".into(),
                worn: vec!["helm".into()],
                carry: vec![],
            },
        ));
        assert_eq!(store.loadouts().len(), 2);
        assert_eq!(store.loadouts()[0].name, "melee2");
        assert!(store.get("melee").is_none());
        assert!(store.get("melee2").is_some());
    }
}
