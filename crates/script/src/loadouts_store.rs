//! Operator loadouts persisted at `<state root>/loadouts.json` (0o600).
//!
//! Wire shape is `{ name, worn: {slot: name}, carry: [{item, qty}], unassigned? }`.
//! Legacy `{ worn: [name], carry: [name] }` files are read without guessing
//! slots or dropping items. `optionsFrom: 'loadouts'` fills setting combos
//! from [`LoadoutsStore::names`]. The neutral values (`Loadout`,
//! `CarryEntry`, the worn-slot table) live in `api::loadout`; this module
//! keeps the frontends' form editor and the behaviour helpers.
//!
//! The form reads and writes nothing itself: it loads from and saves
//! through a [`LoadoutsBacking`], which the composition root binds to the
//! process local-state service (T5.2), so a form save and a slot's lease go
//! through one writer. A save is a list of typed edits against the version
//! the form loaded: an edit made meanwhile by another writer is a conflict,
//! never overwritten. A renamed row is one [`LoadoutOp::Rename`], so it
//! keeps its place and is never removed in one write and re-added in
//! another. Slots and native cards read operator loadouts through the
//! same service ([`crate::local_state::SlotState`]).

use std::collections::BTreeMap;
use std::sync::Arc;

use api::loadout::{CarryEntry, Loadout, WORN_SLOTS};
use api::local_state::{LoadoutOp, StoreVersion};

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

/// The operator loadouts at one store version: what a form loads, and what
/// the store holds after each save.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadoutsBaseline {
    pub rows: Vec<Loadout>,
    pub version: StoreVersion,
}

/// Why a form's save did not land whole.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadoutsSaveError {
    /// Another writer changed the store after the form loaded it: what was
    /// left of the save was not written. `latest` is what the store holds
    /// now.
    Conflict {
        latest: LoadoutsBaseline,
        reason: String,
    },
    /// The store refused or failed the write, or did not settle it in time.
    Failed(String),
}

/// Where a frontend's loadout form loads from and saves to: the process
/// local-state service, bound by the composition root (host-play
/// implements it). Both calls block, bounded, on an operator action, as
/// the old file read and write did.
pub trait LoadoutsBacking: Send + Sync {
    /// The rows as the store holds them now, read again from their file.
    fn load(&self) -> Result<LoadoutsBaseline, String>;

    /// Apply `ops` in order, the first expecting the store at `expected`
    /// and each later one at the version the one before committed, waiting
    /// until each is durable. What the store holds after the last.
    fn commit(
        &self,
        ops: Vec<LoadoutOp>,
        expected: StoreVersion,
    ) -> Result<LoadoutsBaseline, LoadoutsSaveError>;
}

/// The operator's loadout list as a form edits it.
pub struct LoadoutsStore {
    backing: Option<Arc<dyn LoadoutsBacking>>,
    loadouts: Vec<Loadout>,
    /// For each row of `loadouts`, the name of the `base` row it edits
    /// (`None` for a row the form added): a row whose name differs from
    /// its origin is a rename of that row.
    origins: Vec<Option<String>>,
    /// What the store held at `version` when this form last loaded or
    /// saved: a save writes the difference from it.
    base: Vec<Loadout>,
    version: StoreVersion,
    dirty: bool,
    /// Why the last load failed, while it stands.
    load_error: Option<String>,
}

impl Default for LoadoutsStore {
    fn default() -> Self {
        Self::detached(Vec::new())
    }
}

impl LoadoutsStore {
    /// The form over `backing`, loaded from it now. A failed load leaves
    /// the form empty, keeps the reason ([`Self::load_error`]), and the
    /// store refuses saves into a file it refused to read.
    pub fn over(backing: Arc<dyn LoadoutsBacking>) -> Self {
        let mut store = Self {
            backing: Some(backing),
            ..Self::detached(Vec::new())
        };
        if let Err(error) = store.reload() {
            store.load_error = Some(error);
        }
        store
    }

    /// Rows with nowhere to save them (a setting's option list, a test):
    /// [`Self::save`] refuses.
    pub fn detached(loadouts: Vec<Loadout>) -> Self {
        Self {
            backing: None,
            origins: origins_of(&loadouts),
            base: loadouts.clone(),
            loadouts,
            version: StoreVersion::INITIAL,
            dirty: false,
            load_error: None,
        }
    }

    /// Read the rows again from the store, dropping unsaved edits.
    pub fn reload(&mut self) -> Result<(), String> {
        let Some(backing) = &self.backing else {
            return Err(NOT_BOUND.into());
        };
        let loaded = backing.load()?;
        self.adopt(loaded);
        self.load_error = None;
        Ok(())
    }

    /// Why the last load failed, while it stands.
    pub fn load_error(&self) -> Option<&str> {
        self.load_error.as_deref()
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

    /// Drop unsaved edits: the rows return to what the store held when
    /// this form last loaded or saved.
    pub fn discard(&mut self) {
        self.loadouts = self.base.clone();
        self.origins = origins_of(&self.base);
        self.dirty = false;
    }

    pub fn upsert(&mut self, loadout: Loadout) {
        self.dirty = true;
        if let Some(existing) = self.loadouts.iter_mut().find(|l| l.name == loadout.name) {
            *existing = loadout;
        } else {
            self.loadouts.push(loadout);
            self.origins.push(None);
        }
    }

    /// Replace the loadout at `index`, including renames (does not key on
    /// name): the row keeps its place, and a save writes a rename as one
    /// edit. Refused (`false`, nothing changes) when `index` is out of
    /// range or another row already has the new name: rows are keyed by
    /// name in the store, so saving that rename would replace the other row.
    pub fn replace_at(&mut self, index: usize, loadout: Loadout) -> bool {
        if index >= self.loadouts.len()
            || self
                .loadouts
                .iter()
                .enumerate()
                .any(|(at, row)| at != index && row.name == loadout.name)
        {
            return false;
        }
        self.dirty = true;
        self.loadouts[index] = loadout;
        true
    }

    pub fn remove(&mut self, name: &str) -> bool {
        let Some(index) = self.loadouts.iter().position(|l| l.name == name) else {
            return false;
        };
        self.loadouts.remove(index);
        self.origins.remove(index);
        self.dirty = true;
        true
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Write the form's edits through the store: one typed edit per row
    /// added, changed, renamed or removed since the form loaded, each
    /// expecting the version the one before left, and each durable before
    /// this returns. A rename is one edit that replaces the row where it
    /// stands, so a failed save never leaves the store without the row
    /// under either name, and a renamed row keeps its place. On success the
    /// form holds what the store holds. When another writer changed the
    /// store first, nothing more is written and the conflict is the error;
    /// the form then holds what the store holds now with its own edits
    /// applied on top (save again to retry them against it, or
    /// [`Self::discard`] them), so a retry never removes the other writer's
    /// rows. A rename whose row the other writer removed, or whose new name
    /// it took, is not applied, and the error says so. While a load
    /// error stands, every save refuses and writes nothing. The form
    /// never saw the saved rows, so a save from it could replace a
    /// row it never saw. This is the only call that writes to the
    /// backing, so the refusal covers adds, renames and removals.
    pub fn save(&mut self) -> Result<(), String> {
        if let Some(error) = &self.load_error {
            return Err(format!(
                "the loadouts did not load ({error}), so nothing was saved"
            ));
        }
        if !self.dirty {
            return Ok(());
        }
        let Some(backing) = self.backing.clone() else {
            return Err(NOT_BOUND.into());
        };
        let ops = edits_between(&self.base, &self.loadouts, &self.origins);
        if ops.is_empty() {
            self.dirty = false;
            return Ok(());
        }
        match backing.commit(ops.clone(), self.version) {
            Ok(after) => {
                self.adopt(after);
                Ok(())
            }
            Err(LoadoutsSaveError::Conflict { latest, reason }) => {
                let notes = self.replay(latest, &ops);
                let mut error = format!(
                    "{reason}; the loadouts changed since this list was loaded, so the rest \
                     was not written"
                );
                for note in notes {
                    error.push_str("; ");
                    error.push_str(&note);
                }
                Err(error)
            }
            Err(LoadoutsSaveError::Failed(reason)) => Err(reason),
        }
    }

    fn adopt(&mut self, baseline: LoadoutsBaseline) {
        self.origins = origins_of(&baseline.rows);
        self.base = baseline.rows.clone();
        self.loadouts = baseline.rows;
        self.version = baseline.version;
        self.dirty = false;
    }

    /// Hold `latest` with `ops` applied the way the store applies them (an
    /// upsert replaces the row of its name or appends, a removal drops that
    /// name, a rename replaces its row in place), keeping the form dirty.
    /// A rename the store would refuse is left out; what it says about each
    /// is returned.
    fn replay(&mut self, latest: LoadoutsBaseline, ops: &[LoadoutOp]) -> Vec<String> {
        self.origins = origins_of(&latest.rows);
        self.loadouts = latest.rows.clone();
        self.base = latest.rows;
        self.version = latest.version;
        let mut notes = Vec::new();
        for op in ops {
            match op {
                LoadoutOp::Upsert(row) => self.upsert(row.clone()),
                LoadoutOp::Remove { name } => {
                    self.remove(name);
                }
                LoadoutOp::Rename { from, loadout } => {
                    match self.loadouts.iter().position(|row| row.name == *from) {
                        None => notes.push(format!(
                            "\"{from}\" is no longer saved, so renaming it to \"{}\" was not \
                             kept",
                            loadout.name
                        )),
                        Some(index) => {
                            if !self.replace_at(index, loadout.clone()) {
                                notes.push(format!(
                                    "A loadout named \"{}\" already exists.",
                                    loadout.name
                                ));
                            }
                        }
                    }
                }
            }
        }
        self.dirty = true;
        notes
    }
}

fn origins_of(rows: &[Loadout]) -> Vec<Option<String>> {
    rows.iter().map(|row| Some(row.name.clone())).collect()
}

const NOT_BOUND: &str = "loadouts are not bound to the local-state service";

/// The typed edits that turn `base` into `rows` (each row of `rows` from
/// the `base` row named by its `origins` entry, or new): a removal for each
/// base row no row comes from, then a rename for each row whose name
/// differs from its origin's, then an upsert for each row added or
/// changed, in `rows` order. A rename onto a name another rename frees
/// goes after that one; a cycle of renames is left in order, and the store
/// refuses its first (names key the rows).
fn edits_between(base: &[Loadout], rows: &[Loadout], origins: &[Option<String>]) -> Vec<LoadoutOp> {
    let mut ops: Vec<LoadoutOp> = base
        .iter()
        .filter(|old| {
            !origins
                .iter()
                .any(|origin| origin.as_ref() == Some(&old.name))
        })
        .map(|old| LoadoutOp::Remove {
            name: old.name.clone(),
        })
        .collect();
    let mut renames: Vec<(&String, &Loadout)> = rows
        .iter()
        .zip(origins)
        .filter_map(|(row, origin)| {
            origin
                .as_ref()
                .filter(|from| **from != row.name)
                .map(|from| (from, row))
        })
        .collect();
    while !renames.is_empty() {
        let ready = renames
            .iter()
            .position(|(_, row)| !renames.iter().any(|(from, _)| **from == row.name))
            .unwrap_or(0);
        let (from, row) = renames.remove(ready);
        ops.push(LoadoutOp::Rename {
            from: from.clone(),
            loadout: row.clone(),
        });
    }
    ops.extend(
        rows.iter()
            .zip(origins)
            .filter(|(row, origin)| match origin {
                None => true,
                Some(from) => {
                    *from == row.name && base.iter().find(|old| old.name == row.name) != Some(*row)
                }
            })
            .map(|(row, _)| LoadoutOp::Upsert(row.clone())),
    );
    ops
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
/// pulls names from the store; high-alchemy item specs and generated food
/// options resolve from borrowed selected facts without mutating the schema.
/// Recognized imported catalog tables are a last-resort host metadata lookup.
/// W1c equipment idents resolve from borrowed `equipment_names` facts; `AXES`
/// / `DROP_DB` stay empty. `gather:<skill>` resolves from the pinned
/// `gather_resources` slice and `named-banks` from the frozen `BANK_CATALOG`.
/// `gatherer-teleports` combines a literal `Off` with available selected spell facts.
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
    let mixed_equipment_options = def.options.last().is_some_and(|s| s == "Other")
        && def.options_from.as_deref().is_some_and(|from| {
            from.split(',')
                .all(|ident| crate::rs2b0t_registry::w1c_equipment_option_families(ident).is_some())
        });
    if !def.options.is_empty()
        && !mixed_equipment_options
        && !matches!(
            def.options_from.as_deref(),
            Some("named-banks" | "gatherer-teleports")
        )
    {
        return inline_setting_options(def);
    }
    if def.options_from.as_deref() == Some("loadouts") {
        return ResolvedSettingOptions::from_values(loadouts.names());
    }
    if let Some(spec) = def.item_option_spec.as_ref() {
        return resolve_item_option_spec(spec, game_data);
    }
    if let Some(from) = def.options_from.as_deref() {
        if from
            .split(',')
            .all(|ident| crate::rs2b0t_registry::w1c_equipment_option_families(ident).is_some())
        {
            let mut resolved = ResolvedSettingOptions::default();
            for family in from.split(',') {
                let part = resolve_w1c_equipment_options(family, game_data);
                resolved.values.extend(part.values);
                resolved.labels.extend(part.labels);
            }
            for literal in &def.options {
                resolved.values.push(literal.clone());
                resolved.labels.push(literal.clone());
            }
            return resolved;
        }
        if from == "FOOD_OPTIONS" {
            return resolve_food_options(game_data);
        }
        if from == "gatherer-food" {
            return resolve_gatherer_food_options(game_data);
        }
        if crate::rs2b0t_registry::is_revision_fact_option_ident(from) {
            return resolve_w1c_equipment_options(from, game_data);
        }
        if from == "gather:sites" {
            // Site eligibility depends on the settings bag, owned by frontend-core.
            return ResolvedSettingOptions::default();
        }
        if let Some(skill) = from.strip_prefix("gather:") {
            return resolve_gather_options(skill, game_data);
        }
        if from == "named-banks" {
            let mut values = def.options.clone();
            values.extend(
                api::named_banks::BANK_CATALOG
                    .iter()
                    .map(|bank| bank.name.to_string()),
            );
            return ResolvedSettingOptions::from_values(values);
        }
        if from == "gatherer-teleports" {
            let mut values = def.options.clone();
            if let Some(data) = game_data {
                for spell in data.teleports().iter().filter(|spell| spell.available()) {
                    if !values
                        .iter()
                        .any(|value| value.eq_ignore_ascii_case(&spell.name))
                    {
                        values.push(spell.name.clone());
                    }
                }
            }
            return ResolvedSettingOptions::from_values(values);
        }
        if let Some(table) = crate::rs2b0t_registry::catalog_option_table(from) {
            return ResolvedSettingOptions::from_values(
                table.iter().map(|s| (*s).to_string()).collect(),
            );
        }
    }
    ResolvedSettingOptions::default()
}

fn inline_setting_options(def: &crate::rs2b0t_registry::SettingDef) -> ResolvedSettingOptions {
    ResolvedSettingOptions {
        values: def.options.clone(),
        labels: if def.option_labels.len() == def.options.len() {
            def.option_labels.clone()
        } else {
            def.options.clone()
        },
    }
}

fn equipment_row_is_selectable(row: &api::game_data::EquipmentNameEntry) -> bool {
    row.disposition == "resolved"
        && row.id.is_some()
        && row
            .alias
            .as_ref()
            .is_some_and(|alias| !alias.trim().is_empty())
}
fn resolve_food_options(
    game_data: Option<&api::game_data::SelectedGameData>,
) -> ResolvedSettingOptions {
    let Some(data) = game_data else {
        return ResolvedSettingOptions::default();
    };
    ResolvedSettingOptions::from_values(
        data.fixed_food_options_best_first()
            .into_iter()
            .map(str::to_string)
            .collect(),
    )
}
fn resolve_gatherer_food_options(
    game_data: Option<&api::game_data::SelectedGameData>,
) -> ResolvedSettingOptions {
    let Some(data) = game_data else {
        return ResolvedSettingOptions::default();
    };
    let mut values = vec![String::new()];
    let mut labels = vec!["None".to_string()];
    for name in data.fixed_food_options_best_first() {
        if !values.iter().any(|value| value.eq_ignore_ascii_case(name)) {
            values.push(name.to_string());
            labels.push(name.to_string());
        }
    }
    ResolvedSettingOptions { values, labels }
}

fn resolve_w1c_equipment_options(
    ident: &str,
    game_data: Option<&api::game_data::SelectedGameData>,
) -> ResolvedSettingOptions {
    let Some(families) = crate::rs2b0t_registry::w1c_equipment_option_families(ident) else {
        return ResolvedSettingOptions::default();
    };
    let Some(data) = game_data else {
        return ResolvedSettingOptions::default();
    };
    let Some(facts) = data.equipment_names() else {
        return ResolvedSettingOptions::default();
    };
    let mut values = Vec::new();
    let mut labels = Vec::new();
    for family in families {
        let Some(rows) = facts.family(family) else {
            continue;
        };
        for row in rows {
            if !equipment_row_is_selectable(row) {
                continue;
            }
            let key = row.requested_name.clone();
            let label = row
                .selected_name
                .as_deref()
                .filter(|name| !name.is_empty())
                .unwrap_or(&key)
                .to_string();
            values.push(key);
            labels.push(label);
        }
    }
    ResolvedSettingOptions { values, labels }
}

/// Pinned gather options for one skill (`gather:<skill>`): values are the
/// selectable keys (resource keys for woodcutting/mining, grouped keys for
/// fishing) in generator order with parallel display labels. Refused rows stay
/// visible with their admission gap code; Start refuses them. Empty without
/// borrowed selected facts.
fn resolve_gather_options(
    skill: &str,
    game_data: Option<&api::game_data::SelectedGameData>,
) -> ResolvedSettingOptions {
    let Some(data) = game_data else {
        return ResolvedSettingOptions::default();
    };
    let mut values = Vec::new();
    let mut labels = Vec::new();
    for row in data.gather_resources_for(skill) {
        values.push(row.key.clone());
        if row.selectable {
            labels.push(row.label.clone());
        } else if let Some(gap) = row.gap.as_deref() {
            labels.push(format!("{} ({gap})", row.label));
        } else {
            labels.push(row.label.clone());
        }
    }
    ResolvedSettingOptions { values, labels }
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
    if spec.sort_keys_by_label {
        rows.sort_by(|a, b| a.label.cmp(&b.label).then_with(|| a.key.cmp(&b.key)));
    } else {
        rows.sort_by(|a, b| b.alch.cmp(&a.alch).then_with(|| a.label.cmp(&b.label)));
    }
    for row in rows {
        values.push(row.key);
        labels.push(row.label);
    }
    ResolvedSettingOptions { values, labels }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadoutResolutionError {
    NoLoadouts {
        requested: String,
    },
    Unknown {
        requested: String,
        available: Vec<String>,
    },
    Ambiguous {
        requested: String,
        matching: Vec<String>,
        available: Vec<String>,
    },
}

impl std::fmt::Display for LoadoutResolutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoLoadouts { requested } => write!(
                f,
                "The saved loadout setting {requested:?} cannot select a loadout because no loadouts exist. Available loadouts: []"
            ),
            Self::Unknown {
                requested,
                available,
            } => write!(
                f,
                "The saved loadout setting {requested:?} does not match a loadout. Available loadouts: {available:?}"
            ),
            Self::Ambiguous {
                requested,
                matching,
                available,
            } => write!(
                f,
                "The saved loadout setting {requested:?} is ambiguous after trimming and ignoring ASCII case. Matching loadouts: {matching:?}. Available loadouts: {available:?}"
            ),
        }
    }
}

impl std::error::Error for LoadoutResolutionError {}

/// Resolve the saved script setting without changing exact Quester Path lookup.
/// Exact names win; blank uses the first row; other names need one unique
/// trimmed, ASCII case-insensitive match.
pub fn resolve_script_loadout<'a>(
    rows: &'a [Loadout],
    wanted: &str,
) -> Result<&'a Loadout, LoadoutResolutionError> {
    if rows.is_empty() {
        return Err(LoadoutResolutionError::NoLoadouts {
            requested: wanted.to_string(),
        });
    }

    if let Some(row) = rows.iter().find(|row| row.name == wanted) {
        return Ok(row);
    }
    let trimmed = wanted.trim();
    if trimmed.is_empty() {
        return rows
            .first()
            .ok_or_else(|| LoadoutResolutionError::NoLoadouts {
                requested: wanted.to_string(),
            });
    }
    let mut matching = rows
        .iter()
        .filter(|row| row.name.trim().eq_ignore_ascii_case(trimmed));
    let Some(row) = matching.next() else {
        return Err(LoadoutResolutionError::Unknown {
            requested: wanted.to_string(),
            available: rows.iter().map(|row| row.name.clone()).collect(),
        });
    };
    if matching.next().is_some() {
        return Err(LoadoutResolutionError::Ambiguous {
            requested: wanted.to_string(),
            matching: rows
                .iter()
                .filter(|row| row.name.trim().eq_ignore_ascii_case(trimmed))
                .map(|row| row.name.clone())
                .collect(),
            available: rows.iter().map(|row| row.name.clone()).collect(),
        });
    }
    Ok(row)
}

/// Resolve a configured `loadout` field; scripts without that setting are not
/// made dependent on operator loadouts.
pub fn resolve_script_loadout_setting<'a>(
    rows: &'a [Loadout],
    wanted: Option<&str>,
) -> Result<Option<&'a Loadout>, LoadoutResolutionError> {
    wanted
        .map(|wanted| resolve_script_loadout(rows, wanted))
        .transpose()
}

/// Exact case-sensitive lookup for Quester Path loadout names.
pub(crate) fn select_loadout<'a>(
    rows: impl IntoIterator<Item = &'a Loadout>,
    wanted: &str,
) -> Option<&'a Loadout> {
    rows.into_iter().find(|row| row.name == wanted)
}

/// Hat-first worn names, then unassigned, after trim and empty filter.
pub fn project_gear<'a, I>(worn: impl Fn(&str) -> Option<&'a str>, unassigned: I) -> Vec<String>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut out = Vec::new();
    for slot in WORN_SLOTS {
        if let Some(item) = worn(slot).map(str::trim).filter(|item| !item.is_empty()) {
            out.push(item.to_string());
        }
    }
    for item in unassigned {
        let item = item.trim();
        if !item.is_empty() {
            out.push(item.to_string());
        }
    }
    out
}

/// Trimmed righthand, otherwise the caller fallback (including empty).
pub fn project_weapon(righthand: Option<&str>, fallback: Option<&str>) -> Option<String> {
    if let Some(name) = righthand.map(str::trim).filter(|name| !name.is_empty()) {
        return Some(name.to_string());
    }
    fallback.map(str::to_string)
}

/// One carry row after trim. Empty item names are dropped.
pub fn carry_entry(item: &str, qty: u32) -> Option<CarryEntry> {
    let item = item.trim();
    if item.is_empty() {
        None
    } else {
        Some(CarryEntry::new(item, qty))
    }
}

pub fn gear_of(loadout: &serde_json::Value) -> Vec<String> {
    if loadout.is_null() {
        return Vec::new();
    }
    let worn = loadout.get("worn").and_then(|v| v.as_object());
    let unassigned = loadout
        .get("unassigned")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|row| row.as_str());
    project_gear(
        |slot| worn.and_then(|map| map.get(slot).and_then(|v| v.as_str())),
        unassigned,
    )
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
            let item = row.get("item").and_then(|v| v.as_str())?;
            let qty = row
                .get("qty")
                .and_then(|v| v.as_u64())
                .filter(|n| *n > 0)
                .unwrap_or(1) as u32;
            carry_entry(item, qty)
        })
        .collect()
}

pub fn weapon_of(loadout: &serde_json::Value, fallback: Option<&str>) -> serde_json::Value {
    match project_weapon(
        loadout
            .get("worn")
            .and_then(|v| v.get("righthand"))
            .and_then(|v| v.as_str()),
        fallback,
    ) {
        Some(name) => serde_json::Value::String(name),
        None => serde_json::Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::rs2b0t_registry::{ItemOptionCandidate, ItemOptionSpec, SettingDef};

    /// A store held in memory: rows at a version, applying each typed edit
    /// the way the service does (an upsert replaces by name or appends, a
    /// removal drops by name, a rename replaces its row in place or is
    /// refused), conflicting when the expectation is stale.
    #[derive(Default)]
    struct Memory {
        state: Mutex<(Vec<Loadout>, u64)>,
        commits: Mutex<Vec<Vec<LoadoutOp>>>,
    }

    fn version(n: u64) -> StoreVersion {
        (0..n).fold(StoreVersion::INITIAL, |v, _| v.checked_next().unwrap())
    }

    impl Memory {
        fn baseline(&self) -> LoadoutsBaseline {
            let state = self.state.lock().unwrap();
            LoadoutsBaseline {
                rows: state.0.clone(),
                version: version(state.1),
            }
        }

        /// Another writer's edit, landing between a form's load and save.
        fn outside(&self, row: Loadout) {
            let mut state = self.state.lock().unwrap();
            state.0.push(row);
            state.1 += 1;
        }

        /// Another writer's removal, landing between a form's load and save.
        fn outside_remove(&self, name: &str) {
            let mut state = self.state.lock().unwrap();
            state.0.retain(|row| row.name != name);
            state.1 += 1;
        }
    }

    impl LoadoutsBacking for Memory {
        fn load(&self) -> Result<LoadoutsBaseline, String> {
            Ok(self.baseline())
        }

        fn commit(
            &self,
            ops: Vec<LoadoutOp>,
            expected: StoreVersion,
        ) -> Result<LoadoutsBaseline, LoadoutsSaveError> {
            self.commits.lock().unwrap().push(ops.clone());
            let mut expected = expected;
            for op in ops {
                let mut state = self.state.lock().unwrap();
                if version(state.1) != expected {
                    drop(state);
                    return Err(LoadoutsSaveError::Conflict {
                        latest: self.baseline(),
                        reason: "version conflict".into(),
                    });
                }
                match op {
                    LoadoutOp::Upsert(row) => {
                        match state.0.iter_mut().find(|old| old.name == row.name) {
                            Some(old) => *old = row,
                            None => state.0.push(row),
                        }
                    }
                    LoadoutOp::Remove { name } => state.0.retain(|old| old.name != name),
                    LoadoutOp::Rename { from, loadout } => {
                        let Some(at) = state.0.iter().position(|old| old.name == from) else {
                            drop(state);
                            return Err(LoadoutsSaveError::Conflict {
                                latest: self.baseline(),
                                reason: "version conflict".into(),
                            });
                        };
                        if state.0.iter().any(|old| old.name == loadout.name) {
                            return Err(LoadoutsSaveError::Failed("duplicate id".into()));
                        }
                        state.0[at] = loadout;
                    }
                }
                state.1 += 1;
                expected = version(state.1);
            }
            Ok(self.baseline())
        }
    }

    /// A backing whose first load fails, as a frontend load that times
    /// out does, while a later load and every commit would succeed. The
    /// store behind it holds rows the failed form never saw.
    struct FailedFirstLoad {
        inner: Arc<Memory>,
        fail_next_load: Mutex<bool>,
    }

    impl LoadoutsBacking for FailedFirstLoad {
        fn load(&self) -> Result<LoadoutsBaseline, String> {
            let mut fail = self.fail_next_load.lock().unwrap();
            if *fail {
                *fail = false;
                return Err("the load timed out".into());
            }
            drop(fail);
            self.inner.load()
        }

        fn commit(
            &self,
            ops: Vec<LoadoutOp>,
            expected: StoreVersion,
        ) -> Result<LoadoutsBaseline, LoadoutsSaveError> {
            self.inner.commit(ops, expected)
        }
    }

    fn form() -> (Arc<Memory>, LoadoutsStore) {
        let memory = Arc::new(Memory::default());
        let store = LoadoutsStore::over(Arc::clone(&memory) as Arc<dyn LoadoutsBacking>);
        (memory, store)
    }

    /// T5.2 r3: a form saves typed edits through its backing, never a file
    /// of its own: one upsert per new or changed row, one removal per row
    /// gone, and afterwards it holds what the store holds.
    #[test]
    fn a_form_saves_typed_edits_through_its_backing() {
        let (memory, mut store) = form();
        store.upsert(
            Loadout::new("melee")
                .with_slot("hat", "helm")
                .with_carry("food", 1),
        );
        store.upsert(Loadout::new("range").with_slot("hat", "coif"));
        store.save().unwrap();
        assert!(!store.is_dirty());
        assert_eq!(memory.baseline().rows, store.loadouts());
        assert_eq!(store.names(), ["melee", "range"]);

        store.remove("melee");
        store.upsert(Loadout::new("range").with_slot("hat", "helm"));
        store.save().unwrap();
        assert_eq!(
            memory.commits.lock().unwrap().last().unwrap(),
            &vec![
                LoadoutOp::Remove {
                    name: "melee".into()
                },
                LoadoutOp::Upsert(Loadout::new("range").with_slot("hat", "helm")),
            ]
        );
        assert_eq!(store.names(), ["range"]);

        // Nothing changed: nothing is written.
        let commits = memory.commits.lock().unwrap().len();
        store.upsert(Loadout::new("range").with_slot("hat", "helm"));
        store.save().unwrap();
        assert_eq!(memory.commits.lock().unwrap().len(), commits);
    }

    /// T5.2 r4: a rename is one edit that replaces the row where it stands,
    /// keeping the row's gear and its place in the list.
    #[test]
    fn a_rename_is_one_edit_and_keeps_the_rows_gear_and_place() {
        let (memory, mut store) = form();
        store.upsert(Loadout::new("old").with_carry("Lobster", 1));
        store.upsert(Loadout::new("other"));
        store.save().unwrap();
        let mut row = store.get("old").unwrap().clone();
        row.name = "old-renamed".into();
        assert!(store.replace_at(0, row.clone()));
        store.save().unwrap();
        assert_eq!(
            memory.commits.lock().unwrap().last().unwrap(),
            &vec![LoadoutOp::Rename {
                from: "old".into(),
                loadout: row,
            }]
        );
        assert_eq!(store.names(), ["old-renamed", "other"]);
        assert_eq!(memory.baseline().rows, store.loadouts());
        assert_eq!(
            store.get("old-renamed").unwrap().carry,
            vec![CarryEntry::new("Lobster", 1)]
        );
    }

    /// T5.2 r4: renames that free and take each other's names are ordered
    /// so each lands (a → b after b → c); a removed row and a new row under
    /// a renamed-away name are a removal and an upsert around them.
    #[test]
    fn chained_renames_are_ordered_and_new_rows_follow_them() {
        let (memory, mut store) = form();
        for name in ["a", "b", "gone"] {
            store.upsert(Loadout::new(name));
        }
        store.save().unwrap();
        assert!(store.replace_at(1, Loadout::new("c")));
        assert!(store.replace_at(0, Loadout::new("b")));
        assert!(store.remove("gone"));
        store.upsert(Loadout::new("a").with_carry("Shark", 2));
        store.save().unwrap();
        assert_eq!(
            memory.commits.lock().unwrap().last().unwrap(),
            &vec![
                LoadoutOp::Remove {
                    name: "gone".into()
                },
                LoadoutOp::Rename {
                    from: "b".into(),
                    loadout: Loadout::new("c"),
                },
                LoadoutOp::Rename {
                    from: "a".into(),
                    loadout: Loadout::new("b"),
                },
                LoadoutOp::Upsert(Loadout::new("a").with_carry("Shark", 2)),
            ]
        );
        assert_eq!(store.names(), ["b", "c", "a"]);
    }

    /// T5.2 r4: a rename whose row another writer removed meanwhile is not
    /// replayed (nothing is re-inserted under the new name), and the error
    /// names it.
    #[test]
    fn a_rename_of_a_row_removed_meanwhile_is_not_replayed_and_is_named() {
        let (memory, mut store) = form();
        store.upsert(Loadout::new("old"));
        store.upsert(Loadout::new("other"));
        store.save().unwrap();
        assert!(store.replace_at(0, Loadout::new("new")));
        memory.outside_remove("old");
        let error = store.save().unwrap_err();
        assert!(
            error.contains("\"old\" is no longer saved, so renaming it to \"new\" was not kept"),
            "{error}"
        );
        assert_eq!(store.names(), ["other"]);
        assert_eq!(memory.baseline().rows, [Loadout::new("other")]);
    }

    /// T5.2 r3: an edit another writer made after the form loaded is never
    /// overwritten: the save conflicts, writes nothing more, keeps the
    /// form's edit on top of what the store holds now, and saving again
    /// keeps both rows.
    #[test]
    fn a_save_after_another_writer_conflicts_and_keeps_both_rows() {
        let (memory, mut store) = form();
        memory.outside(Loadout::new("saved-elsewhere"));
        store.upsert(Loadout::new("mine"));
        let error = store.save().unwrap_err();
        assert!(error.contains("conflict"), "{error}");
        assert!(store.is_dirty(), "the form keeps its edit");
        assert_eq!(memory.baseline().rows, [Loadout::new("saved-elsewhere")]);

        store.save().unwrap();
        let names: Vec<String> = memory
            .baseline()
            .rows
            .iter()
            .map(|row| row.name.clone())
            .collect();
        assert_eq!(names, ["saved-elsewhere", "mine"]);
    }

    /// T5.2 r3: discarding drops unsaved edits back to what the store held;
    /// a detached form refuses to save instead of pretending it did.
    #[test]
    fn discard_returns_to_the_store_and_a_detached_form_refuses_to_save() {
        let (_memory, mut store) = form();
        store.upsert(Loadout::new("keep").with_carry("Lobster", 4));
        store.save().unwrap();
        store.upsert(Loadout::new("keep").with_carry("Shark", 9));
        store.discard();
        assert_eq!(
            store.get("keep").unwrap().carry[0],
            CarryEntry::new("Lobster", 4)
        );
        assert!(!store.is_dirty());

        let mut detached = LoadoutsStore::detached(vec![Loadout::new("fixture")]);
        detached.upsert(Loadout::new("more"));
        assert_eq!(detached.save().unwrap_err(), NOT_BOUND);
        assert!(detached.is_dirty());
    }

    /// A form whose load failed holds no rows and the first store
    /// version. A save from it must refuse and write nothing. Else an
    /// Add of the default name can replace a saved row of that name
    /// that the form never saw. A reload clears the refusal.
    #[test]
    fn a_form_that_failed_to_load_refuses_every_save_and_writes_nothing() {
        let saved = Loadout::new("loadout").with_carry("Lobster", 8);
        let memory = Arc::new(Memory::default());
        // The row the store holds at its first version. A failed form
        // carries that same version, so the version check alone does
        // not stop its save.
        memory.state.lock().unwrap().0.push(saved.clone());
        let backing = Arc::new(FailedFirstLoad {
            inner: Arc::clone(&memory),
            fail_next_load: Mutex::new(true),
        });
        let mut store = LoadoutsStore::over(backing as Arc<dyn LoadoutsBacking>);
        assert_eq!(store.load_error(), Some("the load timed out"));
        assert!(store.loadouts().is_empty());

        // Add, rename and remove each reach the backing only through
        // a save. Each save refuses while the load error stands.
        store.upsert(Loadout::new("loadout"));
        let error = store.save().unwrap_err();
        assert!(error.contains("did not load"), "{error}");
        assert!(store.is_dirty(), "a refused save keeps the form dirty");
        assert!(store.replace_at(0, Loadout::new("renamed")));
        let error = store.save().unwrap_err();
        assert!(error.contains("did not load"), "{error}");
        assert!(store.remove("renamed"));
        let error = store.save().unwrap_err();
        assert!(error.contains("did not load"), "{error}");
        assert!(
            memory.commits.lock().unwrap().is_empty(),
            "nothing reached the backing"
        );
        assert_eq!(memory.baseline().rows, [saved]);

        // A reload that succeeds clears the error and the refusal.
        store.reload().unwrap();
        assert_eq!(store.load_error(), None);
        assert_eq!(store.names(), ["loadout"]);
        store.upsert(Loadout::new("range"));
        store.save().unwrap();
        assert_eq!(store.names(), ["loadout", "range"]);
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
    fn catalog_metadata_options_when_schema_empty() {
        let store = LoadoutsStore::default();
        let def = SettingDef {
            id: "location".into(),
            ty: "string".into(),
            default: Some("Catherby".into()),
            label: None,
            min: None,
            max: None,
            step: None,
            options: Vec::new(),
            option_labels: Vec::new(),
            group: None,
            show_if: None,
            options_from: Some("COOK_LOCATION_OPTIONS".into()),
            csv_toggle: None,
            help: None,
            item_option_spec: None,
        };
        let opts = resolve_setting_options(&def, &store, None);
        assert!(opts.contains(&"Catherby".into()), "{opts:?}");
        assert!(opts.contains(&"Auto".into()), "{opts:?}");
        assert!(opts.contains(&"Custom".into()), "{opts:?}");
    }

    fn equipment_from(ident: &str) -> SettingDef {
        SettingDef {
            id: ident.to_ascii_lowercase(),
            ty: "string".into(),
            default: None,
            label: None,
            min: None,
            max: None,
            step: None,
            options: Vec::new(),
            option_labels: Vec::new(),
            group: None,
            show_if: None,
            options_from: Some(ident.into()),
            csv_toggle: None,
            help: None,
            item_option_spec: None,
        }
    }

    #[test]
    fn food_options_follow_selected_fixed_heal_facts() {
        let store = LoadoutsStore::default();
        let def = equipment_from("FOOD_OPTIONS");
        assert!(resolve_setting_options(&def, &store, None).is_empty());

        let data = api::game_data::for_revision(client::io::ClientRevision::R289).unwrap();
        let options = resolve_setting_options_with_labels(&def, &store, Some(data.as_ref()));
        assert!(options.values.contains(&"Lobster".to_string()));
        assert!(options.values.contains(&"Shark".to_string()));
        assert!(options.values.contains(&"Chocolaty milk".to_string()));
        assert!(options
            .values
            .iter()
            .all(|name| data.fixed_food_heal(name).is_some()));
        assert_eq!(options.labels, options.values);
        let heals: Vec<i32> = options
            .values
            .iter()
            .map(|name| data.fixed_food_heal(name).unwrap())
            .collect();
        assert!(
            heals.windows(2).all(|pair| pair[0] >= pair[1]),
            "food choices list the best heal first: {heals:?}"
        );
    }

    #[test]
    fn gatherer_food_options_start_with_none_and_inline_labels_are_preserved() {
        let store = LoadoutsStore::default();
        let data = api::game_data::for_revision(client::io::ClientRevision::R289).unwrap();
        let food = equipment_from("gatherer-food");
        let no_facts = resolve_setting_options_with_labels(&food, &store, None);
        assert!(
            no_facts.is_empty(),
            "dynamic food options need selected facts"
        );
        let food_options = resolve_setting_options_with_labels(&food, &store, Some(data.as_ref()));
        assert_eq!(food_options.values.first().map(String::as_str), Some(""));
        assert_eq!(
            food_options.labels.first().map(String::as_str),
            Some("None")
        );
        assert!(food_options.values[1..]
            .iter()
            .all(|name| data.fixed_food_heal(name).is_some()));

        let mut inline = equipment_from("not-dynamic");
        inline.options = vec!["quest_id".into()];
        inline.option_labels = vec!["Quest display name".into()];
        inline.options_from = None;
        let inline_options = resolve_setting_options_with_labels(&inline, &store, None);
        assert_eq!(inline_options.values, ["quest_id"]);
        assert_eq!(inline_options.labels, ["Quest display name"]);
    }
    #[test]
    fn w1c_equipment_options_closed_without_game_data() {
        let store = LoadoutsStore::default();
        let def = equipment_from("STAFFS");
        assert!(resolve_setting_options(&def, &store, None).is_empty());
        assert!(resolve_setting_options(&equipment_from("AXES"), &store, None).is_empty());
    }

    #[test]
    fn w1c_equipment_options_match_curated_families_on_both_pins() {
        let store = LoadoutsStore::default();
        let r274 = api::game_data::for_revision(client::io::ClientRevision::R274).unwrap();
        let r289 = api::game_data::for_revision(client::io::ClientRevision::R289).unwrap();
        for ident in [
            "STAFFS",
            "BOWS",
            "CROSSBOWS",
            "DARTS",
            "ARROWS",
            "MELEE_WEAPONS",
        ] {
            let def = equipment_from(ident);
            let a = resolve_setting_options_with_labels(&def, &store, Some(r274.as_ref()));
            let b = resolve_setting_options_with_labels(&def, &store, Some(r289.as_ref()));
            assert_eq!(a, b, "{ident} must match across pins");
            assert_eq!(a.values.len(), a.labels.len());
            assert!(!a.values.is_empty(), "{ident} must have resolved rows");
        }
        let staffs = resolve_setting_options_with_labels(
            &equipment_from("STAFFS"),
            &store,
            Some(r274.as_ref()),
        );
        assert_eq!(staffs.values.len(), 15);
        assert_eq!(staffs.values[0], "Staff");
        assert_eq!(
            staffs.values.last().map(String::as_str),
            Some("Mystic fire staff")
        );
        assert_eq!(staffs.label_for("Staff of air"), "Staff of air");

        let bows = resolve_setting_options(&equipment_from("BOWS"), &store, Some(r274.as_ref()));
        assert_eq!(bows.len(), 12);
        assert_eq!(bows[0], "Shortbow");
        assert_eq!(bows.last().map(String::as_str), Some("Magic longbow"));

        let crossbows =
            resolve_setting_options(&equipment_from("CROSSBOWS"), &store, Some(r274.as_ref()));
        assert_eq!(crossbows, vec!["Crossbow".to_string()]);
        assert!(
            !crossbows.iter().any(|n| n.contains("Karil")),
            "absent crossbow tiers must not appear: {crossbows:?}"
        );

        let arrows =
            resolve_setting_options(&equipment_from("ARROWS"), &store, Some(r274.as_ref()));
        assert_eq!(arrows.len(), 6);
        assert!(
            !arrows.iter().any(|n| n == "Dragon arrow"),
            "Dragon arrow is absent under exact join: {arrows:?}"
        );

        let bolts = resolve_setting_options(&equipment_from("BOLTS"), &store, Some(r274.as_ref()));
        assert!(
            bolts.is_empty(),
            "bolt tiers stay absent; generic Bolts must not be invented: {bolts:?}"
        );

        let ranged = resolve_setting_options_with_labels(
            &equipment_from("RANGED_WEAPONS"),
            &store,
            Some(r274.as_ref()),
        );
        assert_eq!(ranged.values.len(), 19, "12 bows + 7 darts");
        assert_eq!(ranged.values[0], "Shortbow");
        assert_eq!(ranged.values[12], "Bronze dart");
        let rock = resolve_setting_options(
            &equipment_from("ROCK_CRAB_RANGED_WEAPONS"),
            &store,
            Some(r274.as_ref()),
        );
        assert_eq!(rock, ranged.values);

        assert!(
            resolve_setting_options(&equipment_from("AXES"), &store, Some(r274.as_ref()))
                .is_empty()
        );
        assert!(
            resolve_setting_options(&equipment_from("DROP_DB"), &store, Some(r274.as_ref()))
                .is_empty()
        );
    }

    #[test]
    fn recovered_inline_options_still_win_over_metadata() {
        let store = LoadoutsStore::default();
        let def = SettingDef {
            id: "surface".into(),
            ty: "string".into(),
            default: Some("Range".into()),
            label: None,
            min: None,
            max: None,
            step: None,
            options: vec!["Range".into(), "Fire".into()],
            option_labels: Vec::new(),
            group: None,
            show_if: None,
            options_from: Some("COOK_LOCATION_OPTIONS".into()),
            csv_toggle: None,
            help: None,
            item_option_spec: None,
        };
        assert_eq!(
            resolve_setting_options(&def, &store, None),
            vec!["Range".to_string(), "Fire".to_string()]
        );
    }

    #[test]
    fn resolve_setting_options_lists_loadout_names() {
        let mut store = LoadoutsStore::default();
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
            sort_keys_by_label: false,
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
        let store = LoadoutsStore::default();
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
        assert!(!def.item_option_spec.as_ref().unwrap().sort_keys_by_label);
    }

    #[test]
    fn resolve_item_options_label_sort_orders_prefix_then_keys_on_both_pins() {
        let store = LoadoutsStore::default();
        let spec = ItemOptionSpec {
            prefix: vec!["custom".into()],
            candidates: vec![
                cand("maple_longbow", None),
                cand("rune_platebody", None),
                cand("dragonhide_body", Some("Green d'hide body")),
                cand("not_a_real_selected_item", None),
            ],
            sort_keys_by_label: true,
        };
        let def = item_def(spec);
        let r274 = api::game_data::for_revision(client::io::ClientRevision::R274).unwrap();
        let r289 = api::game_data::for_revision(client::io::ClientRevision::R289).unwrap();
        let a = resolve_setting_options_with_labels(&def, &store, Some(r274.as_ref()));
        let b = resolve_setting_options_with_labels(&def, &store, Some(r289.as_ref()));
        let want_keys = [
            "custom",
            "dragonhide_body",
            "maple_longbow",
            "rune_platebody",
        ];
        let want_labels = [
            "custom",
            "Green d'hide body",
            "Maple longbow",
            "Rune platebody",
        ];
        assert_eq!(
            a.values.iter().map(String::as_str).collect::<Vec<_>>(),
            want_keys
        );
        assert_eq!(
            a.labels.iter().map(String::as_str).collect::<Vec<_>>(),
            want_labels
        );
        assert_eq!(b.values, a.values);
        assert_eq!(b.labels, a.labels);
        assert!(!a.values.iter().any(|k| k == "not_a_real_selected_item"));
        assert!(!b.values.iter().any(|k| k == "not_a_real_selected_item"));
        assert_ne!(
            a.values[1], "rune_platebody",
            "label sort must not use richest-first"
        );
    }

    #[test]
    fn resolve_item_options_none_facts_prefix_only() {
        let store = LoadoutsStore::default();
        let def = item_def(alcher_spec(&[]));
        let opts = resolve_setting_options(&def, &store, None);
        assert_eq!(opts, vec!["custom".to_string()]);
        assert!(!opts.iter().any(|k| k == "maple_longbow"));
    }

    #[test]
    fn resolve_item_options_drops_missing_alias_and_keeps_sort() {
        let store = LoadoutsStore::default();
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
        let store = LoadoutsStore::default();
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
        let store = LoadoutsStore::default();
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
        let mut store = LoadoutsStore::default();
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
        let mut store = LoadoutsStore::default();
        store.upsert(Loadout::new("melee").with_slot("hat", "helm"));
        store.upsert(Loadout::new("range"));
        assert!(store.replace_at(0, Loadout::new("melee2").with_slot("hat", "helm")));
        assert_eq!(store.loadouts().len(), 2);
        assert_eq!(store.loadouts()[0].name, "melee2");
        assert!(store.get("melee").is_none());
        assert!(store.get("melee2").is_some());
    }

    /// Rows are keyed by name in the store, so a rename onto another row's
    /// name would replace that row when saved: it is refused instead.
    #[test]
    fn replace_at_refuses_a_rename_onto_another_rows_name() {
        let mut store = LoadoutsStore::default();
        store.upsert(Loadout::new("melee").with_slot("hat", "helm"));
        store.upsert(Loadout::new("range"));
        let before = store.snapshot();
        assert!(!store.replace_at(1, Loadout::new("melee")));
        assert_eq!(store.loadouts(), before.as_slice());
        assert!(!store.replace_at(2, Loadout::new("tank")), "out of range");
        assert!(
            store.replace_at(0, Loadout::new("melee")),
            "a row keeps its own name"
        );
    }

    #[test]
    fn unique_loadout_name_avoids_collisions() {
        let rows = vec![Loadout::new("loadout"), Loadout::new("loadout 2")];
        assert_eq!(unique_loadout_name(&rows, "loadout"), "loadout 3");
        assert_eq!(unique_loadout_name(&rows, "fresh"), "fresh");
    }

    fn gather_game_data() -> api::game_data::SelectedGameData {
        serde_json::from_value(serde_json::json!({
            "schema_version": 5,
            "revision": 289,
            "provenance": {"cache_identity": {"cache_id": "x"}, "inputs": [], "content_inputs": [], "decoder_sources": []},
            "items": [],
            "consumption": [],
            "pickpocket": [],
            "gather_resources": [
                {"skill": "mining", "method": "mining.copper", "methods": ["mining.copper"], "aliases": [], "key": "copper", "resources": ["copper"], "label": "Copper ore", "level": 1, "selectable": true, "gap": null},
                {"skill": "mining", "method": "mining.iron", "methods": ["mining.iron"], "aliases": [], "key": "iron", "resources": ["iron"], "label": "Iron ore", "level": 15, "selectable": true, "gap": null},
                {"skill": "woodcutting", "method": "woodcutting.jungle", "methods": ["woodcutting.jungle"], "aliases": [], "key": "jungle", "resources": ["jungle"], "label": "Jungle", "selectable": false, "gap": "no-resource-target"},
                {"skill": "fishing", "method": "fishing.saltfish.op1", "methods": ["fishing.saltfish.op1"], "aliases": ["fishing.saltfish.op1"], "key": "fishing.net.tool_303.products_317_321", "resources": ["raw_anchovies", "raw_shrimp"], "label": "Raw shrimps / Raw anchovies — Net (Small fishing net)", "level": 0, "selectable": true, "gap": null}
            ]
        }))
        .expect("gather test facts decode")
    }

    #[test]
    fn resolve_gather_options_values_labels_and_gap_suffix() {
        let store = LoadoutsStore::default();
        let data = gather_game_data();
        let mining = resolve_setting_options_with_labels(
            &equipment_from("gather:mining"),
            &store,
            Some(&data),
        );
        assert_eq!(
            mining.values,
            vec!["copper".to_string(), "iron".to_string()]
        );
        assert_eq!(
            mining.labels,
            vec!["Copper ore".to_string(), "Iron ore".to_string()]
        );
        let jungle = resolve_setting_options_with_labels(
            &equipment_from("gather:woodcutting"),
            &store,
            Some(&data),
        );
        assert_eq!(jungle.values, vec!["jungle".to_string()]);
        assert_eq!(
            jungle.labels,
            vec!["Jungle (no-resource-target)".to_string()]
        );
        let fishing = resolve_setting_options_with_labels(
            &equipment_from("gather:fishing"),
            &store,
            Some(&data),
        );
        assert_eq!(
            fishing.values,
            vec!["fishing.net.tool_303.products_317_321".to_string()]
        );
        assert_eq!(
            fishing.labels,
            vec!["Raw shrimps / Raw anchovies — Net (Small fishing net)".to_string()]
        );
    }

    #[test]
    fn resolve_gather_options_empty_without_facts() {
        let store = LoadoutsStore::default();
        assert!(resolve_setting_options(&equipment_from("gather:mining"), &store, None).is_empty());
        let data = api::game_data::for_revision(client::io::ClientRevision::R274).unwrap();
        assert!(
            resolve_setting_options(
                &equipment_from("gather:unknown-skill"),
                &store,
                Some(data.as_ref())
            )
            .is_empty(),
            "an unknown skill is no options, never the whole slice"
        );
    }

    #[test]
    fn resolve_named_banks_keeps_nearest_before_catalog_names() {
        let store = LoadoutsStore::default();
        let mut def = equipment_from("named-banks");
        def.options.push("Nearest".into());
        let banks = resolve_setting_options(&def, &store, None);
        let mut expected = vec!["Nearest".to_string()];
        expected.extend(
            api::named_banks::BANK_CATALOG
                .iter()
                .map(|bank| bank.name.to_string()),
        );
        assert_eq!(banks, expected);
    }

    #[test]
    fn resolve_gatherer_teleports_uses_only_available_selected_spell_facts() {
        let store = LoadoutsStore::default();
        let mut def = equipment_from("gatherer-teleports");
        def.options.push("Off".into());
        let data = api::game_data::for_revision(client::io::ClientRevision::R289).unwrap();
        let resolved = resolve_setting_options_with_labels(&def, &store, Some(data.as_ref()));
        let mut expected = vec!["Off".to_string()];
        for spell in data.teleports().iter().filter(|spell| spell.available()) {
            if !expected
                .iter()
                .any(|value| value.eq_ignore_ascii_case(&spell.name))
            {
                expected.push(spell.name.clone());
            }
        }
        assert_eq!(resolved.values, expected);
        assert_eq!(resolved.labels, expected);

        let no_facts = resolve_setting_options_with_labels(&def, &store, None);
        assert_eq!(no_facts.values, ["Off"]);
    }
}

#[cfg(test)]
mod compatibility_tests {
    use super::*;
    #[test]
    fn saved_script_loadout_uses_default_and_unique_case_insensitive_match() {
        let rows = vec![
            Loadout::new("First")
                .with_carry("Coins", 1)
                .with_carry("Lobster", 1),
            Loadout::new("Second").with_carry("Shark", 1),
        ];
        assert_eq!(resolve_script_loadout(&rows, "").unwrap().name, "First");
        assert_eq!(
            resolve_script_loadout(&rows, " SECOND ").unwrap().name,
            "Second"
        );
        assert_eq!(
            resolve_script_loadout(&rows, "Second").unwrap().name,
            "Second"
        );
        assert!(select_loadout(&rows, " SECOND ").is_none());
    }

    #[test]
    fn saved_script_loadout_refuses_ambiguous_and_unknown_names_with_available_rows() {
        let rows = vec![
            Loadout::new("First"),
            Loadout::new("Second"),
            Loadout::new("SECOND"),
        ];
        assert_eq!(
            resolve_script_loadout(&rows, "Second").unwrap().name,
            "Second"
        );
        let ambiguous = resolve_script_loadout(&rows, " second ")
            .expect_err("two case-insensitive matches must refuse");
        let message = ambiguous.to_string();
        assert!(message.contains("\" second \""), "{message}");
        assert!(message.contains("ambiguous"), "{message}");
        assert!(message.contains("Second"), "{message}");
        assert!(message.contains("SECOND"), "{message}");
        assert!(message.contains("First"), "{message}");

        let unknown =
            resolve_script_loadout(&rows[..2], "missing").expect_err("unknown name must refuse");
        let message = unknown.to_string();
        assert!(message.contains("\"missing\""), "{message}");
        assert!(message.contains("First"), "{message}");
        assert!(message.contains("Second"), "{message}");
    }

    #[test]
    fn blank_script_loadout_refuses_when_no_loadouts_exist() {
        let error = resolve_script_loadout(&[], "").expect_err("there is no first loadout");
        let message = error.to_string();
        assert!(message.contains("\"\""), "{message}");
        assert!(message.contains("Available loadouts: []"), "{message}");
    }

    #[test]
    fn named_script_loadout_refuses_as_no_loadouts_when_store_empty() {
        let error = resolve_script_loadout(&[], "missing").expect_err("there are no loadouts");
        assert!(matches!(error, LoadoutResolutionError::NoLoadouts { .. }));
    }

    #[test]
    fn accessors_read_righthand_qty_and_unassigned_gear() {
        let row = Loadout::new("melee")
            .with_slot("righthand", "Rune scimitar")
            .with_slot("torso", "Rune chainbody")
            .with_carry("Lobster", 10);
        let mut row = row;
        row.unassigned.push("old helm".into());
        let json = serde_json::to_value(&row).unwrap();
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
