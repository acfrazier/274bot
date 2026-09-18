//! Selected 274/289 shop facts for the ShopBuyout Aemad/Aubury presets.
//!
//! Stock baselines, restock ticks, keeper names and pricing coefficients are
//! parsed from pinned content inv/npc excerpts. Item name, cost, stackability
//! and members flags come from selected-revision `game_data` items — this
//! module does not duplicate catalog/price tables. Only `adventurershop` and
//! `runeshop` are posted; other keepers stay unpublished so preset lookups
//! cannot pretend every shop is supported.

use std::collections::{HashMap, HashSet};

use serde::Serialize;

use crate::game_data::SelectedGameData;

const ADVENTURER_INV: &str = include_str!("../data/shops/adventurershop.inv");
const RUNE_INV: &str = include_str!("../data/shops/runeshop.inv");
const KEEPERS: &str = include_str!("../data/shops/keepers.npc");

/// The only shops this release posts. Other content shops are not implied.
const POSTED_INVS: &[&str] = &["adventurershop", "runeshop"];

/// One stock row after joining inv content with selected item facts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ShopItemFact {
    pub obj: String,
    pub name: String,
    pub baseline: i32,
    #[serde(rename = "restockTicks")]
    pub restock_ticks: i32,
    pub cost: i32,
    pub stackable: bool,
    pub members: bool,
}

/// Posted shop record. Field names match the frozen `ShopRecord` shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ShopRecord {
    pub inv: String,
    pub title: String,
    pub keepers: Vec<String>,
    pub sell: i32,
    pub buy: i32,
    pub delta: i32,
    pub scope: String,
    pub allstock: bool,
    pub items: Vec<ShopItemFact>,
}

/// One planned purchase. `estCost` is the stock-sensitive sum, not units×base.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BuyoutItem {
    pub obj: String,
    pub name: String,
    pub units: i32,
    #[serde(rename = "estCost")]
    pub est_cost: i64,
}

struct ParsedInv {
    inv: String,
    scope: String,
    allstock: bool,
    stock: Vec<(String, i32, i32)>,
}

struct ParsedKeeper {
    name: String,
    owned_shop: String,
    sell: i32,
    buy: i32,
    delta: i32,
    title: String,
}

/// Posted shops for this revision, skipping any inv whose obj aliases are missing.
pub fn shops_for(data: &SelectedGameData) -> Vec<ShopRecord> {
    let invs = parse_invs(&[ADVENTURER_INV, RUNE_INV]);
    let keepers = parse_keepers(KEEPERS);
    POSTED_INVS
        .iter()
        .filter_map(|inv_name| join_shop(*inv_name, &invs, &keepers, data))
        .collect()
}

/// `__rs2b0t_host.content.shops` object keyed by inv. Empty when nothing joined.
pub fn content_json_value(data: &SelectedGameData) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for shop in shops_for(data) {
        map.insert(
            shop.inv.clone(),
            serde_json::to_value(&shop).expect("shop record json"),
        );
    }
    serde_json::Value::Object(map)
}

pub fn shop_by_inv(data: &SelectedGameData, inv: &str) -> Option<ShopRecord> {
    let wanted = inv.trim();
    shops_for(data)
        .into_iter()
        .find(|shop| shop.inv.eq_ignore_ascii_case(wanted))
}

pub fn shop_by_keeper(data: &SelectedGameData, keeper: &str) -> Option<ShopRecord> {
    let wanted = keeper.trim();
    shops_for(data).into_iter().find(|shop| {
        shop.keepers
            .iter()
            .any(|name| name.eq_ignore_ascii_case(wanted))
    })
}

/// Frozen `unitPrice`: stock-sensitive per-unit sell price, never below 1.
pub fn unit_price(item: &ShopItemFact, sell: i32, delta: i32, stock: i32) -> i64 {
    let d = i64::from(stock) - i64::from(item.baseline);
    let haggle = (d * i64::from(delta)).clamp(-5000, 1000);
    let pct = (i64::from(sell) - haggle).max(100);
    (pct * i64::from(item.cost) / 1000).max(1)
}

/// Frozen `buyoutPlan`: casefold name filter, stock>0, descending base cost
/// with stable original order on ties, greedy stock-sensitive units under coins.
pub fn buyout_plan(
    rec: &ShopRecord,
    stock: &HashMap<String, i32>,
    coins: i64,
    chosen: &HashSet<String>,
) -> Vec<BuyoutItem> {
    if coins <= 0 {
        return Vec::new();
    }
    let mut wants: Vec<&ShopItemFact> = rec
        .items
        .iter()
        .filter(|item| {
            chosen.contains(&item.name.to_lowercase())
                && stock.get(&item.obj).copied().unwrap_or(0) > 0
        })
        .collect();
    wants.sort_by(|a, b| b.cost.cmp(&a.cost));

    let mut left = coins;
    let mut plan = Vec::new();
    for item in wants {
        let have = stock.get(&item.obj).copied().unwrap_or(0).max(0);
        let mut units = 0i32;
        let mut est_cost = 0i64;
        while units < have {
            let next = unit_price(item, rec.sell, rec.delta, have - units);
            if est_cost.saturating_add(next) > left {
                break;
            }
            est_cost += next;
            units += 1;
        }
        left -= est_cost;
        if units > 0 {
            plan.push(BuyoutItem {
                obj: item.obj.clone(),
                name: item.name.clone(),
                units,
                est_cost,
            });
        }
    }
    plan
}

fn join_shop(
    inv_name: &str,
    invs: &[ParsedInv],
    keepers: &[ParsedKeeper],
    data: &SelectedGameData,
) -> Option<ShopRecord> {
    let inv = invs.iter().find(|row| row.inv == inv_name)?;
    let owners: Vec<&ParsedKeeper> = keepers
        .iter()
        .filter(|keeper| keeper.owned_shop == inv.inv)
        .collect();
    let first = owners.first()?;
    let mut items = Vec::with_capacity(inv.stock.len());
    for (obj, baseline, restock_ticks) in &inv.stock {
        let fact = data.item_by_alias(obj)?;
        let name = fact.name.as_deref()?.to_string();
        items.push(ShopItemFact {
            obj: obj.clone(),
            name,
            baseline: *baseline,
            restock_ticks: *restock_ticks,
            cost: fact.cost,
            stackable: fact.stackable,
            members: fact.members,
        });
    }
    Some(ShopRecord {
        inv: inv.inv.clone(),
        title: first.title.clone(),
        keepers: owners.iter().map(|owner| owner.name.clone()).collect(),
        sell: first.sell,
        buy: first.buy,
        delta: first.delta,
        scope: inv.scope.clone(),
        allstock: inv.allstock,
        items,
    })
}

fn parse_invs(texts: &[&str]) -> Vec<ParsedInv> {
    let mut out = Vec::new();
    for text in texts {
        out.extend(parse_inv(text));
    }
    out
}

fn parse_inv(text: &str) -> Vec<ParsedInv> {
    let mut out = Vec::new();
    for (id, lines) in blocks(text) {
        let mut stock = Vec::new();
        for line in &lines {
            if let Some(rest) = line.strip_prefix("stock") {
                if let Some((_, payload)) = rest.split_once('=') {
                    let mut parts = payload.split(',');
                    let Some(obj) = parts.next() else { continue };
                    let Some(baseline) = parts.next().and_then(|v| v.parse().ok()) else {
                        continue;
                    };
                    let Some(restock) = parts.next().and_then(|v| v.parse().ok()) else {
                        continue;
                    };
                    if parts.next().is_some() {
                        continue;
                    }
                    stock.push((obj.to_string(), baseline, restock));
                }
            }
        }
        if stock.is_empty() {
            continue;
        }
        out.push(ParsedInv {
            inv: id,
            scope: field(&lines, "scope").unwrap_or_default(),
            allstock: field(&lines, "allstock").as_deref() == Some("yes"),
            stock,
        });
    }
    out
}

fn parse_keepers(text: &str) -> Vec<ParsedKeeper> {
    let mut out = Vec::new();
    for (id, lines) in blocks(text) {
        let Some(owned) = param(&lines, "owned_shop") else {
            continue;
        };
        let Some(sell) = param_i32(&lines, "shop_sell_multiplier") else {
            continue;
        };
        let Some(buy) = param_i32(&lines, "shop_buy_multiplier") else {
            continue;
        };
        let Some(delta) = param_i32(&lines, "shop_delta") else {
            continue;
        };
        let Some(title) = param(&lines, "shop_title") else {
            continue;
        };
        out.push(ParsedKeeper {
            name: field(&lines, "name").unwrap_or(id),
            owned_shop: owned,
            sell,
            buy,
            delta,
            title,
        });
    }
    out
}

fn blocks(text: &str) -> Vec<(String, Vec<String>)> {
    let mut out = Vec::new();
    let mut current: Option<(String, Vec<String>)> = None;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        if let Some(id) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            if let Some(block) = current.take() {
                out.push(block);
            }
            current = Some((id.to_string(), Vec::new()));
            continue;
        }
        if let Some((_, lines)) = current.as_mut() {
            lines.push(line.to_string());
        }
    }
    if let Some(block) = current {
        out.push(block);
    }
    out
}

fn field(lines: &[String], key: &str) -> Option<String> {
    let prefix = format!("{key}=");
    lines
        .iter()
        .find_map(|line| line.strip_prefix(&prefix).map(ToString::to_string))
}

fn param(lines: &[String], key: &str) -> Option<String> {
    let prefix = format!("param={key},");
    lines
        .iter()
        .find_map(|line| line.strip_prefix(&prefix).map(ToString::to_string))
}

fn param_i32(lines: &[String], key: &str) -> Option<i32> {
    param(lines, key)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use client::io::ClientRevision;

    fn data(rev: ClientRevision) -> std::sync::Arc<SelectedGameData> {
        crate::game_data::for_revision(rev).expect("selected game data")
    }

    fn aemad(rev: ClientRevision) -> ShopRecord {
        shop_by_keeper(&data(rev), "Aemad").expect("Aemad posted")
    }

    fn aubury(rev: ClientRevision) -> ShopRecord {
        shop_by_keeper(&data(rev), "Aubury").expect("Aubury posted")
    }

    fn names(chosen: &[&str]) -> HashSet<String> {
        chosen.iter().map(|name| (*name).to_string()).collect()
    }

    fn stock_map(pairs: &[(&str, i32)]) -> HashMap<String, i32> {
        pairs
            .iter()
            .map(|(obj, count)| ((*obj).to_string(), *count))
            .collect()
    }

    #[test]
    fn only_aemad_and_aubury_are_posted_on_both_revisions() {
        for rev in [ClientRevision::R274, ClientRevision::R289] {
            let shops = shops_for(&data(rev));
            let invs: Vec<_> = shops.iter().map(|shop| shop.inv.as_str()).collect();
            assert_eq!(invs, ["adventurershop", "runeshop"]);
            assert!(shop_by_keeper(&data(rev), "Shop keeper").is_none());
            assert!(shop_by_inv(&data(rev), "generalshop1").is_none());
        }
    }

    #[test]
    fn aemad_facts_join_selected_items_without_invented_prices() {
        for rev in [ClientRevision::R274, ClientRevision::R289] {
            let shop = aemad(rev);
            assert_eq!(shop.title, "Aemad's Adventuring Supplies.");
            assert_eq!(shop.keepers, ["Aemad", "Kortan"]);
            assert_eq!(shop.sell, 1300);
            assert_eq!(shop.buy, 400);
            assert_eq!(shop.delta, 20);
            assert!(shop.allstock);
            let vial = shop
                .items
                .iter()
                .find(|item| item.obj == "vial_water")
                .expect("vial");
            let catalog = data(rev);
            let item = catalog.item_by_alias("vial_water").expect("catalog vial");
            assert_eq!(vial.name, "Vial of water");
            assert_eq!(vial.baseline, 500);
            assert_eq!(vial.restock_ticks, 30);
            assert_eq!(Some(vial.cost), Some(item.cost));
            assert_eq!(vial.stackable, item.stackable);
            assert!(!vial.stackable);
            let papyrus = shop
                .items
                .iter()
                .find(|item| item.obj == "papyrus")
                .expect("papyrus");
            assert!(papyrus.members);
            assert_eq!(papyrus.cost, 10);
        }
    }

    #[test]
    fn aubury_facts_join_selected_rune_catalog() {
        for rev in [ClientRevision::R274, ClientRevision::R289] {
            let shop = aubury(rev);
            assert_eq!(shop.title, "Aubury's Rune Shop.");
            assert_eq!(shop.keepers, ["Aubury"]);
            assert_eq!(shop.sell, 1000);
            assert_eq!(shop.delta, 10);
            assert!(!shop.allstock);
            let fire = shop
                .items
                .iter()
                .find(|item| item.obj == "firerune")
                .expect("fire");
            assert_eq!(fire.name, "Fire rune");
            assert_eq!(fire.baseline, 2000);
            assert_eq!(fire.restock_ticks, 10);
            assert!(fire.stackable);
            assert_eq!(fire.cost, 4);
        }
    }

    #[test]
    fn unit_price_is_stock_sensitive_and_never_below_one() {
        let item = ShopItemFact {
            obj: "vial_water".into(),
            name: "Vial of water".into(),
            baseline: 500,
            restock_ticks: 30,
            cost: 2,
            stackable: false,
            members: false,
        };
        assert_eq!(unit_price(&item, 1300, 20, 500), 2);
        assert_eq!(unit_price(&item, 1300, 20, 600), 1);
        assert_eq!(unit_price(&item, 1300, 20, 400), 6);
        let zero = ShopItemFact {
            cost: 0,
            ..item.clone()
        };
        assert_eq!(unit_price(&zero, 1300, 20, 500), 1);
    }

    #[test]
    fn selected_name_casefold_does_not_buy_other_stock() {
        let rec = aemad(ClientRevision::R289);
        let stock = stock_map(&[
            ("vial_water", 500),
            ("bronze_arrow", 500),
            ("iron_axe", 2),
            ("papyrus", 50),
        ]);
        let plan = buyout_plan(&rec, &stock, 200, &names(&["vial of water"]));
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].obj, "vial_water");
        assert_eq!(plan[0].name, "Vial of water");
        assert!(plan[0].units > 0);
        assert!(plan[0].units < 500);
        assert!(plan[0].est_cost <= 200);
    }

    #[test]
    fn empty_or_nonpositive_budget_and_empty_selection_yield_nothing() {
        let rec = aemad(ClientRevision::R289);
        let stock = stock_map(&[("vial_water", 500)]);
        assert!(buyout_plan(&rec, &stock, 200, &names(&[])).is_empty());
        assert!(buyout_plan(&rec, &stock, 0, &names(&["Vial of water"])).is_empty());
        assert!(buyout_plan(&rec, &stock, -5, &names(&["Vial of water"])).is_empty());
        let empty_stock = stock_map(&[("vial_water", 0)]);
        assert!(buyout_plan(&rec, &empty_stock, 200, &names(&["Vial of water"])).is_empty());
    }

    #[test]
    fn budget_stops_mid_item_using_stock_sensitive_est_cost() {
        let rec = aemad(ClientRevision::R289);
        let stock = stock_map(&[("vial_water", 500)]);
        let plan = buyout_plan(&rec, &stock, 5, &names(&["vial of water"]));
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].units, 2);
        assert_eq!(plan[0].est_cost, 4);
    }

    #[test]
    fn chosen_match_is_lowercase_item_name_without_trim() {
        let rec = aemad(ClientRevision::R289);
        let stock = stock_map(&[("vial_water", 500)]);
        assert!(
            buyout_plan(&rec, &stock, 200, &names(&["VIAL OF WATER"])).is_empty(),
            "frozen chosen.has(item.name.toLowerCase()) does not casefold chosen"
        );
        assert!(
            buyout_plan(&rec, &stock, 200, &names(&[" vial of water"])).is_empty(),
            "frozen chosen.has does not trim"
        );
        assert_eq!(
            buyout_plan(&rec, &stock, 200, &names(&["vial of water"])).len(),
            1
        );
    }

    #[test]
    fn descending_base_cost_keeps_stable_order_on_ties() {
        let rec = aubury(ClientRevision::R289);
        let stock = stock_map(&[
            ("firerune", 5),
            ("waterrune", 5),
            ("airrune", 5),
            ("deathrune", 5),
            ("chaosrune", 5),
        ]);
        let plan = buyout_plan(
            &rec,
            &stock,
            10_000,
            &names(&[
                "fire rune",
                "water rune",
                "air rune",
                "death rune",
                "chaos rune",
            ]),
        );
        let objs: Vec<_> = plan.iter().map(|row| row.obj.as_str()).collect();
        assert_eq!(
            objs,
            ["deathrune", "chaosrune", "firerune", "waterrune", "airrune"]
        );
        assert_eq!(plan[0].units, 5);
        assert_eq!(plan[2].units, 5);
        assert!(plan[0].est_cost > plan[2].est_cost);
    }

    #[test]
    fn content_json_is_keyed_by_inv_and_omits_unpublished_shops() {
        let value = content_json_value(&data(ClientRevision::R289));
        let obj = value.as_object().expect("shops object");
        assert_eq!(obj.len(), 2);
        assert!(obj.contains_key("adventurershop"));
        assert!(obj.contains_key("runeshop"));
        assert_eq!(obj["adventurershop"]["items"][0]["name"], "Vial of water");
        assert_eq!(obj["adventurershop"]["items"][0]["restockTicks"], 30);
        assert!(!obj.contains_key("generalshop1"));
    }
}
