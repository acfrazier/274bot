use super::*;

/// Door edges from `scripts/doors/configs/*.loc` plus
/// `scripts/general_use/configs/gates.loc` (fence gates) openable ids +
/// the jm2 LOC placements (on their game plane, every level), two edges
/// per placement: `at` = the door loc tile, `dir` = the crossing direction
/// and its opposite (a door is bidirectional), `to` = each crossing's
/// adjacent landing tile when the collision admits it (otherwise no edge),
/// `open_loc_id` = the config's `param=next_loc_stage` open leaf. `option`
/// 1 is the `Open` op; each `to` is that crossing's arrival side, never a
/// snap. A straight wall (`wall_straight`, shape 0) door crosses along its
/// angle ([`door_far_side`]); a diagonal (`wall_diagonal`, shape 9) door
/// crosses its tile only when its open is the generic `~open_door`
/// ([`diagonal_door_crossings`]); any other shape has no `~door_open`
/// offset in `door_procs.rs2` and is counted and left out.
///
/// Quest-gated doors (`scripts/quests/*/configs/*.loc` and
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
/// generic one — and closed door members declared outside the door
/// configs likewise through [`generic_closed_doors`]. In-place swap doors
/// (curtains, fur doors: [`swap_doors`]) cross like a straight door.
#[allow(clippy::too_many_arguments)]
pub(super) fn door_edges(
    content_root: &Path,
    ids: &HashMap<String, i32>,
    positions: &HashMap<i32, Vec<Placement>>,
    loc_defs: &LocDefs,
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
    let supported_handlers = generic_gate_handlers(content_root);
    let members_gates = members_gate_handlers(content_root);
    let inherited = inherited_closed_gates(
        content_root,
        ids,
        positions,
        &supported_handlers,
        &members_gates,
        skipped,
    );
    door_ids.extend(inherited.keys().copied());
    open_ids.extend(inherited.iter().map(|(&id, &open)| (id, open)));
    // A gate admitted through its canonical members-check override opens
    // only on a members world (`MAP_MEMBERS`).
    let members_only: HashSet<i32> = members_gates
        .keys()
        .filter_map(|name| ids.get(name))
        .copied()
        .filter(|id| inherited.contains_key(id))
        .collect();
    // Closed doors anywhere in the content whose effective handler is a
    // verified generic door category (West Ardougne's `loc_2997`, the
    // Rellekka/Troll Stronghold/games room members): the same crossing.
    let overridden = oploc1_overrides(content_root);
    let generic = generic_closed_doors(content_root, ids, &overridden, skipped);
    door_ids.extend(generic.keys().copied());
    open_ids.extend(generic.iter().map(|(&id, door)| (id, door.open)));
    let swaps = swap_doors(content_root, ids, loc_defs, skipped);
    door_ids.extend(swaps.keys().copied());
    open_ids.extend(swaps.iter().map(|(&id, &open)| (id, open)));
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
        // Only the generic `~open_door` moves the leaf off a diagonal
        // door's tile; a quest door's own open script is not read for it.
        let swings_off_tile = generic
            .get(id)
            .is_some_and(|door| door.category == DOOR_CLOSED)
            && free_arm.is_none()
            && !door_reqs.contains_key(id);
        for p in placements {
            let Some(angle_dir) = door_dir(p.angle) else {
                continue;
            };
            let at = WorldTile {
                x: p.x,
                z: p.z,
                level: p.level,
            };
            // A door is bidirectional: a crossing in each direction, each
            // with an adjacent landing the collision admits. A blocked
            // landing yields no edge; opening a door cannot erase scenery.
            let crossings = match p.shape {
                LocShape::WALL_STRAIGHT => [angle_dir, opposite(angle_dir)]
                    .map(|dir| door_far_side(at, dir, collision).map(|to| (dir, to))),
                LocShape::WALL_DIAGONAL if swings_off_tile => {
                    diagonal_door_crossings(at, p.angle, collision)
                }
                _ => {
                    bump(skipped, SKIP_DOOR_SHAPE, 1);
                    continue;
                }
            };
            let quest_reverse =
                free_arm.and_then(|arm| completed_quest_reverse(*id, arm, ids, &varps));
            for (dir, to) in crossings.into_iter().flatten() {
                let is_free = free_arm.is_some_and(|arm| dir == arm.free_dir(angle_dir));
                let is_gated_reverse = free_arm.is_some() && !is_free;
                // A directional door keeps the crossing its open script
                // proves free. The gated reverse is emitted only with the
                // conservative completed-quest mapping; otherwise it stays
                // omitted rather than ungated.
                if is_gated_reverse && quest_reverse.is_none() {
                    continue;
                }
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
                        members_req: members_only.contains(id),
                        wildy_cap: None,
                    },
                    audit,
                );
            }
        }
    }
}

/// The two crossings of a closed diagonal (`wall_diagonal`, shape 9)
/// `door_closed` door on `at`, from the engine's own rules:
///
/// - The closed door is a 1×1 ground-layer footprint on `at`
///   (`changeLocCollision` → `changeLoc`), so the tile itself is blocked;
///   the diagonal wall line runs corner to corner — SW↔NE for angles 0/2,
///   NW↔SE for 1/3 — leaving the angle's side `{door_dir(angle),
///   door_dir(angle + 1)}` and the opposite side.
/// - It is opened from a cardinal neighbour (`ReachStrategy.reachWall1`,
///   `WALL_DIAGONAL`) and `~open_door` deletes it, freeing `at`, then adds
///   the leaf (`next_loc_stage`, also `wall_diagonal`) on
///   `~door_open(angle, wall_diagonal)` = the cardinal `door_dir(angle +
///   1)`, which the leaf's footprint now blocks.
///
/// So one crossing lands on the angle's side through its free cardinal
/// (`door_dir(angle)`), and the other on the opposite side through
/// `opposite(door_dir(angle))`, else `opposite(door_dir(angle + 1))`. A
/// crossing needs a standable landing the step off `at` may enter, and a
/// standable cardinal on the approach side from which the step onto `at`
/// is not walled (the door's own footprint aside) and `at` is not blocked
/// ground; otherwise it is left out. The landing's `dir` is the step off
/// `at`, so the traveller's half-plane crossing test keeps working.
pub(super) fn diagonal_door_crossings(
    at: WorldTile,
    angle: i32,
    collision: &WorldCollision,
) -> [Option<(DoorDir, WorldTile)>; 2] {
    let (Some(front), Some(leaf)) = (door_dir(angle), door_dir((angle + 1) & 3)) else {
        return [None, None];
    };
    let angle_side = [front, leaf];
    let other_side = [opposite(front), opposite(leaf)];
    let enters_door_tile = |from: DoorDir| {
        let (dx, dz) = dir_delta(from);
        let stand = WorldTile {
            x: at.x + dx,
            z: at.z + dz,
            level: at.level,
        };
        let Some(mask) = crate::router::cardinal_entry_mask((-dx, -dz)) else {
            return false;
        };
        // The door's own footprint is the `SQ_BLOCKED` base the open
        // removes; face walls and blocked ground stay.
        let faces = collision.walkable_word(at.x, at.z, at.level) & !crate::collision::SQ_BLOCKED;
        let ground = collision.flag(at.x, at.z, at.level) & CollisionFlag::WR_GRND as u32;
        collision.standable(stand) && faces & mask == 0 && ground == 0
    };
    let land = |approach: [DoorDir; 2], landings: &[DoorDir]| {
        if !approach.into_iter().any(enters_door_tile) {
            return None;
        }
        landings.iter().find_map(|&dir| {
            let to = door_far_side(at, dir, collision)?;
            crate::router::step_ok(collision, at, dir_delta(dir)).then_some((dir, to))
        })
    };
    [land(other_side, &[front]), land(angle_side, &other_side)]
}
