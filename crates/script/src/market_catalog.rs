//! Frozen `api/market/catalog.ts` over the selected revision's items: which
//! objs an order book lists, the names a shop says back, and whether the
//! content lets an obj cross a trade window. Every answer reads
//! `SelectedGameData::items`; nothing here is a copied foreign table.
//!
//! Not mapped: the frozen hand-written `ITEM_ALIASES` (green hide, unstrung
//! bow and torn-page labels). Only the aliases the frozen generator derives
//! from debugnames (`gen-namecollisions.ts`) are derived here, once per
//! selected revision and shared by every isolate.

use api::game_data::{GameItem, SelectedGameData};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError, Weak};

/// Frozen `ItemAlias`: the words that separate an obj from its same-named
/// siblings and the label the shop says back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ItemAlias {
    pub words: Vec<String>,
    pub label: String,
}

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
    if let Some(alias) = aliases(data).get(&id) {
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

/// Frozen `equippable`: the obj has a worn slot.
pub(crate) fn equippable(item: &GameItem) -> bool {
    item.wear_position >= 0
}

/// Frozen `buildCatalog(...).items`: named unnoted objs that are not a pile
/// model, trade, and a shop would stock as their own row, in name then id
/// order. The name order is a case-folded byte order, not ICU collation.
pub(crate) fn listed<'a>(items: &'a [GameItem]) -> Vec<(&'a GameItem, &'a str)> {
    let mut out: Vec<_> = records(items)
        .filter(|(item, name)| {
            !(item.is_certificate() && item.certificate_link >= 0)
                && !item.stack_variant
                && item.tradeable
                && worth_stocking(name)
        })
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

/// Derived aliases by obj id.
pub(crate) type Aliases = HashMap<i32, ItemAlias>;

/// One derived alias map per selected-data allocation (one per revision in
/// production), shared by every isolate on every thread.
static ALIASES: Mutex<Vec<(Weak<SelectedGameData>, Arc<Aliases>)>> = Mutex::new(Vec::new());

/// Every derived alias of this selected data, keyed by obj id (frozen
/// `buildAliases` without the hand-written `ITEM_ALIASES`). Built on the
/// first call for this data, then shared.
pub(crate) fn aliases(data: &Arc<SelectedGameData>) -> Arc<Aliases> {
    let mut cache = ALIASES.lock().unwrap_or_else(PoisonError::into_inner);
    // A dropped data's slot could be reused by a new allocation: forget it.
    cache.retain(|(owner, _)| owner.strong_count() > 0);
    if let Some((_, map)) = cache
        .iter()
        .find(|(owner, _)| std::ptr::eq(owner.as_ptr(), Arc::as_ptr(data)))
    {
        return Arc::clone(map);
    }
    let map = Arc::new(derive_aliases(data.items()));
    cache.push((Arc::downgrade(data), Arc::clone(&map)));
    map
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
            Some((*id, ItemAlias { words, label }))
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
        let all = aliases(&data);
        assert!(Arc::ptr_eq(&all, &aliases(&data)), "built once, shared");
        assert_eq!(all[&1747].words, ["black"]);
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
}
