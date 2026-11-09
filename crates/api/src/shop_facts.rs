//! Selected shop facts decoded from the local game-data core.
//!
//! Shop records and their item rows use the same layout posted to scripts.
//! Each selected revision owns its join cache; missing local shop facts are
//! unavailable rather than borrowed from another revision.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::game_data::SelectedGameData;
use crate::selected::{FactError, LOCAL_GAME_DATA_FAMILY};

/// One stock row after joining inv content with selected item facts.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ShopItemFact {
    pub obj: String,
    pub name: String,
    pub baseline: i32,
    /// Inv stock rate. `World.ts:1173-1198` moves one item toward baseline
    /// when the engine tick is a multiple of this rate.
    #[serde(rename = "restockTicks")]
    pub restock_ticks: i32,
    pub cost: i32,
    pub stackable: bool,
    pub members: bool,
}

/// Posted shop record. Field names match the frozen `ShopRecord` shape.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ShopRecord {
    pub inv: String,
    pub title: String,
    pub keepers: Vec<String>,
    /// Content keeper configs used for NPC joins, not a posted JavaScript field.
    #[serde(skip_serializing)]
    keeper_configs: Vec<String>,
    pub sell: i32,
    pub buy: i32,
    pub delta: i32,
    pub scope: String,
    pub allstock: bool,
    pub items: Vec<ShopItemFact>,
}
/// A borrowed shop, keeper identity, and stocked item.
#[derive(Debug)]
pub struct VendorFact<'a> {
    /// The posted shop record.
    pub record: &'a ShopRecord,
    /// The keeper's content config alias.
    pub keeper_config: &'a str,
    /// The keeper's display name.
    pub keeper_name: &'a str,
    /// The selected game's numeric NPC id for `keeper_config`.
    pub keeper_id: i32,
    /// The stocked item row from `record`.
    pub item: &'a ShopItemFact,
}

#[derive(Debug)]
struct JoinedKeeper {
    shop_index: usize,
    config: String,
    name: String,
    id: i32,
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

static JOINED_274: OnceLock<JoinedShops> = OnceLock::new();
static JOINED_289: OnceLock<JoinedShops> = OnceLock::new();
static JOINED_377: OnceLock<JoinedShops> = OnceLock::new();

struct JoinedShops {
    shops: Vec<ShopRecord>,
    by_inv: HashMap<String, usize>,
    by_keeper: HashMap<String, usize>,
    vendor_keepers: Vec<JoinedKeeper>,
}

fn local_data_unavailable() -> FactError {
    FactError::FamilyUnavailable((*LOCAL_GAME_DATA_FAMILY).clone())
}

fn joined_shops(data: &SelectedGameData) -> Result<&'static JoinedShops, FactError> {
    let source = data.shop_records()?;
    let npc_names = data.npc_names().ok_or_else(local_data_unavailable)?;
    let revision = data
        .selected_revision()
        .ok_or_else(local_data_unavailable)?;
    let cell = match revision {
        crate::selected::ClientRevision::R274 => &JOINED_274,
        crate::selected::ClientRevision::R289 => &JOINED_289,
        crate::selected::ClientRevision::R377 => &JOINED_377,
    };
    Ok(cell.get_or_init(|| {
        let shops = source.to_vec();
        let mut by_inv = HashMap::with_capacity(shops.len());
        let mut by_keeper = HashMap::new();
        for (index, shop) in shops.iter().enumerate() {
            by_inv.insert(shop.inv.to_ascii_lowercase(), index);
            for keeper in &shop.keepers {
                by_keeper
                    .entry(keeper.to_ascii_lowercase())
                    .or_insert(index);
            }
        }
        let vendor_keepers = join_vendor_keepers(npc_names, &shops);
        JoinedShops {
            shops,
            by_inv,
            by_keeper,
            vendor_keepers,
        }
    }))
}

fn join_vendor_keepers(
    npc_names: &crate::game_data::NpcNameFacts,
    shops: &[ShopRecord],
) -> Vec<JoinedKeeper> {
    let mut joined = Vec::new();
    for (shop_index, shop) in shops.iter().enumerate() {
        for config in &shop.keeper_configs {
            let Some(npc) = npc_names.rows.iter().find(|npc| &npc.config == config) else {
                continue;
            };
            let Some(name) = npc.display.as_ref() else {
                continue;
            };
            if !shop.keepers.contains(name) {
                continue;
            }
            joined.push(JoinedKeeper {
                shop_index,
                config: config.clone(),
                name: name.clone(),
                id: npc.id,
            });
        }
    }
    joined
}
/// Posted shops for this selected revision.
pub fn shops_for(data: &SelectedGameData) -> Result<&'static [ShopRecord], FactError> {
    Ok(&joined_shops(data)?.shops)
}

/// Posted shops object borrowed directly from the selected core, keyed by inventory alias.
///
/// Publication needs no NPC keeper join or prepared-family cache.
/// Serialization keeps the product's insertion order in every configuration.
/// Shops follow the generated order, and each record keeps its declared field
/// order. `serde_json::Map` cannot hold that order without the
/// `preserve_order` feature, so this type writes the object directly.
/// Default serializes an empty object when a host has no selected revision.
#[derive(Debug, Default)]
pub struct ContentShops<'a> {
    shops: &'a [ShopRecord],
}

impl Serialize for ContentShops<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap as _;
        let mut map = serializer.serialize_map(Some(self.shops.len()))?;
        for shop in self.shops {
            map.serialize_entry(&shop.inv, shop)?;
        }
        map.end()
    }
}

/// `__rs2b0t_host.content.shops` object keyed by inv, in generated order.
pub fn content_json_value(data: &SelectedGameData) -> Result<ContentShops<'_>, FactError> {
    Ok(ContentShops {
        shops: data.shop_records()?,
    })
}

pub fn shop_by_inv(data: &SelectedGameData, inv: &str) -> Result<Option<ShopRecord>, FactError> {
    let joined = joined_shops(data)?;
    let shop = joined
        .by_inv
        .get(&inv.trim().to_ascii_lowercase())
        .and_then(|index| joined.shops.get(*index))
        .cloned();
    Ok(shop)
}

pub fn shop_by_keeper(
    data: &SelectedGameData,
    keeper: &str,
) -> Result<Option<ShopRecord>, FactError> {
    let joined = joined_shops(data)?;
    let shop = joined
        .by_keeper
        .get(&keeper.trim().to_ascii_lowercase())
        .and_then(|index| joined.shops.get(*index))
        .cloned();
    Ok(shop)
}

/// Borrowed vendor rows for one numeric item id across the selected shops.
pub fn vendors_for<'a>(
    data: &'a SelectedGameData,
    item_id: i32,
) -> Result<impl Iterator<Item = VendorFact<'a>> + 'a, FactError> {
    let item_alias = data
        .item_by_id(item_id)
        .and_then(|item| item.alias.as_deref());
    let joined = joined_shops(data)?;
    Ok(joined.vendor_keepers.iter().filter_map(move |keeper| {
        let item_alias = item_alias?;
        let record = &joined.shops[keeper.shop_index];
        let item = record.items.iter().find(|item| item.obj == item_alias)?;
        Some(VendorFact {
            record,
            keeper_config: &keeper.config,
            keeper_name: &keeper.name,
            keeper_id: keeper.id,
            item,
        })
    }))
}

/// Borrow the selected shop and item row for an explicit keeper config alias.
pub fn vendor_by_keeper_config<'a>(
    data: &'a SelectedGameData,
    config: &str,
    item_id: i32,
) -> Result<Option<VendorFact<'a>>, FactError> {
    let Some(item_alias) = data
        .item_by_id(item_id)
        .and_then(|item| item.alias.as_deref())
    else {
        return Ok(None);
    };
    let joined = joined_shops(data)?;
    let Some(keeper) = joined
        .vendor_keepers
        .iter()
        .find(|keeper| keeper.config == config)
    else {
        return Ok(None);
    };
    let record = &joined.shops[keeper.shop_index];
    let Some(item) = record.items.iter().find(|item| item.obj == item_alias) else {
        return Ok(None);
    };
    Ok(Some(VendorFact {
        record,
        keeper_config: &keeper.config,
        keeper_name: &keeper.name,
        keeper_id: keeper.id,
        item,
    }))
}

/// Content per-unit buying price, never below 1 gp.
///
/// Formula: `shop.rs2:131-147` and `shop.rs2:183-192`.
/// Integer scaling: `NumberOps.ts:124-126`.
pub fn unit_price(item: &ShopItemFact, sell: i32, delta: i32, stock: i32) -> i64 {
    unit_price_values(item.cost, item.baseline, sell, delta, stock)
}

/// Integer buying price for one unit at this stock, never below 1 gp.
pub fn unit_price_values(base_cost: i32, baseline: i32, sell: i32, delta: i32, stock: i32) -> i64 {
    let d = i64::from(stock) - i64::from(baseline);
    let haggle = d.saturating_mul(i64::from(delta)).clamp(-5000, 1000);
    let pct = (i64::from(sell) - haggle).max(100);
    (pct.saturating_mul(i64::from(base_cost)) / 1000).max(1)
}

/// Frozen `shopBuyPrice`: the unit price after `bought` units left the shelf.
pub fn shop_buy_price(base_cost: i32, bought: i32, sell_multiplier: i32, delta: i32) -> i64 {
    let swing = (-i64::from(bought))
        .saturating_mul(i64::from(delta))
        .clamp(-5000, 1000);
    let pct = (i64::from(sell_multiplier) - swing).max(100);
    (pct.saturating_mul(i64::from(base_cost)) / 1000).max(1)
}

/// Frozen `buyoutCost`, summed one unit at a time with bounded work.
pub fn buyout_cost(base_cost: i32, stock: i32, sell_multiplier: i32, delta: i32) -> i64 {
    let stock = stock.max(0);
    let delta = i64::from(delta);
    let variable_limit = match delta.cmp(&0) {
        std::cmp::Ordering::Greater => (5000 + delta - 1) / delta,
        std::cmp::Ordering::Less => {
            let magnitude = delta.saturating_abs();
            (1000 + magnitude - 1) / magnitude
        }
        std::cmp::Ordering::Equal => 0,
    };
    let variable_units = i64::from(stock).min(variable_limit);
    let mut total = 0i64;
    for bought in 0..variable_units {
        total = total.saturating_add(shop_buy_price(
            base_cost,
            bought as i32,
            sell_multiplier,
            delta as i32,
        ));
    }
    let remaining = i64::from(stock) - variable_units;
    total.saturating_add(
        shop_buy_price(
            base_cost,
            variable_units as i32,
            sell_multiplier,
            delta as i32,
        )
        .saturating_mul(remaining),
    )
}
/// Total price from `stock`, pricing each unit at the stock left by earlier units.
///
/// `units` is the unit count to price, not a held target.
/// Content: `shop.rs2:66-67`, `shop.rs2:131-147` and `shop.rs2:183-192`.
/// Integer scaling: `NumberOps.ts:124-126`.
pub fn quote(rec: &ShopRecord, item: &ShopItemFact, units: i32, stock: i32) -> i64 {
    (0..units.max(0)).fold(0i64, |total, unit| {
        total.saturating_add(unit_price(
            item,
            rec.sell,
            rec.delta,
            stock.saturating_sub(unit),
        ))
    })
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
    wants.sort_by_key(|a| std::cmp::Reverse(a.cost));

    let mut left = coins;
    let mut plan = Vec::new();
    for item in wants {
        let have = stock.get(&item.obj).copied().unwrap_or(0).max(0);
        let mut units = 0i32;
        let mut est_cost = 0i64;
        while units < have {
            let next = quote(rec, item, 1, have - units);
            let next_cost = est_cost.saturating_add(next);
            if next_cost > left {
                break;
            }
            est_cost = next_cost;
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

#[cfg(test)]
mod tests {
    use super::*;
    use client::io::ClientRevision;

    fn data(rev: ClientRevision) -> Option<std::sync::Arc<SelectedGameData>> {
        crate::game_data::test_data(rev)
    }

    fn shops_for(data: &SelectedGameData) -> &'static [ShopRecord] {
        super::shops_for(data).expect("generated shop facts")
    }

    fn shop_by_inv(data: &SelectedGameData, inv: &str) -> Option<ShopRecord> {
        super::shop_by_inv(data, inv).expect("generated shop facts")
    }

    fn shop_by_keeper(data: &SelectedGameData, keeper: &str) -> Option<ShopRecord> {
        super::shop_by_keeper(data, keeper).expect("generated shop facts")
    }

    fn vendors_for<'a>(
        data: &'a SelectedGameData,
        item_id: i32,
    ) -> impl Iterator<Item = VendorFact<'a>> + 'a {
        super::vendors_for(data, item_id).expect("generated shop facts")
    }

    fn vendor_by_keeper_config<'a>(
        data: &'a SelectedGameData,
        config: &str,
        item_id: i32,
    ) -> Option<VendorFact<'a>> {
        super::vendor_by_keeper_config(data, config, item_id).expect("generated shop facts")
    }

    fn content_json_value(data: &SelectedGameData) -> ContentShops<'_> {
        super::content_json_value(data).expect("generated shop facts")
    }

    fn aemad(rev: ClientRevision) -> Option<ShopRecord> {
        let data = data(rev)?;
        shop_by_keeper(&data, "Aemad")
    }

    fn aubury(rev: ClientRevision) -> Option<ShopRecord> {
        let data = data(rev)?;
        shop_by_keeper(&data, "Aubury")
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

    const PRESET_KEEPERS: &[&str] = &[
        "Aemad",
        "Lundail",
        "Betty",
        "Aubury",
        "Lowe",
        "Hickton",
        "Gerrant",
        "Harry",
        "Fernahei",
        "Bob",
        "Nurmof",
        "Magic Store owner",
    ];

    const POSTED_INVS: &[&str] = &[
        "adventurershop",
        "magearena_runeshop",
        "magicshop",
        "runeshop",
        "archeryshop",
        "archeryshop2",
        "fishingshop",
        "fishingshop2",
        "shilofishingshop",
        "axeshop",
        "pickaxeshop",
        "magicguildshop",
    ];

    #[test]
    fn all_twelve_shopbuyout_presets_are_posted_on_both_revisions() {
        for rev in [ClientRevision::R274, ClientRevision::R289] {
            let Some(catalog) = data(rev) else {
                return;
            };
            let shops = shops_for(&catalog);
            let invs: Vec<_> = shops.iter().map(|shop| shop.inv.as_str()).collect();
            assert_eq!(invs, POSTED_INVS);
            for keeper in PRESET_KEEPERS {
                assert!(
                    shop_by_keeper(&catalog, keeper).is_some(),
                    "{keeper} must resolve on {rev:?}"
                );
            }
            // Unpublished identities stay fail-closed.
            assert!(shop_by_keeper(&catalog, "Shop keeper").is_none());
            assert!(shop_by_keeper(&catalog, "Chamber guardian").is_none());
            assert!(shop_by_inv(&catalog, "generalshop1").is_none());
            assert!(shop_by_inv(&catalog, "magearena_staffshop").is_none());
        }
    }

    #[test]
    fn selected_377_shop_facts_are_isolated_in_either_lookup_order() {
        let Some(selected_377) = data(ClientRevision::R377) else {
            return;
        };
        let source_377 = selected_377
            .shop_records()
            .expect("selected 377 shop facts");
        assert_eq!(shops_for(&selected_377), source_377);
        let posted = super::content_json_value(&selected_377).expect("selected 377 shop JSON");
        assert!(std::ptr::eq(posted.shops, source_377));

        for revision in [ClientRevision::R274, ClientRevision::R289] {
            let Some(catalog) = data(revision) else {
                return;
            };
            assert_eq!(
                shops_for(&catalog),
                catalog.shop_records().expect("selected legacy shop facts")
            );
            assert_eq!(shops_for(&catalog).len(), POSTED_INVS.len(), "{revision:?}");
            assert!(shop_by_keeper(&catalog, "Aemad").is_some());
            assert_ne!(
                shops_for(&selected_377).as_ptr(),
                shops_for(&catalog).as_ptr(),
                "each revision must retain its own joined records"
            );
        }

        assert_eq!(shops_for(&selected_377), source_377);
        let harpoon_377 = selected_377
            .item_by_alias("harpoon")
            .expect("selected harpoon");
        let gerrant_377 = selected_377
            .npc_by_config("gerrant")
            .expect("selected keeper");
        let vendor = vendor_by_keeper_config(&selected_377, "gerrant", harpoon_377.id)
            .expect("selected 377 harpoon vendor");
        assert_eq!(vendor.keeper_id, gerrant_377.id);
        assert_eq!(vendor.item.cost, harpoon_377.cost);

        let selected_289 = data(ClientRevision::R289).unwrap();
        let harpoon_289 = selected_289
            .item_by_alias("harpoon")
            .expect("legacy harpoon");
        assert_eq!(vendors_for(&selected_289, harpoon_289.id).count(), 2);
        assert_eq!(shops_for(&selected_377), source_377);
    }

    #[test]
    fn missing_377_shop_facts_report_unavailable_without_a_revision_fallback() {
        let selected: SelectedGameData = serde_json::from_value(serde_json::json!({
            "schema_version": 5,
            "revision": 377,
            "items": [],
            "consumption": [],
            "pickpocket": []
        }))
        .expect("selected core with no shop extraction");
        let expected = local_data_unavailable();
        assert_eq!(super::shops_for(&selected).unwrap_err(), expected);
        assert_eq!(
            super::shop_by_inv(&selected, "adventurershop").unwrap_err(),
            expected
        );
        assert_eq!(
            super::shop_by_keeper(&selected, "Aemad").unwrap_err(),
            expected
        );
        assert!(matches!(
            super::vendors_for(&selected, 0),
            Err(error) if error == expected
        ));
        assert_eq!(super::content_json_value(&selected).unwrap_err(), expected);
        assert!(super::vendor_by_keeper_config(&selected, "gerrant", 0)
            .unwrap()
            .is_none());
    }

    #[test]
    fn aemad_facts_join_selected_items_without_invented_prices() {
        for rev in [ClientRevision::R274, ClientRevision::R289] {
            let Some(shop) = aemad(rev) else {
                return;
            };
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
            let Some(catalog) = data(rev) else {
                return;
            };
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
            let Some(shop) = aubury(rev) else {
                return;
            };
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
    fn remaining_preset_facts_join_selected_catalog() {
        for rev in [ClientRevision::R274, ClientRevision::R289] {
            let Some(catalog) = data(rev) else {
                return;
            };
            let lundail = shop_by_keeper(&catalog, "Lundail").expect("Lundail");
            assert_eq!(lundail.inv, "magearena_runeshop");
            assert_eq!(lundail.title, "Lundail's Arena-side Rune Shop.");
            assert_eq!(lundail.sell, 1000);
            assert_eq!(lundail.buy, 600);
            assert_eq!(lundail.delta, 30);
            assert!(!lundail.allstock);
            let law = lundail
                .items
                .iter()
                .find(|item| item.obj == "lawrune")
                .expect("law");
            assert_eq!(law.name, "Law rune");
            assert_eq!(law.baseline, 250);
            assert_eq!(law.restock_ticks, 300);
            assert_eq!(law.cost, catalog.item_by_alias("lawrune").unwrap().cost);
            assert!(law.stackable);

            let betty = shop_by_keeper(&catalog, "Betty").expect("Betty");
            assert_eq!(betty.inv, "magicshop");
            assert_eq!(betty.title, "Betty's Magic Emporium.");
            assert_eq!(betty.sell, 1000);
            assert_eq!(betty.delta, 10);
            let newt = betty
                .items
                .iter()
                .find(|item| item.obj == "eye_of_newt")
                .expect("newt");
            assert_eq!(newt.name, "Eye of newt");
            assert_eq!(newt.baseline, 300);
            assert_eq!(
                newt.cost,
                catalog.item_by_alias("eye_of_newt").unwrap().cost
            );

            let lowe = shop_by_keeper(&catalog, "Lowe").expect("Lowe");
            assert_eq!(lowe.inv, "archeryshop");
            assert_eq!(lowe.title, "Lowe's Archery Emporium");
            assert_eq!(lowe.sell, 1000);
            assert_eq!(lowe.buy, 550);
            assert_eq!(lowe.delta, 10);
            let bronze = lowe
                .items
                .iter()
                .find(|item| item.obj == "bronze_arrow")
                .expect("bronze arrow");
            assert_eq!(bronze.baseline, 2000);
            assert_eq!(bronze.restock_ticks, 10);
            assert!(bronze.stackable);

            let hickton = shop_by_keeper(&catalog, "Hickton").expect("Hickton");
            assert_eq!(hickton.inv, "archeryshop2");
            assert_eq!(hickton.title, "Hicktons Archery Emporium.");
            assert_eq!(hickton.buy, 500);
            let rune_tips = hickton
                .items
                .iter()
                .find(|item| item.obj == "rune_arrowheads")
                .expect("rune tips");
            assert_eq!(rune_tips.baseline, 100);
            assert!(rune_tips.members);
            assert_eq!(
                rune_tips.cost,
                catalog.item_by_alias("rune_arrowheads").unwrap().cost
            );

            let gerrant = shop_by_keeper(&catalog, "Gerrant").expect("Gerrant");
            assert_eq!(gerrant.inv, "fishingshop");
            assert_eq!(gerrant.title, "Gerrant's Fishy Business.");
            assert_eq!(gerrant.buy, 700);
            let feather = gerrant
                .items
                .iter()
                .find(|item| item.obj == "feather")
                .expect("feather");
            assert_eq!(feather.name, "Feather");
            assert_eq!(feather.baseline, 1000);
            assert_eq!(feather.restock_ticks, 1);
            assert_eq!(feather.cost, 2);
            assert!(feather.stackable);

            let harry = shop_by_keeper(&catalog, "Harry").expect("Harry");
            assert_eq!(harry.inv, "fishingshop2");
            assert_eq!(harry.title, "Harrys Fishing Shop.");
            let big_net = harry
                .items
                .iter()
                .find(|item| item.obj == "big_net")
                .expect("big net");
            assert_eq!(big_net.name, "Big fishing net");
            assert_eq!(big_net.baseline, 5);
            assert!(big_net.members);
            assert_eq!(big_net.cost, 20);

            let fernahei = shop_by_keeper(&catalog, "Fernahei").expect("Fernahei");
            assert_eq!(fernahei.inv, "shilofishingshop");
            assert_eq!(fernahei.title, "Fernahai's Fishing Hut.");
            assert_eq!(fernahei.delta, 20);
            assert_eq!(fernahei.items.len(), 7);
            assert_eq!(fernahei.items[3].obj, "feather");
            assert_eq!(fernahei.items[3].baseline, 800);

            let bob = shop_by_keeper(&catalog, "Bob").expect("Bob");
            assert_eq!(bob.inv, "axeshop");
            assert_eq!(bob.title, "Bob's Brilliant Axes.");
            assert_eq!(bob.sell, 1000);
            assert_eq!(bob.buy, 600);
            assert_eq!(bob.delta, 20);
            let bronze_axe = bob
                .items
                .iter()
                .find(|item| item.obj == "bronze_axe")
                .expect("bronze axe");
            assert_eq!(bronze_axe.name, "Bronze axe");
            assert_eq!(bronze_axe.baseline, 10);
            assert_eq!(bronze_axe.cost, 16);
            assert!(!bronze_axe.stackable);

            let nurmof = shop_by_keeper(&catalog, "Nurmof").expect("Nurmof");
            assert_eq!(nurmof.inv, "pickaxeshop");
            assert_eq!(nurmof.title, "Nurmof's Pickaxe Shop.");
            let rune_pick = nurmof
                .items
                .iter()
                .find(|item| item.obj == "rune_pickaxe")
                .expect("rune pick");
            assert_eq!(rune_pick.baseline, 1);
            assert_eq!(rune_pick.restock_ticks, 700);
            assert_eq!(
                rune_pick.cost,
                catalog.item_by_alias("rune_pickaxe").unwrap().cost
            );

            let magic = shop_by_keeper(&catalog, "Magic Store owner").expect("Magic Store owner");
            assert_eq!(magic.inv, "magicguildshop");
            assert_eq!(magic.title, "Magic Guild Store");
            assert_eq!(magic.keepers, ["Magic Store owner"]);
            assert_eq!(magic.sell, 1000);
            assert_eq!(magic.buy, 600);
            assert_eq!(magic.delta, 10);
            let soul = magic
                .items
                .iter()
                .find(|item| item.obj == "soulrune")
                .expect("soul");
            assert_eq!(soul.name, "Soul rune");
            assert_eq!(soul.baseline, 50);
            assert_eq!(soul.cost, 1250);
            assert!(soul.members);
            assert!(soul.stackable);
            let blood = magic
                .items
                .iter()
                .find(|item| item.obj == "bloodrune")
                .expect("blood");
            assert_eq!(blood.cost, 50);
            assert!(blood.members);
        }
    }

    #[test]
    fn vendor_rows_use_keeper_configs_and_item_ids() {
        for revision in [ClientRevision::R274, ClientRevision::R289] {
            let Some(catalog) = data(revision) else {
                return;
            };
            let harpoon_id = catalog.item_by_alias("harpoon").expect("harpoon").id;
            let vendors: Vec<_> = vendors_for(&catalog, harpoon_id).collect();
            assert_eq!(vendors.len(), 2, "{revision:?}");
            assert_eq!(
                vendors
                    .iter()
                    .map(|vendor| vendor.keeper_config)
                    .collect::<Vec<_>>(),
                ["gerrant", "harry"],
                "{revision:?}"
            );
            let keepers = [
                ("Gerrant", "fishingshop", 558, (3013, 3225, 0)),
                ("Harry", "fishingshop2", 576, (2834, 3445, 0)),
            ];
            for (vendor, (name, inv, keeper_id, tile)) in vendors.iter().zip(keepers) {
                assert_eq!(vendor.keeper_name, name);
                assert_eq!(vendor.record.inv, inv);
                assert_eq!(vendor.item.obj, "harpoon");
                assert_eq!(vendor.keeper_id, keeper_id);
                assert!(
                    catalog
                        .npc_placements()
                        .expect("NPC placements")
                        .rows
                        .iter()
                        .any(|placement| {
                            placement.npc_id == keeper_id
                                && (placement.x, placement.z, placement.plane) == tile
                        }),
                    "{revision:?} {name} has a placement"
                );
            }
        }
    }

    #[test]
    fn explicit_vendor_and_stock_coverage_are_exact() {
        let Some(catalog) = data(ClientRevision::R289) else {
            return;
        };
        let bronze_axe_id = catalog.item_by_alias("bronze_axe").expect("bronze axe").id;
        let bronze_axe_vendors: Vec<_> = vendors_for(&catalog, bronze_axe_id).collect();
        assert_eq!(bronze_axe_vendors.len(), 1);
        assert_eq!(bronze_axe_vendors[0].keeper_config, "bob");
        assert_eq!(bronze_axe_vendors[0].keeper_name, "Bob");

        let mithril_axe_id = catalog
            .item_by_alias("mithril_axe")
            .expect("mithril axe")
            .id;
        assert!(vendors_for(&catalog, mithril_axe_id).next().is_none());

        let big_net_id = catalog.item_by_alias("big_net").expect("big net").id;
        let big_net = vendors_for(&catalog, big_net_id)
            .next()
            .expect("Harry sells the big net");
        assert!(big_net.item.members);

        let harpoon_id = catalog.item_by_alias("harpoon").expect("harpoon").id;
        let gerrant = vendor_by_keeper_config(&catalog, "gerrant", harpoon_id)
            .expect("Gerrant's harpoon row");
        assert_eq!(gerrant.record.inv, "fishingshop");
        assert_eq!(
            gerrant.keeper_id,
            catalog.npc_by_config("gerrant").unwrap().id
        );
        assert_eq!(gerrant.item.obj, "harpoon");
        assert!(vendor_by_keeper_config(&catalog, "gerrant", big_net_id).is_none());
        assert!(vendor_by_keeper_config(&catalog, "not_pinned", harpoon_id).is_none());
    }

    #[test]
    fn quote_prices_each_unit_at_the_stock_left() {
        let Some(catalog) = data(ClientRevision::R289) else {
            return;
        };
        let harpoon_id = catalog.item_by_alias("harpoon").expect("harpoon").id;
        let gerrant = vendor_by_keeper_config(&catalog, "gerrant", harpoon_id)
            .expect("Gerrant's harpoon row");
        assert_eq!(quote(gerrant.record, gerrant.item, 1, 2), 5);

        let soulrune_id = catalog.item_by_alias("soulrune").expect("soul rune").id;
        let soulrune = vendor_by_keeper_config(&catalog, "magic_store_owner", soulrune_id)
            .expect("Magic Store owner's soul rune");
        assert_eq!(quote(soulrune.record, soulrune.item, 1, 50), 1250);
        assert_eq!(quote(soulrune.record, soulrune.item, 1, 49), 1262);
        assert_eq!(quote(soulrune.record, soulrune.item, 2, 50), 2512);
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
    fn shop_buy_price_matches_the_engine_integer_curve() {
        assert_eq!(shop_buy_price(100, 0, 100, 10), 10);
        assert_eq!(shop_buy_price(100, 5, 100, 10), 15);
        assert_eq!(shop_buy_price(100, 500, 100, 10), 510);
        assert_eq!(shop_buy_price(0, 5, 100, 10), 1);
        assert_eq!(shop_buy_price(100, 0, 0, 0), 10);
    }

    #[test]
    fn buyout_cost_is_stock_sensitive_and_bounded() {
        assert_eq!(buyout_cost(100, 3, 100, 10), 33);
        assert_eq!(buyout_cost(100, 0, 100, 10), 0);
        assert_eq!(buyout_cost(100, -1, 100, 10), 0);
        let expected = (0..500)
            .map(|bought| shop_buy_price(100, bought, 100, 10))
            .sum::<i64>()
            + i64::from(i32::MAX - 500) * shop_buy_price(100, 500, 100, 10);
        assert_eq!(buyout_cost(100, i32::MAX, 100, 10), expected);
    }

    #[test]
    fn selected_name_casefold_does_not_buy_other_stock() {
        let Some(rec) = aemad(ClientRevision::R289) else {
            return;
        };
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
        let Some(rec) = aemad(ClientRevision::R289) else {
            return;
        };
        let stock = stock_map(&[("vial_water", 500)]);
        assert!(buyout_plan(&rec, &stock, 200, &names(&[])).is_empty());
        assert!(buyout_plan(&rec, &stock, 0, &names(&["Vial of water"])).is_empty());
        assert!(buyout_plan(&rec, &stock, -5, &names(&["Vial of water"])).is_empty());
        let empty_stock = stock_map(&[("vial_water", 0)]);
        assert!(buyout_plan(&rec, &empty_stock, 200, &names(&["Vial of water"])).is_empty());
    }

    #[test]
    fn budget_stops_mid_item_using_stock_sensitive_est_cost() {
        let Some(rec) = aemad(ClientRevision::R289) else {
            return;
        };
        let stock = stock_map(&[("vial_water", 500)]);
        let plan = buyout_plan(&rec, &stock, 5, &names(&["vial of water"]));
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].units, 2);
        assert_eq!(plan[0].est_cost, 4);
    }

    #[test]
    fn chosen_match_is_lowercase_item_name_without_trim() {
        let Some(rec) = aemad(ClientRevision::R289) else {
            return;
        };
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
        let Some(rec) = aubury(ClientRevision::R289) else {
            return;
        };
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
    fn content_json_borrows_shop_records_without_vendor_joins() {
        let data: SelectedGameData = serde_json::from_value(serde_json::json!({
            "schema_version": 5,
            "revision": 274,
            "items": [],
            "consumption": [],
            "pickpocket": [],
            "shops": [{
                "inv": "fixture_shop",
                "title": "Fixture shop",
                "keepers": ["Fixture keeper"],
                "keeper_configs": ["fixture_keeper"],
                "sell": 100,
                "buy": 50,
                "delta": 2,
                "scope": "shared",
                "allstock": false,
                "items": []
            }]
        }))
        .expect("shop facts without unrelated NPC names or prepared families");
        let posted = super::content_json_value(&data).expect("present shop records publish");
        assert!(std::ptr::eq(
            posted.shops,
            data.shop_records().expect("present fixture shop records")
        ));
        let value = serde_json::to_value(posted).expect("posted shop JSON");
        assert_eq!(value["fixture_shop"]["title"], "Fixture shop");
        assert_eq!(value["fixture_shop"]["keepers"][0], "Fixture keeper");
        assert!(value["fixture_shop"].get("keeper_configs").is_none());
        assert_eq!(value.as_object().expect("shop object").len(), 1);
    }

    #[test]
    fn content_json_is_keyed_by_inv_and_omits_unpublished_shops() {
        let Some(data) = data(ClientRevision::R289) else {
            return;
        };
        let value = serde_json::to_value(content_json_value(&data)).expect("posted shop JSON");
        let obj = value.as_object().expect("shops object");
        assert_eq!(obj.len(), POSTED_INVS.len());
        for inv in POSTED_INVS {
            assert!(obj.contains_key(*inv), "missing {inv}");
        }
        assert_eq!(obj["adventurershop"]["items"][0]["name"], "Vial of water");
        assert_eq!(obj["adventurershop"]["items"][0]["restockTicks"], 30);
        assert_eq!(obj["fishingshop"]["keepers"][0], "Gerrant");
        assert!(
            obj.values()
                .all(|shop| shop.get("keeper_configs").is_none()),
            "keeper config joins must not change the posted ShopRecord shape"
        );
        assert!(!obj.contains_key("generalshop1"));
        assert!(!obj.contains_key("magearena_staffshop"));
    }
}
