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
