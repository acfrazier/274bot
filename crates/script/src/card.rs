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

/// Frozen-source equipment / drop tables that W1 must publish. The host
/// must not invent names here; empty options are the honest schema.
pub fn is_revision_fact_option_ident(ident: &str) -> bool {
    matches!(
        ident,
        "STAFFS"
            | "BOWS"
            | "CROSSBOWS"
            | "DARTS"
            | "ARROWS"
            | "BOLTS"
            | "AXES"
            | "MELEE_WEAPONS"
            | "ROCK_CRAB_RANGED_WEAPONS"
            | "FOOD_OPTIONS"
            | "DROP_DB"
    )
}

/// W1c `equipment_names` families backing a frozen settings `options` /
/// `optionsFrom` ident. `RANGED_WEAPONS` and `ROCK_CRAB_RANGED_WEAPONS` mirror
/// `.superpowers/release-0.1.9/reference/rs2b0t-00d39a17e0/src/bot/api/combat/ranged.ts`
/// (`[...BOWS, ...DARTS]`). `AXES` and `DROP_DB` stay outside W1c.
pub fn w1c_equipment_option_families(ident: &str) -> Option<&'static [&'static str]> {
    Some(match ident {
        "STAFFS" => &["staffs"],
        "BOWS" => &["bows"],
        "CROSSBOWS" => &["crossbows"],
        "DARTS" => &["darts"],
        "ARROWS" => &["arrows"],
        "BOLTS" => &["bolts"],
        "MELEE_WEAPONS" => &["melee_weapons"],
        "RANGED_WEAPONS" | "ROCK_CRAB_RANGED_WEAPONS" => &["bows", "darts"],
        _ => return None,
    })
}

/// Host-owned finite option tables for imported / parent-dir identifiers
/// whose bodies are not in the same-directory settings blob. Values are
/// copied from the frozen rs2b0t pin; this is not a JS evaluator.
///
/// `BANK_LOCATIONS` / `BANK_LOCATION_OPTIONS` are the exception: a bank name
/// is drawn from the same frozen roster as runtime `BANK_LOCATIONS`, including
/// gated entries (eligibility is evaluated when selecting a destination).
pub fn catalog_option_table(ident: &str) -> Option<&'static [&'static str]> {
    Some(match ident {
        "COOK_LOCATION_OPTIONS" => &[
            "Auto",
            "Varrock East",
            "Varrock West",
            "Al Kharid",
            "Draynor",
            "Falador East",
            "Falador West",
            "Edgeville",
            "Seers",
            "Catherby",
            "Yanille",
            "Ardougne West",
            "Ardougne East",
            "Canifis",
            "Shilo Village",
            "Fishing Guild",
            "Shantay Pass",
            "Mage Arena",
            "Grand Tree",
            "Duel Arena",
            "Custom",
        ],
        "LOG_LEVELS" => &[
            "Logs",
            "Oak logs",
            "Willow logs",
            "Maple logs",
            "Yew logs",
            "Magic logs",
        ],
        "FIRE_SPOTS" => &["Varrock East", "Varrock West", "Draynor", "Seers"],
        "HERB_OPTIONS" => &[
            "Guam leaf",
            "Marrentill",
            "Tarromin",
            "Harralander",
            "Ranarr weed",
            "Toadflax",
            "Irit leaf",
            "Avantoe",
            "Kwuarm",
            "Snapdragon",
            "Cadantine",
            "Lantadyme",
            "Dwarf weed",
            "Torstol",
            "Snake weed",
            "Ardrigal",
            "Sito foil",
            "Volencia moss",
            "Rogues purse",
        ],
        "RUNE_OPTIONS" => &[
            "Air rune",
            "Mind rune",
            "Water rune",
            "Earth rune",
            "Fire rune",
            "Body rune",
            "Cosmic rune",
            "Chaos rune",
            "Nature rune",
            "Law rune",
            "Death rune",
        ],
        "BANK_LOCATIONS" | "BANK_LOCATION_OPTIONS" => bank_location_names(ident),
        "PRODUCT_OPTIONS" => &[
            "Gold ring",
            "Sapphire ring",
            "Emerald ring",
            "Ruby ring",
            "Diamond ring",
            "Dragonstone ring",
            "Gold necklace",
            "Sapphire necklace",
            "Emerald necklace",
            "Ruby necklace",
            "Diamond necklace",
            "Dragonstone necklace",
            "Gold amulet",
            "Sapphire amulet",
            "Emerald amulet",
            "Ruby amulet",
            "Diamond amulet",
            "Dragonstone amulet",
        ],
        "JEWEL_OPTIONS" => &[
            "Sapphire ring",
            "Sapphire necklace",
            "Sapphire amulet",
            "Emerald ring",
            "Emerald amulet",
            "Ruby ring",
            "Ruby amulet",
            "Diamond ring",
            "Diamond amulet",
            "Dragonstone ring",
            "Dragonstone amulet",
        ],
        _ => return None,
    })
}

fn bank_location_names(ident: &str) -> &'static [&'static str] {
    /// The complete frozen roster, in the runtime payload's stable order.
    const BANKS: [&str; api::named_banks::BANK_CATALOG.len()] = {
        let mut names = [""; api::named_banks::BANK_CATALOG.len()];
        let mut i = 0;
        while i < names.len() {
            names[i] = api::named_banks::BANK_CATALOG[i].name;
            i += 1;
        }
        names
    };
    const WITH_NEAREST: [&str; api::named_banks::BANK_CATALOG.len() + 1] = {
        let mut names = ["Nearest"; api::named_banks::BANK_CATALOG.len() + 1];
        let mut i = 0;
        while i < api::named_banks::BANK_CATALOG.len() {
            names[i + 1] = api::named_banks::BANK_CATALOG[i].name;
            i += 1;
        }
        names
    };
    if ident == "BANK_LOCATION_OPTIONS" {
        &WITH_NEAREST
    } else {
        &BANKS
    }
}
