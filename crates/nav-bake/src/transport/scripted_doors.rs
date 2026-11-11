use super::*;

// ---------------------------------------------------------------------------
// Scripted wall crossings: skill-guild doors and the West Ardougne fence.
// ---------------------------------------------------------------------------

/// One straight wall loc whose own `[oploc1,<name>]` block walks the player
/// across it either way. A guild door is a `~check_axis` side test whose
/// gated arm refuses below `skill` `(id, level)` (and without `worn`
/// equipped), then `~open_and_close_door` both ways; the fence climbs over
/// with `~agility_exactmove` both ways, ungated. These semantics are modelled
/// by loc name.
pub(super) struct ScriptedDoor {
    pub(super) name: &'static str,
    pub(super) skill: Option<(i32, i32)>,
    pub(super) worn: Option<&'static str>,
    /// The `~check_axis(coord, loc_coord, loc_angle)` value the gated arm
    /// runs on; the crossing from that stand goes along the angle
    /// ([`FreeDoorArm::free_dir`]'s polarity).
    pub(super) gated_when_check_axis: bool,
}

pub(super) const SCRIPTED_DOORS: [ScriptedDoor; 4] = [
    ScriptedDoor {
        name: "chefdoor",
        skill: Some((SKILL_COOKING, 32)),
        worn: Some("chefs_hat"),
        gated_when_check_axis: false,
    },
    ScriptedDoor {
        name: "crafting_guild_door",
        skill: Some((SKILL_CRAFTING, 40)),
        worn: Some("brown_apron"),
        gated_when_check_axis: true,
    },
    ScriptedDoor {
        name: "loc_2025",
        skill: Some((SKILL_FISHING, 68)),
        worn: None,
        gated_when_check_axis: false,
    },
    ScriptedDoor {
        name: "mournerstewfence",
        skill: None,
        worn: None,
        gated_when_check_axis: false,
    },
];

/// Scripted wall crossings: for each [`SCRIPTED_DOORS`] entry whose loc has
/// exactly one `[oploc1,<name>]` block, every straight wall placement gets
/// both crossings; a guild door's gated one carries `skill_req` + the any-one
/// `worn_req`. The script moves the player across and no open leaf ever
/// stands, so none is packed. A missing or second block, an unresolved loc
/// or worn obj, or a non-straight placement is counted and left out.
pub(super) fn scripted_door_edges(
    content_root: &Path,
    ids: &HashMap<String, i32>,
    positions: &HashMap<i32, Vec<Placement>>,
    graph: &mut TransportGraph,
    collision: &WorldCollision,
    skipped: &mut HashMap<&'static str, usize>,
) {
    let mut blocks: HashMap<String, usize> = HashMap::new();
    visit_rs2(&content_root.join("scripts"), &mut |text| {
        for (op, name, _) in script_blocks(text) {
            if op == "oploc1" && SCRIPTED_DOORS.iter().any(|g| g.name == name) {
                *blocks.entry(name).or_default() += 1;
            }
        }
    });
    let objs = obj_ids_by_name(content_root);
    for guild in &SCRIPTED_DOORS {
        let Some(&loc_id) = ids.get(guild.name) else {
            continue;
        };
        let one_block = blocks.get(guild.name) == Some(&1);
        let worn = match guild.worn {
            Some(name) => objs.get(name).map(|&id| vec![id]),
            None => Some(vec![]),
        };
        let Some(worn) = worn.filter(|_| one_block) else {
            bump(skipped, SKIP_SCRIPTED_DOOR_SOURCE, 1);
            continue;
        };
        for p in positions.get(&loc_id).into_iter().flatten() {
            let Some(angle_dir) = door_dir(p.angle).filter(|_| p.shape == LocShape::WALL_STRAIGHT)
            else {
                bump(skipped, SKIP_DOOR_SHAPE, 1);
                continue;
            };
            let at = WorldTile {
                x: p.x,
                z: p.z,
                level: p.level,
            };
            let gated_dir = if guild.gated_when_check_axis {
                angle_dir
            } else {
                opposite(angle_dir)
            };
            for dir in [angle_dir, opposite(angle_dir)] {
                let Some(to) = straight_door_landing(at, angle_dir, dir, collision) else {
                    continue;
                };
                let gated = guild.skill.is_some() && dir == gated_dir;
                graph.edges.push(TransportEdge {
                    takeoff: None,
                    worn_all_req: Vec::new(),
                    forbidden_held: Vec::new(),
                    forbidden_worn: Vec::new(),
                    kind: TransportKind::Door,
                    player_delta: None,
                    at,
                    to,
                    loc_id,
                    option: 1,
                    ticks: 1,
                    dir: Some(dir),
                    open_loc_id: None,
                    skill_req: guild.skill.filter(|_| gated).into_iter().collect(),
                    item_req: vec![],
                    consumed_req: vec![],
                    item_returns: vec![],
                    quest_req: vec![],
                    varp_req: vec![],
                    worn_req: if gated { worn.clone() } else { vec![] },
                    members_req: false,
                    wildy_cap: None,
                    quest_gates: None,
                });
            }
        }
    }
}
