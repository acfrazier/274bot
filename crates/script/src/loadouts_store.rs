//! Operator loadouts persisted at `~/.274bot/loadouts.json` (0o600).
//!
//! Wire shape is `{ name, worn: {slot: name}, carry: [{item, qty}], unassigned? }`.
//! Legacy `{ worn: [name], carry: [name] }` files are read without guessing
//! slots or dropping items. `optionsFrom: 'loadouts'` fills setting combos
//! from [`LoadoutsStore::names`].

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use vault::write_private_file;

/// Loadout equipment slots, named as the content wearpos decoder names them.
pub const WORN_SLOTS: [&str; 11] = [
    "hat",
    "back",
    "front",
    "righthand",
    "torso",
    "lefthand",
    "legs",
    "hands",
    "feet",
    "ring",
    "quiver",
];

/// Compact equipment layout used by the native editor. `None` is a spacer.
pub const WORN_SLOT_LAYOUT: [[Option<&'static str>; 3]; 5] = [
    [None, Some("hat"), None],
    [Some("back"), Some("front"), Some("quiver")],
    [Some("righthand"), Some("torso"), Some("lefthand")],
    [None, Some("legs"), None],
    [Some("hands"), Some("feet"), Some("ring")],
];

pub fn worn_slot_label(slot: &str) -> &'static str {
    match slot {
        "hat" => "Hat",
        "back" => "Cape",
        "front" => "Amulet",
        "righthand" => "Weapon",
        "torso" => "Body",
        "lefthand" => "Shield",
        "legs" => "Legs",
        "hands" => "Hands",
        "feet" => "Feet",
        "ring" => "Ring",
        "quiver" => "Quiver",
        _ => "Slot",
    }
}

pub fn is_worn_slot(slot: &str) -> bool {
    WORN_SLOTS.contains(&slot)
}

/// One carried supply with a required positive quantity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CarryEntry {
    pub item: String,
    pub qty: u32,
}

impl CarryEntry {
    pub fn new(item: impl Into<String>, qty: u32) -> Self {
        Self {
            item: item.into(),
            qty: qty.max(1),
        }
    }
}

/// One named equipment/inventory preset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loadout {
    pub name: String,
    pub worn: BTreeMap<String, String>,
    pub unassigned: Vec<String>,
    pub carry: Vec<CarryEntry>,
}

impl Loadout {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            worn: BTreeMap::new(),
            unassigned: Vec::new(),
            carry: Vec::new(),
        }
    }

    pub fn with_slot(mut self, slot: &str, item: impl Into<String>) -> Self {
        if is_worn_slot(slot) {
            self.worn.insert(slot.to_string(), item.into());
        } else {
            self.unassigned.push(item.into());
        }
        self
    }

    pub fn with_carry(mut self, item: impl Into<String>, qty: u32) -> Self {
        self.carry.push(CarryEntry::new(item, qty));
        self
    }

    pub fn set_slot(&mut self, slot: &str, item: Option<String>) {
        if !is_worn_slot(slot) {
            return;
        }
        match item {
            Some(name) if !name.trim().is_empty() => {
                self.worn.insert(slot.to_string(), name);
            }
            _ => {
                self.worn.remove(slot);
            }
        }
    }
}

impl Serialize for Loadout {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Wire<'a> {
            name: &'a str,
            worn: &'a BTreeMap<String, String>,
            carry: &'a [CarryEntry],
            #[serde(skip_serializing_if = "slice_empty")]
            unassigned: &'a [String],
        }
        Wire {
            name: &self.name,
            worn: &self.worn,
            carry: &self.carry,
            unassigned: &self.unassigned,
        }
        .serialize(serializer)
    }
}

fn slice_empty(value: &[String]) -> bool {
    value.is_empty()
}

impl<'de> Deserialize<'de> for Loadout {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        parse_loadout(&value).ok_or_else(|| serde::de::Error::custom("invalid loadout"))
    }
}

fn parse_loadout(value: &serde_json::Value) -> Option<Loadout> {
    let name = value.get("name")?.as_str()?.trim();
    if name.is_empty() {
        return None;
    }
    let mut worn = BTreeMap::new();
    let mut unassigned = Vec::new();
    match value.get("worn") {
        Some(serde_json::Value::Array(rows)) => {
            for row in rows {
                if let Some(item) = row.as_str().map(str::trim).filter(|s| !s.is_empty()) {
                    unassigned.push(item.to_string());
                }
            }
        }
        Some(serde_json::Value::Object(map)) => {
            for (slot, raw) in map {
                let Some(item) = raw.as_str().map(str::trim).filter(|s| !s.is_empty()) else {
                    continue;
                };
                if is_worn_slot(slot) {
                    worn.insert(slot.clone(), item.to_string());
                } else {
                    unassigned.push(item.to_string());
                }
            }
        }
        _ => {}
    }
    if let Some(serde_json::Value::Array(rows)) = value.get("unassigned") {
        for row in rows {
            if let Some(item) = row.as_str().map(str::trim).filter(|s| !s.is_empty()) {
                unassigned.push(item.to_string());
            }
        }
    }
    let mut carry = Vec::new();
    if let Some(serde_json::Value::Array(rows)) = value.get("carry") {
        for row in rows {
            if let Some(item) = row.as_str().map(str::trim).filter(|s| !s.is_empty()) {
                carry.push(CarryEntry::new(item, 1));
                continue;
            }
            let Some(item) = row
                .get("item")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            else {
                continue;
            };
            let qty = row
                .get("qty")
                .and_then(|v| {
                    v.as_u64()
                        .or_else(|| v.as_i64().and_then(|n| u64::try_from(n).ok()))
                        .or_else(|| v.as_f64().and_then(|n| (n >= 1.0).then_some(n as u64)))
                })
                .unwrap_or(1);
            carry.push(CarryEntry::new(item, qty.min(u32::MAX as u64) as u32));
        }
    }
    Some(Loadout {
        name: name.to_string(),
        worn,
        unassigned,
        carry,
    })
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
            .and_then(|raw| parse_loadouts_file(&raw))
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

    pub fn snapshot(&self) -> Vec<Loadout> {
        self.loadouts.clone()
    }

    pub fn restore(&mut self, loadouts: Vec<Loadout>) {
        self.loadouts = loadouts;
        self.dirty = false;
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

    pub fn is_dirty(&self) -> bool {
        self.dirty
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

fn parse_loadouts_file(raw: &str) -> Option<Vec<Loadout>> {
    let payload: serde_json::Value = serde_json::from_str(raw).ok()?;
    let rows = payload.as_array()?;
    Some(rows.iter().filter_map(parse_loadout).collect())
}

pub fn unique_loadout_name(existing: &[Loadout], base: &str) -> String {
    let taken: Vec<String> = existing
        .iter()
        .map(|l| l.name.to_ascii_lowercase())
        .collect();
    if !taken.iter().any(|n| n == &base.to_ascii_lowercase()) {
        return base.to_string();
    }
    for n in 2.. {
        let candidate = format!("{base} {n}");
        if !taken
            .iter()
            .any(|name| name == &candidate.to_ascii_lowercase())
        {
            return candidate;
        }
    }
    base.to_string()
}

/// Copy observed equipment into worn slots. Supplies are left unchanged.
/// Unknown slot identity is retained as unassigned gear.
pub fn copy_equipment_preserving_supplies(
    loadout: &mut Loadout,
    items: &[(String, i32)],
    data: Option<&api::game_data::SelectedGameData>,
) {
    let mut worn = BTreeMap::new();
    let mut unassigned = Vec::new();
    for (name, id) in items {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            continue;
        }
        let slot = data.and_then(|facts| {
            facts
                .item_by_id(*id)
                .and_then(api::game_data::GameItem::loadout_slot)
                .or_else(|| {
                    facts.items().iter().find_map(|item| {
                        (item.name.as_deref() == Some(trimmed) && !item.is_certificate())
                            .then(|| item.loadout_slot())
                            .flatten()
                    })
                })
        });
        match slot {
            Some(slot) if !worn.contains_key(slot) => {
                worn.insert(slot.to_string(), trimmed.to_string());
            }
            _ => unassigned.push(trimmed.to_string()),
        }
    }
    loadout.worn = worn;
    loadout.unassigned = unassigned;
}

/// Combo values and identity labels in the same order. Labels fall back to
/// values when the source does not supply a distinct identity string.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ResolvedSettingOptions {
    pub values: Vec<String>,
    pub labels: Vec<String>,
}

impl ResolvedSettingOptions {
    fn from_values(values: Vec<String>) -> Self {
        let labels = values.clone();
        Self { values, labels }
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn label_for<'a>(&'a self, value: &'a str) -> &'a str {
        self.values
            .iter()
            .position(|v| v == value)
            .and_then(|i| self.labels.get(i))
            .map(String::as_str)
            .unwrap_or(value)
    }
}

/// Combo options for a setting: inline `options` win; `optionsFrom: 'loadouts'`
/// pulls names from the store; a high-alchemy item spec is resolved from
/// borrowed selected facts without mutating the schema.
pub fn resolve_setting_options(
    def: &crate::rs2b0t_registry::SettingDef,
    loadouts: &LoadoutsStore,
    game_data: Option<&api::game_data::SelectedGameData>,
) -> Vec<String> {
    resolve_setting_options_with_labels(def, loadouts, game_data).values
}

/// Like [`resolve_setting_options`], with parallel identity labels for UI.
pub fn resolve_setting_options_with_labels(
    def: &crate::rs2b0t_registry::SettingDef,
    loadouts: &LoadoutsStore,
    game_data: Option<&api::game_data::SelectedGameData>,
) -> ResolvedSettingOptions {
    if !def.options.is_empty() {
        return ResolvedSettingOptions::from_values(def.options.clone());
    }
    if def.options_from.as_deref() == Some("loadouts") {
        return ResolvedSettingOptions::from_values(loadouts.names());
    }
    if let Some(spec) = def.item_option_spec.as_ref() {
        return resolve_item_option_spec(spec, game_data);
    }
    ResolvedSettingOptions::default()
}

fn resolve_item_option_spec(
    spec: &crate::rs2b0t_registry::ItemOptionSpec,
    game_data: Option<&api::game_data::SelectedGameData>,
) -> ResolvedSettingOptions {
    let mut values = spec.prefix.clone();
    let mut labels = spec.prefix.clone();
    let Some(data) = game_data else {
        return ResolvedSettingOptions { values, labels };
    };
    struct Row {
        key: String,
        label: String,
        alch: i32,
    }
    let mut rows = Vec::new();
    for candidate in &spec.candidates {
        let Some(item) = data.item_by_alias(&candidate.key) else {
            continue;
        };
        let Some(name) = item.name.as_deref().filter(|n| !n.is_empty()) else {
            continue;
        };
        let label = candidate
            .label
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or(name)
            .to_string();
        let alch = (f64::from(item.cost) * 0.6).floor() as i32;
        rows.push(Row {
            key: candidate.key.clone(),
            label,
            alch,
        });
    }
    rows.sort_by(|a, b| b.alch.cmp(&a.alch).then_with(|| a.label.cmp(&b.label)));
    for row in rows {
        values.push(row.key);
        labels.push(row.label);
    }
    ResolvedSettingOptions { values, labels }
}

/// Adapt the host's persisted loadout shape for script accessors.
pub fn selected_compat_loadout(rows: &[Loadout], wanted: &str) -> serde_json::Value {
    let Some(row) = rows
        .iter()
        .find(|r| r.name.eq_ignore_ascii_case(wanted.trim()))
        .or_else(|| rows.first())
    else {
        return serde_json::Value::Null;
    };
    loadout_to_compat(row)
}

pub fn loadout_to_compat(row: &Loadout) -> serde_json::Value {
    let mut worn = serde_json::Map::new();
    for (slot, item) in &row.worn {
        worn.insert(slot.clone(), serde_json::Value::String(item.clone()));
    }
    let mut out = serde_json::Map::new();
    out.insert("name".into(), serde_json::Value::String(row.name.clone()));
    out.insert("worn".into(), serde_json::Value::Object(worn));
    out.insert(
        "carry".into(),
        serde_json::to_value(&row.carry).unwrap_or_else(|_| serde_json::json!([])),
    );
    if !row.unassigned.is_empty() {
        out.insert(
            "unassigned".into(),
            serde_json::to_value(&row.unassigned).unwrap_or_else(|_| serde_json::json!([])),
        );
    }
    serde_json::Value::Object(out)
}

pub fn gear_of(loadout: &serde_json::Value) -> Vec<String> {
    if loadout.is_null() {
        return Vec::new();
    }
    let mut out = Vec::new();
    if let Some(worn) = loadout.get("worn").and_then(|v| v.as_object()) {
        for slot in WORN_SLOTS {
            if let Some(item) = worn.get(slot).and_then(|v| v.as_str()).map(str::trim) {
                if !item.is_empty() {
                    out.push(item.to_string());
                }
            }
        }
    }
    if let Some(rows) = loadout.get("unassigned").and_then(|v| v.as_array()) {
        for row in rows {
            if let Some(item) = row.as_str().map(str::trim).filter(|s| !s.is_empty()) {
                out.push(item.to_string());
            }
        }
    }
    out
}

pub fn supplies_of(loadout: &serde_json::Value) -> Vec<CarryEntry> {
    if loadout.is_null() {
        return Vec::new();
    }
    loadout
        .get("carry")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|row| {
            let item = row
                .get("item")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())?;
            let qty = row
                .get("qty")
                .and_then(|v| v.as_u64())
                .filter(|n| *n > 0)
                .unwrap_or(1) as u32;
            Some(CarryEntry::new(item, qty))
        })
        .collect()
}

pub fn weapon_of(loadout: &serde_json::Value, fallback: Option<&str>) -> serde_json::Value {
    if let Some(name) = loadout
        .get("worn")
        .and_then(|v| v.get("righthand"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return serde_json::Value::String(name.to_string());
    }
    match fallback {
        Some(name) => serde_json::Value::String(name.to_string()),
        None => serde_json::Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::rs2b0t_registry::{ItemOptionCandidate, ItemOptionSpec, SettingDef};

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
            store.upsert(
                Loadout::new("melee")
                    .with_slot("hat", "helm")
                    .with_slot("torso", "plate")
                    .with_carry("food", 1),
            );
            store.upsert(Loadout::new("range").with_slot("hat", "coif"));
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
        assert_eq!(melee.worn.get("hat").map(String::as_str), Some("helm"));
        assert_eq!(melee.worn.get("torso").map(String::as_str), Some("plate"));
        assert_eq!(melee.carry, vec![CarryEntry::new("food", 1)]);
    }

    #[test]
    fn legacy_array_worn_and_carry_are_preserved_without_guessing_slots() {
        let path = tmp_path();
        std::fs::write(
            &path,
            r#"[{"name":"old","worn":["helm","plate"],"carry":["Lobster"]}]"#,
        )
        .unwrap();
        let store = LoadoutsStore::at(path.clone());
        let row = store.get("old").expect("legacy loadout");
        assert!(row.worn.is_empty(), "array order must not invent slots");
        assert_eq!(row.unassigned, vec!["helm", "plate"]);
        assert_eq!(row.carry, vec![CarryEntry::new("Lobster", 1)]);

        let mut store = LoadoutsStore::at(path.clone());
        let mut row = store.get("old").unwrap().clone();
        row.name = "old-renamed".into();
        store.replace_at(0, row);
        store.save().unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(
            raw.contains("helm"),
            "rename/save must keep unassigned gear"
        );
        assert!(raw.contains("Lobster"));
        assert!(!raw.contains("\"worn\": ["), "save writes the slot map");
    }

    #[test]
    fn explicit_slots_and_quantities_survive_file_round_trip() {
        let path = tmp_path();
        let mut store = LoadoutsStore::at(path.clone());
        store.upsert(
            Loadout::new("melee")
                .with_slot("righthand", "Rune scimitar")
                .with_carry("Lobster", 10)
                .with_carry("Prayer potion(4)", 2),
        );
        store.save().unwrap();
        let store = LoadoutsStore::at(path);
        let row = store.get("melee").unwrap();
        assert_eq!(
            row.worn.get("righthand").map(String::as_str),
            Some("Rune scimitar")
        );
        assert_eq!(row.carry[0], CarryEntry::new("Lobster", 10));
        assert_eq!(row.carry[1], CarryEntry::new("Prayer potion(4)", 2));
    }

    #[test]
    fn failed_save_can_restore_the_previous_file_image() {
        let path = tmp_path();
        let mut store = LoadoutsStore::at(path.clone());
        store.upsert(Loadout::new("keep").with_carry("Lobster", 4));
        store.save().unwrap();
        let backup = store.snapshot();
        store.upsert(Loadout::new("keep").with_carry("Shark", 9));
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir_all(&path).unwrap();
        assert!(store.save().is_err());
        store.restore(backup);
        assert_eq!(
            store.get("keep").unwrap().carry[0],
            CarryEntry::new("Lobster", 4)
        );
        assert!(!store.is_dirty());
    }

    #[test]
    fn copy_equipment_keeps_supplies_and_unknown_gear() {
        let mut loadout = Loadout::new("melee").with_carry("Lobster", 8);
        copy_equipment_preserving_supplies(
            &mut loadout,
            &[("Rune scimitar".into(), 1333), ("Mystery hat".into(), 0)],
            Some(&api::game_data::for_revision(client::io::ClientRevision::R274).unwrap()),
        );
        assert_eq!(
            loadout.worn.get("righthand").map(String::as_str),
            Some("Rune scimitar")
        );
        assert_eq!(loadout.unassigned, vec!["Mystery hat"]);
        assert_eq!(loadout.carry, vec![CarryEntry::new("Lobster", 8)]);
    }

    #[test]
    fn resolve_setting_options_lists_loadout_names() {
        let path = tmp_path();
        let mut store = LoadoutsStore::at(path);
        store.upsert(Loadout::new("fish").with_carry("net", 1));
        store.upsert(Loadout::new("mine"));
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
            item_option_spec: None,
        };
        let opts = resolve_setting_options(&def, &store, None);
        assert_eq!(opts, vec!["fish", "mine"]);
    }

    fn cand(key: &str, label: Option<&str>) -> ItemOptionCandidate {
        ItemOptionCandidate {
            key: key.into(),
            label: label.map(str::to_string),
        }
    }

    fn alcher_spec(extra: &[ItemOptionCandidate]) -> ItemOptionSpec {
        let mut candidates = vec![
            cand("maple_longbow", None),
            cand("yew_longbow", None),
            cand("magic_longbow", None),
            cand("steel_platebody", None),
            cand("steel_platelegs", None),
            cand("steel_2h_sword", None),
            cand("black_platebody", None),
            cand("mithril_platebody", None),
            cand("mithril_platelegs", None),
            cand("mithril_kiteshield", None),
            cand("mithril_2h_sword", None),
            cand("adamant_platebody", None),
            cand("adamant_platelegs", None),
            cand("adamant_kiteshield", None),
            cand("adamant_2h_sword", None),
            cand("rune_platebody", None),
            cand("rune_platelegs", None),
            cand("rune_kiteshield", None),
            cand("rune_chainbody", None),
            cand("rune_full_helm", None),
            cand("rune_sq_shield", None),
            cand("rune_scimitar", None),
            cand("rune_2h_sword", None),
            cand("dragonhide_body", Some("Green d'hide body")),
            cand("blue_dragonhide_body", Some("Blue d'hide body")),
            cand("red_dragonhide_body", Some("Red d'hide body")),
            cand("black_dragonhide_body", Some("Black d'hide body")),
            cand("dragonhide_chaps", Some("Green d'hide chaps")),
            cand("blue_dragonhide_chaps", Some("Blue d'hide chaps")),
            cand("red_dragonhide_chaps", Some("Red d'hide chaps")),
            cand("black_dragonhide_chaps", Some("Black d'hide chaps")),
            cand("battlestaff", None),
            cand("air_battlestaff", None),
            cand("water_battlestaff", None),
            cand("earth_battlestaff", None),
            cand("fire_battlestaff", None),
        ];
        candidates.extend_from_slice(extra);
        ItemOptionSpec {
            prefix: vec!["custom".into()],
            candidates,
        }
    }

    fn item_def(spec: ItemOptionSpec) -> SettingDef {
        SettingDef {
            id: "items".into(),
            ty: "string[]".into(),
            default: None,
            label: Some("Items".into()),
            min: None,
            max: None,
            step: None,
            options: Vec::new(),
            option_labels: Vec::new(),
            group: None,
            show_if: None,
            options_from: None,
            csv_toggle: None,
            help: None,
            item_option_spec: Some(spec),
        }
    }

    const FAITHFUL_ALCH_OPTIONS: &[&str] = &[
        "custom",
        "rune_platebody",
        "rune_2h_sword",
        "rune_platelegs",
        "rune_kiteshield",
        "rune_chainbody",
        "rune_sq_shield",
        "rune_full_helm",
        "rune_scimitar",
        "air_battlestaff",
        "earth_battlestaff",
        "fire_battlestaff",
        "water_battlestaff",
        "black_dragonhide_body",
        "adamant_platebody",
        "red_dragonhide_body",
        "blue_dragonhide_body",
        "dragonhide_body",
        "battlestaff",
        "adamant_2h_sword",
        "adamant_platelegs",
        "black_dragonhide_chaps",
        "adamant_kiteshield",
        "mithril_platebody",
        "red_dragonhide_chaps",
        "blue_dragonhide_chaps",
        "dragonhide_chaps",
        "black_platebody",
        "mithril_2h_sword",
        "mithril_platelegs",
        "magic_longbow",
        "mithril_kiteshield",
        "steel_platebody",
        "yew_longbow",
        "steel_2h_sword",
        "steel_platelegs",
        "maple_longbow",
    ];

    #[test]
    fn resolve_item_options_matches_both_pinned_revisions() {
        let path = tmp_path();
        let store = LoadoutsStore::at(path);
        let def = item_def(alcher_spec(&[]));
        let r274 = api::game_data::for_revision(client::io::ClientRevision::R274).unwrap();
        let r289 = api::game_data::for_revision(client::io::ClientRevision::R289).unwrap();
        let a = resolve_setting_options_with_labels(&def, &store, Some(r274.as_ref()));
        let b = resolve_setting_options_with_labels(&def, &store, Some(r289.as_ref()));
        assert_eq!(
            a.values.iter().map(String::as_str).collect::<Vec<_>>(),
            FAITHFUL_ALCH_OPTIONS
        );
        assert_eq!(b.values, a.values);
        assert_eq!(a.values[1], "rune_platebody");
        let hide_ids: Vec<i32> = [
            "dragonhide_body",
            "blue_dragonhide_body",
            "red_dragonhide_body",
            "black_dragonhide_body",
        ]
        .iter()
        .map(|k| r274.item_by_alias(k).unwrap().id)
        .collect();
        assert_eq!(hide_ids, vec![1135, 2499, 2501, 2503]);
        assert!(!a.values.iter().any(|k| k == "castlewars_armour_body"));
        assert!(!b.values.iter().any(|k| k == "castlewars_armour_body"));
        assert!(def.options.is_empty());
    }

    #[test]
    fn resolve_item_options_none_facts_prefix_only() {
        let path = tmp_path();
        let store = LoadoutsStore::at(path);
        let def = item_def(alcher_spec(&[]));
        let opts = resolve_setting_options(&def, &store, None);
        assert_eq!(opts, vec!["custom".to_string()]);
        assert!(!opts.iter().any(|k| k == "maple_longbow"));
    }

    #[test]
    fn resolve_item_options_drops_missing_alias_and_keeps_sort() {
        let path = tmp_path();
        let store = LoadoutsStore::at(path);
        let def = item_def(alcher_spec(&[cand("not_a_real_selected_item", None)]));
        let data = api::game_data::for_revision(client::io::ClientRevision::R274).unwrap();
        let opts = resolve_setting_options(&def, &store, Some(data.as_ref()));
        assert!(!opts.iter().any(|k| k == "not_a_real_selected_item"));
        assert_eq!(
            opts.iter().map(String::as_str).collect::<Vec<_>>(),
            FAITHFUL_ALCH_OPTIONS
        );
    }

    #[test]
    fn resolve_item_options_tie_order_and_identity_labels() {
        let path = tmp_path();
        let store = LoadoutsStore::at(path);
        let def = item_def(alcher_spec(&[]));
        let data = api::game_data::for_revision(client::io::ClientRevision::R274).unwrap();
        let resolved = resolve_setting_options_with_labels(&def, &store, Some(data.as_ref()));
        let two_h = resolved
            .values
            .iter()
            .position(|k| k == "rune_2h_sword")
            .unwrap();
        let legs = resolved
            .values
            .iter()
            .position(|k| k == "rune_platelegs")
            .unwrap();
        assert!(two_h < legs, "rune 2h before platelegs");
        let staves = [
            "air_battlestaff",
            "earth_battlestaff",
            "fire_battlestaff",
            "water_battlestaff",
        ];
        let idx: Vec<_> = staves
            .iter()
            .map(|k| resolved.values.iter().position(|v| v == k).unwrap())
            .collect();
        assert!(idx.windows(2).all(|w| w[0] < w[1]), "Air/Earth/Fire/Water");
        assert_eq!(resolved.label_for("dragonhide_body"), "Green d'hide body");
        assert_ne!(resolved.label_for("dragonhide_body"), "Dragonhide body");
        assert_eq!(resolved.label_for("custom"), "custom");
    }

    #[test]
    fn resolve_item_options_rebounds_without_mutating_schema() {
        let path = tmp_path();
        let store = LoadoutsStore::at(path);
        let def = item_def(alcher_spec(&[]));
        let r274 = api::game_data::for_revision(client::io::ClientRevision::R274).unwrap();
        let r289 = api::game_data::for_revision(client::io::ClientRevision::R289).unwrap();
        let none = resolve_setting_options(&def, &store, None);
        let a = resolve_setting_options(&def, &store, Some(r274.as_ref()));
        let b = resolve_setting_options(&def, &store, Some(r289.as_ref()));
        assert_eq!(none, vec!["custom".to_string()]);
        assert_eq!(a, b);
        assert!(def.options.is_empty());
        assert_eq!(def.item_option_spec.as_ref().unwrap().prefix, ["custom"]);
    }

    #[test]
    fn resolve_item_options_leaves_literals_and_loadouts() {
        let path = tmp_path();
        let mut store = LoadoutsStore::at(path);
        store.upsert(Loadout::new("fish"));
        let literal = SettingDef {
            id: "material".into(),
            ty: "string".into(),
            default: None,
            label: None,
            min: None,
            max: None,
            step: None,
            options: vec!["Logs".into(), "Oak logs".into()],
            option_labels: Vec::new(),
            group: None,
            show_if: None,
            options_from: None,
            csv_toggle: None,
            help: None,
            item_option_spec: Some(alcher_spec(&[])),
        };
        let data = api::game_data::for_revision(client::io::ClientRevision::R274).unwrap();
        assert_eq!(
            resolve_setting_options(&literal, &store, Some(data.as_ref())),
            vec!["Logs".to_string(), "Oak logs".to_string()],
            "inline options win over item spec"
        );
    }

    #[test]
    fn replace_at_renames_without_duplicating() {
        let path = tmp_path();
        let mut store = LoadoutsStore::at(path);
        store.upsert(Loadout::new("melee").with_slot("hat", "helm"));
        store.upsert(Loadout::new("range"));
        assert!(store.replace_at(0, Loadout::new("melee2").with_slot("hat", "helm")));
        assert_eq!(store.loadouts().len(), 2);
        assert_eq!(store.loadouts()[0].name, "melee2");
        assert!(store.get("melee").is_none());
        assert!(store.get("melee2").is_some());
    }

    #[test]
    fn unique_loadout_name_avoids_collisions() {
        let rows = vec![Loadout::new("loadout"), Loadout::new("loadout 2")];
        assert_eq!(unique_loadout_name(&rows, "loadout"), "loadout 3");
        assert_eq!(unique_loadout_name(&rows, "fresh"), "fresh");
    }
}

#[cfg(test)]
mod compatibility_tests {
    use super::*;
    #[test]
    fn selected_loadout_matches_case_and_falls_back_to_first() {
        let rows = vec![
            Loadout::new("First")
                .with_carry("Coins", 1)
                .with_carry("Lobster", 1),
            Loadout::new("Second").with_carry("Shark", 1),
        ];
        assert_eq!(selected_compat_loadout(&rows, " SECOND ")["name"], "Second");
        assert_eq!(
            selected_compat_loadout(&rows, "missing")["carry"][1]["item"],
            "Lobster"
        );
        assert_eq!(
            selected_compat_loadout(&rows, "missing")["carry"][1]["qty"],
            1
        );
        assert!(selected_compat_loadout(&[], "").is_null());
    }

    #[test]
    fn accessors_read_righthand_qty_and_unassigned_gear() {
        let row = Loadout::new("melee")
            .with_slot("righthand", "Rune scimitar")
            .with_slot("torso", "Rune chainbody")
            .with_carry("Lobster", 10);
        let mut row = row;
        row.unassigned.push("old helm".into());
        let json = loadout_to_compat(&row);
        assert_eq!(weapon_of(&json, Some("Bronze sword")), "Rune scimitar");
        assert_eq!(supplies_of(&json), vec![CarryEntry::new("Lobster", 10)]);
        assert_eq!(
            gear_of(&json),
            vec!["Rune scimitar", "Rune chainbody", "old helm"]
        );
        assert_eq!(
            weapon_of(&serde_json::Value::Null, None),
            serde_json::Value::Null
        );
        assert_eq!(
            weapon_of(&serde_json::Value::Null, Some("Bronze sword")),
            "Bronze sword"
        );
        assert!(gear_of(&serde_json::Value::Null).is_empty());
        assert!(supplies_of(&serde_json::Value::Null).is_empty());
    }
}
