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
}
