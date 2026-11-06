//! 377-only vertical aliases and direct absolute climbs. Legacy extraction is
//! unchanged; this producer runs only for the explicitly selected revision.
use super::*;

pub(in crate::transport) fn supplemental_edges_377(
    content_root: &Path,
    ids: &HashMap<String, i32>,
    positions: &HashMap<i32, Vec<Placement>>,
    loc_defs: &LocDefs,
    graph: &mut TransportGraph,
    skipped: &mut HashMap<&'static str, usize>,
) {
    let ladder_source = fs::read_to_string(
        content_root.join("scripts/ladders+stairs/scripts/ladders.rs2"),
    )
    .ok();
    let climb_ticks = ladder_source.as_deref().and_then(climb_helper_ticks);
    let mut rules = HashMap::new();
    let mut extras = HashMap::new();

    // 377 ladders.rs2:90-97 renamed ladder_from_cellar to ladder_1755.
    // Retain the coordinate-specific Zanaris landing and the relative fallback,
    // not an invented unconditional return. Its cost is certified by the same
    // canonical climb helper as direct climbs (ladders.rs2:178-185).
    if let Some((source, helper_ticks)) = ladder_source.as_deref().zip(climb_ticks) {
        parse_script(source, TransportKind::Ladder, &mut rules);
        rules.retain(|(name, option), _| name == "ladder_1755" && *option == 1);
        if let Some(ticks) = ARRIVAL_DELAY_TICKS.checked_add(helper_ticks) {
            extras.insert(("ladder_1755".to_owned(), 1), ticks);
        }
    }

    // Unlike the standard two-phase trapdoor pair, trapdoor_9472 directly
    // p_telejumps to an absolute tile (general_use/scripts/trapdoors.rs2:25-29).
    // Reuse the strict AST recognizer: only presentation, suspension and one
    // fixed move are admitted. Any gate, unknown side effect or non-climb op
    // remains unsupported, and an existing specialized producer always wins.
    visit_rs2(&content_root.join("scripts"), &mut |text| {
        for (op, name, body) in script_blocks(text) {
            let Some(option) = oploc_option(&op) else {
                continue;
            };
            let Some((landing, ticks)) = direct_climb(&body, climb_ticks) else {
                continue;
            };
            if !matches!(landing, Landing::Abs { .. }) {
                continue;
            }
            let Some(&id) = ids.get(&name) else {
                continue;
            };
            if graph.edges.iter().any(|edge| edge.loc_id == id && edge.option == option)
                || !loc_defs.op_at(id, option).is_some_and(|op| op.starts_with("Climb"))
            {
                continue;
            }
            let kind = if name.contains("stair") {
                TransportKind::Stairs
            } else {
                TransportKind::Ladder
            };
            let key = (name, option);
            rules.entry(key.clone()).or_insert((kind, ScriptRule {
                fallback: Some(Outcome::Landing(landing)),
                ..ScriptRule::default()
            }));
            extras.insert(key, ticks);
        }
    });
    resolve_ladder_stair_rules(&rules, &extras, ids, positions, loc_defs, graph, skipped);
    manhole_edges_377(content_root, ids, positions, loc_defs, graph, skipped);
}

/// The palace coin alcove is reached through Varrock's sewer. Its surface
/// entrance is a source-proved Open → Climb pair, not a direct closed-loc jump.
fn manhole_edges_377(
    content_root: &Path,
    ids: &HashMap<String, i32>,
    positions: &HashMap<i32, Vec<Placement>>,
    loc_defs: &LocDefs,
    graph: &mut TransportGraph,
    skipped: &mut HashMap<&'static str, usize>,
) {
    let Ok(source) = fs::read_to_string(
        content_root.join("scripts/general_use/scripts/manholes.rs2"),
    ) else {
        return;
    };
    let mut closed = None;
    let mut open = None;
    for (op, name, body) in script_blocks(&source) {
        if op != "oploc1" {
            continue;
        }
        let slot = match name.as_str() {
            "manholeclosed" => &mut closed,
            "manholeopen" => &mut open,
            _ => continue,
        };
        if slot.replace(body).is_some() {
            return;
        }
    }
    let Some(open_body) = open else {
        return;
    };
    let Some((Landing::FromZ { d: 6400 }, extra)) =
        direct_climb(&open_body, None)
    else {
        return;
    };
    let Some(&open_id) = ids.get("manholeopen") else {
        return;
    };
    if !loc_defs.op_at(open_id, 1).is_some_and(|op| op.starts_with("Climb")) {
        return;
    }
    let mut rules = HashMap::new();
    let mut extras = HashMap::new();
    let mut pair = None;
    for (name, cost) in [("manholeopen", Some(extra)), ("manholeclosed", extra.checked_add(OP_BASE_TICKS))] {
        let Some(cost) = cost else {
            continue;
        };
        let Some(&id) = ids.get(name) else {
            continue;
        };
        if graph.edges.iter().any(|edge| edge.loc_id == id && edge.option == 1) {
            continue;
        }
        if name == "manholeclosed" {
            if loc_defs.op_at(id, 1) != Some("Open")
                || !closed.as_deref().is_some_and(manhole_opener_is_supported)
            {
                continue;
            }
            pair = Some((id, open_id));
        }
        let key = (name.to_owned(), 1);
        rules.insert(key.clone(), (TransportKind::Ladder, ScriptRule {
            fallback: Some(Outcome::Landing(Landing::FromZ { d: 6400 })),
            ..ScriptRule::default()
        }));
        extras.insert(key, cost);
    }
    let first = graph.edges.len();
    resolve_ladder_stair_rules(&rules, &extras, ids, positions, loc_defs, graph, skipped);
    if let Some((closed_id, open_id)) = pair {
        for edge in &mut graph.edges[first..] {
            if edge.loc_id == closed_id {
                edge.open_loc_id = Some(open_id);
            }
        }
    }
}

/// Certify all opener side effects. A gate, extra action, changed cover
/// placement, or unknown statement cannot become an unconditional edge.
fn manhole_opener_is_supported(body: &str) -> bool {
    let Some(statements) = parse_body(body) else {
        return false;
    };
    let mut changed = false;
    let mut cover = false;
    for statement in statements {
        let Stmt::Call(name, args) = statement else {
            return false;
        };
        match (name.as_str(), args.as_slice()) {
            ("mes", [Expr::Str(calls)]) if calls.is_empty() => {}
            ("sound_synth", [Expr::Word(_), Expr::Num(1), Expr::Num(0)]) => {}
            ("loc_change", [Expr::Word(open), Expr::Num(400)])
                if open == "manholeopen" && !changed =>
            {
                changed = true;
            }
            ("loc_add", [Expr::Call(call, coord), Expr::Word(object), Expr::Word(angle), Expr::Word(shape), Expr::Num(400)])
                if call == "movecoord"
                    && matches!(coord.as_slice(), [Expr::Word(origin), Expr::Num(0), Expr::Num(0), Expr::Num(-1)] if origin == "loc_coord")
                    && object == "manhole_cover"
                    && angle == "loc_angle"
                    && shape == "loc_shape"
                    && changed
                    && !cover =>
            {
                cover = true;
            }
            _ => return false,
        }
    }
    changed && cover
}
