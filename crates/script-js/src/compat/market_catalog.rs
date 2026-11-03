//! Frozen `api/market/catalog.ts` over the selected revision's items: which
//! objs an order book lists, the names a shop says back, and whether the
//! content lets an obj cross a trade window. Every answer reads
//! `SelectedGameData::items` supplies item facts.
//!
//! `HAND_ALIASES` and `NAME_SYNONYMS` mirror the frozen rs2b0t
//! `src/bot/api/market/catalog.ts`.
//! Generated collision aliases derive from selected item names.

use api::game_data::{GameItem, SelectedGameData};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError, Weak};

/// Frozen `ItemAlias`: the words that separate an obj from its same-named
/// siblings and the label the shop says back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ItemAlias {
    pub words: Vec<String>,
    normalized_words: Vec<String>,
    pub label: String,
}

impl ItemAlias {
    fn new(words: Vec<String>, label: String) -> Self {
        let normalized_words = words.iter().map(|word| key(word)).collect();
        Self {
            words,
            normalized_words,
            label,
        }
    }
}

const HAND_ALIASES: &[(i32, &[&str], &str)] = &[
    (985, &["tooth"], "Tooth half of key"),
    (987, &["loop"], "Loop half of key"),
    (1745, &["green"], "Green dragon leather"),
    (1065, &["green"], "Green dragon vambraces"),
    (1099, &["green"], "Green dragonhide chaps"),
    (1135, &["green"], "Green dragonhide body"),
    (48, &["u", "unstrung"], "Longbow (u)"),
    (50, &["u", "unstrung"], "Shortbow (u)"),
    (54, &["u", "unstrung"], "Oak shortbow (u)"),
    (56, &["u", "unstrung"], "Oak longbow (u)"),
    (58, &["u", "unstrung"], "Willow longbow (u)"),
    (60, &["u", "unstrung"], "Willow shortbow (u)"),
    (62, &["u", "unstrung"], "Maple longbow (u)"),
    (64, &["u", "unstrung"], "Maple shortbow (u)"),
    (66, &["u", "unstrung"], "Yew longbow (u)"),
    (68, &["u", "unstrung"], "Yew shortbow (u)"),
    (70, &["u", "unstrung"], "Magic longbow (u)"),
    (72, &["u", "unstrung"], "Magic shortbow (u)"),
    (3827, &["saradomin", "sara"], "Saradomin torn page 1"),
    (3828, &["saradomin", "sara"], "Saradomin torn page 2"),
    (3829, &["saradomin", "sara"], "Saradomin torn page 3"),
    (3830, &["saradomin", "sara"], "Saradomin torn page 4"),
    (3831, &["zamorak", "zammy"], "Zamorak torn page 1"),
    (3832, &["zamorak", "zammy"], "Zamorak torn page 2"),
    (3833, &["zamorak", "zammy"], "Zamorak torn page 3"),
    (3834, &["zamorak", "zammy"], "Zamorak torn page 4"),
    (3835, &["guthix"], "Guthix torn page 1"),
    (3836, &["guthix"], "Guthix torn page 2"),
    (3837, &["guthix"], "Guthix torn page 3"),
    (3838, &["guthix"], "Guthix torn page 4"),
    (1673, &["unstrung"], "Gold amulet (u)"),
    (1675, &["unstrung"], "Sapphire amulet (u)"),
    (1677, &["unstrung"], "Emerald amulet (u)"),
    (1679, &["unstrung"], "Ruby amulet (u)"),
    (1681, &["unstrung"], "Diamond amulet (u)"),
    (1683, &["unstrung"], "Dragonstone amulet (u)"),
    (1692, &["strung"], "Gold amulet"),
    (1694, &["strung"], "Sapphire amulet"),
    (1696, &["strung"], "Emerald amulet"),
    (1698, &["strung"], "Ruby amulet"),
    (1700, &["strung"], "Diamond amulet"),
    (1702, &["strung"], "Dragonstone amulet"),
];

const NAME_SYNONYMS: &[(&str, &[&str])] = &[
    ("dragonhide", &["dhide", "d hide", "hide"]),
    ("dragon leather", &["dleather", "d leather"]),
    ("dragonhide body", &["dhide body", "d body", "hide body"]),
    (
        "dragonhide chaps",
        &["dhide chaps", "d chaps", "hide chaps"],
    ),
    (
        "dragon vambraces",
        &["dhide vambs", "d vambs", "vambs", "vambraces"],
    ),
    ("half of a key", &["half key", "key half"]),
    ("sapphire amulet", &["sapp amulet", "sapp ammy"]),
    ("emerald amulet", &["emmy amulet", "emerald ammy"]),
    (
        "dragonstoneamulet",
        &["dragonstone amulet", "dstone amulet", "dstone ammy"],
    ),
];

/// Frozen `tradeable(id)`: an id the selected data does not know is not
/// marked untradeable.
pub(crate) fn tradeable(data: &SelectedGameData, id: i32) -> bool {
    data.item_by_id(id).is_none_or(|item| item.tradeable)
}

/// Frozen `clientName(cat, id)`: the client's own name, the only one a click
/// can be aimed by.
pub(crate) fn client_name(data: &SelectedGameData, id: i32) -> Option<&str> {
    data.item_by_id(id)?.name.as_deref()
}

/// Frozen `displayName(cat, id)`: the alias label, else the plain name, else
/// `item <id>`.
pub(crate) fn display_name(data: &Arc<SelectedGameData>, id: i32) -> String {
    if let Some(alias) = market_index(data).aliases.get(&id) {
        return alias.label.clone();
    }
    match client_name(data, id) {
        Some(name) => name.to_string(),
        None => format!("item {id}"),
    }
}

/// Frozen `unnotedId(cat, id)`: the base item of a note, else the id itself.
pub(crate) fn unnoted_id(data: &SelectedGameData, id: i32) -> i32 {
    data.item_by_id(id)
        .filter(|item| item.is_certificate() && item.certificate_link >= 0)
        .map_or(id, |item| item.certificate_link)
}

/// Frozen `notedId(cat, id)`: the note of a base item, if it has one.
pub(crate) fn noted_id(data: &SelectedGameData, id: i32) -> Option<i32> {
    data.item_by_id(id)
        .filter(|item| !item.is_certificate() && item.certificate_link >= 0)
        .map(|item| item.certificate_link)
}

/// Frozen `ObjRecord` rows: every named obj, notes and pile models included.
pub(crate) fn records(items: &[GameItem]) -> impl Iterator<Item = (&GameItem, &str)> {
    items
        .iter()
        .filter_map(|item| Some((item, item.name.as_deref()?)))
}

/// Frozen `equippable`: the obj has a worn model (`manwear != -1`).
pub(crate) fn equippable(item: &GameItem) -> bool {
    item.worn_model
}

fn listed_name(item: &GameItem) -> Option<&str> {
    let name = item.name.as_deref()?;
    (!(item.is_certificate() && item.certificate_link >= 0)
        && !item.stack_variant
        && item.tradeable
        && worth_stocking(name))
    .then_some(name)
}

/// Frozen `buildCatalog(...).items`: named unnoted objs that are not a pile
/// model, trade, and a shop would stock as their own row, in name then id
/// order. The name order is a case-folded byte order, not ICU collation.
pub(crate) fn listed(items: &[GameItem]) -> Vec<(&GameItem, &str)> {
    let mut out: Vec<_> = items
        .iter()
        .filter_map(|item| listed_name(item).map(|name| (item, name)))
        .collect();
    out.sort_by(|(a, an), (b, bn)| name_order(an, bn).then(a.id.cmp(&b.id)));
    out
}

/// Case-folded first; on a fold tie lowercase sorts ahead, as collation does.
fn name_order(a: &str, b: &str) -> Ordering {
    fn folded(s: &str) -> impl Iterator<Item = u8> + '_ {
        s.bytes().map(|c| c.to_ascii_lowercase())
    }
    folded(a).cmp(folded(b)).then_with(|| b.cmp(a))
}

/// Frozen `worthStocking(name)`: poisoned ammo and fire/lit arrows are not
/// their own shelf row (`/(arrow|bolt|dart|javelin|knife)s?\(p\)$|fire
/// arrows?$|^(un)?lit arrows?$/i`).
pub(crate) fn worth_stocking(name: &str) -> bool {
    let name = name.trim().to_ascii_lowercase();
    let singular = |s: &str| s.strip_suffix('s').unwrap_or(s).to_string();
    if let Some(stem) = name.strip_suffix("(p)") {
        let stem = singular(stem);
        if ["arrow", "bolt", "dart", "javelin", "knife"]
            .iter()
            .any(|kind| stem.ends_with(kind))
        {
            return false;
        }
    }
    let arrows = singular(&name);
    if arrows.ends_with("fire arrow") {
        return false;
    }
    !matches!(arrows.as_str(), "lit arrow" | "unlit arrow")
}

/// Frozen `gen-namecollisions.ts` membership: a named, tradeable, unnoted
/// obj with a debugname. Pile models carry a generated name, not a content
/// one, and are left out.
fn collides(item: &GameItem) -> Option<(&str, &str)> {
    if item.is_certificate() || !item.tradeable || item.stack_variant {
        return None;
    }
    Some((item.alias.as_deref()?, item.name.as_deref()?))
}

type Member<'a> = (i32, &'a str, &'a str);
type Aliases = HashMap<i32, ItemAlias>;

/// One cached, name-sorted market view per selected-data revision.
pub(crate) struct MarketIndex {
    pub aliases: Aliases,
    listed: Vec<ListedItem>,
    listed_ids: HashSet<i32>,
    worn_ids: HashSet<i32>,
    alias_words: HashSet<String>,
    by_label: HashMap<String, HashSet<i32>>,
}

struct ListedItem {
    id: i32,
    name_key: String,
    display_key: String,
    worn: bool,
}

impl MarketIndex {
    pub(crate) fn listed_len(&self) -> usize {
        self.listed.len()
    }

    pub(crate) fn ordered_listed_ids(&self) -> impl Iterator<Item = i32> + '_ {
        self.listed.iter().map(|item| item.id)
    }
}

static MARKET_INDEXES: Mutex<Vec<(Weak<SelectedGameData>, Arc<MarketIndex>)>> =
    Mutex::new(Vec::new());

/// Every selected-revision fact needed by catalog-name resolution.
pub(crate) fn market_index(data: &Arc<SelectedGameData>) -> Arc<MarketIndex> {
    let mut cache = MARKET_INDEXES
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    cache.retain(|(owner, _)| owner.strong_count() > 0);
    if let Some((_, index)) = cache
        .iter()
        .find(|(owner, _)| std::ptr::eq(owner.as_ptr(), Arc::as_ptr(data)))
    {
        return Arc::clone(index);
    }

    let mut aliases = derive_aliases(data.items());
    for &(id, words, label) in HAND_ALIASES {
        aliases.insert(
            id,
            ItemAlias::new(
                words.iter().map(|word| (*word).to_owned()).collect(),
                label.to_owned(),
            ),
        );
    }
    let mut alias_words = HashSet::new();
    let mut by_label: HashMap<String, HashSet<i32>> = HashMap::new();
    for (&id, alias) in &aliases {
        by_label.entry(key(&alias.label)).or_default().insert(id);
        alias_words.extend(alias.normalized_words.iter().cloned());
    }
    let listed = listed(data.items())
        .into_iter()
        .map(|(item, name)| ListedItem {
            id: item.id,
            name_key: key(name),
            display_key: aliases
                .get(&item.id)
                .map_or_else(|| key(name), |alias| key(&alias.label)),
            worn: equippable(item),
        })
        .collect::<Vec<_>>();
    let listed_ids = listed.iter().map(|item| item.id).collect();
    let worn_ids = listed
        .iter()
        .filter(|item| item.worn)
        .map(|item| item.id)
        .collect();
    let index = Arc::new(MarketIndex {
        aliases,
        listed,
        listed_ids,
        worn_ids,
        alias_words,
        by_label,
    });
    cache.push((Arc::downgrade(data), Arc::clone(&index)));
    index
}

/// Frozen `resolveByName` over the same name-sorted rows returned by
/// `liveCatalog`, including hand-written aliases and the frozen synonyms.
pub(crate) fn resolve_by_name(index: &MarketIndex, query: &str, exact_only: bool) -> Vec<i32> {
    let trimmed = query.trim();
    if let Some(id) = trimmed
        .strip_prefix('#')
        .filter(|digits| !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()))
        .and_then(|digits| digits.parse::<i32>().ok())
    {
        return index
            .listed_ids
            .contains(&id)
            .then_some(id)
            .into_iter()
            .collect();
    }

    let query = key(query);
    if query.is_empty() {
        return Vec::new();
    }
    let exact = exactish(index, &query);
    if !exact.is_empty() {
        return prefer_worn(index, exact, false);
    }
    let aliased = resolve_aliased(index, &query);
    if !aliased.is_empty() {
        return aliased;
    }
    if let Some(base) = query.strip_suffix(" u") {
        let base = exactish(index, base);
        if !base.is_empty() {
            return prefer_worn(index, base, true);
        }
    }
    if exact_only {
        return Vec::new();
    }
    let query = expand_synonyms(query);
    let matches = index
        .listed
        .iter()
        .filter(|item| item.name_key.contains(&query) || item.display_key.contains(&query))
        .map(|item| item.id)
        .collect();
    prefer_worn(index, matches, false)
}

fn key(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut space = false;
    for character in value.chars().flat_map(char::to_lowercase) {
        if character.is_ascii_alphanumeric() {
            if space && !out.is_empty() {
                out.push(' ');
            }
            out.push(character);
            space = false;
        } else {
            space = true;
        }
    }
    out
}

fn exactish(index: &MarketIndex, query: &str) -> Vec<i32> {
    let matches = index
        .listed
        .iter()
        .filter(|item| item.name_key == query)
        .map(|item| item.id)
        .collect::<Vec<_>>();
    if !matches.is_empty() || !query.ends_with('s') {
        return matches;
    }
    let singular = &query[..query.len() - 1];
    index
        .listed
        .iter()
        .filter(|item| item.name_key == singular)
        .map(|item| item.id)
        .collect()
}

fn by_label(index: &MarketIndex, query: &str) -> Vec<i32> {
    let matches = with_ids(index, index.by_label.get(query));
    if !matches.is_empty() || !query.ends_with('s') {
        return matches;
    }
    with_ids(index, index.by_label.get(&query[..query.len() - 1]))
}

fn with_ids(index: &MarketIndex, ids: Option<&HashSet<i32>>) -> Vec<i32> {
    let Some(ids) = ids else {
        return Vec::new();
    };
    index
        .listed
        .iter()
        .filter(|item| ids.contains(&item.id))
        .map(|item| item.id)
        .collect()
}

fn resolve_aliased(index: &MarketIndex, query: &str) -> Vec<i32> {
    let spoken = by_label(index, query);
    if !spoken.is_empty() {
        return spoken;
    }

    let mut words = Vec::new();
    let mut rest = String::new();
    for word in query.split_whitespace() {
        if index.alias_words.contains(word) {
            words.push(word);
        } else {
            if !rest.is_empty() {
                rest.push(' ');
            }
            rest.push_str(word);
        }
    }
    let rest = expand_synonyms(rest);
    if rest.is_empty() || (words.is_empty() && rest == query) {
        return Vec::new();
    }
    let base = exactish(index, &rest);
    if base.is_empty() || words.is_empty() {
        return base;
    }
    base.into_iter()
        .filter(|id| {
            index.aliases.get(id).is_some_and(|alias| {
                words.iter().all(|word| {
                    alias
                        .normalized_words
                        .iter()
                        .any(|alias_word| alias_word.as_str() == *word)
                })
            })
        })
        .collect()
}

fn expand_synonyms(mut expanded: String) -> String {
    debug_assert!(expanded.is_ascii());
    for (full, shorts) in NAME_SYNONYMS {
        for short in *shorts {
            let mut start = 0;
            while let Some(relative) = expanded[start..].find(short) {
                let at = start + relative;
                let end = at + short.len();
                let left_boundary = at == 0 || expanded.as_bytes()[at - 1] == b' ';
                let right_boundary = end == expanded.len() || expanded.as_bytes()[end] == b' ';
                if left_boundary && right_boundary {
                    expanded.replace_range(at..end, full);
                    break;
                }
                start = at + 1;
            }
        }
    }
    expanded
}

fn prefer_worn(index: &MarketIndex, matches: Vec<i32>, unstrung: bool) -> Vec<i32> {
    if matches.len() < 2 {
        return matches;
    }
    let mut preferred = None;
    for id in &matches {
        if index.worn_ids.contains(id) != unstrung {
            if preferred.is_some() {
                return matches;
            }
            preferred = Some(*id);
        }
    }
    preferred.map_or(matches, |id| vec![id])
}
fn derive_aliases(items: &[GameItem]) -> Aliases {
    let mut groups: HashMap<String, Vec<Member>> = HashMap::new();
    for item in items {
        if let Some((debugname, name)) = collides(item) {
            groups
                .entry(name.to_lowercase())
                .or_default()
                .push((item.id, debugname, name));
        }
    }
    groups.into_values().flat_map(distinguish).collect()
}

/// Frozen `distinguish` + `usable` + `titled` over one same-name group: the
/// debugname tokens every member shares are dropped, and what is left (minus
/// digits and single letters) names the member.
fn distinguish(mut members: Vec<Member>) -> Vec<(i32, ItemAlias)> {
    if members.len() < 2 {
        return Vec::new();
    }
    members.sort_by_key(|(id, _, _)| *id);
    let tokens: Vec<Vec<&str>> = members
        .iter()
        .map(|(_, debugname, _)| debugname.split('_').filter(|t| !t.is_empty()).collect())
        .collect();
    let shared: Vec<&str> = tokens[0]
        .iter()
        .copied()
        .filter(|t| tokens.iter().all(|other| other.contains(t)))
        .collect();
    let group = members[0].2;
    members
        .iter()
        .zip(&tokens)
        .filter_map(|((id, _, _), tokens)| {
            let words: Vec<String> = tokens
                .iter()
                .filter(|t| !shared.contains(t) && usable(t))
                .map(|t| t.to_string())
                .collect();
            let label = titled(group, words.first()?);
            Some((*id, ItemAlias::new(words, label)))
        })
        .collect()
}

fn usable(word: &str) -> bool {
    word.chars().count() > 1 && !word.chars().any(|c| c.is_ascii_digit())
}

fn titled(name: &str, word: &str) -> String {
    let mut chars = word.chars();
    let head: String = chars
        .next()
        .into_iter()
        .flat_map(char::to_uppercase)
        .collect();
    format!("{head}{} {}", chars.as_str(), name.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use client::io::ClientRevision;
    const COLD_CHILD_ENV: &str = "SCRIPT_MARKET_INDEX_COLD_TEST_CHILD";
    const COLD_TEST_NAME: &str =
        "market_catalog::tests::market_index_cold_build_has_bounded_process_allocation";

    #[test]
    fn market_index_cold_build_has_bounded_process_allocation() {
        if std::env::var_os(COLD_CHILD_ENV).is_none() {
            let output =
                std::process::Command::new(std::env::current_exe().expect("test executable"))
                    .args(["--exact", COLD_TEST_NAME, "--nocapture"])
                    .env(COLD_CHILD_ENV, "1")
                    .output()
                    .expect("run isolated market index allocation test");
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(
                output.status.success() && stdout.contains("1 passed"),
                "isolated market index test failed\nstdout: {stdout}\nstderr: {stderr}"
            );
            return;
        }

        let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
        let size = std::mem::size_of::<MarketIndex>();
        let expected_size = if cfg!(target_pointer_width = "64") {
            264
        } else {
            132
        };
        assert_eq!(size, expected_size, "MarketIndex stack size");

        let mut built = None;
        let cold = allocation_counter::measure(|| {
            built = Some(market_index(&data));
        });
        let built = built.expect("cold MarketIndex");
        let warm = allocation_counter::measure(|| {
            assert!(Arc::ptr_eq(&built, &market_index(&data)));
        });
        // The isolated cold build measured 1,097,161 B total, 293,206 B retained,
        // and 326,622 B peak; each ceiling adds 25% headroom.
        let total_limit = 1_097_161 + 1_097_161 / 4;
        let retained_limit = 293_206 + 293_206 / 4;
        let peak_limit = 326_622 + 326_622 / 4;
        println!(
            "MarketIndex process allocation: size={size} cold={cold:?} warm={warm:?} total_limit={total_limit} retained_limit={retained_limit} peak_limit={peak_limit}"
        );
        assert!(
            cold.bytes_total <= total_limit,
            "total MarketIndex allocation exceeded {total_limit}: {cold:?}"
        );
        assert!(
            cold.bytes_current <= retained_limit as i64,
            "retained MarketIndex allocation exceeded {retained_limit}: {cold:?}"
        );
        assert!(
            cold.bytes_max <= peak_limit,
            "peak MarketIndex allocation exceeded {peak_limit}: {cold:?}"
        );
        assert_eq!(warm, allocation_counter::AllocationInfo::default());
    }

    #[test]
    fn worth_stocking_drops_poisoned_ammo_and_lit_arrows_only() {
        for name in [
            "Bronze arrow(p)",
            "Iron darts(p)",
            "Bronze fire arrows",
            "Lit arrow",
            "Unlit arrows",
        ] {
            assert!(!worth_stocking(name), "{name}");
        }
        for name in ["Dragon dagger(p)", "Bronze arrow", "Coins", "Arrow shafts"] {
            assert!(worth_stocking(name), "{name}");
        }
    }

    #[test]
    fn selected_289_answers_the_frozen_catalog_questions() {
        let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
        // Dragonhides differ only by their debugname colour.
        assert_eq!(display_name(&data, 1753), "Green dragonhide");
        assert_eq!(client_name(&data, 1753), Some("Dragonhide"));
        let all = market_index(&data);
        assert!(
            Arc::ptr_eq(&all, &market_index(&data)),
            "built once, shared"
        );
        assert_eq!(all.aliases[&1747].words, ["black"]);
        assert_eq!(display_name(&data, 1113), "Rune chainbody");
        assert_eq!(display_name(&data, -5), "item -5");
        // Coins trade; the bank note of an untradeable is untradeable too.
        assert!(tradeable(&data, 995));
        let untradeable = data
            .items()
            .iter()
            .find(|item| !item.tradeable && !item.is_certificate())
            .expect("an untradeable obj");
        assert!(!tradeable(&data, untradeable.id));
        let note = data.item_by_id(1113).unwrap().certificate_link;
        assert_eq!(unnoted_id(&data, note), 1113);
        assert_eq!(unnoted_id(&data, 1113), 1113);
        assert_eq!(noted_id(&data, 1113), Some(note));
        assert_eq!(noted_id(&data, note), None);
        let listed = listed(data.items());
        assert!(listed.iter().any(|(item, _)| item.id == 995));
        assert!(listed
            .iter()
            .all(|(item, _)| item.tradeable && !item.stack_variant && !item.is_certificate()));
        assert!(listed.windows(2).all(|pair| {
            name_order(pair[0].1, pair[1].1).then(pair[0].0.id.cmp(&pair[1].0.id))
                != Ordering::Greater
        }));
    }
    #[test]
    fn resolve_by_name_matches_exact_names_aliases_and_synonyms() {
        let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
        let index = market_index(&data);
        let resolve = |query, exact_only| resolve_by_name(&index, query, exact_only);
        assert_eq!(resolve(" #62 ", false), [62]);
        assert!(resolve("#441", false).is_empty());
        assert!(resolve("  ", false).is_empty());
        assert_eq!(resolve("maple longbow", false), [851]);
        assert_eq!(resolve("maple longbow", true), [851]);
        assert_eq!(resolve("maple longbow u", false), [62]);
        assert_eq!(resolve("maple longbows", false), [851]);
        assert_eq!(resolve("maple longbows u", false), [62]);
        assert_eq!(resolve("green dragonhide", false), [1753]);
        assert_eq!(resolve("green dragonhide", true), [1753]);
        assert_eq!(resolve("loop half of a key", false), [987]);
        assert_eq!(resolve("blue dhide", false), [1751]);
        assert_eq!(resolve("blue dhide", true), [1751]);
        assert_eq!(resolve("blue d hide", false), [1751]);
        assert_eq!(resolve("blue hide", false), [1751]);
        assert_eq!(resolve("black d hide", false), [1747]);
        assert_eq!(resolve("green dhide body", false), [1135]);
        assert_eq!(resolve("red d body", false), [2501]);
        assert_eq!(resolve("green dleather", false), [1745]);
        // Synonym expansion leaves both shared-name variants; alias words disambiguate them.
        assert_eq!(resolve("sapp ammy", false), [1675, 1694]);
        assert_eq!(resolve("sapp ammy strung", false), [1694]);
        assert_eq!(resolve("sapp ammy unstrung", false), [1675]);
        assert_eq!(resolve("dhide", false), [1747, 1749, 1751, 1753]);
        assert_eq!(resolve("guam leaf", false), [249]);
        assert_eq!(resolve("guam herb", false), [199]);
        assert!(resolve("green", true).is_empty());
        assert!(resolve("not a real item", true).is_empty());
        assert_eq!(resolve("iron ore u", false), [440]);
        assert_eq!(resolve("iron ore u", true), [440]);
    }

    #[test]
    fn synonym_expansion_replaces_whole_terms_in_upstream_order() {
        assert_eq!(
            expand_synonyms("dhide blue d hide".to_owned()),
            "dragonhide blue dragonhide"
        );
        assert_eq!(expand_synonyms("dhideful".to_owned()), "dhideful");
    }
}
