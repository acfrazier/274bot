//! Fishing methods, gear, shop prices and gear vendors of the rs2b0t script API
//! (`data/fishingMethods.ts`, and the fishing facts of
//! `api/acquisition/ToolAcquire.ts`), generated from the rs2b0t sources into `gather-catalog.json` by
//! `tools/game-data/generate-gather-catalogs.ts` and compiled by this crate's
//! build script. Immutable process-wide statics; the calculations over them (restock and buy plans) live in
//! `script::fishing_gear`.

use crate::game_data::SelectedGameData;
use crate::selected::FactError;
use crate::snapshot::WorldTile;

/// One gear piece a method needs (`FishingGearPiece`, `fishingMethods.ts:1-8`): held count to fish, and the
/// held target a restock tops it up to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GearPiece {
    /// Item name.
    pub name: &'static str,
    /// Minimum held count to fish.
    pub min: i32,
    /// Held target of a restock.
    pub restock: i32,
}

/// One fishing method (`FishingMethod`, `fishingMethods.ts:10-18`): the spot action pair and the gear.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FishingMethod {
    /// Method name, the persisted setting.
    pub name: &'static str,
    /// Primary spot action.
    pub op: &'static str,
    /// The action the spot pairs with it; empty when unpaired.
    pub pair: &'static str,
    /// Gear in the order the method lists it.
    pub gear: &'static [GearPiece],
}

/// A gear shopkeeper (`ToolVendor`, `ToolAcquire.ts:59-67`, without the surface-hop fields these two never set).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FishingVendor {
    /// The shopkeeper's name.
    pub keeper: &'static str,
    /// Where to stand to trade.
    pub stand: WorldTile,
    /// The bank stand to restock coins at.
    pub bank_stand: WorldTile,
}

#[cfg(all(feature = "embedded-game-data", not(no_local_game_data)))]
mod generated {
    use super::*;
    include!(concat!(env!("OUT_DIR"), "/fishing_gear.rs"));
}

/// `FISHING_METHODS` in table order (`fishingMethods.ts:20-82`).
pub fn fishing_methods() -> Result<&'static [FishingMethod], FactError> {
    #[cfg(all(feature = "embedded-game-data", not(no_local_game_data)))]
    {
        Ok(generated::FISHING_METHODS)
    }
    #[cfg(not(all(feature = "embedded-game-data", not(no_local_game_data))))]
    {
        Err(crate::selected::local_game_data_unavailable())
    }
}

/// The pinned `WHIRLPOOL_IDS` (`fishingMethods.ts`); the runtime ids come from [`whirlpool_ids`].
pub fn pinned_whirlpool_ids() -> Result<&'static [i32], FactError> {
    #[cfg(all(feature = "embedded-game-data", not(no_local_game_data)))]
    {
        Ok(generated::WHIRLPOOL_IDS)
    }
    #[cfg(not(all(feature = "embedded-game-data", not(no_local_game_data))))]
    {
        Err(crate::selected::local_game_data_unavailable())
    }
}

/// `FISHING_SHOP_COSTS` in table order (`ToolAcquire.ts`).
pub fn fishing_shop_costs() -> Result<&'static [(&'static str, i32)], FactError> {
    #[cfg(all(feature = "embedded-game-data", not(no_local_game_data)))]
    {
        Ok(generated::FISHING_SHOP_COSTS)
    }
    #[cfg(not(all(feature = "embedded-game-data", not(no_local_game_data))))]
    {
        Err(crate::selected::local_game_data_unavailable())
    }
}

/// `GERRANT_ONLY_FISHING`: the lowercase item names only Gerrant stocks (`ToolAcquire.ts`).
pub fn gerrant_only_fishing() -> Result<&'static [&'static str], FactError> {
    #[cfg(all(feature = "embedded-game-data", not(no_local_game_data)))]
    {
        Ok(generated::GERRANT_ONLY_FISHING)
    }
    #[cfg(not(all(feature = "embedded-game-data", not(no_local_game_data))))]
    {
        Err(crate::selected::local_game_data_unavailable())
    }
}

/// The two fishing gear shops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FishingShop {
    /// Gerrant, Port Sarim (`GERRANT_VENDOR`).
    Gerrant,
    /// Harry, Catherby (`HARRY_VENDOR`).
    Harry,
}

/// The generated gear vendors and the Gerrant-only item list, read together.
#[derive(Debug, Clone, Copy)]
pub struct FishingTables {
    /// Lowercase item names only Gerrant stocks (`GERRANT_ONLY_FISHING`).
    pub gerrant_only: &'static [&'static str],
    /// Harry, Catherby.
    pub harry: &'static FishingVendor,
    /// Gerrant, Port Sarim.
    pub gerrant: &'static FishingVendor,
}

fn harry_vendor() -> Result<&'static FishingVendor, FactError> {
    #[cfg(all(feature = "embedded-game-data", not(no_local_game_data)))]
    {
        Ok(&generated::HARRY_VENDOR)
    }
    #[cfg(not(all(feature = "embedded-game-data", not(no_local_game_data))))]
    {
        Err(crate::selected::local_game_data_unavailable())
    }
}

fn gerrant_vendor() -> Result<&'static FishingVendor, FactError> {
    #[cfg(all(feature = "embedded-game-data", not(no_local_game_data)))]
    {
        Ok(&generated::GERRANT_VENDOR)
    }
    #[cfg(not(all(feature = "embedded-game-data", not(no_local_game_data))))]
    {
        Err(crate::selected::local_game_data_unavailable())
    }
}

/// The vendor tables, or the refusal of a build without local game data.
pub fn fishing_tables() -> Result<FishingTables, FactError> {
    Ok(FishingTables {
        gerrant_only: gerrant_only_fishing()?,
        harry: harry_vendor()?,
        gerrant: gerrant_vendor()?,
    })
}

impl FishingTables {
    /// The generated vendor row.
    pub const fn vendor(&self, shop: FishingShop) -> &'static FishingVendor {
        match shop {
            FishingShop::Gerrant => self.gerrant,
            FishingShop::Harry => self.harry,
        }
    }

    /// Frozen `fishingVendorFor` (`ToolAcquire.ts:427-441`) for a name already lowercased. Items only Gerrant
    /// stocks go to him; otherwise, given a position, the nearer of the two stands (Harry on a tie); with no
    /// position, bait and the big net go to Harry and the rest to Gerrant. `near` receives Harry's and Gerrant's
    /// rows and answers the two distances, or `None` for no position; it runs only when the position decides.
    pub fn vendor_for<E>(
        &self,
        lowered_name: &str,
        near: impl FnOnce(&FishingVendor, &FishingVendor) -> Result<Option<(f64, f64)>, E>,
    ) -> Result<FishingShop, E> {
        if self.gerrant_only.contains(&lowered_name) {
            return Ok(FishingShop::Gerrant);
        }
        if let Some((harry, gerrant)) = near(self.harry, self.gerrant)? {
            return Ok(if harry <= gerrant {
                FishingShop::Harry
            } else {
                FishingShop::Gerrant
            });
        }
        Ok(
            if lowered_name == "fishing bait" || lowered_name == "big fishing net" {
                FishingShop::Harry
            } else {
                FishingShop::Gerrant
            },
        )
    }
}

/// The NPC configs of the four whirlpools, `npc.pack:404-407` of the R289 content.
const WHIRLPOOL_CONFIGS: [&str; 4] = [
    "macro_whirlpool_freshfish",
    "macro_whirlpool_saltfish",
    "macro_whirlpool_rarefish",
    "macro_whirlpool_memberfish",
];

/// `WHIRLPOOL_IDS` derived from the selected content's NPC records, in config order. `None` when the selected
/// data has no NPC names or lacks one of the four whirlpool configs, never a partial list.
pub fn whirlpool_ids(data: &SelectedGameData) -> Option<Vec<i32>> {
    WHIRLPOOL_CONFIGS
        .iter()
        .map(|config| data.npc_by_config(config).map(|row| row.id))
        .collect()
}

/// `FISHING_METHOD_OPTIONS`: the method names in table order (`fishingMethods.ts:84`).
pub fn method_names() -> Result<impl ExactSizeIterator<Item = &'static str>, FactError> {
    Ok(fishing_methods()?.iter().map(|method| method.name))
}

/// `ALL_FISHING_GEAR_NAMES`: every gear name once, in first-seen order (`fishingMethods.ts:86-88`).
pub fn all_gear_names() -> Result<Vec<&'static str>, FactError> {
    let mut names: Vec<&'static str> = Vec::new();
    for piece in fishing_methods()?.iter().flat_map(|method| method.gear) {
        if !names.contains(&piece.name) {
            names.push(piece.name);
        }
    }
    Ok(names)
}

/// `fishingShopCost` over a name already lowercased the way the script's `toLowerCase` did
/// (`ToolAcquire.ts:443-446`): the first price-table row whose key lowercases to it.
pub fn shop_cost(lowered_name: &str) -> Result<Option<i32>, FactError> {
    Ok(fishing_shop_costs()?
        .iter()
        .find(|(key, _)| key.to_ascii_lowercase() == lowered_name)
        .map(|(_, cost)| *cost))
}

/// Frozen `isFishingBaitPiece` (`ToolAcquire.ts:449-455`): a bait or feather stack with a restock target above 1.
/// `lowered_name` runs only past the restock test, as the frozen early return does.
pub fn is_bait_piece<E>(
    restock: f64,
    lowered_name: impl FnOnce() -> Result<String, E>,
) -> Result<bool, E> {
    if restock <= 1.0 {
        return Ok(false);
    }
    let lowered = lowered_name()?;
    Ok(lowered == "fishing bait" || lowered == "feather")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::convert::Infallible;

    fn no_position(_: &FishingVendor, _: &FishingVendor) -> Result<Option<(f64, f64)>, Infallible> {
        Ok(None)
    }

    #[test]
    fn methods_and_gear_names_keep_table_order() {
        if !crate::test_support::embedded_game_data_available() {
            return;
        }
        let methods = fishing_methods().expect("fishing methods");
        assert_eq!(methods.len(), 9);
        assert_eq!(
            method_names().expect("fishing methods").next(),
            Some("Small net — shrimp/anchovy")
        );
        assert_eq!(
            all_gear_names().expect("fishing methods"),
            [
                "Small fishing net",
                "Fishing rod",
                "Fishing bait",
                "Fly fishing rod",
                "Feather",
                "Big fishing net",
                "Lobster pot",
                "Harpoon",
                "Oily fishing rod",
            ]
        );
        let oily = methods.last().expect("methods");
        assert_eq!((oily.op, oily.pair), ("Bait", ""));
    }

    #[test]
    fn shop_costs_match_case_insensitively() {
        if !crate::test_support::embedded_game_data_available() {
            return;
        }
        let cost = |name: &str| shop_cost(name).expect("shop costs");
        assert_eq!(cost("fishing bait"), Some(3));
        assert_eq!(cost("harpoon"), Some(5));
        assert_eq!(cost("lobster pot"), Some(20));
        assert_eq!(cost("oily fishing rod"), None);
    }

    #[test]
    fn vendors_follow_the_frozen_rules() {
        if !crate::test_support::embedded_game_data_available() {
            return;
        }
        let tables = fishing_tables().expect("vendor tables");
        let pick = |name: &str, near| {
            tables
                .vendor_for(name, |_, _| Ok::<_, Infallible>(near))
                .unwrap()
        };
        assert_eq!(pick("feather", Some((1.0, 9.0))), FishingShop::Gerrant);
        assert_eq!(pick("fly fishing rod", None), FishingShop::Gerrant);
        assert_eq!(
            pick("lobster pot", Some((3.0, 3.0))),
            FishingShop::Harry,
            "a tie goes to Harry"
        );
        assert_eq!(pick("lobster pot", Some((4.0, 3.0))), FishingShop::Gerrant);
        assert_eq!(
            pick("lobster pot", Some((f64::NAN, 3.0))),
            FishingShop::Gerrant
        );
        assert_eq!(pick("fishing bait", None), FishingShop::Harry);
        assert_eq!(pick("big fishing net", None), FishingShop::Harry);
        assert_eq!(pick("harpoon", None), FishingShop::Gerrant);
        let mut asked = false;
        tables
            .vendor_for("feather", |_, _| {
                asked = true;
                Ok::<_, Infallible>(None)
            })
            .unwrap();
        assert!(!asked, "a Gerrant-only item never reads the position");
        assert_eq!(
            tables
                .vendor(tables.vendor_for("harpoon", no_position).unwrap())
                .keeper,
            "Gerrant"
        );
        assert_eq!(tables.vendor(FishingShop::Harry).keeper, "Harry");
    }

    #[test]
    fn bait_pieces_need_a_restock_above_one_and_a_bait_name() {
        let bait = |restock, name: &str| {
            is_bait_piece(restock, || Ok::<_, Infallible>(name.to_string())).unwrap()
        };
        assert!(bait(2.0, "fishing bait"));
        assert!(bait(100.0, "feather"));
        assert!(!bait(1.0, "feather"));
        assert!(!bait(2.0, "harpoon"));
        assert!(
            bait(f64::NAN, "feather"),
            "NaN <= 1 is false, so the name decides"
        );
        let mut read = false;
        assert!(!is_bait_piece(1.0, || {
            read = true;
            Ok::<_, Infallible>(String::new())
        })
        .unwrap());
        assert!(!read, "the name is read only past the restock test");
    }

    #[test]
    fn whirlpool_ids_come_from_selected_content_and_match_the_pinned_set() {
        if !crate::test_support::game_data_available() {
            return;
        }
        use client::io::ClientRevision;
        for revision in [ClientRevision::R274, ClientRevision::R289] {
            let data = crate::game_data::for_revision(revision).expect("game data");
            assert_eq!(
                whirlpool_ids(&data).as_deref(),
                Some(pinned_whirlpool_ids().expect("whirlpool ids")),
                "{revision:?}"
            );
        }
    }
}
