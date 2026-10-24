use api::query::loc_approach::LocApproach;
use api::selected::{FactKey, QuestGate};
use api::snapshot::WorldTile;
use nav_planner::collision::WorldCollision;
use nav_planner::grid::StepGrid;
use nav_planner::pack::{
    validate_resource_requirements, validate_takeoff_admission, BankAccess, BankStand, PackError,
    DOOR_BYTES, MAGIC, MAGIC_GRID, MAX_GRID, PLAYER_RELATIVE, VERSION, VERSION_GRID,
};
use nav_planner::quest_gates::{QuestFamilyId, QuestGates};
use nav_planner::transport::{DoorDir, TransportGraph, TransportKind, WildernessRules};

use super::{banks::write_bank_stands, zones};

/// Serialize `g` to the 274N grid byte format.
pub fn encode_grid(g: &StepGrid) -> Vec<u8> {
    let mut out =
        Vec::with_capacity(4 + 1 + 12 + 8 + g.walk.len() + 4 + g.doors.len() * DOOR_BYTES);
    out.extend_from_slice(MAGIC_GRID);
    out.push(VERSION_GRID);
    for v in [g.origin.x, g.origin.z, g.origin.level] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&(g.width as u32).to_le_bytes());
    out.extend_from_slice(&(g.height as u32).to_le_bytes());
    out.extend_from_slice(&g.walk);
    out.extend_from_slice(&(g.doors.len() as u32).to_le_bytes());
    for d in &g.doors {
        for v in [
            d.loc.x,
            d.loc.z,
            d.loc.level,
            d.loc_id,
            d.from.x,
            d.from.z,
            d.from.level,
            d.to.x,
            d.to.z,
            d.to.level,
        ] {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    out
}

/// Serialize the whole-world collision + transport graph + bank stand and
/// zone tables to the v17 pack byte format. The graph's `at` index is not
/// stored; [`nav_planner::pack::decode`] rebuilds it from the edges and collision. Teleports
/// (kind-4 edges) are written after ordinary edges and always carry absent
/// approach geometry. The raw flags are not on the wire (see the flags
/// sidecar); the zone bucket index is rebuilt at decode.
///
/// Returns an error if an edge's quest gates name a family other than
/// [`TransportGraph::quest_family`], rather than silently rebinding the keys.
/// Invalid model data and unrepresentable sizes are also reported as errors.
pub fn encode(
    collision: &WorldCollision,
    graph: &TransportGraph,
    banks: &[BankStand],
) -> Result<Vec<u8>, PackError> {
    let capacity = encoded_size(collision, graph, banks)?;
    let mut out = Vec::new();
    out.try_reserve_exact(capacity).map_err(|error| {
        PackError::BadLength(format!(
            "cannot reserve {capacity} bytes for nav pack: {error}"
        ))
    })?;
    out.extend_from_slice(MAGIC);
    out.push(VERSION);
    write_quest_family(&mut out, graph.quest_family.as_ref());
    for v in [
        collision.origin.x,
        collision.origin.z,
        collision.origin.level,
    ] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&(collision.width as u32).to_le_bytes());
    out.extend_from_slice(&(collision.height as u32).to_le_bytes());
    out.extend_from_slice(&collision.walk);
    for w in &collision.blocked {
        out.extend_from_slice(&w.to_le_bytes());
    }
    let edge_count = graph.edges.len() + graph.teleports.len();
    out.extend_from_slice(&(edge_count as u32).to_le_bytes());
    for (edge_index, e) in graph.edges.iter().chain(&graph.teleports).enumerate() {
        out.push(
            kind_to_u8(e.kind)?
                | if e.player_delta.is_some() {
                    PLAYER_RELATIVE
                } else {
                    0
                },
        );
        let to = e
            .landing_from(e.at)
            .ok_or_else(|| PackError::BadLength("transport landing overflows its anchor".into()))?;
        for v in [
            e.at.x, e.at.z, e.at.level, to.x, to.z, to.level, e.loc_id, e.option, e.ticks,
        ] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.push(dir_to_u8(e.dir));
        out.extend_from_slice(&e.open_loc_id.unwrap_or(-1).to_le_bytes());
        write_req_pairs(&mut out, &e.skill_req);
        write_req_pairs(&mut out, &e.item_req);
        write_req_pairs(&mut out, &e.consumed_req);
        write_req_pairs(&mut out, &e.item_returns);
        write_req_strings(&mut out, &e.quest_req);
        write_req_pairs(&mut out, &e.varp_req);
        write_req_ids(&mut out, &e.worn_req);
        write_req_ids(&mut out, &e.worn_all_req);
        write_req_ids(&mut out, &e.forbidden_held);
        write_req_ids(&mut out, &e.forbidden_worn);
        out.push(u8::from(e.members_req));
        out.extend_from_slice(&e.wildy_cap.unwrap_or(-1).to_le_bytes());
        write_quest_gates(&mut out, e.quest_gates.as_ref());
        write_takeoff(&mut out, e.takeoff);
        write_approach(
            &mut out,
            if edge_index < graph.edges.len() {
                graph.approaches.get(edge_index).copied().flatten()
            } else {
                None
            },
        );
    }
    write_bank_stands(&mut out, banks)?;
    write_wilderness_rules(&mut out, &graph.wilderness)?;
    zones::write_table(&mut out, graph.zones.as_ref())?;
    if out.len() != capacity {
        return Err(PackError::BadLength(format!(
            "encoded pack length {} differs from checked size {capacity}",
            out.len()
        )));
    }
    Ok(out)
}
fn encoded_size(
    collision: &WorldCollision,
    graph: &TransportGraph,
    banks: &[BankStand],
) -> Result<usize, PackError> {
    if collision.width == 0
        || collision.height == 0
        || collision.width > MAX_GRID
        || collision.height > MAX_GRID
    {
        return Err(PackError::BadLength(format!(
            "grid {}x{} exceeds the {MAX_GRID} tile cap",
            collision.width, collision.height
        )));
    }
    let width = u32::try_from(collision.width)
        .map_err(|_| PackError::BadLength("grid width exceeds u32".into()))?;
    let height = u32::try_from(collision.height)
        .map_err(|_| PackError::BadLength("grid height exceeds u32".into()))?;
    let cells = collision
        .width
        .checked_mul(collision.height)
        .and_then(|cells| cells.checked_mul(4))
        .ok_or_else(|| PackError::BadLength("grid size overflows".into()))?;
    if collision.walk.len() != cells {
        return Err(PackError::BadLength(format!(
            "walk length {} differs from grid cell count {cells}",
            collision.walk.len()
        )));
    }
    let blocked_words = cells
        .checked_add(63)
        .ok_or_else(|| PackError::BadLength("blocked grid size overflows".into()))?
        / 64;
    if collision.blocked.len() != blocked_words {
        return Err(PackError::BadLength(format!(
            "blocked word count {} differs from expected {blocked_words}",
            collision.blocked.len()
        )));
    }

    let edge_count = graph
        .edges
        .len()
        .checked_add(graph.teleports.len())
        .ok_or_else(|| PackError::BadLength("transport edge count overflows".into()))?;
    checked_count(edge_count, "transport edge")?;
    let mut size = 4 + 1 + if graph.quest_family.is_some() { 35 } else { 1 };
    size = add_encoded_size(size, 12 + 8, "collision header")?;
    size = add_encoded_size(size, collision.walk.len(), "walk grid")?;
    size = add_encoded_size(
        size,
        collision
            .blocked
            .len()
            .checked_mul(8)
            .ok_or_else(|| PackError::BadLength("blocked grid size overflows".into()))?,
        "blocked grid",
    )?;
    size = add_encoded_size(size, 4, "transport edge count")?;

    for (edge_index, edge) in graph.edges.iter().chain(&graph.teleports).enumerate() {
        kind_to_u8(edge.kind)?;
        validate_resource_requirements(edge)?;
        if edge.player_delta.is_some()
            && !matches!(edge.kind, TransportKind::Ladder | TransportKind::Stairs)
        {
            return Err(PackError::BadLength(
                "player-relative landing is only valid for ladders and stairs".into(),
            ));
        }
        edge.landing_from(edge.at)
            .ok_or_else(|| PackError::BadLength("transport landing overflows its anchor".into()))?;
        if let Some(cap) = edge.wildy_cap {
            if cap < 0 {
                return Err(PackError::BadLength(
                    "wildy_cap must be non-negative or absent".into(),
                ));
            }
        }
        let approach = if edge_index < graph.edges.len() {
            graph.approaches.get(edge_index).copied().flatten()
        } else {
            None
        };
        validate_approach(edge.kind, approach)?;
        // The sizing helpers below add every vector and gate count prefix.
        let mut edge_size = 48usize;
        for (requirements, name) in [
            (&edge.skill_req, "skill requirements"),
            (&edge.item_req, "item requirements"),
            (&edge.consumed_req, "consumed requirements"),
            (&edge.item_returns, "item returns"),
            (&edge.varp_req, "varp requirements"),
        ] {
            size_pairs(&mut edge_size, requirements.len(), name)?;
        }
        size_strings(&mut edge_size, &edge.quest_req, "quest requirements")?;
        size_ids(&mut edge_size, edge.worn_req.len(), "worn requirements")?;
        size_ids(
            &mut edge_size,
            edge.worn_all_req.len(),
            "conjunctive worn requirements",
        )?;
        size_ids(&mut edge_size, edge.forbidden_held.len(), "held exclusions")?;
        size_ids(&mut edge_size, edge.forbidden_worn.len(), "worn exclusions")?;
        edge_size = add_encoded_size(edge_size, 13, "transport takeoff")?;
        size_quest_gates(&mut edge_size, edge.quest_gates.as_ref())?;
        if let Some(takeoff) = edge.takeoff {
            validate_takeoff_admission(collision, edge, takeoff, approach)?;
        }
        if approach.is_some() {
            edge_size = add_encoded_size(edge_size, 3, "approach geometry")?;
        }
        if let Some(gates) = &edge.quest_gates {
            if graph.quest_family.as_ref() != Some(gates.family()) {
                return Err(PackError::BadLength(format!(
                    "{:?} loc {} carries quest gates of another quest family than the pack's",
                    edge.kind, edge.loc_id
                )));
            }
        }
        size = add_encoded_size(size, edge_size, "transport edge")?;
    }

    checked_count(banks.len(), "bank stand")?;
    size = add_encoded_size(size, 4, "bank stand count")?;
    for bank in banks {
        size_string(&mut size, &bank.name, "bank stand name")?;
        size = add_encoded_size(size, 12, "bank stand tile")?;
        match &bank.access {
            BankAccess::Booth { .. } => {
                size = add_encoded_size(size, 5, "booth bank access")?;
            }
            BankAccess::Npc { name, choose, .. } => {
                size = add_encoded_size(size, 1, "NPC bank access tag")?;
                size_string(&mut size, name, "bank teller name")?;
                size = add_encoded_size(size, 4, "NPC bank operation")?;
                size = add_encoded_size(size, 1, "bank dialog choice tag")?;
                if let Some(choice) = choose {
                    size_string(&mut size, choice, "bank dialog choice")?;
                }
            }
        }
    }

    checked_count(graph.wilderness.zones.len(), "wilderness zone")?;
    let wilderness_size = graph
        .wilderness
        .zones
        .len()
        .checked_mul(28)
        .and_then(|zones| zones.checked_add(12))
        .ok_or_else(|| PackError::BadLength("wilderness table size overflows".into()))?;
    size = add_encoded_size(size, wilderness_size, "wilderness table")?;

    if let Some(table) = graph.zones.as_ref() {
        if table.bounds() != (collision.origin, width, height) {
            return Err(PackError::BadLength(
                "zone table bounds must match packed collision".into(),
            ));
        }
    }
    size = add_encoded_size(size, zones::wire_size(graph.zones.as_ref())?, "zone table")?;
    Ok(size)
}
fn checked_count(count: usize, what: &str) -> Result<u32, PackError> {
    u32::try_from(count).map_err(|_| PackError::BadLength(format!("{what} count exceeds u32")))
}

fn add_encoded_size(total: usize, extra: usize, what: &str) -> Result<usize, PackError> {
    total
        .checked_add(extra)
        .ok_or_else(|| PackError::BadLength(format!("{what} size overflows usize")))
}

fn size_string(total: &mut usize, value: &str, what: &str) -> Result<(), PackError> {
    u32::try_from(value.len())
        .map_err(|_| PackError::BadLength(format!("{what} length exceeds u32")))?;
    *total = add_encoded_size(
        *total,
        value
            .len()
            .checked_add(4)
            .ok_or_else(|| PackError::BadLength(format!("{what} size overflows usize")))?,
        what,
    )?;
    Ok(())
}

fn size_pairs(total: &mut usize, count: usize, what: &str) -> Result<(), PackError> {
    checked_count(count, what)?;
    let bytes = count
        .checked_mul(8)
        .and_then(|bytes| bytes.checked_add(4))
        .ok_or_else(|| PackError::BadLength(format!("{what} size overflows usize")))?;
    *total = add_encoded_size(*total, bytes, what)?;
    Ok(())
}

fn size_ids(total: &mut usize, count: usize, what: &str) -> Result<(), PackError> {
    checked_count(count, what)?;
    let bytes = count
        .checked_mul(4)
        .and_then(|bytes| bytes.checked_add(4))
        .ok_or_else(|| PackError::BadLength(format!("{what} size overflows usize")))?;
    *total = add_encoded_size(*total, bytes, what)?;
    Ok(())
}

fn size_strings(total: &mut usize, strings: &[String], what: &str) -> Result<(), PackError> {
    checked_count(strings.len(), what)?;
    *total = add_encoded_size(*total, 4, what)?;
    for string in strings {
        size_string(total, string, what)?;
    }
    Ok(())
}

fn size_key(total: &mut usize, key: &FactKey) -> Result<(), PackError> {
    size_string(total, &key.0, "quest gate key")
}

fn size_quest_gates(total: &mut usize, gates: Option<&QuestGates>) -> Result<(), PackError> {
    let gates = gates.map_or(&[][..], QuestGates::gates);
    checked_count(gates.len(), "quest gate")?;
    *total = add_encoded_size(*total, 4, "quest gate count")?;
    for gate in gates {
        *total = add_encoded_size(*total, 1, "quest gate tag")?;
        match gate {
            QuestGate::Complete(quest) => size_key(total, quest)?,
            QuestGate::Window(window) => {
                size_key(total, &window.quest)?;
                size_key(total, &window.signal)?;
                for bound in [window.values.min, window.values.max] {
                    *total = add_encoded_size(
                        *total,
                        if bound.is_some() { 5 } else { 1 },
                        "quest gate bound",
                    )?;
                }
            }
        }
    }
    Ok(())
}

fn validate_approach(kind: TransportKind, approach: Option<LocApproach>) -> Result<(), PackError> {
    if let Some(approach) = approach {
        if approach.width == 0 || approach.length == 0 {
            return Err(PackError::BadLength(
                "approach geometry dimensions must be positive".into(),
            ));
        }
        if approach.blocked_sides & !0x0f != 0 {
            return Err(PackError::BadLength(format!(
                "approach blocked-side mask {:#04x} exceeds 0x0f",
                approach.blocked_sides
            )));
        }
        if !matches!(
            kind,
            TransportKind::Ladder
                | TransportKind::Stairs
                | TransportKind::AgilityShortcut
                | TransportKind::SpiritTree
        ) {
            return Err(PackError::BadLength(format!(
                "approach geometry is not valid for {kind:?}"
            )));
        }
    }
    Ok(())
}

/// `TransportKind` as a wire byte.
fn kind_to_u8(k: TransportKind) -> Result<u8, PackError> {
    match k {
        TransportKind::Door => Ok(0),
        TransportKind::Ladder => Ok(1),
        TransportKind::Stairs => Ok(2),
        TransportKind::Boat => Ok(3),
        TransportKind::Teleport => Ok(4),
        TransportKind::AgilityShortcut => Ok(5),
        TransportKind::Glider => Ok(6),
        TransportKind::SpiritTree => Ok(7),
        TransportKind::Npc => Ok(8),
        TransportKind::EssenceExit => Err(PackError::BadLength(
            "the essence return is synthesized at runtime and cannot be packed".into(),
        )),
    }
}
/// `Option<DoorDir>` as a wire byte: `0=None, 1=N, 2=E, 3=S, 4=W`.
fn dir_to_u8(d: Option<DoorDir>) -> u8 {
    match d {
        None => 0,
        Some(DoorDir::N) => 1,
        Some(DoorDir::E) => 2,
        Some(DoorDir::S) => 3,
        Some(DoorDir::W) => 4,
    }
}
/// A requirement vector as `(id, value)` i32le pairs, count-prefixed.
fn write_req_pairs(out: &mut Vec<u8>, reqs: &[(i32, i32)]) {
    out.extend_from_slice(&(reqs.len() as u32).to_le_bytes());
    for (a, b) in reqs {
        out.extend_from_slice(&a.to_le_bytes());
        out.extend_from_slice(&b.to_le_bytes());
    }
}

/// A quest-name vector as length-prefixed UTF-8 strings, count-prefixed.
fn write_req_strings(out: &mut Vec<u8>, reqs: &[String]) {
    out.extend_from_slice(&(reqs.len() as u32).to_le_bytes());
    for s in reqs {
        out.extend_from_slice(&(s.len() as u32).to_le_bytes());
        out.extend_from_slice(s.as_bytes());
    }
}

/// A worn-item requirement vector as count-prefixed i32le ids.
fn write_req_ids(out: &mut Vec<u8>, reqs: &[i32]) {
    out.extend_from_slice(&(reqs.len() as u32).to_le_bytes());
    for id in reqs {
        out.extend_from_slice(&id.to_le_bytes());
    }
}

/// The v11 header's quest-family binding: `0`, or `1` + digest + schema.
fn write_quest_family(out: &mut Vec<u8>, family: Option<&QuestFamilyId>) {
    match family {
        None => out.push(0),
        Some(family) => {
            out.push(1);
            out.extend_from_slice(family.quest_facts_sha256());
            out.extend_from_slice(&family.quest_extractor_schema().get().to_le_bytes());
        }
    }
}

/// An edge's quest-stage gates, count-prefixed (`0` = ungated).
fn write_quest_gates(out: &mut Vec<u8>, gates: Option<&QuestGates>) {
    let gates = gates.map_or(&[][..], QuestGates::gates);
    out.extend_from_slice(&(gates.len() as u32).to_le_bytes());
    for gate in gates {
        match gate {
            QuestGate::Complete(quest) => {
                out.push(0);
                write_key(out, quest);
            }
            QuestGate::Window(window) => {
                out.push(1);
                write_key(out, &window.quest);
                write_key(out, &window.signal);
                write_bound(out, window.values.min);
                write_bound(out, window.values.max);
            }
        }
    }
}

/// Write a fixed-size v16 exact-takeoff field.
fn write_takeoff(out: &mut Vec<u8>, takeoff: Option<WorldTile>) {
    match takeoff {
        None => {
            out.push(0);
            out.extend_from_slice(&[0; 12]);
        }
        Some(tile) => {
            out.push(1);
            for value in [tile.x, tile.z, tile.level] {
                out.extend_from_slice(&value.to_le_bytes());
            }
        }
    }
}

/// Write the v13 per-edge footprint-approach geometry.
fn write_approach(out: &mut Vec<u8>, approach: Option<LocApproach>) {
    match approach {
        None => out.push(0),
        Some(approach) => {
            out.push(1);
            out.extend_from_slice(&[approach.width, approach.length, approach.blocked_sides]);
        }
    }
}

/// A fact key as a length-prefixed UTF-8 string.
fn write_key(out: &mut Vec<u8>, key: &FactKey) {
    out.extend_from_slice(&(key.0.len() as u32).to_le_bytes());
    out.extend_from_slice(key.0.as_bytes());
}

/// One stage-window bound: `0` unbounded, `1` + i32le.
fn write_bound(out: &mut Vec<u8>, bound: Option<i32>) {
    match bound {
        None => out.push(0),
        Some(value) => {
            out.push(1);
            out.extend_from_slice(&value.to_le_bytes());
        }
    }
}

fn write_wilderness_rules(out: &mut Vec<u8>, rules: &WildernessRules) -> Result<(), PackError> {
    out.extend_from_slice(
        &u32::try_from(rules.zones.len())
            .map_err(|_| PackError::BadLength("wilderness zone count exceeds u32".into()))?
            .to_le_bytes(),
    );
    for z in &rules.zones {
        for v in [z.x1, z.z1, z.x2, z.z2, z.level1, z.level2, z.origin_z] {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    out.extend_from_slice(&rules.divisor.to_le_bytes());
    out.extend_from_slice(&rules.offset.to_le_bytes());
    Ok(())
}
