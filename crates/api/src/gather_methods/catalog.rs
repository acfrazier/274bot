//! Typed gathering family: the shared, lazily prepared catalog of resource methods, their targets, products,
//! tools, consumed supplies, requirements and world placements. Built once per selected pin from the checked
//! `gathering.json` family (see `wire` and `cache`), shared by `Arc` and released after its last consumer.
//!
//! Facts are three-valued. `Known` is an examined complete set, `Partial` keeps useful rows next to explicit
//! gaps and `Unknown` is an honest extraction gap: no query turns Unknown into an empty answer.

use super::SceneRegionInput;
use crate::selected::FactError;
use crate::selected::{
    EntityId, FactKey, Gap, ItemAmount, Knowledge, Requirement, SelectedPin, SkillMinimum,
    SourceSpan, Truth,
};
use crate::WorldTile;
use serde::{Deserialize, Serialize};
use std::collections::{BinaryHeap, HashMap};
use std::sync::Arc;

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
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

/// What a content zone does to a method's yield for placements inside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoneEffect {
    /// The catch is handed to an NPC (Miscellania): the method's product never reaches the inventory.
    YieldIntercepted,
    /// A different item is produced (the Family Crest perfect gold): not the method's product.
    ProductSubstituted,
    /// A quest-state guard can refuse the action; whether it does depends on state the facts cannot see.
    StateGated,
}

/// One content-defined rectangle (inclusive, one level) with its effect on a set of methods.
#[derive(Debug, Clone)]
pub struct ZoneRule {
    pub level: i32,
    pub min_x: i32,
    pub min_z: i32,
    pub max_x: i32,
    pub max_z: i32,
    pub effect: ZoneEffect,
    pub sources: Arc<[SourceSpan]>,
}

/// A resource entity the extraction saw but does not offer as a method target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LooseClass {
    /// An empty rock decoration: content marks it `mining_rock_empty`, no resource turns into it.
    Depleted,
    /// Content could not classify it; see the gap.
    Unclassified,
}

#[derive(Debug, Clone)]
pub struct LooseEntity {
    pub skill: GatherSkill,
    pub entity: EntityId,
    pub class: LooseClass,
    pub gap: Option<Gap>,
}

/// Classification of one rock loc: its class, its method when unambiguous, and why it is unclassified.
#[derive(Debug, Clone, Copy)]
pub struct RockFact<'a> {
    pub loc: i32,
    pub alias: &'a str,
    pub class: TargetClass,
    /// `None` for shared depleted stages (`rocks1`) and for rocks no method offers.
    pub method: Option<&'a GatherMethod>,
    pub gap: Option<&'a Gap>,
}

/// One bounded page of placements. `next` is the cursor to pass as `after` for the following page.
#[derive(Debug)]
pub struct SpotPage<'a> {
    pub spots: Vec<&'a GatherSpot>,
    pub truncated: bool,
    pub next: Option<SpotId>,
}

/// How strictly a nearest-placement search treats zone rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessPolicy {
    /// Only placements whose yield content proves reachable (`Truth::True`).
    Usable,
    /// Placements that are not proven unusable (`True` or `Unknown`).
    Possible,
    /// Every placement, whatever its zone effect.
    Any,
}

#[derive(Debug, Clone, Copy)]
pub struct Nearest<'a> {
    pub spot: &'a GatherSpot,
    pub distance_sq: u64,
    pub access: Truth,
}

/// One method's spots sharing a level and 64x64 map square: contiguous in the method's spot slice.
#[derive(Debug, Clone, Copy)]
pub(super) struct Bucket {
    pub level: i32,
    pub mx: i32,
    pub mz: i32,
    pub start: u32,
    pub end: u32,
}

/// Per-method data that lives beside the seam-declared `GatherMethod`.
#[derive(Debug)]
pub(super) struct MethodExtra {
    pub op: Option<(u8, Arc<str>)>,
    pub zones: Box<[u16]>,
    pub buckets: Box<[Bucket]>,
    /// `SpotId` of this method's first placement; ids are contiguous per method.
    pub first_spot: u32,
}

pub(super) type EntityKey = (u8, i32);
pub(super) const KIND_LOC: u8 = 0;
pub(super) const KIND_NPC: u8 = 1;
pub(super) const KIND_OBJ: u8 = 2;

pub(super) fn entity_key(entity: EntityId) -> EntityKey {
    match entity {
        EntityId::Loc(id) => (KIND_LOC, id),
        EntityId::Npc(id) => (KIND_NPC, id),
        EntityId::Obj(id) => (KIND_OBJ, id),
    }
}

#[derive(Debug)]
pub(super) struct RockEntry {
    pub class: TargetClass,
    pub method: Option<usize>,
    pub gap: Option<Gap>,
}

/// Shared, immutable gathering catalog for one selected pin.
pub struct GatherCatalog {
    pub(super) pin: Arc<SelectedPin>,
    pub(super) methods: Box<[GatherMethod]>,
    pub(super) by_id: HashMap<Arc<str>, usize>,
    pub(super) extras: Box<[MethodExtra]>,
    pub(super) zones: Box<[ZoneRule]>,
    pub(super) aliases: Box<[(EntityKey, Arc<str>)]>,
    pub(super) loose: Box<[LooseEntity]>,
    pub(super) rocks: HashMap<i32, RockEntry>,
}

impl std::fmt::Debug for GatherCatalog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GatherCatalog")
            .field("revision", &self.pin.revision)
            .field("methods", &self.methods.len())
            .finish()
    }
}

fn no_coverage(method: &GatherMethod, code: &str, source: Option<&SourceSpan>) -> FactError {
    FactError::Invalid {
        source: source.cloned().unwrap_or_else(|| SourceSpan {
            file: Arc::from("gathering.json"),
            first: 0,
            last: 0,
        }),
        reason: Arc::from(format!("{}: {code}", method.id.0)),
    }
}

/// The first gap explaining why a fact is not complete.
pub fn first_gap<T>(knowledge: &Knowledge<T>) -> Option<&Gap> {
    match knowledge {
        Knowledge::Known(_) => None,
        Knowledge::Partial { gaps, .. } => gaps.first(),
        Knowledge::Unknown(gap) => Some(gap),
    }
}

/// The rows a fact carries, whether or not it is complete.
pub fn known_rows<T>(knowledge: &Knowledge<Arc<[T]>>) -> &[T] {
    match knowledge {
        Knowledge::Known(rows) => rows,
        Knowledge::Partial { known, .. } => known,
        Knowledge::Unknown(_) => &[],
    }
}

impl GatherCatalog {
    /// The selected pin this catalog was verified against.
    pub fn pin(&self) -> &SelectedPin {
        &self.pin
    }

    /// Every method: woodcutting, then mining, then fishing, each in content order.
    pub fn methods(&self) -> &[GatherMethod] {
        &self.methods
    }

    pub fn method(&self, id: &str) -> Result<&GatherMethod, FactError> {
        self.by_id
            .get(id)
            .map(|index| &self.methods[*index])
            .ok_or_else(|| FactError::UnknownKey(FactKey::new(id)))
    }

    /// Methods whose `resources` contain `key` (ASCII case-insensitive, trimmed). Woodcutting and mining keys are
    /// wood/ore keys; fishing keys are fish item aliases, so one key can name several methods.
    pub fn methods_for_resource<'a>(
        &'a self,
        key: &'a str,
    ) -> impl Iterator<Item = &'a GatherMethod> + 'a {
        let wanted = key.trim();
        self.methods.iter().filter(move |method| {
            !wanted.is_empty()
                && method
                    .resources
                    .iter()
                    .any(|resource| resource.0.eq_ignore_ascii_case(wanted))
        })
    }

    fn index_of(&self, method: &GatherMethod) -> Result<usize, FactError> {
        match self.by_id.get(&*method.id.0) {
            Some(index) if std::ptr::eq(&self.methods[*index], method) => Ok(*index),
            _ => Err(FactError::UnknownKey(method.id.clone())),
        }
    }

    /// The method's complete placements, or why there is nothing trustworthy to search: unknown or partial
    /// coverage is an error, never an empty iterator. Callers that accept incomplete data read `method.spots`.
    fn complete_spots<'a>(
        &'a self,
        method: &'a GatherMethod,
    ) -> Result<(&'a [GatherSpot], usize), FactError> {
        let index = self.index_of(method)?;
        match &method.spots {
            Knowledge::Known(spots) => Ok((spots, index)),
            Knowledge::Partial { gaps, .. } => Err(no_coverage(
                method,
                &format!(
                    "placements partial ({})",
                    gaps.first().map_or("unspecified", |gap| &*gap.code)
                ),
                gaps.first().and_then(|gap| gap.sources.first()),
            )),
            Knowledge::Unknown(gap) => Err(no_coverage(
                method,
                &format!("placements unknown ({})", gap.code),
                gap.sources.first(),
            )),
        }
    }

    /// Placements of `method` inside `region`, ascending by `SpotId`. Uncapped and lazy: the caller decides how
    /// many to take, and nothing is collected up front (the iterator walks the method's buckets and their spot
    /// slices in place, capturing only the copied region). Unknown or partial placements are an error, never an
    /// empty iterator.
    pub fn spots<'a>(
        &'a self,
        method: &'a GatherMethod,
        region: &SceneRegionInput,
    ) -> Result<impl Iterator<Item = &'a GatherSpot>, FactError> {
        let (spots, index) = self.complete_spots(method)?;
        let region = *region;
        let (min_mx, max_mx) = (region.min_x >> 6, region.max_x >> 6);
        let (min_mz, max_mz) = (region.min_z >> 6, region.max_z >> 6);
        Ok(self.extras[index]
            .buckets
            .iter()
            .filter(move |bucket| {
                bucket.level == region.level
                    && (min_mx..=max_mx).contains(&bucket.mx)
                    && (min_mz..=max_mz).contains(&bucket.mz)
            })
            .flat_map(move |bucket| spots[bucket.start as usize..bucket.end as usize].iter())
            .filter(move |spot| {
                (region.min_x..=region.max_x).contains(&spot.origin.x)
                    && (region.min_z..=region.max_z).contains(&spot.origin.z)
            }))
    }

    /// One bounded page of `spots`, resuming after the `SpotId` cursor. `truncated` is true when more placements
    /// remain; `next` is then the cursor for the following page. `limit` must be at least 1.
    pub fn page<'a>(
        &'a self,
        method: &'a GatherMethod,
        region: &SceneRegionInput,
        after: Option<SpotId>,
        limit: usize,
    ) -> Result<SpotPage<'a>, FactError> {
        if limit == 0 {
            return Err(no_coverage(method, "page limit must be at least 1", None));
        }
        let mut remaining = self
            .spots(method, region)?
            .filter(|spot| after.is_none_or(|cursor| spot.id > cursor));
        let mut spots: Vec<&GatherSpot> = Vec::with_capacity(limit.min(64));
        for spot in remaining.by_ref() {
            if spots.len() == limit {
                return Ok(SpotPage {
                    next: spots.last().map(|last| last.id),
                    spots,
                    truncated: true,
                });
            }
            spots.push(spot);
        }
        Ok(SpotPage {
            spots,
            truncated: false,
            next: None,
        })
    }

    /// Whether a placement's yield reaches the player, from the method's content zones: `False` when a zone
    /// intercepts or substitutes the product, `Unknown` when only a quest-state guard applies, else `True`.
    pub fn access(&self, method: &GatherMethod, spot: &GatherSpot) -> Result<Truth, FactError> {
        Ok(self.access_at(self.index_of(method)?, spot))
    }

    fn access_at(&self, index: usize, spot: &GatherSpot) -> Truth {
        let mut truth = Truth::True;
        for zone in self.extras[index]
            .zones
            .iter()
            .map(|each| &self.zones[*each as usize])
        {
            let inside = zone.level == spot.origin.level
                && (zone.min_x..=zone.max_x).contains(&spot.origin.x)
                && (zone.min_z..=zone.max_z).contains(&spot.origin.z);
            if !inside {
                continue;
            }
            match zone.effect {
                ZoneEffect::YieldIntercepted | ZoneEffect::ProductSubstituted => {
                    return Truth::False
                }
                ZoneEffect::StateGated => truth = Truth::Unknown,
            }
        }
        truth
    }

    /// The closest placements to `from` on its level, searched over the method's complete domain (never one
    /// page), nearest first with `SpotId` breaking ties. `within` optionally bounds the candidates.
    pub fn nearest<'a>(
        &'a self,
        method: &'a GatherMethod,
        from: WorldTile,
        within: Option<&SceneRegionInput>,
        policy: AccessPolicy,
        limit: usize,
    ) -> Result<Vec<Nearest<'a>>, FactError> {
        let (spots, index) = self.complete_spots(method)?;
        if limit == 0 {
            return Ok(Vec::new());
        }
        let mut best: BinaryHeap<(u64, SpotId, usize)> =
            BinaryHeap::with_capacity(limit.min(64) + 1);
        for (position, spot) in spots.iter().enumerate() {
            if spot.origin.level != from.level {
                continue;
            }
            if within.is_some_and(|region| {
                region.level != from.level
                    || !(region.min_x..=region.max_x).contains(&spot.origin.x)
                    || !(region.min_z..=region.max_z).contains(&spot.origin.z)
            }) {
                continue;
            }
            let allowed = match (policy, self.access_at(index, spot)) {
                (AccessPolicy::Any, _) => true,
                (AccessPolicy::Possible, access) => access != Truth::False,
                (AccessPolicy::Usable, access) => access == Truth::True,
            };
            if !allowed {
                continue;
            }
            let (dx, dz) = (
                i64::from(spot.origin.x) - i64::from(from.x),
                i64::from(spot.origin.z) - i64::from(from.z),
            );
            best.push(((dx * dx + dz * dz) as u64, spot.id, position));
            if best.len() > limit {
                best.pop();
            }
        }
        Ok(best
            .into_sorted_vec()
            .into_iter()
            .map(|(distance_sq, _, position)| Nearest {
                spot: &spots[position],
                distance_sq,
                access: self.access_at(index, &spots[position]),
            })
            .collect())
    }

    /// The placement with this id and the method that owns it. Ids are valid only within this catalog.
    pub fn spot(&self, id: SpotId) -> Option<(&GatherMethod, &GatherSpot)> {
        let index = self
            .extras
            .partition_point(|extra| extra.first_spot <= id.0)
            .checked_sub(1)?;
        let spot = known_rows(&self.methods[index].spots)
            .get((id.0 - self.extras[index].first_spot) as usize)?;
        (spot.id == id).then_some((&self.methods[index], spot))
    }

    /// Content alias (`copperrock1`, `0_48_53_freshfish`, `raw_trout`) of an entity the catalog mentions.
    pub fn alias(&self, entity: EntityId) -> Option<&str> {
        let key = entity_key(entity);
        self.aliases
            .binary_search_by_key(&key, |(each, _)| *each)
            .ok()
            .map(|at| &*self.aliases[at].1)
    }

    /// The interaction option the method's targets are used with: slot and its content label.
    pub fn op(&self, method: &GatherMethod) -> Result<Option<(u8, &str)>, FactError> {
        let index = self.index_of(method)?;
        Ok(self.extras[index]
            .op
            .as_ref()
            .map(|(slot, label)| (*slot, &**label)))
    }

    /// Zone rules that apply to the method.
    pub fn zones<'a>(
        &'a self,
        method: &'a GatherMethod,
    ) -> Result<impl Iterator<Item = &'a ZoneRule>, FactError> {
        let index = self.index_of(method)?;
        Ok(self.extras[index]
            .zones
            .iter()
            .map(|each| &self.zones[*each as usize]))
    }

    /// Entities the extraction saw but does not offer as method targets, with why.
    pub fn loose(&self) -> &[LooseEntity] {
        &self.loose
    }

    /// Classification of one rock loc. `None` means the loc is not a rock the content offers a Mine op on; an
    /// unclassified rock is `Some` with `class: Unclassified` and a gap, never empty and never resource.
    pub fn rock(&self, loc: i32) -> Option<RockFact<'_>> {
        let entry = self.rocks.get(&loc)?;
        Some(RockFact {
            loc,
            alias: self.alias(EntityId::Loc(loc))?,
            class: entry.class,
            method: entry.method.map(|index| &self.methods[index]),
            gap: entry.gap.as_ref(),
        })
    }

    /// Every rock loc with its classification, ascending by loc id.
    pub fn rocks(&self) -> Vec<RockFact<'_>> {
        let mut locs: Vec<i32> = self.rocks.keys().copied().collect();
        locs.sort_unstable();
        locs.into_iter().filter_map(|loc| self.rock(loc)).collect()
    }
}
