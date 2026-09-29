//! Gather methods: the typed catalog decoded from the checked `gathering.json` family (`catalog`, `wire`, `cache`)
//! and the JSON query adapters the V8 helpers read through it. The adapters take the catalog directly, so an
//! unavailable family is an error token, never an empty list, and every incomplete fact says so in its row.

use crate::selected::{EntityId, Gap, Knowledge, RequirementKind, SourceSpan};
use serde_json::{json, Value};

mod cache;
mod catalog;
mod wire;

pub use cache::{cached, prepare};
pub use catalog::{
    first_gap, known_rows, AccessPolicy, GatherCatalog, GatherMethod, GatherSkill, GatherSpot,
    GatherTarget, GatherYield, LooseClass, LooseEntity, Nearest, RespawnFact, RespawnScale,
    RockFact, SpotId, SpotPage, TargetClass, ToolUse, ZoneEffect, ZoneRule,
};

pub const FAMILY_UNAVAILABLE: &str = "family-unavailable:gather_methods";
pub const FAMILY_UNAVAILABLE_PLACEMENTS: &str = "family-unavailable:gather_placements";
pub const UNKNOWN_SKILL: &str = "unknown-skill";
pub const UNKNOWN_RESOURCE: &str = "unknown-resource";
pub const INVALID_ARGS: &str = "invalid-args";

/// `{ rows, coverage }`: one row per method (woodcutting, mining, fishing, each in content order) and every gap
/// the catalog carries. Skill `None` is the omitted-skill call; a skill filter never filters coverage.
pub fn gather_methods(
    catalog: Option<&GatherCatalog>,
    skill: Option<&str>,
) -> Result<Value, &'static str> {
    let Some(catalog) = catalog else {
        return Err(FAMILY_UNAVAILABLE);
    };
    let wanted = skill.map(accepted_skill).transpose()?;
    let rows: Vec<Value> = catalog
        .methods()
        .iter()
        .filter(|method| wanted.is_none_or(|skill| method.skill == skill))
        .map(|method| method_row(catalog, method))
        .collect();
    Ok(json!({ "rows": rows, "coverage": coverage(catalog) }))
}

/// Gas-event rock loc ids, derived from the selected mining methods' hazard targets rather than copied from the
/// JavaScript catalog: the selectable ore ladder plus quest-only blurite, not the separate gem rock.
pub fn gas_rock_ids(catalog: Option<&GatherCatalog>) -> Result<Vec<i32>, &'static str> {
    let Some(catalog) = catalog else {
        return Err(FAMILY_UNAVAILABLE);
    };
    Ok(catalog.hazard_locs(|key| {
        key.eq_ignore_ascii_case("blurite")
            || crate::content::ROCK_TYPE_NAMES
                .iter()
                .any(|name| key.eq_ignore_ascii_case(name))
    }))
}

/// `{ rows }` of the methods a resource key names (trim, ASCII case-insensitive). Zero matches is
/// `unknown-resource`, not an empty list.
pub fn gather_resource(catalog: Option<&GatherCatalog>, name: &str) -> Result<Value, &'static str> {
    let Some(catalog) = catalog else {
        return Err(FAMILY_UNAVAILABLE);
    };
    let rows: Vec<Value> = catalog
        .methods_for_resource(name)
        .map(|method| method_row(catalog, method))
        .collect();
    if rows.is_empty() {
        return Err(UNKNOWN_RESOURCE);
    }
    Ok(json!({ "rows": rows }))
}

/// One level's box, the rust mirror of the v2 `SceneRegionInput` fields.
/// `level` is compared against the placement's level; there is no `plane`
/// key and no radius form.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneRegionInput {
    pub min_x: i32,
    pub min_z: i32,
    pub max_x: i32,
    pub max_z: i32,
    pub level: i32,
}

/// `{ rows, truncated, next, resource_ids, qualification }` of placements of the methods `resource` names,
/// inside `region`, ascending by placement id. `limit` caps `rows`; `after` is the previous page's `next`
/// cursor. `truncated` says more rows remain and `next` resumes after the last returned row.
///
/// Unknown or partial placement coverage is never an empty page: it is `{ rows: [], qualification: "unknown",
/// gaps }` with `resource_ids` still naming the resource. An absent family is
/// `family-unavailable:gather_placements`; a key no method names is `unknown-resource`.
pub fn gather_placements(
    catalog: Option<&GatherCatalog>,
    resource: &str,
    region: &SceneRegionInput,
    after: Option<&str>,
    limit: usize,
) -> Result<Value, &'static str> {
    let Some(catalog) = catalog else {
        return Err(FAMILY_UNAVAILABLE_PLACEMENTS);
    };
    let cursor = after
        .map(|text| text.parse::<u32>().map(SpotId).map_err(|_| INVALID_ARGS))
        .transpose()?;
    let methods: Vec<&GatherMethod> = catalog.methods_for_resource(resource).collect();
    if methods.is_empty() {
        return Err(UNKNOWN_RESOURCE);
    }
    let mut resource_ids: Vec<Value> = Vec::new();
    let mut seen: Vec<EntityId> = Vec::new();
    for method in &methods {
        for target in known_rows(&method.targets) {
            if target.class == TargetClass::Resource && !seen.contains(&target.entity) {
                seen.push(target.entity);
                resource_ids.push(id_row(catalog, target.entity));
            }
        }
    }
    let gaps: Vec<Value> = methods
        .iter()
        .filter_map(|method| first_gap(&method.spots).map(|gap| (method, gap)))
        .map(|(method, gap)| json!({ "method": method.id.0, "gap": gap_json(gap) }))
        .collect();
    if !gaps.is_empty() {
        return Ok(json!({
            "rows": [],
            "truncated": false,
            "next": null,
            "resource_ids": resource_ids,
            "qualification": "unknown",
            "gaps": gaps,
        }));
    }
    let mut rows = Vec::new();
    let mut truncated = false;
    'methods: for method in methods {
        let spots = catalog
            .spots(method, region)
            .map_err(|_| FAMILY_UNAVAILABLE_PLACEMENTS)?
            .filter(|spot| cursor.is_none_or(|cursor| spot.id > cursor));
        for spot in spots {
            if rows.len() >= limit {
                truncated = true;
                break 'methods;
            }
            rows.push((method, spot));
        }
    }
    let next = truncated
        .then(|| rows.last().map(|(_, spot)| spot.id.0.to_string()))
        .flatten();
    Ok(json!({
        "rows": rows.iter().map(|(method, spot)| spot_row(catalog, method, spot)).collect::<Vec<_>>(),
        "truncated": truncated,
        "next": next,
        "resource_ids": resource_ids,
        "qualification": "complete",
    }))
}

fn accepted_skill(skill: &str) -> Result<GatherSkill, &'static str> {
    match skill.trim().to_ascii_lowercase().as_str() {
        "woodcutting" => Ok(GatherSkill::Woodcutting),
        "mining" => Ok(GatherSkill::Mining),
        "fishing" => Ok(GatherSkill::Fishing),
        _ => Err(UNKNOWN_SKILL),
    }
}

fn skill_name(skill: GatherSkill) -> &'static str {
    match skill {
        GatherSkill::Woodcutting => "woodcutting",
        GatherSkill::Mining => "mining",
        GatherSkill::Fishing => "fishing",
    }
}

fn class_name(class: TargetClass) -> &'static str {
    match class {
        TargetClass::Resource => "resource",
        TargetClass::Depleted => "depleted",
        TargetClass::Hazard => "hazard",
        TargetClass::Unclassified => "unclassified",
    }
}

fn kind_and_id(entity: EntityId) -> (&'static str, i32) {
    match entity {
        EntityId::Loc(id) => ("loc", id),
        EntityId::Npc(id) => ("npc", id),
        EntityId::Obj(id) => ("obj", id),
    }
}

fn span_text(span: &SourceSpan) -> String {
    format!("{}:{}-{}", span.file, span.first, span.last)
}

fn gap_json(gap: &Gap) -> Value {
    json!({
        "code": gap.code,
        "sources": gap.sources.iter().map(span_text).collect::<Vec<_>>(),
    })
}

fn id_row(catalog: &GatherCatalog, entity: EntityId) -> Value {
    json!({ "alias": catalog.alias(entity), "id": kind_and_id(entity).1 })
}

/// `{ state, value?, gaps? }`: the fact's rows next to what is missing, so no caller can read an incomplete set
/// as complete.
fn cell<T>(knowledge: &Knowledge<T>, value: impl FnOnce(&T) -> Value) -> Value {
    match knowledge {
        Knowledge::Known(known) => json!({ "state": "known", "value": value(known) }),
        Knowledge::Partial { known, gaps } => json!({
            "state": "partial",
            "value": value(known),
            "gaps": gaps.iter().map(gap_json).collect::<Vec<_>>(),
        }),
        Knowledge::Unknown(gap) => json!({ "state": "unknown", "gaps": [gap_json(gap)] }),
    }
}

fn respawn_json(fact: &Option<RespawnFact>) -> Value {
    let Some(fact) = fact else {
        return Value::Null;
    };
    json!({
        "raw": fact.raw,
        "scale": cell(&fact.scale, |scale| json!({
            "rule": scale.rule.0,
            "min_ticks": scale.min_ticks,
            "max_ticks": scale.max_ticks,
            "sources": scale.sources.iter().map(span_text).collect::<Vec<_>>(),
        })),
        "source": span_text(&fact.source),
    })
}

fn method_row(catalog: &GatherCatalog, method: &GatherMethod) -> Value {
    let targets = known_rows(&method.targets);
    let ids = |class: TargetClass, kind: Option<&str>| -> Vec<Value> {
        targets
            .iter()
            .filter(|target| {
                target.class == class
                    && kind.is_none_or(|kind| kind_and_id(target.entity).0 == kind)
            })
            .map(|target| id_row(catalog, target.entity))
            .collect()
    };
    let item = |item: i32| id_row(catalog, EntityId::Obj(item));
    let gate = |gate: &Option<crate::selected::SkillMinimum>| match gate {
        Some(gate) => json!({ "skill": gate.skill, "level": gate.level }),
        None => Value::Null,
    };
    let complete = [
        matches!(method.targets, Knowledge::Known(_)),
        matches!(method.products, Knowledge::Known(_)),
        matches!(method.tools, Knowledge::Known(_)),
        matches!(method.consumes, Knowledge::Known(_)),
        matches!(method.requirements, Knowledge::Known(_)),
        matches!(method.spots, Knowledge::Known(_)),
    ]
    .iter()
    .all(|known| *known);
    let op = catalog
        .op(method)
        .ok()
        .flatten()
        .map(|(slot, label)| json!({ "slot": slot, "label": label }));
    json!({
        "id": method.id.0,
        "skill": skill_name(method.skill),
        "resource_key": method.resources.first().map(|key| &*key.0),
        "resources": method.resources.iter().map(|key| &*key.0).collect::<Vec<_>>(),
        "op": op,
        "loc_ids": ids(TargetClass::Resource, Some("loc")),
        "npc_ids": ids(TargetClass::Resource, Some("npc")),
        "empty_ids": ids(TargetClass::Depleted, None),
        "hazard_ids": ids(TargetClass::Hazard, None),
        "unclassified_ids": ids(TargetClass::Unclassified, None),
        "targets": cell(&method.targets, |rows| Value::Array(rows.iter().map(|target| {
            let (kind, id) = kind_and_id(target.entity);
            json!({
                "kind": kind,
                "id": id,
                "alias": catalog.alias(target.entity),
                "op": target.op,
                "class": class_name(target.class),
                "respawn": cell(&target.respawn, respawn_json),
            })
        }).collect())),
        "products": cell(&method.products, |rows| Value::Array(rows.iter().map(|row| {
            let mut value = item(row.item);
            value["level"] = json!(row.level);
            value
        }).collect())),
        "tools": cell(&method.tools, |rows| Value::Array(rows.iter().map(|row| {
            let mut value = item(row.item);
            value["use_gate"] = gate(&row.use_gate);
            value["wield_gate"] = gate(&row.wield_gate);
            value
        }).collect())),
        "consumes": cell(&method.consumes, |rows| Value::Array(rows.iter().map(|row| {
            let mut value = item(row.item);
            value["count"] = json!(row.count);
            value
        }).collect())),
        "requirements": cell(&method.requirements, |rows| Value::Array(rows.iter().map(|row| {
            let mut value = json!({ "id": row.id.0, "source": span_text(&row.source) });
            match &row.kind {
                RequirementKind::Skill(minimum) => {
                    value["kind"] = json!("skill");
                    value["skill"] = json!(minimum.skill);
                    value["level"] = json!(minimum.level);
                }
                RequirementKind::MembersWorld => value["kind"] = json!("members"),
                other => value["kind"] = json!(format!("{other:?}")),
            }
            value
        }).collect())),
        "placements": cell(&method.spots, |rows| json!({ "count": rows.len() })),
        "qualification": if complete { "complete" } else { "partial" },
    })
}

/// Every gap in the catalog: one entry per incomplete method cell and one per entity content could not classify.
fn coverage(catalog: &GatherCatalog) -> Vec<Value> {
    let mut out = Vec::new();
    for method in catalog.methods() {
        let mut push = |cell: &str, class: &str, gap: &Gap| {
            let mut entry = gap_json(gap);
            entry["class"] = json!(class);
            entry["method"] = json!(method.id.0);
            entry["cell"] = json!(cell);
            out.push(entry);
        };
        macro_rules! gaps {
            ($name:literal, $field:expr) => {
                match &$field {
                    Knowledge::Known(_) => {}
                    Knowledge::Partial { gaps, .. } => {
                        gaps.iter().for_each(|gap| push($name, "partial", gap))
                    }
                    Knowledge::Unknown(gap) => push($name, "unknown", gap),
                }
            };
        }
        gaps!("targets", method.targets);
        gaps!("products", method.products);
        gaps!("tools", method.tools);
        gaps!("consumes", method.consumes);
        gaps!("requirements", method.requirements);
        gaps!("placements", method.spots);
    }
    for loose in catalog.loose() {
        let Some(gap) = &loose.gap else { continue };
        let (kind, id) = kind_and_id(loose.entity);
        let mut entry = gap_json(gap);
        entry["class"] = json!("unclassified");
        entry["skill"] = json!(skill_name(loose.skill));
        entry["kind"] = json!(kind);
        entry["id"] = json!(id);
        entry["alias"] = json!(catalog.alias(loose.entity));
        out.push(entry);
    }
    out
}

fn spot_row(catalog: &GatherCatalog, method: &GatherMethod, spot: &GatherSpot) -> Value {
    let (kind, id) = kind_and_id(spot.entity);
    let mut row = json!({
        "spot": spot.id.0.to_string(),
        "method": method.id.0,
        "kind": kind,
        "id": id,
        "alias": catalog.alias(spot.entity),
        "x": spot.origin.x,
        "z": spot.origin.z,
        "plane": spot.origin.level,
        "width": spot.width,
        "length": spot.length,
        "movement": cell(&spot.movement, |region| match region {
            Some(region) => json!({
                "min_x": region.min_x,
                "min_z": region.min_z,
                "max_x": region.max_x,
                "max_z": region.max_z,
                "level": region.level,
            }),
            None => Value::Null,
        }),
        "source": span_text(&spot.source),
    });
    row[if kind == "loc" { "loc_id" } else { "npc_id" }] = json!(id);
    row
}
