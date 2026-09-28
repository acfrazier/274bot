//! Typed gathering family. Extraction, complete spatial index and preparation
//! belong to M-306; legacy queries cut over atomically with those assets.

use super::SceneRegionInput;
use crate::selected::FactError;
use crate::selected::{
    EntityId, FactKey, ItemAmount, Knowledge, Requirement, SkillMinimum, SourceSpan,
};
use crate::WorldTile;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub struct GatherCatalog {
    _private: (),
}

impl GatherCatalog {
    /// Body owned by M-306; typed family assets are not installed yet.
    pub fn method(&self, _id: &str) -> Result<&GatherMethod, FactError> {
        Err(FactError::FamilyUnavailable(
            crate::selected::GATHERING_FAMILY.clone(),
        ))
    }

    /// Body owned by M-306; unavailable coverage is not an empty iterator.
    pub fn spots<'a>(
        &'a self,
        _method: &GatherMethod,
        _region: &SceneRegionInput,
    ) -> Result<impl Iterator<Item = &'a GatherSpot>, FactError> {
        Err::<std::iter::Empty<&'a GatherSpot>, _>(FactError::FamilyUnavailable(
            crate::selected::GATHERING_FAMILY.clone(),
        ))
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GatherSkill {
    Woodcutting,
    Mining,
    Fishing,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TargetClass {
    Resource,
    Depleted,
    Hazard,
    Unclassified,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GatherTarget {
    pub entity: EntityId,
    pub op: u8,
    pub class: TargetClass,
    pub respawn: Knowledge<Option<RespawnFact>>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolUse {
    pub item: i32,
    #[serde(deserialize_with = "Deserialize::deserialize")]
    pub use_gate: Option<SkillMinimum>,
    #[serde(deserialize_with = "Deserialize::deserialize")]
    pub wield_gate: Option<SkillMinimum>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GatherYield {
    pub item: i32,
    pub level: u16,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SpotId(pub u32);
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RespawnScale {
    pub rule: FactKey,
    pub min_ticks: u32,
    pub max_ticks: u32,
    pub sources: Arc<[SourceSpan]>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RespawnFact {
    pub raw: u32,
    pub scale: Knowledge<RespawnScale>,
    pub source: SourceSpan,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GatherSpot {
    pub id: SpotId,
    pub entity: EntityId,
    pub origin: WorldTile,
    pub width: u16,
    pub length: u16,
    pub movement: Knowledge<Option<SceneRegionInput>>,
    pub source: SourceSpan,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GatherMethod {
    pub id: FactKey,
    pub skill: GatherSkill,
    pub resources: Arc<[FactKey]>,
    pub targets: Knowledge<Arc<[GatherTarget]>>,
    pub products: Knowledge<Arc<[GatherYield]>>,
    pub tools: Knowledge<Arc<[ToolUse]>>,
    pub consumes: Knowledge<Arc<[ItemAmount]>>,
    pub requirements: Knowledge<Arc<[Requirement]>>,
    pub spots: Knowledge<Arc<[GatherSpot]>>,
}
