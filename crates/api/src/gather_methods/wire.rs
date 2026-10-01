//! Wire format of `gathering.json` (extractor schema 2) and its decode into the typed catalog. Decoding runs once
//! per prepared catalog, off-pump; every violation is a typed `FactError`, nothing is defaulted or guessed.
//!
//! Field meanings mirror `tools/game-data/extractors/gathering.ts`: a `Know` cell is `known`, `partial` (rows plus
//! gaps) or `unknown` (a gap), refs are `file:first-last` source spans, and placement rows are
//! `line plane lx lz {l|n}id angle` grouped by map file.

use super::catalog::{
    entity_key, Bucket, EntityKey, GatherCatalog, GatherMethod, GatherSkill, GatherSpot,
    GatherTarget, GatherYield, LooseClass, LooseEntity, MethodExtra, RespawnFact, RespawnScale,
    RockEntry, SpotId, TargetClass, ToolUse, ZoneEffect, ZoneRule, KIND_LOC, KIND_NPC, KIND_OBJ,
};
use super::SceneRegionInput;
use crate::selected::{
    EntityId, FactError, FactKey, FactStrings, Gap, ItemAmount, Knowledge, Requirement,
    RequirementAt, RequirementKind, SelectedPin, SkillMinimum, SourceSpan,
};
use crate::WorldTile;
use serde::de::IgnoredAny;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;

/// The extractor schema this decoder reads. The manifest descriptor and the family header must both carry it.
pub(super) const SCHEMA: u16 = 2;

/// Identity fields every family file leads with; read before any fact so a stale or foreign file is refused with
/// its typed cause even if a newer schema changed the fact layout.
#[derive(Deserialize)]
struct Header<'a> {
    schema: u16,
    revision: i32,
    engine_commit: &'a str,
    content_commit: &'a str,
    cache_id: &'a str,
    content_id: &'a str,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Family<'a> {
    // Identity was checked against the pin by `Header`; the input inventory is audit data for the generator.
    #[serde(rename = "schema")]
    _schema: IgnoredAny,
    #[serde(rename = "revision")]
    _revision: IgnoredAny,
    #[serde(rename = "engine_commit")]
    _engine_commit: IgnoredAny,
    #[serde(rename = "content_commit")]
    _content_commit: IgnoredAny,
    #[serde(rename = "cache_id")]
    _cache_id: IgnoredAny,
    #[serde(rename = "content_id")]
    _content_id: IgnoredAny,
    #[serde(rename = "inputs")]
    _inputs: IgnoredAny,
    #[serde(borrow)]
    facts: Facts<'a>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Facts<'a> {
    #[serde(borrow)]
    entities: Vec<&'a str>,
    hazard_npcs: Vec<i32>,
    incidental_gem_ids: Vec<i32>,
    methods: Vec<MethodWire>,
    loose: Vec<LooseWire>,
    zones: Vec<ZoneWire>,
    movements: Vec<MovementWire>,
    #[serde(borrow)]
    placements: Vec<PlacementFile<'a>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GapWire {
    code: String,
    sources: Vec<String>,
}

#[derive(Deserialize)]
#[serde(tag = "state", rename_all = "lowercase", deny_unknown_fields)]
enum Know<T> {
    Known { value: T },
    Partial { value: T, gaps: Vec<GapWire> },
    Unknown { gap: GapWire },
}

#[derive(Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum SkillWire {
    Woodcutting,
    Mining,
    Fishing,
}

#[derive(Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum KindWire {
    Loc,
    Npc,
}

#[derive(Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum ClassWire {
    Resource,
    Depleted,
    Hazard,
    Unclassified,
}

#[derive(Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum ReqKindWire {
    Skill,
    Members,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "kebab-case")]
enum EffectWire {
    YieldIntercepted,
    ProductSubstituted,
    StateGated,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OpWire {
    slot: u8,
    label: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScaleWire {
    rule: String,
    min_ticks: u32,
    max_ticks: u32,
    sources: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RespawnWire {
    raw: u32,
    scale: Know<ScaleWire>,
    source: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TargetWire {
    kind: KindWire,
    id: i32,
    op: u8,
    class: ClassWire,
    respawn: Know<Option<RespawnWire>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct YieldWire {
    item: i32,
    level: u16,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(deny_unknown_fields)]
struct GateWire {
    skill: u8,
    level: u16,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolWire {
    item: i32,
    use_gate: Option<GateWire>,
    wield_gate: Option<GateWire>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AmountWire {
    item: i32,
    count: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RequirementWire {
    id: String,
    source: String,
    kind: ReqKindWire,
    #[serde(default)]
    skill: Option<u8>,
    #[serde(default)]
    level: Option<u16>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MethodWire {
    id: String,
    skill: SkillWire,
    resources: Vec<String>,
    op: Option<OpWire>,
    targets: Know<Vec<TargetWire>>,
    products: Know<Vec<YieldWire>>,
    tools: Know<Vec<ToolWire>>,
    consumes: Know<Vec<AmountWire>>,
    requirements: Know<Vec<RequirementWire>>,
    spots: Know<()>,
    /// Provenance spans of the method as a whole; each fact carries its own sources.
    #[serde(rename = "sources")]
    _sources: IgnoredAny,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(deny_unknown_fields)]
struct RegionWire {
    min_x: i32,
    min_z: i32,
    max_x: i32,
    max_z: i32,
    level: i32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MovementWire {
    npc: i32,
    region: Know<Option<RegionWire>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ZoneWire {
    methods: Vec<String>,
    level: i32,
    min_x: i32,
    min_z: i32,
    max_x: i32,
    max_z: i32,
    effect: EffectWire,
    sources: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LooseWire {
    skill: SkillWire,
    kind: KindWire,
    id: i32,
    class: LooseClassWire,
    gap: Option<GapWire>,
}

#[derive(Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum LooseClassWire {
    Depleted,
    Unclassified,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlacementFile<'a> {
    file: &'a str,
    #[serde(borrow)]
    rows: Vec<&'a str>,
}

fn invalid(reason: impl Into<String>) -> FactError {
    FactError::Invalid {
        source: SourceSpan {
            file: Arc::from("gathering.json"),
            first: 0,
            last: 0,
        },
        reason: Arc::from(reason.into()),
    }
}

/// The family's own identity against the selected pin: schema first (typed), then every pinned identity.
fn check_header(pin: &SelectedPin, header: &Header) -> Result<(), FactError> {
    if header.schema != SCHEMA {
        return Err(FactError::Schema {
            expected: SCHEMA,
            actual: header.schema,
        });
    }
    if header.revision != pin.revision.as_i32()
        || header.engine_commit != &*pin.engine_commit
        || header.content_commit != &*pin.content_commit
        || header.cache_id != &*pin.cache_id
        || header.content_id != &*pin.content_id
    {
        return Err(FactError::PinMismatch);
    }
    Ok(())
}

/// Decode one verified family file into the shared catalog. `bytes` must already match the manifest descriptor
/// (see `cache`); the header is still checked here so a decoder never trusts a caller's promise about identity.
pub(super) fn decode(pin: &Arc<SelectedPin>, bytes: &[u8]) -> Result<GatherCatalog, FactError> {
    let header: Header = serde_json::from_slice(bytes)
        .map_err(|error| invalid(format!("gathering family header: {error}")))?;
    check_header(pin, &header)?;
    let family: Family = serde_json::from_slice(bytes)
        .map_err(|error| invalid(format!("gathering family: {error}")))?;
    Decoder::default().build(pin, family.facts)
}

struct Entity {
    key: EntityKey,
    alias: Arc<str>,
    width: u16,
    length: u16,
}

/// Entity rows sorted by key: the alias/footprint join every id in the family must resolve through.
struct Table(Vec<Entity>);

impl Table {
    fn get(&self, key: EntityKey, what: &str) -> Result<&Entity, FactError> {
        self.0
            .binary_search_by_key(&key, |entity| entity.key)
            .map(|at| &self.0[at])
            .map_err(|_| invalid(format!("{what} {key:?} has no entity row")))
    }
}

/// One placement row parsed from a map file, before it is assigned to methods.
struct Placed {
    file: u32,
    line: u32,
    level: i32,
    x: i32,
    z: i32,
    key: EntityKey,
    angle: u8,
}

#[derive(Default)]
struct Decoder {
    strings: FactStrings,
}

type Cells<T> = Knowledge<Arc<[T]>>;

impl Decoder {
    fn span(&mut self, text: &str) -> Result<SourceSpan, FactError> {
        let bad = || invalid(format!("source ref {text:?} is not file:first-last"));
        let (file, range) = text.rsplit_once(':').ok_or_else(bad)?;
        let (first, last) = range.split_once('-').ok_or_else(bad)?;
        let (first, last): (u32, u32) = (
            first.parse().map_err(|_| bad())?,
            last.parse().map_err(|_| bad())?,
        );
        if file.is_empty() || first == 0 || last < first {
            return Err(bad());
        }
        Ok(SourceSpan {
            file: self.strings.intern(file),
            first,
            last,
        })
    }

    fn spans(&mut self, refs: &[String]) -> Result<Arc<[SourceSpan]>, FactError> {
        refs.iter().map(|text| self.span(text)).collect()
    }

    fn gap(&mut self, wire: GapWire) -> Result<Gap, FactError> {
        if wire.code.is_empty() {
            return Err(invalid("a gap must carry a code"));
        }
        Ok(Gap {
            code: self.strings.intern(&wire.code),
            sources: self.spans(&wire.sources)?,
        })
    }

    fn key(&mut self, text: &str) -> FactKey {
        FactKey(self.strings.intern(text))
    }

    fn know<T, U>(
        &mut self,
        wire: Know<T>,
        convert: impl FnOnce(&mut Self, T) -> Result<U, FactError>,
    ) -> Result<Knowledge<U>, FactError> {
        Ok(match wire {
            Know::Known { value } => Knowledge::Known(convert(self, value)?),
            Know::Partial { value, gaps } => {
                if gaps.is_empty() {
                    return Err(invalid("a partial fact must carry at least one gap"));
                }
                let known = convert(self, value)?;
                let gaps = gaps
                    .into_iter()
                    .map(|gap| self.gap(gap))
                    .collect::<Result<Arc<[Gap]>, _>>()?;
                Knowledge::Partial { known, gaps }
            }
            Know::Unknown { gap } => Knowledge::Unknown(self.gap(gap)?),
        })
    }

    fn cells<T, U>(
        &mut self,
        wire: Know<Vec<T>>,
        mut convert: impl FnMut(&mut Self, T) -> Result<U, FactError>,
    ) -> Result<Cells<U>, FactError> {
        self.know(wire, |this, rows| {
            rows.into_iter().map(|row| convert(this, row)).collect()
        })
    }

    fn build(mut self, pin: &Arc<SelectedPin>, facts: Facts) -> Result<GatherCatalog, FactError> {
        let table = parse_entities(&facts.entities)?;
        let hazard_npcs = checked_entity_ids(facts.hazard_npcs, &table, KIND_NPC, "hazard NPC")?;
        let incidental_gem_ids =
            checked_entity_ids(facts.incidental_gem_ids, &table, KIND_OBJ, "incidental gem")?;

        // Convert every method except its placements, which need every method's target set first.
        let mut methods: Vec<GatherMethod> = Vec::with_capacity(facts.methods.len());
        let mut extras: Vec<MethodExtra> = Vec::with_capacity(facts.methods.len());
        let mut span_states: Vec<Know<()>> = Vec::with_capacity(facts.methods.len());
        let mut by_id: HashMap<Arc<str>, usize> = HashMap::with_capacity(facts.methods.len());
        for wire in facts.methods {
            let id = self.key(&wire.id);
            if by_id.insert(Arc::clone(&id.0), methods.len()).is_some() {
                return Err(invalid(format!("duplicate method id {}", wire.id)));
            }
            let skill = match wire.skill {
                SkillWire::Woodcutting => GatherSkill::Woodcutting,
                SkillWire::Mining => GatherSkill::Mining,
                SkillWire::Fishing => GatherSkill::Fishing,
            };
            let resources = wire.resources.iter().map(|key| self.key(key)).collect();
            let targets = self.cells(wire.targets, |this, target| {
                let (key, entity) = match target.kind {
                    KindWire::Loc => ((KIND_LOC, target.id), EntityId::Loc(target.id)),
                    KindWire::Npc => ((KIND_NPC, target.id), EntityId::Npc(target.id)),
                };
                table.get(key, "target")?;
                let respawn = this.know(target.respawn, |this, fact| {
                    fact.map(|fact| this.respawn(fact)).transpose()
                })?;
                Ok(GatherTarget {
                    entity,
                    op: target.op,
                    class: match target.class {
                        ClassWire::Resource => TargetClass::Resource,
                        ClassWire::Depleted => TargetClass::Depleted,
                        ClassWire::Hazard => TargetClass::Hazard,
                        ClassWire::Unclassified => TargetClass::Unclassified,
                    },
                    respawn,
                })
            })?;
            let item = |item: i32, what: &str| table.get((KIND_OBJ, item), what).map(|_| item);
            let products = self.cells(wire.products, |_, row| {
                Ok(GatherYield {
                    item: item(row.item, "product")?,
                    level: row.level,
                })
            })?;
            let tools = self.cells(wire.tools, |_, row| {
                let gate = |gate: Option<GateWire>| {
                    gate.map(|gate| SkillMinimum {
                        skill: gate.skill,
                        level: gate.level,
                    })
                };
                Ok(ToolUse {
                    item: item(row.item, "tool")?,
                    use_gate: gate(row.use_gate),
                    wield_gate: gate(row.wield_gate),
                })
            })?;
            let consumes = self.cells(wire.consumes, |_, row| {
                Ok(ItemAmount {
                    item: item(row.item, "consumed item")?,
                    count: row.count,
                })
            })?;
            let requirements = self.cells(wire.requirements, |this, row| {
                let source = this.span(&row.source)?;
                let kind = match (row.kind, row.skill, row.level) {
                    (ReqKindWire::Skill, Some(skill), Some(level)) => {
                        RequirementKind::Skill(SkillMinimum { skill, level })
                    }
                    (ReqKindWire::Members, None, None) => RequirementKind::MembersWorld,
                    _ => return Err(invalid(format!("requirement {} is malformed", row.id))),
                };
                Ok(Requirement {
                    id: this.key(&row.id),
                    at: RequirementAt::Action(id.clone()),
                    kind,
                    source,
                })
            })?;
            let op = wire.op.map(|op| (op.slot, self.strings.intern(&op.label)));
            let placeholder = Knowledge::Known(Arc::<[GatherSpot]>::from(Vec::new()));
            methods.push(GatherMethod {
                id,
                skill,
                resources,
                targets,
                products,
                tools,
                consumes,
                requirements,
                spots: placeholder,
            });
            extras.push(MethodExtra {
                op,
                zones: Box::default(),
                buckets: Box::default(),
                first_spot: 0,
            });
            span_states.push(wire.spots);
        }

        // Placements: parse every map row once, then hand each method the rows of its resource targets.
        let (files, placed) = self.parse_placements(&facts.placements, &table)?;
        let mut rows_by_entity: HashMap<EntityKey, Vec<u32>> = HashMap::new();
        for (index, row) in placed.iter().enumerate() {
            rows_by_entity
                .entry(row.key)
                .or_default()
                .push(index as u32);
        }
        let movements = self.movements(facts.movements)?;
        let mut next_spot = 0u32;
        for ((method, extra), state) in methods.iter_mut().zip(&mut extras).zip(span_states) {
            let keys = resource_keys(method);
            if matches!(state, Know::Unknown { .. }) {
                method.spots =
                    self.know(state, |_, ()| Ok(Arc::<[GatherSpot]>::from(Vec::new())))?;
                extra.first_spot = next_spot;
                continue;
            }
            if matches!(method.targets, Knowledge::Unknown(_)) {
                return Err(invalid(format!(
                    "method {} has placements without targets",
                    method.id.0
                )));
            }
            let mut order: Vec<u32> = keys
                .iter()
                .flat_map(|key| rows_by_entity.get(key).into_iter().flatten().copied())
                .collect();
            order.sort_unstable_by_key(|at| {
                let row = &placed[*at as usize];
                (
                    row.level,
                    row.x >> 6,
                    row.z >> 6,
                    row.x,
                    row.z,
                    row.key,
                    row.file,
                    row.line,
                )
            });
            let mut spots: Vec<GatherSpot> = Vec::with_capacity(order.len());
            let mut buckets: Vec<Bucket> = Vec::new();
            for (offset, at) in order.iter().enumerate() {
                let row = &placed[*at as usize];
                let entity = table.get(row.key, "placed")?;
                let (width, length) = if row.angle & 1 == 1 {
                    (entity.length, entity.width)
                } else {
                    (entity.width, entity.length)
                };
                let movement = match row.key.0 {
                    KIND_NPC => movements
                        .get(&row.key.1)
                        .cloned()
                        .ok_or_else(|| invalid(format!("npc {} has no movement row", row.key.1)))?,
                    _ => Knowledge::Known(None),
                };
                let (mx, mz) = (row.x >> 6, row.z >> 6);
                match buckets.last_mut() {
                    Some(last) if (last.level, last.mx, last.mz) == (row.level, mx, mz) => {
                        last.end = offset as u32 + 1;
                    }
                    _ => buckets.push(Bucket {
                        level: row.level,
                        mx,
                        mz,
                        start: offset as u32,
                        end: offset as u32 + 1,
                    }),
                }
                spots.push(GatherSpot {
                    id: SpotId(
                        next_spot
                            .checked_add(offset as u32)
                            .ok_or_else(|| invalid("placement ids overflow"))?,
                    ),
                    entity: match row.key.0 {
                        KIND_LOC => EntityId::Loc(row.key.1),
                        _ => EntityId::Npc(row.key.1),
                    },
                    origin: WorldTile {
                        x: row.x,
                        z: row.z,
                        level: row.level,
                    },
                    width,
                    length,
                    movement,
                    source: SourceSpan {
                        file: Arc::clone(&files[row.file as usize]),
                        first: row.line,
                        last: row.line,
                    },
                });
            }
            extra.first_spot = next_spot;
            next_spot = next_spot
                .checked_add(spots.len() as u32)
                .ok_or_else(|| invalid("placement ids overflow"))?;
            extra.buckets = buckets.into_boxed_slice();
            let rows: Arc<[GatherSpot]> = spots.into();
            method.spots = match state {
                Know::Known { .. } => Knowledge::Known(rows),
                Know::Partial { gaps, .. } => {
                    if gaps.is_empty() {
                        return Err(invalid("a partial fact must carry at least one gap"));
                    }
                    Knowledge::Partial {
                        known: rows,
                        gaps: gaps
                            .into_iter()
                            .map(|gap| self.gap(gap))
                            .collect::<Result<Arc<[Gap]>, _>>()?,
                    }
                }
                Know::Unknown { .. } => unreachable!("handled above"),
            };
        }

        // Zones name methods by id; each method keeps the indices of its own rules.
        let mut zones: Vec<ZoneRule> = Vec::with_capacity(facts.zones.len());
        let mut method_zones: Vec<Vec<u16>> = vec![Vec::new(); methods.len()];
        for wire in facts.zones {
            let index = u16::try_from(zones.len()).map_err(|_| invalid("too many zone rules"))?;
            if wire.min_x > wire.max_x || wire.min_z > wire.max_z {
                return Err(invalid("zone rectangle is inverted"));
            }
            for name in &wire.methods {
                let at = *by_id
                    .get(name.as_str())
                    .ok_or_else(|| invalid(format!("zone names unknown method {name}")))?;
                method_zones[at].push(index);
            }
            zones.push(ZoneRule {
                level: wire.level,
                min_x: wire.min_x,
                min_z: wire.min_z,
                max_x: wire.max_x,
                max_z: wire.max_z,
                effect: match wire.effect {
                    EffectWire::YieldIntercepted => ZoneEffect::YieldIntercepted,
                    EffectWire::ProductSubstituted => ZoneEffect::ProductSubstituted,
                    EffectWire::StateGated => ZoneEffect::StateGated,
                },
                sources: self.spans(&wire.sources)?,
            });
        }
        for (extra, indices) in extras.iter_mut().zip(method_zones) {
            extra.zones = indices.into_boxed_slice();
        }

        // Entities the extraction saw but offers no method for.
        let mut loose = Vec::with_capacity(facts.loose.len());
        for wire in facts.loose {
            let (key, entity) = match wire.kind {
                KindWire::Loc => ((KIND_LOC, wire.id), EntityId::Loc(wire.id)),
                KindWire::Npc => ((KIND_NPC, wire.id), EntityId::Npc(wire.id)),
            };
            table.get(key, "loose")?;
            let class = match wire.class {
                LooseClassWire::Depleted => LooseClass::Depleted,
                LooseClassWire::Unclassified => LooseClass::Unclassified,
            };
            let gap = wire.gap.map(|gap| self.gap(gap)).transpose()?;
            if class == LooseClass::Unclassified && gap.is_none() {
                return Err(invalid(format!(
                    "unclassified {key:?} must say why it is unclassified"
                )));
            }
            loose.push(LooseEntity {
                skill: match wire.skill {
                    SkillWire::Woodcutting => GatherSkill::Woodcutting,
                    SkillWire::Mining => GatherSkill::Mining,
                    SkillWire::Fishing => GatherSkill::Fishing,
                },
                entity,
                class,
                gap,
            });
        }

        let rocks = rock_classes(&methods, &loose)?;
        Ok(GatherCatalog {
            pin: Arc::clone(pin),
            hazard_npcs,
            incidental_gem_ids,
            methods: methods.into_boxed_slice(),
            by_id,
            extras: extras.into_boxed_slice(),
            zones: zones.into_boxed_slice(),
            aliases: table
                .0
                .iter()
                .map(|entity| (entity.key, Arc::clone(&entity.alias)))
                .collect(),
            loose: loose.into_boxed_slice(),
            rocks,
        })
    }

    fn respawn(&mut self, wire: RespawnWire) -> Result<RespawnFact, FactError> {
        let source = self.span(&wire.source)?;
        let scale = self.know(wire.scale, |this, scale| {
            if scale.min_ticks > scale.max_ticks {
                return Err(invalid("respawn scale bounds are inverted"));
            }
            Ok(RespawnScale {
                rule: this.key(&scale.rule),
                min_ticks: scale.min_ticks,
                max_ticks: scale.max_ticks,
                sources: this.spans(&scale.sources)?,
            })
        })?;
        Ok(RespawnFact {
            raw: wire.raw,
            scale,
            source,
        })
    }

    fn movements(
        &mut self,
        wires: Vec<MovementWire>,
    ) -> Result<HashMap<i32, Knowledge<Option<SceneRegionInput>>>, FactError> {
        let mut out = HashMap::with_capacity(wires.len());
        for wire in wires {
            let region = self.know(wire.region, |_, region| {
                Ok(region.map(|region| SceneRegionInput {
                    min_x: region.min_x,
                    min_z: region.min_z,
                    max_x: region.max_x,
                    max_z: region.max_z,
                    level: region.level,
                }))
            })?;
            if out.insert(wire.npc, region).is_some() {
                return Err(invalid(format!("npc {} has two movement rows", wire.npc)));
            }
        }
        Ok(out)
    }

    fn parse_placements(
        &mut self,
        files: &[PlacementFile],
        table: &Table,
    ) -> Result<(Vec<Arc<str>>, Vec<Placed>), FactError> {
        let mut names = Vec::with_capacity(files.len());
        let mut placed = Vec::with_capacity(files.iter().map(|file| file.rows.len()).sum());
        for (index, file) in files.iter().enumerate() {
            let bad =
                |row: &str| invalid(format!("{}: malformed placement row {row:?}", file.file));
            let (mx, mz) = file
                .file
                .strip_prefix("maps/m")
                .and_then(|rest| rest.strip_suffix(".jm2"))
                .and_then(|rest| rest.split_once('_'))
                .and_then(|(mx, mz)| Some((mx.parse::<i32>().ok()?, mz.parse::<i32>().ok()?)))
                .ok_or_else(|| invalid(format!("{}: not a map file name", file.file)))?;
            names.push(self.strings.intern(file.file));
            for row in &file.rows {
                let bad_row = || bad(row);
                let mut parts = row.split(' ');
                let (Some(line), Some(level), Some(lx), Some(lz), Some(entity), Some(angle), None) = (
                    parts.next(),
                    parts.next(),
                    parts.next(),
                    parts.next(),
                    parts.next(),
                    parts.next(),
                    parts.next(),
                ) else {
                    return Err(bad_row());
                };
                let number = |text: &str| text.parse::<i32>().map_err(|_| bad_row());
                let (line, level, lx, lz, angle) = (
                    number(line)?,
                    number(level)?,
                    number(lx)?,
                    number(lz)?,
                    number(angle)?,
                );
                if line < 1
                    || !(0..4).contains(&level)
                    || !(0..64).contains(&lx)
                    || !(0..64).contains(&lz)
                    || !(0..4).contains(&angle)
                {
                    return Err(bad_row());
                }
                let (kind, id) = entity.split_at(entity.len().min(1));
                let key = (
                    match kind {
                        "l" => KIND_LOC,
                        "n" => KIND_NPC,
                        _ => return Err(bad_row()),
                    },
                    id.parse::<i32>().map_err(|_| bad_row())?,
                );
                table.get(key, "placed")?;
                placed.push(Placed {
                    file: index as u32,
                    line: line as u32,
                    level,
                    x: mx * 64 + lx,
                    z: mz * 64 + lz,
                    key,
                    angle: angle as u8,
                });
            }
        }
        Ok((names, placed))
    }
}

fn parse_entities(lines: &[&str]) -> Result<Table, FactError> {
    let mut entities = Vec::with_capacity(lines.len());
    for line in lines {
        let bad = || invalid(format!("entity row {line:?} is malformed"));
        let mut parts = line.split(' ');
        let kind = match parts.next() {
            Some("loc") => KIND_LOC,
            Some("npc") => KIND_NPC,
            Some("obj") => KIND_OBJ,
            _ => return Err(bad()),
        };
        let id: i32 = parts
            .next()
            .and_then(|id| id.parse().ok())
            .ok_or_else(bad)?;
        let alias = parts
            .next()
            .filter(|alias| !alias.is_empty())
            .ok_or_else(bad)?;
        let (width, length) = if kind == KIND_LOC {
            let mut dimension = || -> Result<u16, FactError> {
                let value: u16 = parts
                    .next()
                    .and_then(|part| part.parse().ok())
                    .ok_or_else(bad)?;
                if (1..=255).contains(&value) {
                    Ok(value)
                } else {
                    Err(bad())
                }
            };
            (dimension()?, dimension()?)
        } else {
            (1, 1)
        };
        if parts.next().is_some() {
            return Err(bad());
        }
        entities.push(Entity {
            key: (kind, id),
            alias: Arc::from(alias),
            width,
            length,
        });
    }
    entities.sort_unstable_by_key(|entity| entity.key);
    if let Some(pair) = entities.windows(2).find(|pair| pair[0].key == pair[1].key) {
        return Err(invalid(format!("duplicate entity row {:?}", pair[0].key)));
    }
    Ok(Table(entities))
}

fn checked_entity_ids(
    ids: Vec<i32>,
    table: &Table,
    kind: u8,
    what: &str,
) -> Result<Arc<[i32]>, FactError> {
    if ids.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(invalid(format!("{what} ids must be sorted and unique")));
    }
    for id in &ids {
        table.get((kind, *id), what)?;
    }
    Ok(ids.into())
}

/// Sorted, distinct entity keys of a method's resource-class targets: the entities whose placements are its spots.
fn resource_keys(method: &GatherMethod) -> Vec<EntityKey> {
    let mut keys: Vec<EntityKey> = super::catalog::known_rows(&method.targets)
        .iter()
        .filter(|target| target.class == TargetClass::Resource)
        .map(|target| entity_key(target.entity))
        .collect();
    keys.sort_unstable();
    keys.dedup();
    keys
}

/// Every mining loc the extraction saw, classified once: target classes from methods (ambiguous method links are
/// dropped, never guessed) and loose depleted/unclassified rocks with their gaps.
fn rock_classes(
    methods: &[GatherMethod],
    loose: &[LooseEntity],
) -> Result<HashMap<i32, RockEntry>, FactError> {
    let mut rocks: HashMap<i32, RockEntry> = HashMap::new();
    let mut add = |loc: i32,
                   class: TargetClass,
                   method: Option<usize>,
                   gap: Option<Gap>|
     -> Result<(), FactError> {
        match rocks.get_mut(&loc) {
            None => {
                rocks.insert(loc, RockEntry { class, method, gap });
            }
            Some(existing) if existing.class == class => {
                if existing.method != method {
                    existing.method = None;
                }
            }
            Some(_) => return Err(invalid(format!("rock loc {loc} has two classes"))),
        }
        Ok(())
    };
    for (index, method) in methods.iter().enumerate() {
        if method.skill != GatherSkill::Mining {
            continue;
        }
        for target in super::catalog::known_rows(&method.targets) {
            if let EntityId::Loc(loc) = target.entity {
                add(loc, target.class, Some(index), None)?;
            }
        }
    }
    for entity in loose {
        if entity.skill != GatherSkill::Mining {
            continue;
        }
        if let EntityId::Loc(loc) = entity.entity {
            let class = match entity.class {
                LooseClass::Depleted => TargetClass::Depleted,
                LooseClass::Unclassified => TargetClass::Unclassified,
            };
            add(loc, class, None, entity.gap.clone())?;
        }
    }
    Ok(rocks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game_data::for_revision;
    use crate::gather_methods::{known_rows, AccessPolicy};
    use crate::selected::{ClientRevision, Truth};
    use serde_json::{json, Value};

    const REAL: &[u8] = include_bytes!("../../data/game-data/289/gathering.json");

    fn pin() -> Arc<SelectedPin> {
        for_revision(ClientRevision::R289)
            .unwrap()
            .selected_pin()
            .unwrap()
    }

    /// The real 289 family with `edit` applied to its parsed JSON, decoded under the real pin.
    fn decode_edited(edit: impl FnOnce(&mut Value)) -> Result<GatherCatalog, FactError> {
        let mut family: Value = serde_json::from_slice(REAL).unwrap();
        edit(&mut family);
        decode(&pin(), &serde_json::to_vec(&family).unwrap())
    }

    fn method_mut<'a>(family: &'a mut Value, id: &str) -> &'a mut Value {
        family["facts"]["methods"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|method| method["id"] == id)
            .unwrap_or_else(|| panic!("no method {id}"))
    }

    fn region(min_x: i32, min_z: i32, max_x: i32, max_z: i32) -> SceneRegionInput {
        SceneRegionInput {
            min_x,
            min_z,
            max_x,
            max_z,
            level: 0,
        }
    }

    #[test]
    fn every_placement_id_resolves_to_its_own_method_and_spot() {
        let catalog = decode(&pin(), REAL).unwrap();
        let mut last = None;
        for method in catalog.methods() {
            for spot in known_rows(&method.spots) {
                assert!(
                    last.is_none_or(|last| last < spot.id),
                    "ids ascend across methods"
                );
                last = Some(spot.id);
                let (owner, found) = catalog.spot(spot.id).expect("id resolves");
                assert_eq!((&owner.id, found.id), (&method.id, spot.id));
            }
        }
    }

    #[test]
    fn partial_placement_coverage_keeps_its_rows_but_no_query_answers_from_it() {
        let catalog = decode_edited(|family| {
            method_mut(family, "mining.copper")["spots"] = json!({
                "state": "partial",
                "value": null,
                "gaps": [{ "code": "map-scan-cut", "sources": ["maps/m38_50.jm2:1-2"] }],
            });
        })
        .unwrap();
        let copper = catalog.method("mining.copper").unwrap();
        let Knowledge::Partial { known, gaps } = &copper.spots else {
            panic!("partial placements decode as partial")
        };
        assert!(
            !known.is_empty(),
            "the rows found are kept for callers that accept partial data"
        );
        assert_eq!(&*gaps[0].code, "map-scan-cut");
        let world = region(-1000, -1000, 20_000, 20_000);
        let from = WorldTile {
            x: 2470,
            z: 3255,
            level: 0,
        };
        assert!(catalog.spots(copper, &world).is_err());
        assert!(catalog.page(copper, &world, None, 5).is_err());
        assert!(catalog
            .nearest(copper, from, None, AccessPolicy::Any, 3)
            .is_err());
    }

    #[test]
    fn access_is_unknown_when_only_a_quest_state_guard_applies() {
        let catalog = decode_edited(|family| {
            let zones = family["facts"]["zones"].as_array_mut().unwrap();
            zones.retain(|zone| zone["effect"] != "yield-intercepted");
        })
        .unwrap();
        let maple = catalog.method("woodcutting.maple").unwrap();
        let island = region(2496, 3840, 2582, 3903);
        let inside: Vec<_> = catalog.spots(maple, &island).unwrap().collect();
        assert!(!inside.is_empty());
        for spot in &inside {
            assert_eq!(catalog.access(maple, spot).unwrap(), Truth::Unknown);
        }
        let from = WorldTile {
            x: 2540,
            z: 3870,
            level: 0,
        };
        let near = |policy| catalog.nearest(maple, from, None, policy, 5).unwrap();
        assert!(near(AccessPolicy::Usable)
            .iter()
            .all(|n| n.access == Truth::True));
        assert!(near(AccessPolicy::Possible)
            .iter()
            .any(|n| n.access == Truth::Unknown));
        assert!(near(AccessPolicy::Any)
            .iter()
            .all(|n| n.access == Truth::Unknown));
    }

    #[test]
    fn a_placed_loc_is_as_wide_as_its_rotation_makes_it() {
        let mut family: Value = serde_json::from_slice(REAL).unwrap();
        let entities = family["facts"]["entities"].as_array_mut().unwrap();
        let oak = entities
            .iter_mut()
            .find(|row| row.as_str() == Some("loc 1281 oaktree 3 3"))
            .unwrap();
        *oak = json!("loc 1281 oaktree 3 1");
        let angles: HashMap<(String, u32), u8> = family["facts"]["placements"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|file| {
                let name = file["file"].as_str().unwrap().to_owned();
                file["rows"].as_array().unwrap().iter().map(move |row| {
                    let parts: Vec<&str> = row.as_str().unwrap().split(' ').collect();
                    (
                        (name.clone(), parts[0].parse().unwrap()),
                        parts[5].parse().unwrap(),
                    )
                })
            })
            .collect();
        let catalog = decode(&pin(), &serde_json::to_vec(&family).unwrap()).unwrap();
        let oaks = known_rows(&catalog.method("woodcutting.oak").unwrap().spots);
        let (mut upright, mut turned) = (0, 0);
        for spot in oaks
            .iter()
            .filter(|spot| spot.entity == EntityId::Loc(1281))
        {
            let angle = angles[&(spot.source.file.to_string(), spot.source.first)];
            if angle % 2 == 1 {
                assert_eq!((spot.width, spot.length), (1, 3), "turned a quarter");
                turned += 1;
            } else {
                assert_eq!((spot.width, spot.length), (3, 1));
                upright += 1;
            }
        }
        assert!(
            upright > 0 && turned > 0,
            "the real oaks include both orientations"
        );
    }

    #[test]
    fn a_family_that_contradicts_itself_is_refused_not_repaired() {
        type Edit = Box<dyn Fn(&mut Value)>;
        let cases: Vec<(&str, Edit)> = vec![
            (
                "zone names a method that does not exist",
                Box::new(|family| {
                    family["facts"]["zones"][0]["methods"]
                        .as_array_mut()
                        .unwrap()
                        .push(json!("no.such.method"));
                }),
            ),
            (
                "zone rectangle is inverted",
                Box::new(|family| {
                    family["facts"]["zones"][0]["min_x"] = json!(9999);
                }),
            ),
            (
                "placement of an entity with no entity row",
                Box::new(|family| {
                    family["facts"]["placements"][0]["rows"]
                        .as_array_mut()
                        .unwrap()
                        .push(json!("1 0 1 1 l999999 0"));
                }),
            ),
            (
                "placement angle out of range",
                Box::new(|family| {
                    family["facts"]["placements"][0]["rows"]
                        .as_array_mut()
                        .unwrap()
                        .push(json!("1 0 1 1 l1281 7"));
                }),
            ),
            (
                "placement row with a missing field",
                Box::new(|family| {
                    family["facts"]["placements"][0]["rows"]
                        .as_array_mut()
                        .unwrap()
                        .push(json!("1 0 1 1 l1281"));
                }),
            ),
            (
                "placements outside a map file name",
                Box::new(|family| {
                    family["facts"]["placements"][0]["file"] = json!("maps/elsewhere.jm2");
                }),
            ),
            (
                "npc spawns without any movement row",
                Box::new(|family| {
                    family["facts"]["movements"] = json!([]);
                }),
            ),
            (
                "a partial fact with no gap",
                Box::new(|family| {
                    method_mut(family, "mining.copper")["products"] =
                        json!({ "state": "partial", "value": [], "gaps": [] });
                }),
            ),
            (
                "a gap with no code",
                Box::new(|family| {
                    method_mut(family, "mining.copper")["products"] =
                        json!({ "state": "unknown", "gap": { "code": "", "sources": [] } });
                }),
            ),
            (
                "a source that is not file:first-last",
                Box::new(|family| {
                    method_mut(family, "mining.copper")["requirements"] = json!({ "state": "unknown", "gap": { "code": "x", "sources": ["nowhere"] } });
                }),
            ),
            (
                "two methods with one id",
                Box::new(|family| {
                    let copy = method_mut(family, "mining.copper").clone();
                    family["facts"]["methods"]
                        .as_array_mut()
                        .unwrap()
                        .push(copy);
                }),
            ),
            (
                "an unknown field is not ignored",
                Box::new(|family| {
                    method_mut(family, "mining.copper")["surprise"] = json!(true);
                }),
            ),
            (
                "hazard NPC ids require NPC entity rows",
                Box::new(|family| {
                    let ids: Vec<i32> = family["facts"]["entities"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter_map(|row| {
                            row.as_str()?
                                .strip_prefix("npc ")?
                                .split(' ')
                                .next()?
                                .parse()
                                .ok()
                        })
                        .collect();
                    let missing: i32 = (0..).find(|id: &i32| !ids.contains(id)).unwrap();
                    family["facts"]["hazard_npcs"] = json!([missing]);
                }),
            ),
            (
                "incidental gem ids require Obj entity rows",
                Box::new(|family| {
                    let ids: Vec<i32> = family["facts"]["entities"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter_map(|row| {
                            row.as_str()?
                                .strip_prefix("obj ")?
                                .split(' ')
                                .next()?
                                .parse()
                                .ok()
                        })
                        .collect();
                    let missing: i32 = (0..).find(|id: &i32| !ids.contains(id)).unwrap();
                    family["facts"]["incidental_gem_ids"] = json!([missing]);
                }),
            ),
            (
                "hazard NPC ids are required",
                Box::new(|family| {
                    family["facts"]
                        .as_object_mut()
                        .unwrap()
                        .remove("hazard_npcs");
                }),
            ),
            (
                "incidental gem ids are required",
                Box::new(|family| {
                    family["facts"]
                        .as_object_mut()
                        .unwrap()
                        .remove("incidental_gem_ids");
                }),
            ),
            (
                "hazard NPC ids must be sorted and unique",
                Box::new(|family| {
                    let id = family["facts"]["hazard_npcs"][0].clone();
                    family["facts"]["hazard_npcs"] = json!([id.clone(), id]);
                }),
            ),
            (
                "incidental gem ids must be sorted and unique",
                Box::new(|family| {
                    let id = family["facts"]["incidental_gem_ids"][0].clone();
                    family["facts"]["incidental_gem_ids"] = json!([id.clone(), id]);
                }),
            ),
            (
                "a product that names no known item",
                Box::new(|family| {
                    method_mut(family, "mining.copper")["products"] =
                        json!({ "state": "known", "value": [{ "item": 999999, "level": 1 }] });
                }),
            ),
            (
                "placements for a method whose targets are unknown",
                Box::new(|family| {
                    method_mut(family, "mining.copper")["targets"] =
                        json!({ "state": "unknown", "gap": { "code": "x", "sources": [] } });
                }),
            ),
            (
                "one rock loc in two classes",
                Box::new(|family| {
                    let tin = method_mut(family, "mining.tin");
                    let stage = tin["targets"]["value"]
                        .as_array_mut()
                        .unwrap()
                        .iter_mut()
                        .find(|t| t["id"] == 450)
                        .unwrap();
                    stage["class"] = json!("resource");
                }),
            ),
            (
                "an unclassified entity with no reason",
                Box::new(|family| {
                    let loose = family["facts"]["loose"].as_array_mut().unwrap();
                    let rock = loose
                        .iter_mut()
                        .find(|row| row["class"] == "unclassified")
                        .unwrap();
                    rock["gap"] = Value::Null;
                }),
            ),
            (
                "a requirement that is not a skill or members",
                Box::new(|family| {
                    method_mut(family, "woodcutting.oak")["requirements"] = json!({
                        "state": "known",
                        "value": [{ "id": "r", "source": "a.dbrow:1-1", "kind": "skill" }],
                    });
                }),
            ),
        ];
        for (name, edit) in cases {
            let mut family: Value = serde_json::from_slice(REAL).unwrap();
            edit(&mut family);
            let bytes = serde_json::to_vec(&family).unwrap();
            assert!(
                matches!(decode(&pin(), &bytes), Err(FactError::Invalid { .. })),
                "{name}: must be refused as Invalid"
            );
        }
    }
}
