/// How a script card is executed in the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptKind {
    Compat,
    NativeTick,
    Compiled,
}

/// Where a script card's metadata came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptSource {
    Catalog,
    File,
    Builtin,
}

/// One catalog-declared selected-item candidate. `key` is the persisted
/// alias; `label` is an optional identity override from the source table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemOptionCandidate {
    pub key: String,
    pub label: Option<String>,
}

/// High-alchemy selected-item choice descriptor. Prefix keys are always
/// published; candidates are filtered/sorted later from borrowed facts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemOptionSpec {
    pub prefix: Vec<String>,
    pub candidates: Vec<ItemOptionCandidate>,
    /// When true, resolved chip keys follow alphabetical label order (frozen
    /// `[...ALCH_ITEMS].sort((a,b)=>a.label.localeCompare(b.label)).map`).
    pub sort_keys_by_label: bool,
}

/// One setting field from a script's `settingsSchema` export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingDef {
    pub id: String,
    pub ty: String,
    pub default: Option<String>,
    pub label: Option<String>,
    pub min: Option<String>,
    pub max: Option<String>,
    pub step: Option<String>,
    pub options: Vec<String>,
    pub option_labels: Vec<String>,
    pub group: Option<String>,
    pub show_if: Option<String>,
    pub options_from: Option<String>,
    pub csv_toggle: Option<String>,
    pub help: Option<String>,
    pub item_option_spec: Option<ItemOptionSpec>,
}

/// One catalog card: register metadata plus the `./…` import path of the
/// script file to read on Start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryCard {
    pub name: String,
    pub rel_path: String,
    pub description: String,
    pub category: String,
    pub tags: Vec<String>,
    pub version: String,
    pub settings_schema: Vec<SettingDef>,
    pub kind: ScriptKind,
    pub source: ScriptSource,
}

/// A picker selection: a compiled id or a loaded JS card by `(source, name)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptSel {
    Compiled(crate::registry::CompiledId),
    Loaded(ScriptSource, String),
}

impl ScriptSel {
    /// The label the picker shows and Start keys on.
    pub fn label(&self) -> String {
        match self {
            ScriptSel::Compiled(id) => id.0.to_string(),
            ScriptSel::Loaded(_, name) => name.clone(),
        }
    }
}
