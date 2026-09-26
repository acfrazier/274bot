use super::*;

/// The Al Kharid border toll: 10 coins (`inv_del(inv, coins, 10)` in
/// `border_gate.rs2`'s `pass_toll_gate`, guarded by
/// `inv_total(inv, coins) < 10`).
pub(super) const AL_KHARID_TOLL_COINS: i32 = 10;
/// The Shantay henge doorway's `to`: `p_teleport(0_51_48_40_46)`
/// (3304,3118) then `p_telejump(movecoord(coord,0,0,-3))` → (3304,3115)
/// in `shantay_pass.rs2`'s `[queue,shantay_pass_enter]`.
pub(super) const SHANTAY_NORTH_TO: WorldTile = WorldTile {
    x: 3304,
    z: 3115,
    level: 0,
};

/// The Shantay henge edge ticks: OP_BASE 1 + the `p_teleport` tick + the
/// `p_telejump` tick (both `p_delay(0)` in the queue block).
pub(super) const SHANTAY_NORTH_TICKS: i32 = 3;

/// The Shantay henge free desert exit's `to`: the desert branch
/// (`coordz(coord) <= coordz(loc_coord)`) telejumps the player
/// `movecoord(coord,0,0,3)` — north of the loc — from wherever they
/// stand. From the desert-side stand directly south-east of the henge
/// ((3303,3115)) that landing is (3303,3118), an open tile north of the
/// gate; the hop's close-enough-2 arrive arm also covers the adjacent
/// desert-side approaches' landings ((3303,3117), (3302,3118)).
pub(super) const SHANTAY_SOUTH_TO: WorldTile = WorldTile {
    x: 3303,
    z: 3118,
    level: 0,
};
/// The free desert exit ticks: OP_BASE 1 + the `p_telejump` tick (the
/// branch's own `p_delay(0)`).
pub(super) const SHANTAY_SOUTH_TICKS: i32 = 2;

/// Al Kharid border-toll and Shantay-pass edges, derived from the loc
/// config, scripts and jm2 placements. Both toll locs have paid crossings
/// (`item_req` coins) and, when `border_gate.rs2` proves the free branch,
/// parallel crossings gated by the corresponding completed quest journal
/// row. `%princequest` is not transmitted to the client; `quests.rs2`
/// links it to a green `questlist:prince` row only at `^prince_complete`,
/// which must be at least the free-branch `^prince_saved` threshold. A
/// missing content fact omits the free crossing instead of assuming it open.
///
/// The toll gates (`border_gate_toll_left`/`_right`, loc 2882/2883) parse
/// as doors under [`parse_door_config_ids`], at the m51_50 (4,27)/(4,28)
/// placements (3268,3227)/(3268,3228). `open_loc_id` is the config's
/// `next_loc_stage` leaf (loc 1562/1563).
/// The Shantay henge doorway (loc 4031,
/// `op1=Go-through`) derives two `TransportKind::Door` edges, one per
/// script branch — the gated hop (`at` the m51_48 (38,44) placement =
/// (3302,3116), `to` [`SHANTAY_NORTH_TO`], `item_req` one Shantay pass,
/// from the pass/Al Kharid side `coordz(coord) > coordz(loc_coord)`, the
/// `inv_del(inv, shantay_pass, 1)` + `[queue,shantay_pass_enter]`
/// teleport) and the free desert exit (`at` one tile south of the
/// placement, on the desert side `coordz(coord) <= coordz(loc_coord)`,
/// `to` [`SHANTAY_SOUTH_TO`], **no** `item_req` — the same block's
/// `p_telejump(movecoord(coord,0,0,3))`). The pack edge is **not** a
/// plain walk: both hops are op interactions with the loc, and the gated
/// hop is the only edge into the desert (a pass-side player routes
/// through it; the free edge's `at` sits on the desert side, so the
/// interaction never fires the pass branch).
pub(super) fn toll_edges(
    content_root: &Path,
    ids: &HashMap<String, i32>,
    positions: &HashMap<i32, Vec<Placement>>,
    graph: &mut TransportGraph,
    collision: &WorldCollision,
    skipped: &mut HashMap<&'static str, usize>,
) {
    let gate_applicable = toll_gate_applicable(ids);
    let henge_applicable = ids.contains_key(SHANTAY_HENGE_LOC_NAME);
    // The toll charge and the Shantay pass resolve by name through
    // `pack/obj.pack`; a missing pack skips the family instead of faking
    // an item id.
    let objs = obj_ids_by_name(content_root);
    let (Some(&coins_id), Some(&pass_id)) = (objs.get("coins"), objs.get("shantay_pass")) else {
        bump(
            skipped,
            SKIP_TOLL_OBJ_PACK,
            toll_applicable_route_count(gate_applicable, henge_applicable),
        );
        return;
    };

    let alkharid = content_root
        .join("scripts")
        .join("areas")
        .join("area_alkharid");
    let Ok(config) = fs::read_to_string(alkharid.join("configs").join("border_gate.loc")) else {
        if gate_applicable > 0 {
            bump(skipped, SKIP_TOLL_CONFIG, gate_applicable);
        }
        if henge_applicable {
            bump(skipped, SKIP_TOLL_HENGE, 1);
        }
        return;
    };
    let toll_ids = parse_door_config_ids(&config, ids);
    let open_ids = parse_door_open_ids(&config, ids);
    // Packed declared gate ids whose named openable block was not admitted.
    // Ids that entered `toll_ids` already use SKIP_TOLL_GATE on zero-emission.
    bump_unadmitted_packed_ids(
        skipped,
        SKIP_TOLL_CONFIG,
        ids,
        TOLL_GATE_LOC_NAMES,
        &toll_ids,
    );
    let waiver = toll_waiver_gate(content_root, &alkharid);
    for id in toll_ids {
        let edge_start = graph.edges.len();
        let Some(placements) = positions.get(&id) else {
            if ids.values().any(|&packed| packed == id) {
                bump(skipped, SKIP_TOLL_GATE, 1);
            }
            continue;
        };
        for p in placements {
            if p.level != 0 || p.shape != 0 {
                continue;
            }
            let Some(dir) = door_dir(p.angle) else {
                continue;
            };
            let at = WorldTile {
                x: p.x,
                z: p.z,
                level: p.level,
            };
            for dir in [dir, opposite(dir)] {
                let Some(to) = door_far_side(at, dir, collision) else {
                    continue;
                };
                let edge = |item_req, quest_req| TransportEdge {
                    kind: TransportKind::Door,
                    at,
                    to,
                    loc_id: id,
                    option: 1, // Open (oploc1, `@find_and_talk_to_border_guard`)
                    ticks: 1,
                    dir: Some(dir),
                    open_loc_id: open_ids.get(&id).copied(),
                    skill_req: vec![],
                    item_req,
                    quest_req,
                    varp_req: vec![],
                    worn_req: vec![],
                    members_req: false,
                    wildy_cap: None,
                };
                // Prefer the free crossing on equal-cost relaxed searches
                // (bank-fetch diagnosis); both alternatives remain available.
                if let Some(quest) = &waiver {
                    graph.edges.push(edge(vec![], vec![quest.clone()]));
                }
                graph
                    .edges
                    .push(edge(vec![(coins_id, AL_KHARID_TOLL_COINS)], vec![]));
            }
        }
        if graph.edges.len() == edge_start && ids.values().any(|&packed| packed == id) {
            bump(skipped, SKIP_TOLL_GATE, 1);
        }
    }

    toll_shantay_henge_edges(ids, positions, graph, pass_id, skipped);
}

/// Resolve the only free arm in `[label,talk_to_border_guard]`: a threshold
/// guard must call `@pass_toll_gate` before the coin-dialogue arm. The label
/// has formal parameters, so the no-parameter `script_blocks` parser cannot
/// identify it. Match its varp to the journal-colour script, require that
/// journal green implies the guard threshold, then resolve its visible row
/// name from `questlist.if`. Unproven content never waives the toll.
fn toll_waiver_gate(content_root: &Path, alkharid: &Path) -> Option<String> {
    let script = fs::read_to_string(alkharid.join("scripts").join("border_gate.rs2")).ok()?;
    let body = script
        .split_once("[label,talk_to_border_guard]")?
        .1
        .split("\n[")
        .next()?;
    let guard = body.lines().find(|line| line.trim().starts_with("if (%"))?;
    let condition = guard.trim().strip_prefix("if (")?.strip_suffix(") {")?;
    let (varp, threshold) = condition.split_once(">=")?;
    let varp = varp.trim().strip_prefix('%')?;
    let threshold = threshold.trim().strip_prefix('^')?;
    let arm = body.split_once(guard)?.1.split_once('}')?.0;
    if !arm
        .lines()
        .any(|line| line.trim().starts_with("@pass_toll_gate("))
    {
        return None;
    }
    varp_ids_by_name(content_root).get(varp)?;
    let journal = JournalLinks::from_content(content_root);
    let saved = journal.constant(threshold)?;
    journal.completed_name(varp, saved).map(str::to_owned)
}

pub(super) fn toll_shantay_henge_edges(
    ids: &HashMap<String, i32>,
    positions: &HashMap<i32, Vec<Placement>>,
    graph: &mut TransportGraph,
    pass_id: i32,
    skipped: &mut HashMap<&'static str, usize>,
) {
    // The Shantay henge: two edges, one per `[oploc1,shantay_pass_
    // henge_doorway]` branch — the gated hop `at` the loc's placement
    // tile (shape 10, unlike the wall doors), and the free desert exit
    // `at` one tile south of it (the desert side `coordz(coord) <=
    // coordz(loc_coord)`; the interaction from any tile on that side of
    // the gate always fires the free branch).
    let Some(&henge_id) = ids.get("shantay_pass_henge_doorway") else {
        return;
    };
    let Some(placements) = positions.get(&henge_id) else {
        bump(skipped, SKIP_TOLL_HENGE, 1);
        return;
    };
    let edge_start = graph.edges.len();
    for p in placements {
        if p.level != 0 {
            continue;
        }
        let at = WorldTile {
            x: p.x,
            z: p.z,
            level: p.level,
        };
        graph.edges.push(TransportEdge {
            kind: TransportKind::Door,
            at,
            to: SHANTAY_NORTH_TO,
            loc_id: henge_id,
            option: 1, // Go-through (oploc1)
            ticks: SHANTAY_NORTH_TICKS,
            dir: None,
            open_loc_id: None,
            skill_req: vec![],
            item_req: vec![(pass_id, 1)],
            quest_req: vec![],
            varp_req: vec![],
            worn_req: vec![],
            members_req: false,
            wildy_cap: None,
        });
        graph.edges.push(TransportEdge {
            kind: TransportKind::Door,
            at: WorldTile {
                x: p.x,
                z: p.z - 1,
                level: p.level,
            },
            to: SHANTAY_SOUTH_TO,
            loc_id: henge_id,
            option: 1, // Go-through (oploc1)
            ticks: SHANTAY_SOUTH_TICKS,
            dir: None,
            open_loc_id: None,
            skill_req: vec![],
            item_req: vec![],
            quest_req: vec![],
            varp_req: vec![],
            worn_req: vec![],
            members_req: false,
            wildy_cap: None,
        });
    }
    if graph.edges.len() == edge_start {
        bump(skipped, SKIP_TOLL_HENGE, 1);
    }
}
