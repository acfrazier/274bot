use super::*;

/// Door edges from `scripts/doors/configs/*.loc` plus
/// `scripts/general_use/configs/gates.loc` (fence gates) openable ids +
/// the jm2 LOC placements, two edges per placement: `at` = the door loc
/// tile, `dir` =
/// the placement angle's wall orientation and its opposite (a door is
/// bidirectional), `to` = each direction's adjacent tile when
/// [`WorldCollision::standable`] accepts it (otherwise no edge), `open_loc_id` = the
/// config's `param=next_loc_stage` open leaf. `option` 1 is the `Open` op;
/// each `to` is that crossing's arrival side, never a snap. Quest-gated
/// doors (`scripts/quests/*/configs/*.loc` and
/// `scripts/areas/*/configs/*.loc` named blocks) join the door set when
/// their `[oploc1,<name>]` open script declares a varp gate; producers
/// carry transmitted varps or convert the gate to a unique completed
/// journal proof, omitting edges with no live-observable proof.
/// A door whose open script instead proves a directional free arm
/// ([`quest_door_free_arms`]) keeps that crossing with empty requirements.
/// The gated reverse is omitted unless [`completed_quest_reverse`] proves a
/// conservative completed-quest requirement for that loc (Death Plateau hut
/// doors only); a missing or incomplete quest fact still fails closed. The
/// v8 pack carries no masked bitfield, so raw varp 315 is never the gate.
/// Closed fence-gate members declared outside `gates.loc` (the quest/area
/// configs) join the door set only through [`inherited_closed_gates`] —
/// the closed gate categories whose effective handler is the verified
/// generic one.
pub(super) fn door_edges(
    content_root: &Path,
    ids: &HashMap<String, i32>,
    graph: &mut TransportGraph,
    skipped: &mut HashMap<&'static str, usize>,
    collision: &WorldCollision,
    observable: &ObservableGates,
    audit: &mut VarpGateAudit,
) {
    let configs = content_root.join("scripts").join("doors").join("configs");
    let mut door_ids = HashSet::new();
    let mut open_ids: HashMap<i32, i32> = HashMap::new();
    if let Ok(entries) = fs::read_dir(&configs) {
        for ent in entries.flatten() {
            let path = ent.path();
            if path.extension().and_then(|s| s.to_str()) != Some("loc") {
                continue;
            }
            if let Ok(text) = fs::read_to_string(&path) {
                door_ids.extend(parse_door_config(&text));
                open_ids.extend(parse_door_open_ids(&text, ids));
            }
        }
    }
    // Fence gates live outside the door configs dir: `gates.loc` under
    // `scripts/general_use/configs`. The closed gate categories count as
    // openable like `door_closed`, so the same parse collects them.
    let gates = content_root
        .join("scripts")
        .join("general_use")
        .join("configs")
        .join("gates.loc");
    if let Ok(text) = fs::read_to_string(&gates) {
        door_ids.extend(parse_door_config(&text));
        open_ids.extend(parse_door_open_ids(&text, ids));
    }
    // Closed gate members declared outside `gates.loc` (the quest/area
    // configs) join the same set only while their effective handler is the
    // verified generic category open behavior.
    let positions = loc_positions(content_root);
    let supported_handlers = generic_gate_handlers(content_root);
    let inherited =
        inherited_closed_gates(content_root, ids, &positions, &supported_handlers, skipped);
    door_ids.extend(inherited.keys().copied());
    open_ids.extend(inherited.iter().map(|(&id, &open)| (id, open)));
    let constants = script_constants(content_root);
    let varps = varp_ids_by_name(content_root);
    let door_names = door_config_names(content_root, ids);
    let door_reqs = quest_door_reqs(content_root, &door_names, ids, &constants, &varps);
    let mut free_arms = quest_door_free_arms(content_root, &door_names, ids, &constants, &varps);
    // A door that also declares a readable direct-varp gate keeps the
    // existing two-edge gate: the two readings would disagree about the
    // same crossing, and the free arm is never a fallback for a gate.
    for id in door_reqs.keys() {
        if free_arms.remove(id).is_some() {
            bump(skipped, SKIP_FREE_ARM_GATE_CONFLICT, 1);
        }
    }
    door_ids.extend(door_reqs.keys().copied());
    door_ids.extend(free_arms.keys().copied());

    if door_ids.is_empty() {
        bump(skipped, SKIP_NO_DOOR_CONFIGS, 1);
        return;
    }
    for id in &door_ids {
        let Some(placements) = positions.get(id) else {
            continue;
        };
        let free_arm = free_arms.get(id);
        let varp_req = door_reqs.get(id).cloned().unwrap_or_default();
        for p in placements {
            // The collision bake (and its `standable`) is level 0 only.
            if p.level != 0 || p.shape != 0 {
                continue;
            }
            let Some(angle_dir) = door_dir(p.angle) else {
                continue;
            };
            let at = WorldTile {
                x: p.x,
                z: p.z,
                level: p.level,
            };
            // A door is bidirectional: an edge in `dir` and one in its
            // opposite, each with an adjacent standable destination. A blocked
            // neighbor yields no edge; opening a door cannot erase scenery.
            let quest_reverse =
                free_arm.and_then(|arm| completed_quest_reverse(*id, arm, ids, &varps));
            for dir in [angle_dir, opposite(angle_dir)] {
                let is_free = free_arm.is_some_and(|arm| dir == arm.free_dir(angle_dir));
                let is_gated_reverse = free_arm.is_some() && !is_free;
                // A directional door keeps the crossing its open script
                // proves free. The gated reverse is emitted only with the
                // conservative completed-quest mapping; otherwise it stays
                // omitted rather than ungated.
                if is_gated_reverse && quest_reverse.is_none() {
                    continue;
                }
                let Some(to) = door_far_side(at, dir, collision) else {
                    continue;
                };
                observable.admit_edge(
                    graph,
                    TransportEdge {
                        kind: TransportKind::Door,
                        at,
                        to,
                        loc_id: *id,
                        option: 1,
                        ticks: 1,
                        dir: Some(dir),
                        open_loc_id: open_ids.get(id).copied(),
                        skill_req: vec![],
                        item_req: vec![],
                        quest_req: if is_gated_reverse {
                            quest_reverse
                                .map(|q| vec![q.to_string()])
                                .unwrap_or_default()
                        } else {
                            vec![]
                        },
                        // The proven free crossing has no requirement; a door
                        // with a readable gate carries it on both crossings.
                        // The quest-gated reverse is not a varp gate.
                        varp_req: if free_arm.is_some() {
                            vec![]
                        } else {
                            varp_req.clone()
                        },
                        worn_req: vec![],
                        members_req: false,
                        wildy_cap: None,
                    },
                    audit,
                );
            }
        }
    }
}
