//! Gather-methods query. Takes the family directly so a missing family is not an empty list.
//! The V8 wrapper is the only production caller of `SelectedGameData::gather_methods`
//! and `SelectedGameData::gather_placements`.

use crate::game_data::{
    GatherCoverageRecord, GatherFishingMethod, GatherLocId, GatherLocResource, GatherMethodsFacts,
    GatherOutput, GatherPlacementsFacts,
};
use serde_json::{json, Map, Value};

pub const FAMILY_UNAVAILABLE: &str = "family-unavailable:gather_methods";
pub const FAMILY_UNAVAILABLE_PLACEMENTS: &str = "family-unavailable:gather_placements";
pub const UNKNOWN_SKILL: &str = "unknown-skill";
pub const UNKNOWN_RESOURCE: &str = "unknown-resource";

/// The one publication class the placements extract covers. The other stored
/// classes are `unpublished` and `conditional`, which have no world rows.
const PUBLISHED: &str = "published";

/// `{ rows, coverage }`. Skill `None` is the omitted-skill call.
pub fn gather_methods(
    facts: Option<&GatherMethodsFacts>,
    skill: Option<&str>,
) -> Result<Value, &'static str> {
    let Some(facts) = facts else {
        return Err(FAMILY_UNAVAILABLE);
    };
    let bucket = match skill {
        None => None,
        Some(raw) => Some(accepted_skill(raw)?),
    };
    let mut rows = Vec::new();
    if bucket.is_none() || bucket == Some("woodcutting") {
        rows.extend(facts.woods.iter().map(|row| loc_row("woodcutting", row)));
    }
    if bucket.is_none() || bucket == Some("mining") {
        rows.extend(facts.mining.iter().map(|row| loc_row("mining", row)));
    }
    if bucket.is_none() || bucket == Some("fishing") {
        rows.extend(facts.fishing.iter().map(fishing_row));
    }
    Ok(json!({
        "rows": rows,
        "coverage": facts.coverage.iter().map(coverage_row).collect::<Vec<_>>(),
    }))
}

/// `{ rows }` of loc-resource hits. Zero matches is `unknown-resource`, not an empty list.
pub fn gather_resource(
    facts: Option<&GatherMethodsFacts>,
    name: &str,
) -> Result<Value, &'static str> {
    let Some(facts) = facts else {
        return Err(FAMILY_UNAVAILABLE);
    };
    let mut rows = Vec::new();
    rows.extend(
        facts
            .woods
            .iter()
            .filter(|row| key_eq(&row.resource_key, name))
            .map(|row| loc_row("woodcutting", row)),
    );
    rows.extend(
        facts
            .mining
            .iter()
            .filter(|row| key_eq(&row.resource_key, name))
            .map(|row| loc_row("mining", row)),
    );
    if rows.is_empty() {
        return Err(UNKNOWN_RESOURCE);
    }
    Ok(json!({ "rows": rows }))
}

/// One level's box, the rust mirror of the v2 `SceneRegionInput` fields.
/// `level` is compared against the stored row's `plane`; there is no `plane`
/// key and no radius form.
#[derive(Debug, Clone, Copy)]
pub struct SceneRegionInput {
    pub min_x: i32,
    pub min_z: i32,
    pub max_x: i32,
    pub max_z: i32,
    pub level: i32,
}

/// `{ rows, truncated, resource_ids, qualification }` of published-woods world
/// placements inside `region`. `resource` is a methods `resource_key` and
/// `limit` caps `rows`; the public wrappers gate both before this call.
///
/// Takes both families directly. Absent placements is
/// `family-unavailable:gather_placements` before `unknown-resource`, never an
/// empty list; methods absence on a present placements family is
/// `unknown-resource`, because the key cannot be identified.
pub fn gather_placements(
    facts: Option<&GatherPlacementsFacts>,
    methods: Option<&GatherMethodsFacts>,
    resource: &str,
    region: &SceneRegionInput,
    limit: usize,
) -> Result<Value, &'static str> {
    let Some(facts) = facts else {
        return Err(FAMILY_UNAVAILABLE_PLACEMENTS);
    };
    let Some(methods) = methods else {
        return Err(UNKNOWN_RESOURCE);
    };
    // Identity is woods then mining, the same order as `gather_resource`.
    if let Some(wood) = methods
        .woods
        .iter()
        .find(|row| key_eq(&row.resource_key, resource))
    {
        // Unpublished and conditional woods are not placement-queryable, and
        // their coverage class is not mining's unknown mark.
        if wood.publication.as_deref() != Some(PUBLISHED) {
            return Err(UNKNOWN_RESOURCE);
        }
        return Ok(published_placements(facts, wood, region, limit));
    }
    if methods
        .mining
        .iter()
        .any(|row| key_eq(&row.resource_key, resource))
    {
        // Coverage unknown is not an empty extract: short-circuit before the
        // spatial filter and never copy mining loc ids into `resource_ids`.
        return Ok(json!({
            "rows": [],
            "truncated": false,
            "resource_ids": [],
            "qualification": "unknown",
        }));
    }
    Err(UNKNOWN_RESOURCE)
}

/// The published join: that methods row's `loc_ids` only (never `empty_ids` /
/// stumps), family order, the box, and `row.plane == region.level`. More
/// matches than `limit` sets `truncated` and drops the rest. `resource_ids` is
/// the methods set, so a region miss still carries it.
fn published_placements(
    facts: &GatherPlacementsFacts,
    wood: &GatherLocResource,
    region: &SceneRegionInput,
    limit: usize,
) -> Value {
    let mut rows = Vec::new();
    let mut truncated = false;
    for row in &facts.rows {
        if !wood.loc_ids.iter().any(|id| id.id == row.loc_id) {
            continue;
        }
        if row.plane != region.level
            || row.x < region.min_x
            || row.x > region.max_x
            || row.z < region.min_z
            || row.z > region.max_z
        {
            continue;
        }
        if rows.len() >= limit {
            truncated = true;
            break;
        }
        rows.push(json!({
            "loc_id": row.loc_id,
            "x": row.x,
            "z": row.z,
            "plane": row.plane,
        }));
    }
    json!({
        "rows": rows,
        "truncated": truncated,
        "resource_ids": ids(&wood.loc_ids),
        "qualification": wood.qualification,
    })
}

fn accepted_skill(skill: &str) -> Result<&'static str, &'static str> {
    match skill.trim().to_ascii_lowercase().as_str() {
        "woodcutting" => Ok("woodcutting"),
        "mining" => Ok("mining"),
        "fishing" => Ok("fishing"),
        _ => Err(UNKNOWN_SKILL),
    }
}

fn key_eq(key: &str, name: &str) -> bool {
    let wanted = name.trim();
    !wanted.is_empty() && key.eq_ignore_ascii_case(wanted)
}

fn loc_row(skill: &str, row: &GatherLocResource) -> Value {
    json!({
        "skill": skill,
        "table": row.table,
        "resource_key": row.resource_key,
        "loc_ids": ids(&row.loc_ids),
        "empty_ids": ids(&row.empty_ids),
        "output": output(&row.output),
        "level": row.level,
        "qualification": row.qualification,
        "partial_sides": row.partial_sides,
        "missing_transform": row.missing_transform,
        "publication": row.publication,
    })
}

fn fishing_row(row: &GatherFishingMethod) -> Value {
    json!({
        "skill": "fishing",
        "category": row.category,
        "primary_op": row.primary_op,
        "pair_op": row.pair_op,
        "level": row.level,
        "output": output(&row.output),
        "qualification": row.qualification,
        "partial_sides": row.partial_sides,
    })
}

fn ids(rows: &[GatherLocId]) -> Value {
    Value::Array(
        rows.iter()
            .map(|row| json!({ "alias": row.alias, "id": row.id }))
            .collect(),
    )
}

fn output(value: &Option<GatherOutput>) -> Value {
    match value {
        Some(row) => json!({ "alias": row.alias, "id": row.id }),
        None => Value::Null,
    }
}

fn coverage_row(row: &GatherCoverageRecord) -> Value {
    let mut map = Map::new();
    map.insert("class".into(), json!(row.class));
    insert_str(&mut map, "table", row.table.as_deref());
    insert_str(&mut map, "resource_key", row.resource_key.as_deref());
    insert_str(&mut map, "alias", row.alias.as_deref());
    if let Some(value) = row.on_revision {
        map.insert("on_revision".into(), json!(value));
    }
    if let Some(value) = row.other_pin_id {
        map.insert("other_pin_id".into(), json!(value));
    }
    if let Some(value) = row.copied {
        map.insert("copied".into(), json!(value));
    }
    map.insert("reason".into(), json!(row.reason));
    Value::Object(map)
}

fn insert_str(map: &mut Map<String, Value>, key: &str, value: Option<&str>) {
    if let Some(value) = value {
        map.insert(key.to_string(), json!(value));
    }
}
