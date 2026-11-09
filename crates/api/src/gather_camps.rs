//! Curated gather camps: the woodcutting, mining and fishing camp tables of the
//! rs2b0t script API (`data/woodcuttingLocations.ts`, `data/miningLocations.ts`,
//! `data/fishingLocations.ts`), generated from the rs2b0t sources into
//! `gather-catalog.json` by `tools/game-data/generate-gather-catalogs.ts` (an AST
//! read; nothing is evaluated) and compiled by this crate's build script. The
//! tables are immutable process-wide statics; a V8 isolate
//! materializes its own copy once.
//!
//! Fields upstream computes with code are computed here from the generated
//! inputs: the mining name order (`.sort(a.name.localeCompare(b.name))`,
//! `miningLocations.ts:279`) is applied by the generator, and the fish-camp
//! cook surface (`withCampCook`, `fishingLocations.ts:18-36`) by
//! [`fishing_rows`].

use crate::cook_locations::{self, fish_camp_cook_surface_index, CookRole, FishCookSurface};
use crate::selected::{embedded_table, FactError};
use crate::snapshot::WorldTile;
use std::sync::LazyLock;

/// Own properties of a camp object, named as the script API names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CampKey {
    /// `name`.
    Name,
    /// `spot`.
    Spot,
    /// `bankStand`.
    BankStand,
    /// `boothName`.
    BoothName,
    /// `boothOp`.
    BoothOp,
    /// `verified`.
    Verified,
    /// `obstacles`.
    Obstacles,
    /// `campRadius`.
    CampRadius,
    /// `chaseRadius`.
    ChaseRadius,
    /// `resources`.
    Resources,
    /// `avoidSpots`.
    AvoidSpots,
    /// `sweep`.
    Sweep,
    /// `baitVendor`.
    BaitVendor,
    /// `notes`.
    Notes,
    /// `recommendedCombat`.
    RecommendedCombat,
    /// `rangeStand`, set by `withCampCook`.
    RangeStand,
    /// `rangeName`, set by `withCampCook`.
    RangeName,
}

impl CampKey {
    /// The JavaScript property name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Spot => "spot",
            Self::BankStand => "bankStand",
            Self::BoothName => "boothName",
            Self::BoothOp => "boothOp",
            Self::Verified => "verified",
            Self::Obstacles => "obstacles",
            Self::CampRadius => "campRadius",
            Self::ChaseRadius => "chaseRadius",
            Self::Resources => "resources",
            Self::AvoidSpots => "avoidSpots",
            Self::Sweep => "sweep",
            Self::BaitVendor => "baitVendor",
            Self::Notes => "notes",
            Self::RecommendedCombat => "recommendedCombat",
            Self::RangeStand => "rangeStand",
            Self::RangeName => "rangeName",
        }
    }
}

/// A feather vendor beside a fishing camp (`BaitVendor`, `gatheringLocations.ts:44-49`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BaitVendor {
    /// The shopkeeper's name.
    pub keeper: &'static str,
    /// Where to stand to trade.
    pub stand: WorldTile,
    /// List price of one item.
    pub price: i32,
    /// The item sold.
    pub item: &'static str,
}

/// The Shilo water vendor (`SHILO_WATER_VENDOR`, `fishingLocations.ts:4`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WaterVendor {
    /// The shopkeeper's name.
    pub keeper: &'static str,
    /// Where to stand to trade.
    pub stand: WorldTile,
    /// The item sold.
    pub item: &'static str,
}

/// One camp row as the upstream object literal wrote it. An absent optional
/// field and a key whose value is `undefined` differ only in [`Self::keys`],
/// which lists every own property in source order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GatherCamp {
    /// Camp name, the persisted location setting.
    pub name: &'static str,
    /// Home pin.
    pub spot: WorldTile,
    /// Bank stand.
    pub bank_stand: WorldTile,
    /// Bank booth name, when the row sets it.
    pub booth_name: Option<&'static str>,
    /// Bank booth operation, when the row sets it.
    pub booth_op: Option<&'static str>,
    /// Confirmed by live pathability and resource checks.
    pub verified: bool,
    /// Obstacle kinds on the walk.
    pub obstacles: Option<&'static [&'static str]>,
    /// Camp membership radius.
    pub camp_radius: Option<i32>,
    /// Player-relative fishing hop radius.
    pub chase_radius: Option<i32>,
    /// Resource tags.
    pub resources: Option<&'static [&'static str]>,
    /// Tiles to avoid.
    pub avoid_spots: Option<&'static [WorldTile]>,
    /// Fishing sweep tiles.
    pub sweep: Option<&'static [WorldTile]>,
    /// Feather vendor.
    pub bait_vendor: Option<BaitVendor>,
    /// Free-text notes.
    pub notes: Option<&'static str>,
    /// Combat level at which resident aggressive NPCs stop attacking (mining).
    pub recommended_combat: Option<i32>,
    /// Own properties in source order.
    pub keys: &'static [CampKey],
}

#[cfg(all(feature = "embedded-game-data", not(no_local_game_data)))]
mod generated {
    use super::*;
    include!(concat!(env!("OUT_DIR"), "/gather_camps.rs"));
}

/// Woodcutting camps in table order (`WOODCUTTING_LOCATIONS`, `woodcuttingLocations.ts:63-170`).
pub fn woodcutting_camps() -> Result<&'static [GatherCamp], FactError> {
    embedded_table!(generated::WOODCUTTING_CAMPS)
}

/// Mining camps in name order (`MINING_LOCATIONS`, `miningLocations.ts:61-279`).
pub fn mining_camps() -> Result<&'static [GatherCamp], FactError> {
    embedded_table!(generated::MINING_CAMPS)
}

/// Fishing camps before `withCampCook`, in table order (`FISHING_LOCATIONS`, `fishingLocations.ts`).
pub fn fishing_camps() -> Result<&'static [GatherCamp], FactError> {
    embedded_table!(generated::FISHING_CAMPS)
}

/// The Shilo water vendor (`SHILO_WATER_VENDOR`, `fishingLocations.ts:4`).
pub fn shilo_water_vendor() -> Result<&'static WaterVendor, FactError> {
    embedded_table!(&generated::SHILO_WATER_VENDOR)
}

/// `MINING_LOCATION_OPTION_LABELS`: only camps with a combat recommendation, in table order, each as
/// `Name (N Combat recommended)` (`miningLocations.ts:54-59,284-286`).
pub fn mining_option_labels() -> Result<impl Iterator<Item = (&'static str, String)>, FactError> {
    Ok(mining_camps()?.iter().filter_map(|camp| {
        camp.recommended_combat
            .map(|combat| (camp.name, mining_label(camp.name, Some(combat))))
    }))
}

/// `miningLocationLabel` for a typed row: the name, or `Name (N Combat recommended)`.
pub fn mining_label(name: &str, recommended_combat: Option<i32>) -> String {
    match recommended_combat {
        Some(combat) => format!("{name} ({combat} Combat recommended)"),
        None => name.to_string(),
    }
}

/// The obstacle default `withCampCook` applies (`fishingLocations.ts:31`).
const DEFAULT_COOK_OBSTACLES: &[&str] = &["door", "gate"];

/// One fishing camp after `withCampCook`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FishingRow {
    /// The generated camp literal.
    pub camp: &'static GatherCamp,
    /// Own properties after the spread: the literal's keys, then the keys `withCampCook` adds.
    pub keys: Vec<CampKey>,
    /// Index into [`cook_locations::fish_camp_cook_surfaces`] of the pier cook surface that set `rangeStand` and
    /// `rangeName`.
    pub range: Option<usize>,
    /// `obstacles`, with the cook default when the literal has none.
    pub obstacles: Option<&'static [&'static str]>,
    /// `notes`, joined with the cook label when a surface was added.
    pub notes: Option<String>,
}

impl FishingRow {
    /// The pier cook surface joined to this camp.
    pub fn surface(&self) -> Result<Option<&'static FishCookSurface>, FactError> {
        let surfaces = cook_locations::fish_camp_cook_surfaces()?;
        Ok(self.range.and_then(|index| surfaces.get(index)))
    }
}

/// Frozen `withCampCook` (`fishingLocations.ts:18-36`) over the generated camp literals: a camp with a curated
/// pier cook surface gains `rangeStand`, `rangeName`, an obstacle default and notes joined with `; `.
pub fn fishing_rows() -> Result<&'static [FishingRow], FactError> {
    static ROWS: LazyLock<Result<Vec<FishingRow>, FactError>> = LazyLock::new(|| {
        fishing_camps()?
            .iter()
            .map(with_camp_cook)
            .collect::<Result<Vec<_>, _>>()
    });
    match &*ROWS {
        Ok(rows) => Ok(rows),
        Err(error) => Err(error.clone()),
    }
}

fn with_camp_cook(camp: &'static GatherCamp) -> Result<FishingRow, FactError> {
    let plain = FishingRow {
        camp,
        keys: camp.keys.to_vec(),
        range: None,
        obstacles: camp.obstacles,
        notes: camp.notes.map(str::to_string),
    };
    let Some(range) = fish_camp_cook_surface_index(camp.name, CookRole::Pier)? else {
        return Ok(plain);
    };
    let cook = &cook_locations::fish_camp_cook_surfaces()?[range];
    let mut keys = plain.keys;
    for key in [
        CampKey::RangeStand,
        CampKey::RangeName,
        CampKey::Obstacles,
        CampKey::Notes,
    ] {
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    let cook_note = cook
        .notes
        .filter(|notes| !notes.is_empty())
        .map(|_| format!("cook: {}", cook.label.unwrap_or(cook.loc_name)));
    let notes = [camp.notes.map(str::to_string), cook_note]
        .into_iter()
        .flatten()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("; ");
    Ok(FishingRow {
        camp,
        keys,
        range: Some(range),
        obstacles: Some(camp.obstacles.unwrap_or(DEFAULT_COOK_OBSTACLES)),
        notes: Some(notes),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(camps: &[GatherCamp]) -> Vec<&str> {
        camps.iter().map(|camp| camp.name).collect()
    }

    fn woodcutting() -> &'static [GatherCamp] {
        woodcutting_camps().expect("woodcutting camps")
    }

    fn mining() -> &'static [GatherCamp] {
        mining_camps().expect("mining camps")
    }

    fn fishing() -> &'static [GatherCamp] {
        fishing_camps().expect("fishing camps")
    }

    #[test]
    fn tables_keep_the_pinned_counts_and_order() {
        if !crate::test_support::embedded_game_data_available() {
            return;
        }
        assert_eq!(woodcutting().len(), 14);
        assert_eq!(mining().len(), 24);
        assert_eq!(fishing().len(), 9);
        assert_eq!(woodcutting()[0].name, "Draynor (trees)");
        assert_eq!(
            names(mining())[..3],
            ["Al Kharid Mine", "Barbarian Village", "Coal Trucks"]
        );
        assert_eq!(fishing()[0].name, "Draynor Village");
        let mut sorted = names(mining());
        sorted.sort_by_key(|name| name.to_lowercase());
        assert_eq!(sorted, names(mining()), "mining rows are in name order");
    }

    #[test]
    fn helper_rows_keep_undefined_keys_and_literal_rows_omit_absent_ones() {
        if !crate::test_support::embedded_game_data_available() {
            return;
        }
        let wood = &woodcutting()[0];
        assert!(wood.keys.contains(&CampKey::Notes));
        assert!(!wood.keys.contains(&CampKey::RecommendedCombat));
        let rock = mining()
            .iter()
            .find(|camp| camp.name == "Rimmington Mine")
            .expect("row");
        assert_eq!(rock.recommended_combat, None);
        assert!(rock.keys.contains(&CampKey::RecommendedCombat));
        let shilo = fishing()
            .iter()
            .find(|camp| camp.name == "Shilo Village")
            .expect("row");
        assert_eq!(shilo.booth_name, None);
        assert!(!shilo.keys.contains(&CampKey::BoothName));
    }

    #[test]
    fn labels_cover_only_recommended_camps() {
        if !crate::test_support::embedded_game_data_available() {
            return;
        }
        let labels: Vec<_> = mining_option_labels().expect("mining camps").collect();
        assert_eq!(labels.len(), 9);
        assert!(labels.contains(&(
            "Dwarven Mine",
            "Dwarven Mine (65 Combat recommended)".to_string()
        )));
        assert_eq!(mining_label("Plain", None), "Plain");
    }

    #[test]
    fn with_camp_cook_adds_the_pier_surface_in_spread_order() {
        if !crate::test_support::embedded_game_data_available() {
            return;
        }
        let rows = fishing_rows().expect("fishing rows");
        let catherby = rows
            .iter()
            .find(|row| row.camp.name == "Catherby")
            .expect("row");
        let surface = catherby
            .surface()
            .expect("fish camp plans")
            .expect("Catherby has a pier surface");
        assert_eq!(surface.loc_name, "Range");
        assert_eq!(
            catherby.notes.as_deref(),
            Some("cook: Catherby range (bank house)")
        );
        assert_eq!(
            catherby.keys[catherby.keys.len() - 3..],
            [CampKey::RangeStand, CampKey::RangeName, CampKey::Notes],
            "obstacles was already in the literal, so only the other three keys are appended"
        );
        let guild = rows
            .iter()
            .find(|row| row.camp.name == "Fishing Guild")
            .expect("row");
        assert_eq!(
            guild.notes.as_deref(),
            Some("Bank requires Fishing 68; cook: Ardougne range S of guild")
        );
        assert_eq!(guild.obstacles, Some(DEFAULT_COOK_OBSTACLES));
        let taverley = rows
            .iter()
            .find(|row| row.camp.name == "Taverley Dungeon (lava eels)")
            .expect("row");
        assert!(taverley.range.is_none());
        assert_eq!(taverley.keys.as_slice(), taverley.camp.keys);
    }
}
