//! Immutable generated game facts selected by the bound client revision.

use std::sync::{Arc, OnceLock};

use client::io::ClientRevision;
use serde::Deserialize;

const SCHEMA_VERSION: u16 = 3;
const REVISION_274: &[u8] = include_bytes!("../data/game-data/274.json");
const REVISION_289: &[u8] = include_bytes!("../data/game-data/289.json");

static DATA_274: OnceLock<Result<Arc<SelectedGameData>, String>> = OnceLock::new();
static DATA_289: OnceLock<Result<Arc<SelectedGameData>, String>> = OnceLock::new();

#[derive(Debug, Deserialize)]
struct CacheIdentity {
    cache_id: String,
}

#[derive(Debug, Deserialize)]
struct Provenance {
    cache_identity: CacheIdentity,
}

/// One selected-cache object row. Aliases come from generated server data;
/// no runtime normalization or alias invention is applied.
#[derive(Debug, Deserialize)]
pub struct GameItem {
    pub alias: Option<String>,
    pub id: i32,
    pub name: Option<String>,
    pub cost: i32,
    pub stackable: bool,
    pub members: bool,
    pub certificate_link: i32,
    pub certificate_template: i32,
    pub wear_position: i32,
    pub wear_position_2: i32,
    pub wear_position_3: i32,
}

#[derive(Debug, Deserialize)]
struct NamedFact {
    name: String,
}

#[derive(Debug, Deserialize)]
struct StatHeal {
    stat: String,
    base: i32,
    percent: i32,
}

#[derive(Debug, Deserialize)]
struct ConsumptionFact {
    item: NamedFact,
    stat_heal: Vec<StatHeal>,
    qualification: String,
}

impl ConsumptionFact {
    fn fixed_hp_heal(&self) -> Option<i32> {
        let heal = self.stat_heal.as_slice();
        (self.qualification == "fixed_hp_heal"
            && heal.len() == 1
            && heal[0].stat == "hitpoints"
            && heal[0].percent == 0
            && heal[0].base > 0)
            .then(|| heal[0].base)
    }
}

#[derive(Debug, Deserialize)]
struct PickpocketFact {
    npcs: Vec<NamedFact>,
    level: i32,
}

/// Generated immutable facts for one client/cache revision.
#[derive(Debug, Deserialize)]
pub struct SelectedGameData {
    schema_version: u16,
    revision: i32,
    provenance: Provenance,
    items: Vec<GameItem>,
    consumption: Vec<ConsumptionFact>,
    pickpocket: Vec<PickpocketFact>,
}

impl SelectedGameData {
    fn decode(bytes: &[u8], expected_revision: ClientRevision) -> Result<Arc<Self>, String> {
        let data: Self = serde_json::from_slice(bytes)
            .map_err(|error| format!("generated game data decode: {error}"))?;
        if data.schema_version != SCHEMA_VERSION {
            return Err(format!(
                "generated game data schema mismatch: expected {SCHEMA_VERSION}, got {}",
                data.schema_version
            ));
        }
        if data.revision != expected_revision.as_i32() {
            return Err(format!(
                "generated game data revision mismatch: expected {}, got {}",
                expected_revision.as_i32(),
                data.revision
            ));
        }
        Ok(Arc::new(data))
    }

    pub fn revision(&self) -> i32 {
        self.revision
    }

    pub fn cache_id(&self) -> &str {
        &self.provenance.cache_identity.cache_id
    }

    pub fn items(&self) -> &[GameItem] {
        &self.items
    }

    pub fn item_by_alias(&self, alias: &str) -> Option<&GameItem> {
        self.items
            .iter()
            .find(|item| item.alias.as_deref() == Some(alias))
    }

    /// Generated consumption rows that are qualified as a fixed hitpoint heal.
    pub fn fixed_food_heals(&self) -> impl Iterator<Item = (&str, i32)> {
        self.consumption
            .iter()
            .enumerate()
            .filter_map(move |(index, fact)| {
                let name = fact.item.name.as_str();
                if self.consumption[..index]
                    .iter()
                    .any(|prior| prior.item.name.eq_ignore_ascii_case(name))
                {
                    return None;
                }
                let mut matching = self
                    .consumption
                    .iter()
                    .filter(|other| other.item.name.eq_ignore_ascii_case(name));
                let heal = matching.next()?.fixed_hp_heal()?;
                matching
                    .all(|other| other.fixed_hp_heal() == Some(heal))
                    .then_some((name, heal))
            })
    }

    pub fn fixed_food_heal(&self, name: &str) -> Option<i32> {
        self.fixed_food_heals()
            .find_map(|(known, heal)| known.eq_ignore_ascii_case(name).then_some(heal))
    }

    /// Required level for a generated pickpocket NPC display name.
    pub fn required_thieving(&self, name: &str) -> Option<i32> {
        self.pickpocket.iter().find_map(|fact| {
            fact.npcs
                .iter()
                .any(|npc| npc.name.eq_ignore_ascii_case(name))
                .then_some(fact.level)
        })
    }

    pub fn item_by_id(&self, id: i32) -> Option<&GameItem> {
        self.items.iter().find(|item| item.id == id)
    }

    /// Bounded slot search over generated wearpos facts. Empty query still
    /// returns only a capped page so the editor never serializes the table.
    pub fn search_slot_items(&self, slot: &str, query: &str, limit: usize) -> Vec<ItemSearchHit> {
        let Some(pos) = wearpos_for_loadout_slot(slot) else {
            return Vec::new();
        };
        search_items(
            self.items.iter().filter(|item| {
                !item.is_certificate() && item.wear_position == pos && item.name.is_some()
            }),
            query,
            limit,
        )
    }

    /// Bounded name search for supply rows. Requires a query so the full
    /// object table is not copied into the UI.
    pub fn search_named_items(&self, query: &str, limit: usize) -> Vec<ItemSearchHit> {
        if query.trim().is_empty() {
            return Vec::new();
        }
        search_items(
            self.items
                .iter()
                .filter(|item| !item.is_certificate() && item.name.is_some()),
            query,
            limit,
        )
    }
}

/// One filtered item row for native editors. `alias` and `id` distinguish
/// equal display names without guessing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemSearchHit {
    pub name: String,
    pub alias: String,
    pub id: i32,
}

impl GameItem {
    /// Primary loadout slot from generated `wear_position`, or none for
    /// appearance-only wearpos values (arms/head/jaw) and unequippable rows.
    pub fn loadout_slot(&self) -> Option<&'static str> {
        loadout_slot_for_wearpos(self.wear_position)
    }

    pub fn is_two_handed(&self) -> bool {
        self.wear_position == WEARPOS_RIGHTHAND && self.wear_position_2 == WEARPOS_LEFTHAND
    }

    pub fn is_certificate(&self) -> bool {
        self.certificate_template != -1
    }
}

/// Content `wearpos` ids from the selected ObjType decoder (`getWearPosId`).
pub const WEARPOS_HAT: i32 = 0;
pub const WEARPOS_BACK: i32 = 1;
pub const WEARPOS_FRONT: i32 = 2;
pub const WEARPOS_RIGHTHAND: i32 = 3;
pub const WEARPOS_TORSO: i32 = 4;
pub const WEARPOS_LEFTHAND: i32 = 5;
pub const WEARPOS_LEGS: i32 = 7;
pub const WEARPOS_HANDS: i32 = 9;
pub const WEARPOS_FEET: i32 = 10;
pub const WEARPOS_RING: i32 = 12;
pub const WEARPOS_QUIVER: i32 = 13;

/// Map a generated wearpos onto the loadout slot names used by callers.
/// Appearance slots 6/8/11 (arms/head/jaw) stay unmapped.
pub fn loadout_slot_for_wearpos(pos: i32) -> Option<&'static str> {
    match pos {
        WEARPOS_HAT => Some("hat"),
        WEARPOS_BACK => Some("back"),
        WEARPOS_FRONT => Some("front"),
        WEARPOS_RIGHTHAND => Some("righthand"),
        WEARPOS_TORSO => Some("torso"),
        WEARPOS_LEFTHAND => Some("lefthand"),
        WEARPOS_LEGS => Some("legs"),
        WEARPOS_HANDS => Some("hands"),
        WEARPOS_FEET => Some("feet"),
        WEARPOS_RING => Some("ring"),
        WEARPOS_QUIVER => Some("quiver"),
        _ => None,
    }
}

pub fn wearpos_for_loadout_slot(slot: &str) -> Option<i32> {
    match slot {
        "hat" => Some(WEARPOS_HAT),
        "back" => Some(WEARPOS_BACK),
        "front" => Some(WEARPOS_FRONT),
        "righthand" => Some(WEARPOS_RIGHTHAND),
        "torso" => Some(WEARPOS_TORSO),
        "lefthand" => Some(WEARPOS_LEFTHAND),
        "legs" => Some(WEARPOS_LEGS),
        "hands" => Some(WEARPOS_HANDS),
        "feet" => Some(WEARPOS_FEET),
        "ring" => Some(WEARPOS_RING),
        "quiver" => Some(WEARPOS_QUIVER),
        _ => None,
    }
}

fn search_items<'a, I>(items: I, query: &str, limit: usize) -> Vec<ItemSearchHit>
where
    I: Iterator<Item = &'a GameItem>,
{
    let q = query.trim().to_ascii_lowercase();
    items
        .filter(|item| {
            if q.is_empty() {
                return true;
            }
            let name = item.name.as_deref().unwrap_or("");
            name.to_ascii_lowercase().contains(&q)
                || item
                    .alias
                    .as_deref()
                    .is_some_and(|alias| alias.to_ascii_lowercase().contains(&q))
                || item.id.to_string() == q
        })
        .take(limit)
        .map(|item| ItemSearchHit {
            name: item.name.clone().unwrap_or_default(),
            alias: item.alias.clone().unwrap_or_default(),
            id: item.id,
        })
        .collect()
}

/// Load one revision exactly once and share it for the process lifetime.
pub fn for_revision(revision: ClientRevision) -> Result<Arc<SelectedGameData>, String> {
    let (cell, bytes) = match revision {
        ClientRevision::R274 => (&DATA_274, REVISION_274),
        ClientRevision::R289 => (&DATA_289, REVISION_289),
    };
    match cell.get_or_init(|| SelectedGameData::decode(bytes, revision)) {
        Ok(data) => Ok(Arc::clone(data)),
        Err(error) => Err(error.clone()),
    }
}

/// Load the revision static and require the immutable profile's selected cache.
pub fn for_profile(
    revision: ClientRevision,
    cache_id: &str,
) -> Result<Arc<SelectedGameData>, String> {
    let data = for_revision(revision)?;
    if data.cache_id() != cache_id {
        return Err(format!(
            "generated game data cache identity mismatch: expected {}, got {cache_id}",
            data.cache_id()
        ));
    }
    Ok(data)
}

/// Return selected facts only when this profile uses the generated asset's
/// audited cache identity. Other caches remain honestly metadata-free.
pub fn for_optional_profile(
    revision: ClientRevision,
    cache_id: &str,
) -> Result<Option<Arc<SelectedGameData>>, String> {
    let data = for_revision(revision)?;
    Ok((data.cache_id() == cache_id).then_some(data))
}
