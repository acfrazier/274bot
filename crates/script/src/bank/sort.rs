//! Pure bank categories, in-category ranks, and a bounded native sort planner.
//!
//! The game's 289 item names are ASCII. That borrowed fast path uses ASCII-insensitive
//! comparisons without allocating; non-ASCII names use Rust's Unicode lowercase fallback.

use std::borrow::Cow;
use std::cmp::Ordering;

use thiserror::Error;

/// One bank-sort category, in the same order and spelling exposed by the compatibility API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum BankCategory {
    /// Coins.
    Coins,
    /// Runes and rune essence.
    Runes,
    /// Staves.
    Staves,
    /// Ammunition.
    Ammunition,
    /// Melee weapons and bows.
    Weapons,
    /// Melee armour.
    Armour,
    /// Ranged armour.
    RangedArmour,
    /// Food.
    Food,
    /// Potions.
    Potions,
    /// Jewellery.
    Jewellery,
    /// Herbs and herb-making ingredients.
    Herbs,
    /// Ores, bars, keys, and gems.
    OresBarsGems,
    /// Logs and planks.
    Logs,
    /// General supplies.
    Supplies,
    /// Tools.
    Tools,
    /// Teleport items.
    Teleports,
    /// Live quest items.
    QuestLive,
    /// Obsolete quest items.
    QuestObsolete,
    /// Junk.
    Junk,
}

impl BankCategory {
    /// Returns the exact compatibility API category spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Coins => "coins",
            Self::Runes => "runes",
            Self::Staves => "staves",
            Self::Ammunition => "ammunition",
            Self::Weapons => "weapons",
            Self::Armour => "armour",
            Self::RangedArmour => "rangedArmour",
            Self::Food => "food",
            Self::Potions => "potions",
            Self::Jewellery => "jewellery",
            Self::Herbs => "herbs",
            Self::OresBarsGems => "oresBarsGems",
            Self::Logs => "logs",
            Self::Supplies => "supplies",
            Self::Tools => "tools",
            Self::Teleports => "teleports",
            Self::QuestLive => "questLive",
            Self::QuestObsolete => "questObsolete",
            Self::Junk => "junk",
        }
    }

    const fn order_index(self) -> usize {
        self as usize
    }
}

/// The compatibility API order. Every category appears exactly once.
pub const CATEGORY_ORDER: [BankCategory; 19] = [
    BankCategory::Coins,
    BankCategory::Runes,
    BankCategory::Staves,
    BankCategory::Ammunition,
    BankCategory::Weapons,
    BankCategory::Armour,
    BankCategory::RangedArmour,
    BankCategory::Food,
    BankCategory::Potions,
    BankCategory::Jewellery,
    BankCategory::Herbs,
    BankCategory::OresBarsGems,
    BankCategory::Logs,
    BankCategory::Supplies,
    BankCategory::Tools,
    BankCategory::Teleports,
    BankCategory::QuestLive,
    BankCategory::QuestObsolete,
    BankCategory::Junk,
];

/// Borrowed item facts needed by bank sorting.
pub trait BankSortRow {
    /// The row's zero-based bank slot.
    fn slot(&self) -> i32;
    /// The item's object id.
    fn id(&self) -> i32;
    /// The item's optional display name.
    fn name(&self) -> Option<&str>;
    /// The item's catalogue cost.
    fn cost(&self) -> i32;
}

impl BankSortRow for api::snapshot::ItemView {
    fn slot(&self) -> i32 {
        self.slot
    }

    fn id(&self) -> i32 {
        self.def.id
    }

    fn name(&self) -> Option<&str> {
        self.def.name.as_deref()
    }

    fn cost(&self) -> i32 {
        self.def.base_value
    }
}

/// Rank used when no named entry matches within a category.
pub const UNRANKED: i32 = 9999;

const RUNE_IDS: &[i32] = &[
    554, 555, 556, 557, 558, 559, 560, 561, 562, 563, 564, 565, 566, 1436,
];

const AMMUNITION_WORDS: &[&str] = &[
    "arrow",
    "bolt",
    "dart",
    "javelin",
    "knife",
    "knives",
    "cannonball",
];
const POTION_WORDS: &[&str] = &[
    "potion",
    "vial",
    "brew",
    "antipoison",
    "restore",
    "antifire",
    "super attack",
    "super strength",
    "super defence",
    "super energy",
];
const TOOL_WORDS: &[&str] = &[
    "pickaxe",
    "hatchet",
    "hammer",
    "chisel",
    "tinderbox",
    "needle",
    "spade",
    "rope",
    "harpoon",
    "lobster pot",
    "fishing rod",
    "fishing net",
    "bucket",
    "jug",
    "tiara",
];
const WEAPON_WORDS: &[&str] = &[
    "scimitar",
    "mace",
    "dagger",
    "battleaxe",
    "warhammer",
    "halberd",
    "spear",
    "crossbow",
    "wand",
    "claws",
    "whip",
];
const RANGED_ARMOUR_WORDS: &[&str] = &[
    "dragonhide body",
    "dragonhide chaps",
    "dragon vambraces",
    "studded ",
    "hardleather",
    "leather ",
    "coif",
];
const ARMOUR_WORDS: &[&str] = &[
    "platebody",
    "plateskirt",
    "platelegs",
    "chainbody",
    "helm",
    "shield",
    "boots",
    "gloves",
    "gauntlets",
    "vambraces",
    "chaps",
    "cape",
    "cloak",
    "robe",
    "hat",
    "body",
    "legs",
    "skirt",
];
const FOOD_WORDS: &[&str] = &[
    "shark",
    "lobster",
    "swordfish",
    "tuna",
    "trout",
    "salmon",
    "bass",
    "pike",
    "shrimps",
    "anchovies",
    "herring",
    "sardine",
    "mackerel",
    "manta ray",
    "bread",
    "cake",
    "stew",
    "meat",
    "chicken",
    "kebab",
    "wine",
    "beer",
];
const HERB_WORDS: &[&str] = &[
    "grimy",
    "clean ",
    "guam",
    "marrentill",
    "tarromin",
    "harralander",
    "ranarr",
    "irit",
    "avantoe",
    "kwuarm",
    "cadantine",
    "dwarf weed",
    "torstol",
    "lantadyme",
    "snapdragon",
    "snake weed",
    "ardrigal",
    "sito foil",
    "volencia moss",
    "rogues purse",
    "eye of newt",
    "unicorn horn",
    "limpwurt",
    "snape grass",
    "white berries",
    "red spiders",
    "chocolate dust",
    "toadflax",
];
const GEM_WORDS: &[&str] = &[
    "sapphire",
    "emerald",
    "ruby",
    "diamond",
    "dragonstone",
    "opal",
    "jade",
    "topaz",
];
const SUPPLY_WORDS: &[&str] = &[
    "feather",
    "leather",
    "hide",
    "thread",
    "bowstring",
    "bow string",
    "seed",
    "bait",
    "shaft",
    "wool",
    "flax",
    "molten glass",
    "soda ash",
];

/// Returns the first matching rule for a name that has not already been lowercased.
pub fn rule_for(id: i32, item_name: Option<&str>) -> Option<BankCategory> {
    rule_for_name(id, LowercaseName::new(item_name.unwrap_or("")))
}

/// Returns the first matching rule for an already-lowercased compatibility name.
///
/// This keeps the V8 binding's `String.toLowerCase()` behavior while sharing the
/// same Rust-owned rules without lowering the name a second time.
pub fn rule_for_lowercase(id: i32, item_name: &str) -> Option<BankCategory> {
    rule_for_name(
        id,
        LowercaseName {
            text: Cow::Borrowed(item_name),
        },
    )
}

fn rule_for_name(id: i32, name: LowercaseName<'_>) -> Option<BankCategory> {
    if id == 995 {
        return Some(BankCategory::Coins);
    }
    if RUNE_IDS.contains(&id) {
        return Some(BankCategory::Runes);
    }
    if contains_any(&name, AMMUNITION_WORDS) {
        return Some(BankCategory::Ammunition);
    }
    if name.contains("teleport") {
        return Some(BankCategory::Teleports);
    }
    if contains_any(&name, POTION_WORDS) {
        return Some(BankCategory::Potions);
    }
    if contains_any(&name, &["amulet", "necklace", "bracelet"]) || contains_word(&name, "ring") {
        return Some(BankCategory::Jewellery);
    }
    if contains_any(&name, TOOL_WORDS)
        || (ends_with_word(&name, "axe") && !name.contains("battleaxe"))
    {
        return Some(BankCategory::Tools);
    }
    if contains_any(&name, &["bowstring", "bow string"]) {
        return Some(BankCategory::Supplies);
    }
    if name.contains("staff") {
        return Some(BankCategory::Staves);
    }
    if contains_any(&name, WEAPON_WORDS)
        || contains_sword_not_fish(&name)
        || contains_word_after(&name, "bow")
    {
        return Some(BankCategory::Weapons);
    }
    if contains_any(&name, RANGED_ARMOUR_WORDS) {
        return Some(BankCategory::RangedArmour);
    }
    if contains_any(&name, ARMOUR_WORDS) {
        return Some(BankCategory::Armour);
    }
    if contains_any(&name, FOOD_WORDS)
        || contains_word(&name, "cod")
        || contains_word(&name, "pie")
        || name.contains("strange fruit")
    {
        return Some(BankCategory::Food);
    }
    if contains_any(&name, HERB_WORDS) || name.is("herb") {
        return Some(BankCategory::Herbs);
    }
    if name.is("half of a key") || name.is("crystal key") {
        return Some(BankCategory::OresBarsGems);
    }
    if contains_any(&name, GEM_WORDS)
        || ["ore", "bar", "bars", "coal", "gem"]
            .iter()
            .any(|word| contains_word(&name, word))
    {
        return Some(BankCategory::OresBarsGems);
    }
    if name.contains("logs") || name.contains("plank") {
        return Some(BankCategory::Logs);
    }
    if contains_any(&name, SUPPLY_WORDS) {
        return Some(BankCategory::Supplies);
    }
    if id == 526 || id == 532 {
        return Some(BankCategory::Junk);
    }
    None
}

/// Returns the category for a row, defaulting unmatched names to `junk`.
pub fn category_of<R: BankSortRow + ?Sized>(item: &R) -> BankCategory {
    rule_for(item.id(), item.name()).unwrap_or(BankCategory::Junk)
}

/// Returns whether no rule (including an explicit junk id) matched the row.
pub fn is_unmatched<R: BankSortRow + ?Sized>(item: &R) -> bool {
    rule_for(item.id(), item.name()).is_none()
}

#[derive(Debug)]
struct LowercaseName<'a> {
    text: Cow<'a, str>,
}

impl<'a> LowercaseName<'a> {
    fn new(name: &'a str) -> Self {
        let text = if name.is_ascii() {
            Cow::Borrowed(name)
        } else {
            Cow::Owned(name.to_lowercase())
        };
        Self { text }
    }

    fn as_str(&self) -> &str {
        self.text.as_ref()
    }

    fn is(&self, expected: &str) -> bool {
        self.as_str().eq_ignore_ascii_case(expected)
    }

    fn contains(&self, needle: &str) -> bool {
        find_ascii(self.as_str(), needle, 0).is_some()
    }

    fn starts_with_at(&self, start: usize, prefix: &str) -> bool {
        let Some(bytes) = self
            .as_str()
            .as_bytes()
            .get(start..start.saturating_add(prefix.len()))
        else {
            return false;
        };
        bytes.eq_ignore_ascii_case(prefix.as_bytes())
    }

    fn equal_at(&self, start: usize, expected: &str) -> bool {
        self.as_str().len() == start.saturating_add(expected.len())
            && self.starts_with_at(start, expected)
    }
}

fn find_ascii(name: &str, needle: &str, from: usize) -> Option<usize> {
    let bytes = name.as_bytes();
    let pattern = needle.as_bytes();
    if pattern.is_empty() {
        return (from <= bytes.len()).then_some(from);
    }
    if pattern.len() > bytes.len() || from > bytes.len().saturating_sub(pattern.len()) {
        return None;
    }
    (from..=bytes.len() - pattern.len())
        .find(|&start| bytes[start..start + pattern.len()].eq_ignore_ascii_case(pattern))
}

fn contains_any(name: &LowercaseName<'_>, fragments: &[&str]) -> bool {
    fragments.iter().any(|fragment| name.contains(fragment))
}

fn is_js_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn contains_word(name: &LowercaseName<'_>, word: &str) -> bool {
    let text = name.as_str();
    let mut from = 0;
    while let Some(start) = find_ascii(text, word, from) {
        let end = start + word.len();
        if (start == 0 || !is_js_word_byte(text.as_bytes()[start - 1]))
            && (end == text.len() || !is_js_word_byte(text.as_bytes()[end]))
        {
            return true;
        }
        from = start + 1;
    }
    false
}

fn ends_with_word(name: &LowercaseName<'_>, suffix: &str) -> bool {
    let text = name.as_str();
    let Some(start) = text.len().checked_sub(suffix.len()) else {
        return false;
    };
    name.starts_with_at(start, suffix)
        && (start == 0 || !is_js_word_byte(text.as_bytes()[start - 1]))
}

fn contains_word_after(name: &LowercaseName<'_>, fragment: &str) -> bool {
    let text = name.as_str();
    let mut from = 0;
    while let Some(start) = find_ascii(text, fragment, from) {
        let end = start + fragment.len();
        if end == text.len() || !is_js_word_byte(text.as_bytes()[end]) {
            return true;
        }
        from = start + 1;
    }
    false
}

fn contains_sword_not_fish(name: &LowercaseName<'_>) -> bool {
    let text = name.as_str();
    let mut from = 0;
    while let Some(start) = find_ascii(text, "sword", from) {
        let end = start + "sword".len();
        if !name.starts_with_at(end, "fish") {
            return true;
        }
        from = start + 1;
    }
    false
}

const METALS: &[&str] = &[
    "dragon", "rune", "adamant", "mithril", "black", "steel", "iron", "bronze",
];
const ORES_BARS_KEYS_GEMS: &[&str] = &[
    "copper ore",
    "tin ore",
    "iron ore",
    "silver ore",
    "coal",
    "gold ore",
    "mithril ore",
    "adamantite ore",
    "runite ore",
    "bronze bar",
    "iron bar",
    "silver bar",
    "steel bar",
    "gold bar",
    "mithril bar",
    "adamantite bar",
    "runite bar",
    "half of a key",
    "crystal key",
    "dragonstone",
    "diamond",
    "ruby",
    "emerald",
    "sapphire",
    "uncut dragonstone",
    "uncut diamond",
    "uncut ruby",
    "uncut emerald",
    "uncut sapphire",
];
const STAVES: &[&str] = &[
    "mystic lava staff",
    "mystic fire staff",
    "mystic water staff",
    "mystic earth staff",
    "mystic air staff",
    "lava battlestaff",
    "fire battlestaff",
    "water battlestaff",
    "earth battlestaff",
    "air battlestaff",
    "battlestaff",
    "staff of saradomin",
    "staff of guthix",
    "staff of zamorak",
    "staff of iban",
    "staff of fire",
    "staff of water",
    "staff of earth",
    "staff of air",
    "magic staff",
    "dramen staff",
    "staff",
];
const AMMUNITION: &[&str] = &[
    "dragon arrow",
    "rune arrow",
    "adamant arrow",
    "mithril arrow",
    "black arrow",
    "steel arrow",
    "iron arrow",
    "bronze arrow",
    "dragon arrowtips",
    "rune arrowtips",
    "adamant arrowtips",
    "mithril arrowtips",
    "black arrowtips",
    "steel arrowtips",
    "iron arrowtips",
    "bronze arrowtips",
    "headless arrow",
    "barbed bolts",
    "pearl bolts",
    "opal bolts",
    "bolts",
    "barb bolttips",
    "pearl bolttips",
    "opal bolttips",
    "dragon dart",
    "rune dart",
    "adamant dart",
    "mithril dart",
    "black dart",
    "steel dart",
    "iron dart",
    "bronze dart",
    "dragon dart tip",
    "rune dart tip",
    "adamant dart tip",
    "mithril dart tip",
    "black dart tip",
    "steel dart tip",
    "iron dart tip",
    "bronze dart tip",
    "dragon javelin",
    "rune javelin",
    "adamant javelin",
    "mithril javelin",
    "black javelin",
    "steel javelin",
    "iron javelin",
    "bronze javelin",
    "dragon knife",
    "rune knife",
    "adamant knife",
    "mithril knife",
    "black knife",
    "steel knife",
    "iron knife",
    "bronze knife",
    "dragon thrownaxe",
    "rune thrownaxe",
    "adamant thrownaxe",
    "mithril thrownaxe",
    "black thrownaxe",
    "steel thrownaxe",
    "iron thrownaxe",
    "bronze thrownaxe",
    "cannonball",
];
const WEAPONS: &[&str] = &[
    "dragon",
    "rune",
    "adamant",
    "mithril",
    "black",
    "steel",
    "iron",
    "bronze",
    "magic longbow",
    "magic shortbow",
    "yew longbow",
    "yew shortbow",
    "maple longbow",
    "maple shortbow",
    "willow longbow",
    "willow shortbow",
    "oak longbow",
    "oak shortbow",
    "longbow",
    "shortbow",
];
const LOGS: &[&str] = &[
    "logs",
    "achey tree logs",
    "oak logs",
    "willow logs",
    "maple logs",
    "yew logs",
    "magic logs",
];
const HERBS: &[&str] = &[
    "guam leaf",
    "snake weed",
    "ardrigal",
    "sito foil",
    "volencia moss",
    "rogues purse",
    "marrentill",
    "tarromin",
    "harralander",
    "ranarr weed",
    "toadflax",
    "irit leaf",
    "avantoe",
    "kwuarm",
    "snapdragon",
    "cadantine",
    "lantadyme",
    "dwarf weed",
    "torstol",
];
const RANGED_ARMOUR: &[&str] = &[
    "dragonhide body",
    "dragonhide chaps",
    "dragon vambraces",
    "studded body",
    "studded chaps",
    "hardleather body",
    "leather body",
    "leather chaps",
    "leather vambraces",
    "leather gloves",
    "leather boots",
    "coif",
    "leather cowl",
];
const FOOD: &[&str] = &[
    "manta ray",
    "shark",
    "swordfish",
    "bass",
    "lobster",
    "tuna",
    "salmon",
    "pike",
    "cod",
    "trout",
    "mackerel",
    "herring",
    "sardine",
    "anchovies",
    "shrimps",
    "chocolate cake",
    "cake",
    "meat pie",
    "apple pie",
    "redberry pie",
    "stew",
    "pitta bread",
    "bread",
    "chicken",
    "meat",
    "ugthanki kebab",
    "kebab",
    "jug of wine",
    "beer",
];
const POTIONS: &[&str] = &[
    "super attack",
    "super strength",
    "super defence",
    "attack potion",
    "strength potion",
    "defence potion",
    "ranging potion",
    "magic potion",
    "zamorak potion",
    "prayer potion",
    "super restore",
    "restore potion",
    "superantipoison",
    "antipoison",
    "antifire potion",
    "super energy",
    "energy potion",
    "agility potion",
    "fishing potion",
];
const RUNES: &[&str] = &[
    "blood rune",
    "death rune",
    "soul rune",
    "chaos rune",
    "mind rune",
    "body rune",
    "nature rune",
    "cosmic rune",
    "law rune",
    "air rune",
    "earth rune",
    "water rune",
    "fire rune",
    "rune essence",
];
const ARMOUR: &[&str] = METALS;
const DOSES: i32 = 4;
const DHIDE_RANKS: i32 = 12;

const UNID_HERB_RANK: &[(i32, i32)] = &[
    (199, 19),
    (200, 19),
    (1525, 20),
    (1527, 21),
    (1529, 22),
    (1531, 23),
    (1533, 24),
    (201, 25),
    (202, 25),
    (203, 26),
    (204, 26),
    (205, 27),
    (206, 27),
    (207, 28),
    (208, 28),
    (3049, 29),
    (3050, 29),
    (209, 30),
    (210, 30),
    (211, 31),
    (212, 31),
    (213, 32),
    (214, 32),
    (3051, 33),
    (3052, 33),
    (215, 34),
    (216, 34),
    (2485, 35),
    (2486, 35),
    (217, 36),
    (218, 36),
    (219, 37),
    (220, 37),
];

const DHIDE_RANK: &[(i32, i32)] = &[
    (2503, 0),
    (2504, 0),
    (2501, 1),
    (2502, 1),
    (2499, 2),
    (2500, 2),
    (1135, 3),
    (1136, 3),
    (2497, 4),
    (2498, 4),
    (2495, 5),
    (2496, 5),
    (2493, 6),
    (2494, 6),
    (1099, 7),
    (1100, 7),
    (2491, 8),
    (2492, 8),
    (2489, 9),
    (2490, 9),
    (2487, 10),
    (2488, 10),
    (1065, 11),
    (1066, 11),
];

fn name_order(category: BankCategory) -> Option<&'static [&'static str]> {
    match category {
        BankCategory::OresBarsGems => Some(ORES_BARS_KEYS_GEMS),
        BankCategory::Staves => Some(STAVES),
        BankCategory::Ammunition => Some(AMMUNITION),
        BankCategory::Weapons => Some(WEAPONS),
        BankCategory::Armour => Some(ARMOUR),
        BankCategory::RangedArmour => Some(RANGED_ARMOUR),
        BankCategory::Runes => Some(RUNES),
        BankCategory::Logs => Some(LOGS),
        BankCategory::Herbs => Some(HERBS),
        BankCategory::Coins
        | BankCategory::Food
        | BankCategory::Potions
        | BankCategory::Jewellery
        | BankCategory::Supplies
        | BankCategory::Tools
        | BankCategory::Teleports
        | BankCategory::QuestLive
        | BankCategory::QuestObsolete
        | BankCategory::Junk => None,
    }
}

fn table_rank(order: &[&str], name: &LowercaseName<'_>, start: usize) -> i32 {
    if let Some(index) = order.iter().position(|entry| name.equal_at(start, entry)) {
        return index as i32;
    }
    order
        .iter()
        .position(|entry| {
            name.starts_with_at(start, entry)
                && name
                    .as_str()
                    .as_bytes()
                    .get(start + entry.len())
                    .is_some_and(|&byte| byte == b' ' || byte == b'(')
        })
        .map_or(UNRANKED, |index| index as i32)
}

fn food_rank(name: &LowercaseName<'_>) -> i32 {
    let (prefix_len, block) = if name.starts_with_at(0, "raw ") {
        (4, 1)
    } else if name.starts_with_at(0, "burnt ") {
        (6, 2)
    } else if name.starts_with_at(0, "cooked ") {
        (7, 0)
    } else {
        (0, 0)
    };
    let rank = table_rank(FOOD, name, prefix_len);
    if rank == UNRANKED {
        UNRANKED
    } else {
        rank + block * FOOD.len() as i32
    }
}

fn potion_rank(name: &LowercaseName<'_>) -> i32 {
    let Some(without_close) = name.as_str().strip_suffix(')') else {
        return UNRANKED;
    };
    let Some((potion, dose)) = without_close.rsplit_once('(') else {
        return UNRANKED;
    };
    if dose.len() != 1 || !dose.as_bytes()[0].is_ascii_digit() {
        return UNRANKED;
    }
    let Some(potion_rank) = POTIONS
        .iter()
        .position(|entry| potion.eq_ignore_ascii_case(entry))
    else {
        return UNRANKED;
    };
    potion_rank as i32 * DOSES + (DOSES - i32::from(dose.as_bytes()[0] - b'0'))
}

/// Returns an item's position inside its category; unknown names sort after the tables.
pub fn rank_within<R: BankSortRow + ?Sized>(category: BankCategory, item: &R) -> i32 {
    let by_id = match category {
        BankCategory::Herbs => UNID_HERB_RANK
            .iter()
            .find_map(|&(id, rank)| (id == item.id()).then_some(rank)),
        BankCategory::RangedArmour => DHIDE_RANK
            .iter()
            .find_map(|&(id, rank)| (id == item.id()).then_some(rank)),
        _ => None,
    };
    if let Some(rank) = by_id {
        return rank;
    }

    let name = LowercaseName::new(item.name().unwrap_or(""));
    if name.as_str().is_empty() {
        return UNRANKED;
    }
    if category == BankCategory::Food {
        return food_rank(&name);
    }
    if category == BankCategory::Potions {
        return potion_rank(&name);
    }
    let Some(order) = name_order(category) else {
        return UNRANKED;
    };
    let rank = table_rank(order, &name, 0);
    if category == BankCategory::RangedArmour && rank != UNRANKED {
        rank + DHIDE_RANKS
    } else {
        rank
    }
}

/// The arrange mode used to interpret a bank move.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BankSortMode {
    /// Exchange the two indexed slots.
    Swap,
    /// Remove from the source slot and insert at the post-removal target index.
    Insert,
}

/// One zero-based bank-slot move.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BankMove {
    /// Source slot.
    pub from: u16,
    /// Destination slot. Insert mode uses the post-removal index.
    pub to: u16,
}

/// Typed failures from constructing or using a bounded planner.
#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum SortPlanError {
    /// Scratch capacity exceeds the largest representable slot count.
    #[error("bank capacity {capacity} exceeds u16::MAX")]
    CapacityTooLarge {
        /// Requested slot capacity.
        capacity: usize,
    },
    /// The input rows do not fit the caller's scratch.
    #[error("bank has {rows} rows, exceeding scratch capacity {capacity}")]
    RowsExceedCapacity {
        /// Scratch slot capacity.
        capacity: usize,
        /// Number of supplied rows.
        rows: usize,
    },
}

/// Results of one plan, including both strategy costs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanSummary {
    /// Chosen strategy (ties choose swap, unless `force` selected one).
    pub mode: BankSortMode,
    /// Total swaps in the swap strategy.
    pub swap_move_count: usize,
    /// Total inserts in the insert strategy.
    pub insert_move_count: usize,
    /// Total moves in the chosen strategy.
    pub move_count: usize,
    /// Whether the observed slots were dense `0..rows.len()`.
    pub is_dense: bool,
}

/// Reusable caller-owned planner scratch.
///
/// The six full `u16` arrays and one packed anchor bitset use at most
/// `12*N + 2*ceil(N/16)` bytes of backing capacity (below the required
/// `16*N` payload ceiling), plus the fixed `Vec` headers and plan metadata.
/// Rows and move lists are never copied into owned storage. Construct this once
/// for the bank capacity and reuse it for every observed-round re-plan.
pub struct BankSortScratch {
    capacity: usize,
    len: usize,
    dense: bool,
    mode: BankSortMode,
    swap_move_count: usize,
    insert_move_count: usize,
    rows_by_slot: Vec<u16>,
    target_of: Vec<u16>,
    slot_of: Vec<u16>,
    insert_order: Vec<u16>,
    lis_tails: Vec<u16>,
    lis_prev: Vec<u16>,
    anchor_bits: Vec<u16>,
}

impl BankSortScratch {
    /// Creates reusable scratch for a bank with `capacity` slots.
    pub fn new(capacity: usize) -> Result<Self, SortPlanError> {
        if capacity > usize::from(u16::MAX) {
            return Err(SortPlanError::CapacityTooLarge { capacity });
        }
        let anchor_words = capacity.div_ceil(u16::BITS as usize);
        Ok(Self {
            capacity,
            len: 0,
            dense: false,
            mode: BankSortMode::Swap,
            swap_move_count: 0,
            insert_move_count: 0,
            rows_by_slot: vec![0; capacity],
            target_of: vec![0; capacity],
            slot_of: vec![0; capacity],
            insert_order: vec![0; capacity],
            lis_tails: vec![0; capacity],
            lis_prev: vec![u16::MAX; capacity],
            anchor_bits: vec![0; anchor_words],
        })
    }

    /// Returns the bank capacity this scratch was created for.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Plans a bank sort without owning or converting input rows.
    ///
    /// `overrides` is checked before the built-in category table. Rows with
    /// non-dense slots produce the upstream no-move swap plan.
    pub fn plan<R, F>(
        &mut self,
        rows: &[R],
        overrides: F,
        force: Option<BankSortMode>,
    ) -> Result<PlanSummary, SortPlanError>
    where
        R: BankSortRow,
        F: Fn(i32) -> Option<BankCategory>,
    {
        self.len = 0;
        self.dense = false;
        self.mode = BankSortMode::Swap;
        self.swap_move_count = 0;
        self.insert_move_count = 0;
        if rows.len() > self.capacity {
            return Err(SortPlanError::RowsExceedCapacity {
                capacity: self.capacity,
                rows: rows.len(),
            });
        }
        self.len = rows.len();
        for (index, row_index) in self.rows_by_slot[..self.len].iter_mut().enumerate() {
            *row_index = u16::try_from(index).expect("row index is within the u16 capacity");
        }
        self.rows_by_slot[..self.len].sort_unstable_by(|&left, &right| {
            rows[usize::from(left)]
                .slot()
                .cmp(&rows[usize::from(right)].slot())
                .then_with(|| left.cmp(&right))
        });
        self.dense = self.rows_by_slot[..self.len]
            .iter()
            .enumerate()
            .all(|(slot, &row_index)| rows[usize::from(row_index)].slot() == slot as i32);
        if !self.dense {
            for (index, target_slot) in self.slot_of[..self.len].iter_mut().enumerate() {
                *target_slot = u16::try_from(index).expect("slot index is within the u16 capacity");
            }
            return Ok(PlanSummary {
                mode: BankSortMode::Swap,
                swap_move_count: 0,
                insert_move_count: 0,
                move_count: 0,
                is_dense: false,
            });
        }

        for (slot, target) in self.insert_order[..self.len].iter_mut().enumerate() {
            *target = u16::try_from(slot).expect("slot index is within the u16 capacity");
        }
        let len = self.len;
        let rows_by_slot = &self.rows_by_slot;
        self.insert_order[..len].sort_unstable_by(|&left_slot, &right_slot| {
            let left_row = &rows[usize::from(rows_by_slot[usize::from(left_slot)])];
            let right_row = &rows[usize::from(rows_by_slot[usize::from(right_slot)])];
            compare_rows(left_row, right_row, &overrides)
                // JavaScript's array sort is stable; slot order preserves that for equal keys.
                .then_with(|| left_slot.cmp(&right_slot))
        });
        for (target_index, &source_slot) in self.insert_order[..self.len].iter().enumerate() {
            self.target_of[usize::from(source_slot)] =
                u16::try_from(target_index).expect("target index is within the u16 capacity");
            self.slot_of[target_index] = source_slot;
        }

        self.swap_move_count = self.count_swaps();
        self.insert_move_count = self.count_inserts();
        let cheaper = if self.insert_move_count < self.swap_move_count {
            BankSortMode::Insert
        } else {
            BankSortMode::Swap
        };
        self.mode = force.unwrap_or(cheaper);
        let move_count = match self.mode {
            BankSortMode::Swap => self.swap_move_count,
            BankSortMode::Insert => self.insert_move_count,
        };
        Ok(PlanSummary {
            mode: self.mode,
            swap_move_count: self.swap_move_count,
            insert_move_count: self.insert_move_count,
            move_count,
            is_dense: true,
        })
    }

    /// Fills `out` with the first chosen moves, in upstream send order.
    ///
    /// The same first moves are returned on each call; the bank operation
    /// observes its batch and re-plans from the new snapshot.
    pub fn first_moves(&mut self, out: &mut [BankMove]) -> usize {
        let count = out.len().min(match self.mode {
            BankSortMode::Swap => self.swap_move_count,
            BankSortMode::Insert => self.insert_move_count,
        });
        if !self.dense || count == 0 {
            return 0;
        }
        match self.mode {
            BankSortMode::Swap => self.write_swap_moves(out, count),
            BankSortMode::Insert => self.write_insert_moves(out, count),
        }
    }

    /// Returns the item id at `target_index` in the planned layout.
    ///
    /// Use the same `rows` slice supplied to the preceding `plan` call.
    pub fn layout_id<R: BankSortRow>(&self, rows: &[R], target_index: usize) -> Option<i32> {
        if target_index >= self.len {
            return None;
        }
        let source_slot = usize::from(self.slot_of[target_index]);
        let row_index = usize::from(*self.rows_by_slot.get(source_slot)?);
        rows.get(row_index).map(|row| row.id())
    }

    /// Iterates unmatched ids in bank-slot order, excluding category overrides.
    /// A non-dense input has no unmatched result, matching the early plan exit.
    pub fn unmatched_ids<'a, R, F>(
        &'a self,
        rows: &'a [R],
        overrides: F,
    ) -> impl Iterator<Item = i32> + 'a
    where
        R: BankSortRow + 'a,
        F: Fn(i32) -> Option<BankCategory> + 'a,
    {
        let len = if self.dense { self.len } else { 0 };
        self.rows_by_slot[..len]
            .iter()
            .filter_map(move |&row_index| {
                let row = rows.get(usize::from(row_index))?;
                (overrides(row.id()).is_none() && is_unmatched(row)).then_some(row.id())
            })
    }

    fn count_swaps(&mut self) -> usize {
        self.insert_order[..self.len].copy_from_slice(&self.target_of[..self.len]);
        for (slot, &target) in self.target_of[..self.len].iter().enumerate() {
            self.lis_prev[usize::from(target)] =
                u16::try_from(slot).expect("slot index is within the u16 capacity");
        }
        let mut moves = 0;
        for want in 0..self.len {
            let want_u16 = u16::try_from(want).expect("slot index is within the u16 capacity");
            if self.insert_order[want] == want_u16 {
                continue;
            }
            let from = usize::from(self.lis_prev[want]);
            let displaced = usize::from(self.insert_order[want]);
            self.insert_order[want] = want_u16;
            self.insert_order[from] = u16::try_from(displaced).expect("target is within capacity");
            self.lis_prev[want] = want_u16;
            self.lis_prev[displaced] = u16::try_from(from).expect("slot is within capacity");
            moves += 1;
        }
        moves
    }

    fn count_inserts(&mut self) -> usize {
        self.build_anchor_bits();
        self.insert_order[..self.len].copy_from_slice(&self.target_of[..self.len]);
        let mut moves = 0;
        for want in 0..self.len {
            if self.is_anchor(want) {
                continue;
            }
            let want_u16 = u16::try_from(want).expect("target index is within the u16 capacity");
            let from = self.insert_order[..self.len]
                .iter()
                .position(|&value| value == want_u16)
                .expect("target permutation contains every rank");
            let to = self.insert_destination(from, want_u16);
            if to != from {
                self.move_insert_order(from, to, want_u16);
                moves += 1;
            }
        }
        moves
    }

    fn write_swap_moves(&mut self, out: &mut [BankMove], count: usize) -> usize {
        self.insert_order[..self.len].copy_from_slice(&self.target_of[..self.len]);
        for (slot, &target) in self.target_of[..self.len].iter().enumerate() {
            self.lis_prev[usize::from(target)] =
                u16::try_from(slot).expect("slot index is within the u16 capacity");
        }
        let mut written = 0;
        for want in 0..self.len {
            let want_u16 = u16::try_from(want).expect("slot index is within the u16 capacity");
            if self.insert_order[want] == want_u16 {
                continue;
            }
            let from = usize::from(self.lis_prev[want]);
            let displaced = usize::from(self.insert_order[want]);
            out[written] = BankMove {
                from: u16::try_from(from).expect("slot is within capacity"),
                to: want_u16,
            };
            written += 1;
            self.insert_order[want] = want_u16;
            self.insert_order[from] = u16::try_from(displaced).expect("target is within capacity");
            self.lis_prev[want] = want_u16;
            self.lis_prev[displaced] = u16::try_from(from).expect("slot is within capacity");
            if written == count {
                break;
            }
        }
        written
    }

    fn write_insert_moves(&mut self, out: &mut [BankMove], count: usize) -> usize {
        self.build_anchor_bits();
        self.insert_order[..self.len].copy_from_slice(&self.target_of[..self.len]);
        let mut written = 0;
        for want in 0..self.len {
            if self.is_anchor(want) {
                continue;
            }
            let want_u16 = u16::try_from(want).expect("target index is within the u16 capacity");
            let from = self.insert_order[..self.len]
                .iter()
                .position(|&value| value == want_u16)
                .expect("target permutation contains every rank");
            let to = self.insert_destination(from, want_u16);
            if to != from {
                out[written] = BankMove {
                    from: u16::try_from(from).expect("slot is within capacity"),
                    to: u16::try_from(to).expect("slot is within capacity"),
                };
                self.move_insert_order(from, to, want_u16);
                written += 1;
                if written == count {
                    break;
                }
            }
        }
        written
    }

    fn insert_destination(&self, from: usize, want: u16) -> usize {
        let mut rest_index = 0;
        let mut to = 0;
        for index in 0..self.len {
            if index == from {
                continue;
            }
            if self.insert_order[index] < want {
                to = rest_index + 1;
            }
            rest_index += 1;
        }
        to
    }

    fn move_insert_order(&mut self, from: usize, to: usize, item: u16) {
        if from > to {
            self.insert_order[to..=from].rotate_right(1);
        } else if from < to {
            self.insert_order[from..=to].rotate_left(1);
        }
        self.insert_order[to] = item;
    }

    fn build_anchor_bits(&mut self) {
        self.anchor_bits.fill(0);
        let mut lis_len = 0;
        for index in 0..self.len {
            let value = self.target_of[index];
            let mut low = 0;
            let mut high = lis_len;
            while low < high {
                let middle = (low + high) / 2;
                if self.target_of[usize::from(self.lis_tails[middle])] < value {
                    low = middle + 1;
                } else {
                    high = middle;
                }
            }
            self.lis_prev[index] = if low == 0 {
                u16::MAX
            } else {
                self.lis_tails[low - 1]
            };
            self.lis_tails[low] = u16::try_from(index).expect("LIS index is within capacity");
            if low == lis_len {
                lis_len += 1;
            }
        }
        if lis_len == 0 {
            return;
        }
        let mut index = self.lis_tails[lis_len - 1];
        loop {
            let target = usize::from(self.target_of[usize::from(index)]);
            self.set_anchor(target);
            let previous = self.lis_prev[usize::from(index)];
            if previous == u16::MAX {
                break;
            }
            index = previous;
        }
    }

    fn set_anchor(&mut self, index: usize) {
        let bits_per_word = u16::BITS as usize;
        self.anchor_bits[index / bits_per_word] |= 1 << (index % bits_per_word);
    }

    fn is_anchor(&self, index: usize) -> bool {
        let bits_per_word = u16::BITS as usize;
        (self.anchor_bits[index / bits_per_word] & (1 << (index % bits_per_word))) != 0
    }
}

fn compare_rows<R: BankSortRow + ?Sized, F: Fn(i32) -> Option<BankCategory>>(
    left: &R,
    right: &R,
    overrides: &F,
) -> Ordering {
    let left_category = overrides(left.id()).unwrap_or_else(|| category_of(left));
    let right_category = overrides(right.id()).unwrap_or_else(|| category_of(right));
    left_category
        .order_index()
        .cmp(&right_category.order_index())
        .then_with(|| rank_within(left_category, left).cmp(&rank_within(right_category, right)))
        .then_with(|| right.cost().cmp(&left.cost()))
        .then_with(|| left.id().cmp(&right.id()))
}

#[cfg(test)]
mod tests {
    use super::{
        category_of, is_unmatched, rank_within, rule_for, BankCategory, BankMove, BankSortMode,
        BankSortRow, BankSortScratch, PlanSummary, SortPlanError, CATEGORY_ORDER, UNRANKED,
    };
    use std::collections::HashSet;
    use std::mem::size_of;

    const COINS: i32 = 995;
    const AIR_RUNE: i32 = 556;
    const LAW_RUNE: i32 = 563;
    const SHARK: i32 = 385;
    const LOBSTER: i32 = 379;
    const MITHRIL_ORE: i32 = 447;
    const JUNK_A: i32 = 31_337;
    const JUNK_B: i32 = 31_338;

    #[derive(Debug, Clone, Copy)]
    struct Row<'a> {
        slot: i32,
        id: i32,
        name: Option<&'a str>,
        cost: i32,
    }

    impl BankSortRow for Row<'_> {
        fn slot(&self) -> i32 {
            self.slot
        }

        fn id(&self) -> i32 {
            self.id
        }

        fn name(&self) -> Option<&str> {
            self.name
        }

        fn cost(&self) -> i32 {
            self.cost
        }
    }

    fn item<'a>(id: i32, name: &'a str, cost: i32) -> Row<'a> {
        Row {
            slot: 0,
            id,
            name: Some(name),
            cost,
        }
    }

    fn bank(ids: &[i32]) -> Vec<Row<'static>> {
        ids.iter()
            .enumerate()
            .map(|(slot, &id)| {
                let (name, cost) = facts(id);
                Row {
                    slot: slot as i32,
                    id,
                    name,
                    cost,
                }
            })
            .collect()
    }

    fn facts(id: i32) -> (Option<&'static str>, i32) {
        match id {
            COINS => (Some("Coins"), 1),
            AIR_RUNE => (Some("Air rune"), 4),
            LAW_RUNE => (Some("Law rune"), 240),
            SHARK => (Some("Shark"), 200),
            LOBSTER => (Some("Lobster"), 100),
            MITHRIL_ORE => (Some("Mithril ore"), 81),
            JUNK_A => (Some("Nameless thing a"), 1),
            JUNK_B => (Some("Nameless thing b"), 1),
            _ => (None, 0),
        }
    }

    fn layout(scratch: &BankSortScratch, rows: &[Row<'_>]) -> Vec<i32> {
        (0..rows.len())
            .map(|index| scratch.layout_id(rows, index).expect("planned layout row"))
            .collect()
    }

    fn plan(
        scratch: &mut BankSortScratch,
        rows: &[Row<'_>],
        force: Option<BankSortMode>,
    ) -> PlanSummary {
        scratch
            .plan(rows, |_| None, force)
            .expect("rows fit the preallocated scratch")
    }

    fn ascending(category: BankCategory, names: &[&'static str]) {
        let mut previous = None;
        for name in names {
            let rank = rank_within(category, &item(1, name, 1));
            assert_ne!(rank, UNRANKED, "{name} fell through");
            if let Some(previous) = previous {
                assert!(
                    previous < rank,
                    "rank {previous} is not before {name} ({rank})"
                );
            }
            previous = Some(rank);
        }
    }

    fn replay_in_batches(ids: &[i32], mode: BankSortMode) -> Vec<i32> {
        let mut order = ids.to_vec();
        let mut scratch = BankSortScratch::new(order.len()).expect("test bank fits u16 slots");
        for _round in 0..200 {
            let rows: Vec<_> = order
                .iter()
                .enumerate()
                .map(|(slot, &id)| {
                    let (name, cost) = facts(id);
                    Row {
                        slot: slot as i32,
                        id,
                        name,
                        cost,
                    }
                })
                .collect();
            let summary = plan(&mut scratch, &rows, Some(mode));
            if summary.move_count == 0 {
                return order;
            }
            let mut moves = [BankMove::default(); 4];
            let count = scratch.first_moves(&mut moves);
            assert!(count > 0, "a nonzero plan must produce a move batch");
            for move_ in &moves[..count] {
                match mode {
                    BankSortMode::Swap => {
                        order.swap(usize::from(move_.from), usize::from(move_.to));
                    }
                    BankSortMode::Insert => {
                        let moved = order.remove(usize::from(move_.from));
                        order.insert(usize::from(move_.to), moved);
                    }
                }
            }
        }
        panic!("bank did not converge in bounded move batches");
    }

    #[test]
    fn category_strange_fruit_sits_with_food() {
        assert_eq!(
            category_of(&item(464, "Strange fruit", 1)),
            BankCategory::Food
        );
    }

    #[test]
    fn category_coins_and_runes_are_matched_by_id() {
        assert_eq!(category_of(&item(995, "Coins", 1)), BankCategory::Coins);
        assert_ne!(category_of(&item(617, "Coins", 1)), BankCategory::Coins);
        for id in [
            554, 555, 556, 557, 558, 559, 560, 561, 562, 563, 564, 565, 566, 1436,
        ] {
            assert_eq!(rule_for(id, Some("whatever")), Some(BankCategory::Runes));
        }
    }

    #[test]
    fn category_ammunition_beats_weapons_and_armour_split() {
        for (id, name) in [
            (892, "Rune arrow"),
            (9143, "Mithril bolts"),
            (811, "Rune dart"),
            (868, "Rune knife"),
        ] {
            assert_eq!(category_of(&item(id, name, 1)), BankCategory::Ammunition);
        }
        assert_eq!(
            category_of(&item(1333, "Rune scimitar", 1)),
            BankCategory::Weapons
        );
        assert_eq!(
            category_of(&item(861, "Magic shortbow", 1)),
            BankCategory::Weapons
        );
        assert_eq!(
            category_of(&item(1127, "Rune platebody", 1)),
            BankCategory::Armour
        );
        assert_eq!(
            category_of(&item(1201, "Rune kiteshield", 1)),
            BankCategory::Armour
        );
        assert_eq!(
            category_of(&item(1163, "Rune full helm", 1)),
            BankCategory::Armour
        );
    }

    #[test]
    fn category_unidentified_herbs_are_not_unmatched() {
        for id in [199, 3049] {
            let herb = item(id, "Herb", 1);
            assert_eq!(category_of(&herb), BankCategory::Herbs);
            assert!(!is_unmatched(&herb));
        }
    }

    #[test]
    fn category_herbs_added_to_the_table_are_recognized() {
        for name in [
            "Lantadyme",
            "Snapdragon",
            "Snake weed",
            "Ardrigal",
            "Sito foil",
            "Volencia moss",
            "Rogues purse",
        ] {
            assert_eq!(
                category_of(&item(1, name, 1)),
                BankCategory::Herbs,
                "{name}"
            );
        }
    }

    #[test]
    fn category_dragonstone_keys_file_with_gems() {
        assert_eq!(
            category_of(&item(985, "Half of a key", 1)),
            BankCategory::OresBarsGems
        );
        let key = item(989, "Crystal key", 1);
        assert_eq!(category_of(&key), BankCategory::OresBarsGems);
        assert!(!is_unmatched(&key));
    }

    #[test]
    fn category_quest_keys_are_junk_not_gems() {
        assert_eq!(category_of(&item(432, "Chest key", 1)), BankCategory::Junk);
        assert_eq!(category_of(&item(1590, "Dusty key", 1)), BankCategory::Junk);
    }

    #[test]
    fn category_food_potions_and_herbs() {
        for (id, name, expected) in [
            (385, "Shark", BankCategory::Food),
            (379, "Lobster", BankCategory::Food),
            (2434, "Prayer potion(4)", BankCategory::Potions),
            (229, "Vial", BankCategory::Potions),
            (207, "Grimy ranarr weed", BankCategory::Herbs),
            (261, "Clean ranarr weed", BankCategory::Herbs),
        ] {
            assert_eq!(category_of(&item(id, name, 1)), expected);
        }
    }

    #[test]
    fn category_swordfish_is_food_not_a_sword() {
        assert_eq!(category_of(&item(371, "Swordfish", 1)), BankCategory::Food);
        assert_eq!(
            category_of(&item(373, "Raw swordfish", 1)),
            BankCategory::Food
        );
        assert_eq!(
            category_of(&item(1289, "Rune longsword", 1)),
            BankCategory::Weapons
        );
        assert_eq!(
            category_of(&item(1319, "Rune 2h sword", 1)),
            BankCategory::Weapons
        );
    }

    #[test]
    fn category_all_dosed_potions_are_potions() {
        let bases = [
            "Super attack",
            "Super strength",
            "Super defence",
            "Super energy",
            "Super restore",
            "Attack potion",
            "Strength potion",
            "Defence potion",
            "Ranging potion",
            "Magic potion",
            "Prayer potion",
            "Restore potion",
            "Zamorak potion",
            "Antipoison",
            "Superantipoison",
            "Antifire potion",
            "Energy potion",
            "Agility potion",
            "Fishing potion",
        ];
        for base in bases {
            for dose in 1..=4 {
                let name = format!("{base}({dose})");
                let row = Row {
                    slot: 0,
                    id: 1,
                    name: Some(&name),
                    cost: 1,
                };
                assert_eq!(category_of(&row), BankCategory::Potions);
            }
        }
    }

    #[test]
    fn category_cooked_fish_is_food() {
        for name in [
            "Manta ray",
            "Shark",
            "Swordfish",
            "Bass",
            "Lobster",
            "Tuna",
            "Salmon",
            "Pike",
            "Cod",
            "Trout",
            "Mackerel",
            "Herring",
            "Sardine",
            "Anchovies",
            "Shrimps",
        ] {
            assert_eq!(category_of(&item(1, name, 1)), BankCategory::Food, "{name}");
        }
    }

    #[test]
    fn category_staves_are_separate_from_weapons() {
        for name in [
            "Staff",
            "Magic staff",
            "Staff of air",
            "Battlestaff",
            "Fire battlestaff",
            "Mystic fire staff",
            "Staff of iban",
        ] {
            assert_eq!(
                category_of(&item(1, name, 1)),
                BankCategory::Staves,
                "{name}"
            );
        }
        assert_eq!(
            category_of(&item(1333, "Rune scimitar", 1)),
            BankCategory::Weapons
        );
    }

    #[test]
    fn category_dragonhide_and_leather_classes_match_upstream() {
        for name in [
            "Dragonhide body",
            "Dragonhide chaps",
            "Dragon vambraces",
            "Studded body",
            "Leather body",
            "Coif",
        ] {
            assert_eq!(
                category_of(&item(1, name, 1)),
                BankCategory::RangedArmour,
                "{name}"
            );
        }
        assert_eq!(
            category_of(&item(1127, "Rune platebody", 1)),
            BankCategory::Armour
        );
        for (id, name) in [
            (1747, "Dragonhide"),
            (1741, "Leather"),
            (1743, "Hard leather"),
        ] {
            assert_eq!(category_of(&item(id, name, 1)), BankCategory::Supplies);
        }
    }

    #[test]
    fn category_materials_logs_supplies_and_tools() {
        for (id, name, expected) in [
            (447, "Mithril ore", BankCategory::OresBarsGems),
            (2361, "Adamantite bar", BankCategory::OresBarsGems),
            (1615, "Uncut dragonstone", BankCategory::OresBarsGems),
            (1513, "Magic logs", BankCategory::Logs),
            (314, "Feather", BankCategory::Supplies),
            (1747, "Black dragonhide", BankCategory::Supplies),
            (1359, "Rune axe", BankCategory::Tools),
            (1275, "Rune pickaxe", BankCategory::Tools),
            (590, "Tinderbox", BankCategory::Tools),
        ] {
            assert_eq!(category_of(&item(id, name, 1)), expected);
        }
    }

    #[test]
    fn category_bowstrings_are_supplies_not_weapons() {
        assert_eq!(
            category_of(&item(1777, "Bow string", 1)),
            BankCategory::Supplies
        );
        assert_eq!(
            category_of(&item(1779, "Bowstring", 1)),
            BankCategory::Supplies
        );
        assert_eq!(
            category_of(&item(841, "Shortbow", 1)),
            BankCategory::Weapons
        );
    }

    #[test]
    fn category_topaz_is_a_gem_not_a_top() {
        assert_eq!(
            category_of(&item(1613, "Uncut topaz", 1)),
            BankCategory::OresBarsGems
        );
    }

    #[test]
    fn category_hide_armour_and_raw_hide_are_split() {
        assert_eq!(
            category_of(&item(1129, "Leather body", 1)),
            BankCategory::RangedArmour
        );
        assert_eq!(
            category_of(&item(1095, "Leather chaps", 1)),
            BankCategory::RangedArmour
        );
        assert_eq!(
            category_of(&item(1741, "Cowhide", 1)),
            BankCategory::Supplies
        );
    }

    #[test]
    fn category_short_material_words_need_boundaries() {
        for (id, name, expected) in [
            (440, "Iron ore", BankCategory::OresBarsGems),
            (2349, "Bronze bar", BankCategory::OresBarsGems),
            (453, "Coal", BankCategory::OresBarsGems),
            (1, "Storeroom key", BankCategory::Junk),
            (2, "Barrel", BankCategory::Junk),
            (3, "Barcrawl card", BankCategory::Junk),
        ] {
            assert_eq!(category_of(&item(id, name, 1)), expected);
        }
    }

    #[test]
    fn category_bowl_is_not_a_bow() {
        assert_eq!(category_of(&item(4, "Golden bowl", 1)), BankCategory::Junk);
        assert_eq!(
            category_of(&item(841, "Shortbow", 1)),
            BankCategory::Weapons
        );
        assert_eq!(
            category_of(&item(837, "Crossbow", 1)),
            BankCategory::Weapons
        );
    }

    #[test]
    fn category_cannonballs_are_ammunition() {
        assert_eq!(
            category_of(&item(2, "Cannonball", 1)),
            BankCategory::Ammunition
        );
    }

    #[test]
    fn category_lobster_pots_are_tools_and_lobsters_are_food() {
        assert_eq!(
            category_of(&item(301, "Lobster pot", 1)),
            BankCategory::Tools
        );
        assert_eq!(category_of(&item(379, "Lobster", 1)), BankCategory::Food);
    }

    #[test]
    fn category_jewellery_matches_names() {
        assert_eq!(
            category_of(&item(1712, "Amulet of glory", 1)),
            BankCategory::Jewellery
        );
        assert_eq!(
            category_of(&item(2552, "Ring of dueling(8)", 1)),
            BankCategory::Jewellery
        );
    }

    #[test]
    fn category_unmatched_and_explicit_junk_are_distinct() {
        let stray = item(31_337, "Some unheard of thing", 1);
        assert_eq!(category_of(&stray), BankCategory::Junk);
        assert!(is_unmatched(&stray));
        let bones = item(526, "Bones", 1);
        assert_eq!(category_of(&bones), BankCategory::Junk);
        assert!(!is_unmatched(&bones));
    }

    #[test]
    fn category_null_names_never_throw_and_order_is_complete() {
        let null = Row {
            slot: 0,
            id: 4242,
            name: None,
            cost: 0,
        };
        assert_eq!(category_of(&null), BankCategory::Junk);
        assert_eq!(CATEGORY_ORDER.len(), 19);
        assert_eq!(
            CATEGORY_ORDER.map(BankCategory::as_str),
            [
                "coins",
                "runes",
                "staves",
                "ammunition",
                "weapons",
                "armour",
                "rangedArmour",
                "food",
                "potions",
                "jewellery",
                "herbs",
                "oresBarsGems",
                "logs",
                "supplies",
                "tools",
                "teleports",
                "questLive",
                "questObsolete",
                "junk",
            ]
        );
        assert_eq!(CATEGORY_ORDER[0], BankCategory::Coins);
        assert_eq!(CATEGORY_ORDER[18], BankCategory::Junk);
        assert_eq!(
            CATEGORY_ORDER.iter().copied().collect::<HashSet<_>>().len(),
            19
        );
    }

    #[test]
    fn rank_first_pass_metal_mats_gems_and_weapons() {
        ascending(
            BankCategory::OresBarsGems,
            &[
                "Copper ore",
                "Tin ore",
                "Iron ore",
                "Silver ore",
                "Coal",
                "Gold ore",
                "Mithril ore",
                "Adamantite ore",
                "Runite ore",
            ],
        );
        ascending(
            BankCategory::OresBarsGems,
            &[
                "Bronze bar",
                "Iron bar",
                "Silver bar",
                "Steel bar",
                "Gold bar",
                "Mithril bar",
                "Adamantite bar",
                "Runite bar",
            ],
        );
        assert!(
            rank_within(BankCategory::OresBarsGems, &item(1, "Runite ore", 1))
                < rank_within(BankCategory::OresBarsGems, &item(1, "Bronze bar", 1))
        );
        ascending(
            BankCategory::OresBarsGems,
            &[
                "Half of a key",
                "Crystal key",
                "Dragonstone",
                "Diamond",
                "Ruby",
                "Emerald",
                "Sapphire",
                "Uncut dragonstone",
                "Uncut diamond",
                "Uncut ruby",
                "Uncut emerald",
                "Uncut sapphire",
            ],
        );
        assert!(
            rank_within(BankCategory::OresBarsGems, &item(1, "Runite bar", 1))
                < rank_within(BankCategory::OresBarsGems, &item(1, "Half of a key", 1))
        );
        assert_eq!(
            rank_within(BankCategory::OresBarsGems, &item(1, "Uncut opal", 1)),
            UNRANKED
        );
        assert_eq!(
            rank_within(BankCategory::OresBarsGems, &item(1, "Jade", 1)),
            UNRANKED
        );
        ascending(
            BankCategory::Weapons,
            &[
                "Dragon dagger",
                "Rune scimitar",
                "Adamant scimitar",
                "Mithril scimitar",
                "Black scimitar",
                "Steel scimitar",
                "Iron scimitar",
                "Bronze scimitar",
            ],
        );
        ascending(
            BankCategory::Weapons,
            &[
                "Magic longbow",
                "Magic shortbow",
                "Yew longbow",
                "Yew shortbow",
                "Maple longbow",
                "Maple shortbow",
                "Willow longbow",
                "Willow shortbow",
                "Oak longbow",
                "Oak shortbow",
                "Longbow",
                "Shortbow",
            ],
        );
        assert!(
            rank_within(BankCategory::Weapons, &item(1, "Bronze scimitar", 1))
                < rank_within(BankCategory::Weapons, &item(1, "Magic longbow", 1))
        );
        assert_eq!(
            rank_within(BankCategory::Weapons, &item(1, "Abyssal whip", 1)),
            UNRANKED
        );
        assert_eq!(
            rank_within(BankCategory::Weapons, &item(1, "Staff of air", 1)),
            UNRANKED
        );
    }

    #[test]
    fn rank_first_pass_ammunition_runes_logs_herbs_and_fallbacks() {
        ascending(
            BankCategory::Ammunition,
            &[
                "Rune arrow",
                "Adamant arrow",
                "Mithril arrow",
                "Steel arrow",
                "Iron arrow",
                "Bronze arrow",
            ],
        );
        assert_eq!(
            rank_within(BankCategory::Ammunition, &item(1, "Ice arrow", 1)),
            UNRANKED
        );
        assert_eq!(
            rank_within(BankCategory::Ammunition, &item(1, "Ogre arrow", 1)),
            UNRANKED
        );
        ascending(
            BankCategory::Runes,
            &[
                "Blood rune",
                "Death rune",
                "Soul rune",
                "Chaos rune",
                "Mind rune",
                "Body rune",
                "Nature rune",
                "Cosmic rune",
                "Law rune",
                "Air rune",
                "Earth rune",
                "Water rune",
                "Fire rune",
                "Rune essence",
            ],
        );
        ascending(
            BankCategory::Logs,
            &[
                "Logs",
                "Achey tree logs",
                "Oak logs",
                "Willow logs",
                "Maple logs",
                "Yew logs",
                "Magic logs",
            ],
        );
        assert_eq!(
            rank_within(BankCategory::Logs, &item(1, "Plank", 1)),
            UNRANKED
        );
        ascending(
            BankCategory::Herbs,
            &[
                "Guam leaf",
                "Snake weed",
                "Ardrigal",
                "Sito foil",
                "Volencia moss",
                "Rogues purse",
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
            ],
        );
        let unids = [
            ("guam", 199),
            ("snake weed", 1525),
            ("marrentill", 201),
            ("ranarr", 207),
            ("toadflax", 3049),
            ("irit", 209),
            ("snapdragon", 3051),
            ("cadantine", 215),
            ("lantadyme", 2485),
            ("torstol", 219),
        ];
        let ranks: Vec<_> = unids
            .iter()
            .map(|&(_, id)| rank_within(BankCategory::Herbs, &item(id, "Herb", 1)))
            .collect();
        assert!(ranks.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(
            rank_within(BankCategory::Herbs, &item(200, "Herb", 1)),
            rank_within(BankCategory::Herbs, &item(199, "Herb", 1))
        );
        assert_eq!(
            rank_within(BankCategory::Herbs, &item(2486, "Herb", 1)),
            rank_within(BankCategory::Herbs, &item(2485, "Herb", 1))
        );
        assert!(
            rank_within(BankCategory::Herbs, &item(1, "Torstol", 1))
                < rank_within(BankCategory::Herbs, &item(199, "Herb", 1))
        );
        assert_eq!(
            rank_within(BankCategory::Supplies, &item(1, "Feather", 1)),
            UNRANKED
        );
        assert_eq!(
            rank_within(BankCategory::Tools, &item(1, "Rune pickaxe", 1)),
            UNRANKED
        );
        assert_eq!(
            rank_within(BankCategory::Junk, &item(1, "Bones", 1)),
            UNRANKED
        );
        for category in [
            BankCategory::Runes,
            BankCategory::Food,
            BankCategory::Potions,
            BankCategory::RangedArmour,
        ] {
            assert_eq!(
                rank_within(
                    category,
                    &Row {
                        slot: 0,
                        id: 4242,
                        name: None,
                        cost: 0
                    }
                ),
                UNRANKED
            );
        }
        assert_eq!(
            rank_within(BankCategory::OresBarsGems, &item(437, "Copper ore", 1)),
            rank_within(BankCategory::OresBarsGems, &item(436, "Copper ore", 1))
        );
    }

    #[test]
    fn rank_second_pass_ammunition_blocks_and_poisoned_names() {
        ascending(
            BankCategory::Ammunition,
            &[
                "Rune arrow",
                "Adamant arrow",
                "Mithril arrow",
                "Steel arrow",
                "Iron arrow",
                "Bronze arrow",
                "Rune arrowtips",
                "Adamant arrowtips",
                "Mithril arrowtips",
                "Steel arrowtips",
                "Iron arrowtips",
                "Bronze arrowtips",
            ],
        );
        assert_eq!(
            rank_within(BankCategory::Ammunition, &item(1, "Bronze arrow(p)", 1)),
            rank_within(BankCategory::Ammunition, &item(1, "Bronze arrow", 1))
        );
        assert_ne!(
            rank_within(BankCategory::Ammunition, &item(1, "Bronze arrowtips", 1)),
            rank_within(BankCategory::Ammunition, &item(1, "Bronze arrow", 1))
        );
        ascending(
            BankCategory::Ammunition,
            &[
                "Rune arrow",
                "Rune arrowtips",
                "Headless arrow",
                "Barbed bolts",
                "Bolts",
                "Opal bolttips",
                "Rune dart",
                "Bronze dart",
                "Rune dart tip",
                "Bronze dart tip",
                "Rune javelin",
                "Rune knife",
                "Rune thrownaxe",
                "Cannonball",
            ],
        );
        assert!(
            rank_within(BankCategory::Ammunition, &item(1, "Rune dart", 1))
                < rank_within(BankCategory::Ammunition, &item(1, "Bronze dart", 1))
        );
        assert!(
            rank_within(BankCategory::Ammunition, &item(1, "Bronze dart", 1))
                < rank_within(BankCategory::Ammunition, &item(1, "Rune dart tip", 1))
        );
    }

    #[test]
    fn rank_second_pass_food_cooking_and_potion_doses() {
        ascending(
            BankCategory::Food,
            &[
                "Manta ray",
                "Shark",
                "Swordfish",
                "Bass",
                "Lobster",
                "Tuna",
                "Salmon",
                "Pike",
                "Cod",
                "Trout",
                "Mackerel",
                "Herring",
                "Sardine",
                "Anchovies",
                "Shrimps",
                "Cake",
                "Meat pie",
                "Stew",
                "Bread",
                "Cooked chicken",
                "Cooked meat",
                "Raw shark",
                "Raw lobster",
                "Burnt shark",
            ],
        );
        assert!(
            rank_within(BankCategory::Food, &item(1, "Meat pie", 1))
                < rank_within(BankCategory::Food, &item(1, "Cooked meat", 1))
        );
        ascending(
            BankCategory::Potions,
            &[
                "Super attack(4)",
                "Super attack(3)",
                "Super attack(2)",
                "Super attack(1)",
                "Super strength(4)",
                "Super defence(4)",
                "Attack potion(4)",
                "Attack potion(1)",
                "Strength potion(4)",
                "Defence potion(3)",
                "Ranging potion(4)",
                "Magic potion(4)",
                "Zamorak potion(4)",
                "Prayer potion(4)",
                "Super restore(4)",
                "Restore potion(4)",
                "Superantipoison(4)",
                "Antipoison(4)",
                "Antifire potion(4)",
                "Super energy(4)",
                "Energy potion(4)",
                "Agility potion(4)",
                "Fishing potion(4)",
            ],
        );
        assert_eq!(
            rank_within(BankCategory::Potions, &item(1, "Vial", 1)),
            UNRANKED
        );
        assert_eq!(
            rank_within(BankCategory::Potions, &item(1, "Vial of water", 1)),
            UNRANKED
        );
        assert_eq!(
            rank_within(BankCategory::Potions, &item(1, "Super attack(9)", 1)),
            -5
        );
    }

    #[test]
    fn rank_second_pass_armour_dragonhide_and_staff_tiers() {
        ascending(
            BankCategory::Armour,
            &[
                "Dragon platebody",
                "Rune platebody",
                "Adamant platebody",
                "Mithril platebody",
                "Black platebody",
                "Steel platebody",
                "Iron platebody",
                "Bronze platebody",
            ],
        );
        let body = [
            ("black", 2503),
            ("red", 2501),
            ("blue", 2499),
            ("green", 1135),
        ];
        let ranks: Vec<_> = body
            .iter()
            .map(|&(_, id)| {
                rank_within(BankCategory::RangedArmour, &item(id, "Dragonhide body", 1))
            })
            .collect();
        assert!(ranks.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(
            rank_within(
                BankCategory::RangedArmour,
                &item(2504, "Dragonhide body", 1)
            ),
            rank_within(
                BankCategory::RangedArmour,
                &item(2503, "Dragonhide body", 1)
            )
        );
        assert!(
            rank_within(
                BankCategory::RangedArmour,
                &item(2497, "Dragonhide chaps", 1)
            ) < rank_within(BankCategory::RangedArmour, &item(1, "Studded body", 1))
        );
        assert!(
            rank_within(BankCategory::RangedArmour, &item(1, "Studded body", 1))
                < rank_within(BankCategory::RangedArmour, &item(1, "Leather body", 1))
        );
        ascending(
            BankCategory::Staves,
            &[
                "Mystic lava staff",
                "Mystic fire staff",
                "Mystic air staff",
                "Lava battlestaff",
                "Fire battlestaff",
                "Air battlestaff",
                "Battlestaff",
                "Staff of saradomin",
                "Staff of iban",
                "Staff of fire",
                "Staff of air",
                "Magic staff",
                "Dramen staff",
                "Staff",
            ],
        );
    }

    #[test]
    fn rank_uses_unicode_lowercase_fallback() {
        let item = item(1, "Éclair", 1);
        assert_eq!(rank_within(BankCategory::Food, &item), UNRANKED);
        assert_eq!(category_of(&item), BankCategory::Junk);
        assert_eq!(rule_for(2, Some("Ｒｕｎｅ item")), None);
    }

    #[test]
    fn lis_helper_cases_are_exercised_by_insert_plans() {
        let cases: &[(&[i32], &[i32])] = &[
            (&[3, 0, 1, 2], &[0, 1, 2]),
            (&[0, 1, 2, 3], &[0, 1, 2, 3]),
            (&[3, 2, 1, 0], &[0]),
            (&[], &[]),
        ];
        for &(values, expected) in cases {
            let rows: Vec<_> = values
                .iter()
                .enumerate()
                .map(|(slot, value)| Row {
                    slot: slot as i32,
                    id: *value,
                    name: None,
                    cost: 0,
                })
                .collect();
            let mut scratch = BankSortScratch::new(rows.len()).expect("small bank");
            let result = plan(&mut scratch, &rows, Some(BankSortMode::Insert));
            assert!(result.is_dense);
            scratch.build_anchor_bits();
            let anchors: Vec<_> = (0..rows.len())
                .filter(|&rank| scratch.is_anchor(rank))
                .collect();
            let expected: Vec<_> = expected.iter().map(|&rank| rank as usize).collect();
            assert_eq!(anchors, expected);
        }
    }

    #[test]
    fn empty_already_sorted_and_dense_slot_cases() {
        let mut empty_scratch = BankSortScratch::new(0).expect("empty bank");
        let empty = plan(&mut empty_scratch, &[], None);
        assert_eq!(empty.move_count, 0);
        assert_eq!(empty.mode, BankSortMode::Swap);

        let sorted = bank(&[
            COINS,
            LAW_RUNE,
            AIR_RUNE,
            SHARK,
            LOBSTER,
            MITHRIL_ORE,
            JUNK_A,
        ]);
        let mut sorted_scratch = BankSortScratch::new(sorted.len()).expect("sorted bank");
        assert_eq!(plan(&mut sorted_scratch, &sorted, None).move_count, 0);
        assert_eq!(
            layout(&sorted_scratch, &sorted),
            [
                COINS,
                LAW_RUNE,
                AIR_RUNE,
                SHARK,
                LOBSTER,
                MITHRIL_ORE,
                JUNK_A
            ]
        );

        let sparse = [
            Row {
                slot: 0,
                id: SHARK,
                name: Some("Shark"),
                cost: 200,
            },
            Row {
                slot: 4,
                id: COINS,
                name: Some("Coins"),
                cost: 1,
            },
        ];
        let mut sparse_scratch = BankSortScratch::new(5).expect("capacity five");
        let sparse_plan = plan(&mut sparse_scratch, &sparse, None);
        assert_eq!(sparse_plan.mode, BankSortMode::Swap);
        assert_eq!(sparse_plan.move_count, 0);
        assert!(!sparse_plan.is_dense);
        assert_eq!(layout(&sparse_scratch, &sparse), [SHARK, COINS]);
        assert_eq!(sparse_scratch.unmatched_ids(&sparse, |_| None).count(), 0);
    }

    #[test]
    fn planner_orders_categories_price_and_id_then_reports_unmatched() {
        let ids = [
            JUNK_A,
            MITHRIL_ORE,
            COINS,
            LOBSTER,
            AIR_RUNE,
            JUNK_B,
            SHARK,
            LAW_RUNE,
        ];
        let rows = bank(&ids);
        let mut scratch = BankSortScratch::new(rows.len()).expect("fixture bank");
        let summary = plan(&mut scratch, &rows, None);
        let target = layout(&scratch, &rows);
        assert_eq!(target[0], COINS);
        assert_eq!(target[1..3], [LAW_RUNE, AIR_RUNE]);
        assert_eq!(target[target.len() - 2..], [JUNK_A, JUNK_B]);
        assert_eq!(target.len(), ids.len());
        assert_eq!(
            scratch.unmatched_ids(&rows, |_| None).collect::<Vec<_>>(),
            [JUNK_A, JUNK_B]
        );
        assert!(summary.move_count <= ids.len());

        let price = bank(&[LOBSTER, SHARK]);
        let mut price_scratch = BankSortScratch::new(price.len()).expect("price bank");
        plan(&mut price_scratch, &price, None);
        assert_eq!(layout(&price_scratch, &price), [SHARK, LOBSTER]);
    }

    #[test]
    fn planner_insert_choice_for_top_up_and_swap_choice_for_reverse() {
        let top_up = bank(&[
            COINS,
            AIR_RUNE,
            SHARK,
            LOBSTER,
            MITHRIL_ORE,
            JUNK_A,
            LAW_RUNE,
        ]);
        let mut scratch = BankSortScratch::new(top_up.len()).expect("top-up bank");
        let summary = plan(&mut scratch, &top_up, None);
        assert_eq!(summary.mode, BankSortMode::Insert);
        assert_eq!(summary.move_count, 1);
        assert_eq!(summary.insert_move_count, 1);
        assert_eq!(
            scratch.unmatched_ids(&top_up, |_| None).collect::<Vec<_>>(),
            [JUNK_A]
        );

        let reversed = bank(&[
            JUNK_A,
            MITHRIL_ORE,
            LOBSTER,
            SHARK,
            AIR_RUNE,
            LAW_RUNE,
            COINS,
        ]);
        let mut reversed_scratch = BankSortScratch::new(reversed.len()).expect("reversed bank");
        let reversed_plan = plan(&mut reversed_scratch, &reversed, None);
        assert_eq!(reversed_plan.mode, BankSortMode::Swap);
        assert!(reversed_plan.insert_move_count > reversed_plan.swap_move_count);
    }

    #[test]
    fn planner_overrides_force_and_unmatched_ids() {
        let rows = bank(&[COINS, SHARK, LAW_RUNE]);
        let mut scratch = BankSortScratch::new(rows.len()).expect("override bank");
        scratch
            .plan(
                &rows,
                |id| (id == SHARK).then_some(BankCategory::QuestObsolete),
                None,
            )
            .expect("rows fit scratch");
        assert_eq!(layout(&scratch, &rows), [COINS, LAW_RUNE, SHARK]);
        assert!(scratch
            .unmatched_ids(&rows, |id| (id == SHARK)
                .then_some(BankCategory::QuestObsolete))
            .next()
            .is_none());

        let items = bank(&[
            COINS,
            AIR_RUNE,
            SHARK,
            LOBSTER,
            MITHRIL_ORE,
            JUNK_A,
            LAW_RUNE,
        ]);
        let mut mode_scratch = BankSortScratch::new(items.len()).expect("mode bank");
        let default = plan(&mut mode_scratch, &items, None);
        assert_eq!(default.mode, BankSortMode::Insert);
        let forced = plan(&mut mode_scratch, &items, Some(BankSortMode::Swap));
        assert_eq!(forced.mode, BankSortMode::Swap);
        assert_eq!(forced.move_count, forced.swap_move_count);
        let forced_insert = plan(&mut mode_scratch, &items, Some(BankSortMode::Insert));
        assert_eq!(forced_insert.mode, BankSortMode::Insert);
        assert_eq!(forced_insert.move_count, forced_insert.insert_move_count);
    }

    #[test]
    fn both_move_strategies_reach_the_ranked_layout_in_batches() {
        let ids = [
            JUNK_B,
            MITHRIL_ORE,
            COINS,
            LOBSTER,
            AIR_RUNE,
            JUNK_A,
            SHARK,
            LAW_RUNE,
        ];
        for mode in [BankSortMode::Swap, BankSortMode::Insert] {
            let result = replay_in_batches(&ids, mode);
            let rows = bank(&result);
            let mut scratch = BankSortScratch::new(rows.len()).expect("bank fits scratch");
            plan(&mut scratch, &rows, Some(mode));
            assert_eq!(result, layout(&scratch, &rows));
        }
    }

    #[test]
    fn seeded_shuffled_banks_replay_in_each_forced_mode() {
        let pool = [
            COINS,
            AIR_RUNE,
            LAW_RUNE,
            SHARK,
            LOBSTER,
            MITHRIL_ORE,
            JUNK_A,
            JUNK_B,
        ];
        let mut seed = 12_345_u32;
        let mut next = || {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345) & 0x7fff_ffff;
            seed
        };
        for _round in 0..200 {
            let count = 2 + (next() as usize % (pool.len() - 1));
            let mut ids = pool[..count].to_vec();
            for index in (1..ids.len()).rev() {
                let other = next() as usize % (index + 1);
                ids.swap(index, other);
            }
            for mode in [BankSortMode::Swap, BankSortMode::Insert] {
                let result = replay_in_batches(&ids, mode);
                let rows = bank(&result);
                let mut scratch = BankSortScratch::new(rows.len()).expect("bank fits scratch");
                plan(&mut scratch, &rows, Some(mode));
                assert_eq!(result, layout(&scratch, &rows));
            }
        }
    }

    #[test]
    fn swap_plan_respects_cycle_bound_and_equal_rows_remain_stable() {
        let ids = [
            JUNK_B,
            MITHRIL_ORE,
            COINS,
            LOBSTER,
            AIR_RUNE,
            JUNK_A,
            SHARK,
            LAW_RUNE,
        ];
        let rows = bank(&ids);
        let mut scratch = BankSortScratch::new(rows.len()).expect("bank fits scratch");
        let summary = plan(&mut scratch, &rows, Some(BankSortMode::Swap));
        let stationary = layout(&scratch, &rows)
            .iter()
            .zip(ids)
            .filter(|(planned, original)| **planned == *original)
            .count();
        assert!(summary.move_count <= ids.len() - stationary.max(1));
        let duplicate_rows = [
            Row {
                slot: 0,
                id: 9,
                name: Some("Thing"),
                cost: 1,
            },
            Row {
                slot: 1,
                id: 9,
                name: Some("Other thing"),
                cost: 1,
            },
        ];
        let mut duplicate_scratch = BankSortScratch::new(2).expect("two slots");
        assert_eq!(
            plan(&mut duplicate_scratch, &duplicate_rows, None).move_count,
            0
        );
    }

    #[test]
    fn full_reversed_240_slot_counts_match_upstream() {
        let rows: Vec<_> = (0..240)
            .map(|slot| Row {
                slot,
                id: 239 - slot,
                name: Some("unlisted item"),
                cost: 1,
            })
            .collect();
        let mut scratch = BankSortScratch::new(240).expect("bank capacity");
        let summary = plan(&mut scratch, &rows, None);
        assert_eq!(summary.swap_move_count, 120);
        assert_eq!(summary.insert_move_count, 239);
        assert_eq!(summary.mode, BankSortMode::Swap);
        assert_eq!(summary.move_count, 120);
        assert_eq!(scratch.layout_id(&rows, 0), Some(0));
        assert_eq!(scratch.layout_id(&rows, 239), Some(239));
    }

    #[test]
    fn move_batches_are_emitted_in_the_selected_strategy_order() {
        let rows = bank(&[
            JUNK_B,
            MITHRIL_ORE,
            COINS,
            LOBSTER,
            AIR_RUNE,
            JUNK_A,
            SHARK,
            LAW_RUNE,
        ]);
        for mode in [BankSortMode::Swap, BankSortMode::Insert] {
            let mut scratch = BankSortScratch::new(rows.len()).expect("bank fits scratch");
            let summary = plan(&mut scratch, &rows, Some(mode));
            let mut first = [BankMove::default(); 4];
            let count = scratch.first_moves(&mut first);
            assert_eq!(count, summary.move_count.min(first.len()));
            let mut full_output = [BankMove::default(); 32];
            assert_eq!(scratch.first_moves(&mut full_output), summary.move_count);
            assert_eq!(&first[..count], &full_output[..count]);
        }
    }

    #[test]
    fn scratch_capacity_is_bounded_by_u16_arrays_and_anchor_bitset() {
        let maximum = usize::from(u16::MAX);
        for capacity in [0, 1, 240, maximum] {
            let scratch = BankSortScratch::new(capacity).expect("selected bank capacity fits");
            assert_eq!(scratch.capacity(), capacity);
            let backing_capacity_bytes = [
                scratch.rows_by_slot.capacity(),
                scratch.target_of.capacity(),
                scratch.slot_of.capacity(),
                scratch.insert_order.capacity(),
                scratch.lis_tails.capacity(),
                scratch.lis_prev.capacity(),
                scratch.anchor_bits.capacity(),
            ]
            .into_iter()
            .sum::<usize>()
                * size_of::<u16>();
            let fixed = size_of::<BankSortScratch>();
            assert!(
                backing_capacity_bytes <= 16 * capacity + fixed,
                "N={capacity}: capacities consume {backing_capacity_bytes} bytes plus {fixed} fixed"
            );
        }
        assert_eq!(
            BankSortScratch::new(maximum + 1).err(),
            Some(SortPlanError::CapacityTooLarge {
                capacity: maximum + 1
            })
        );
    }

    #[test]
    fn scratch_reuses_storage_without_ascii_plan_allocations() {
        let rows = bank(&[
            JUNK_B,
            MITHRIL_ORE,
            COINS,
            LOBSTER,
            AIR_RUNE,
            JUNK_A,
            SHARK,
            LAW_RUNE,
        ]);
        let mut scratch = BankSortScratch::new(rows.len()).expect("fixture bank");
        let _ = scratch.plan(&rows, |_| None, None).expect("warm bank plan");
        let mut warm_moves = [BankMove::default(); 4];
        std::hint::black_box(scratch.first_moves(&mut warm_moves));
        let measured = allocation_counter::measure(|| {
            for _ in 0..20 {
                let summary = scratch.plan(&rows, |_| None, None).expect("rows fit");
                std::hint::black_box(summary);
                let mut moves = [BankMove::default(); 4];
                std::hint::black_box(scratch.first_moves(&mut moves));
            }
        });
        assert_eq!(
            measured.count_total, 0,
            "warm native planning allocates: {measured:?}"
        );
        assert_eq!(
            measured.bytes_total, 0,
            "warm native planning allocates bytes: {measured:?}"
        );
    }

    #[test]
    fn too_many_rows_are_a_typed_error() {
        let rows = bank(&[COINS, AIR_RUNE]);
        let mut scratch = BankSortScratch::new(1).expect("one slot");
        assert_eq!(
            scratch.plan(&rows, |_| None, None),
            Err(SortPlanError::RowsExceedCapacity {
                capacity: 1,
                rows: 2
            })
        );
    }
}
