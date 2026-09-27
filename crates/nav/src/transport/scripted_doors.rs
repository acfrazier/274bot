use super::*;

// ---------------------------------------------------------------------------
// Scripted wall crossings: skill-guild doors and the West Ardougne fence.
// ---------------------------------------------------------------------------

/// One straight wall loc whose own `[oploc1,<name>]` block walks the player
/// across it either way: the block must read exactly `body` (comments and
/// whitespace aside). A guild door is a `~check_axis` side test whose gated
/// arm refuses below `skill` `(id, level)` (and without `worn` equipped),
/// then `~open_and_close_door` both ways; the fence climbs over with
/// `~agility_exactmove` both ways, ungated.
pub(super) struct ScriptedDoor {
    pub(super) name: &'static str,
    pub(super) body: &'static str,
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
        body: r#"def_boolean $is_inside = ~check_axis(coord, loc_coord, loc_angle);
if ($is_inside = false) {
    if (stat(cooking) < 32) {
        mes("You need a cooking level of 32 to enter the Chef's Guild.");
        def_string $fail_message = "<p,neutral>Sorry. Only the finest chefs are allowed in here. Get your cooking level up to 32";
        if (inv_total(worn, chefs_hat) < 1) {
            $fail_message = append($fail_message, " and come back wearing a chef's hat");
        }
        $fail_message = append($fail_message, ".");
        ~chatnpc_specific("Head chef", head_chef, $fail_message);
        return;
    }
    if (inv_total(worn, chefs_hat) < 1) {
        ~chatnpc_specific("Head chef", head_chef, "<p,neutral>You can't come in here unless you're wearing a chef's hat, or something like that.");
        return;
    }
}
~open_and_close_door(loc_param(next_loc_stage), $is_inside, false);"#,
        skill: Some((SKILL_COOKING, 32)),
        worn: Some("chefs_hat"),
        gated_when_check_axis: false,
    },
    ScriptedDoor {
        name: "crafting_guild_door",
        body: r#"if (~check_axis(coord, loc_coord, loc_angle) = true) {
    if (stat(crafting) < 40) {
        ~chatnpc_specific(nc_name(master_crafter), master_crafter, "<p,neutral>Sorry only experienced craftsmen are allowed in here.|You must be level 40 or above to enter.");
        return;
    }
    if (inv_total(worn, brown_apron) < 1) {
        ~chatnpc_specific(nc_name(master_crafter), master_crafter, "<p,neutral>Where's your brown apron? You can't come in here unless you're wearing one.");
        if (inv_total(inv, brown_apron) < 1) {
            ~chatplayer("<p,neutral>Err... I haven't got one.");
        }
        return;
    }
    ~open_and_close_door(loc_param(next_loc_stage), true, false);
    ~chatnpc_specific(nc_name(master_crafter), master_crafter, "<p,neutral>Welcome to the Guild of Master Craftsmen.");
} else {
    ~open_and_close_door(loc_param(next_loc_stage), false, false);
}"#,
        skill: Some((SKILL_CRAFTING, 40)),
        worn: Some("brown_apron"),
        gated_when_check_axis: true,
    },
    ScriptedDoor {
        name: "loc_2025",
        body: r#"def_boolean $is_inside = ~check_axis(coord, loc_coord, loc_angle);
if ($is_inside = false) {
    if (stat(fishing) < 68) {
        if (npc_find(coord, master_fisher, 10, 0) = true) {
            ~chatnpc("<p,happy>Hello, only the top fishers are allowed in here, you need a fishing level of 68 to enter.");
        }
        return;
    }
}
~open_and_close_door(loc_param(next_loc_stage), $is_inside, false);"#,
        skill: Some((SKILL_FISHING, 68)),
        worn: None,
        gated_when_check_axis: false,
    },
    ScriptedDoor {
        name: "mournerstewfence",
        body: r#"def_coord $start = loc_coord;
def_coord $end = ~movecoord_loc_return(~door_open(loc_angle, loc_shape));
def_int $dir;
switch_int(loc_angle) {
    case ^loc_west : $dir = ^exact_west;
    case ^loc_north : $dir = ^exact_north;
    case ^loc_east : $dir = ^exact_east;
    case ^loc_south : $dir = ^exact_south;
}
if(~check_axis(coord, loc_coord, loc_angle) = false) {
    $start = ~movecoord_loc_return(~door_open(loc_angle, loc_shape));
    $end = loc_coord;
    switch_int(loc_angle) {
        case ^loc_west : $dir = ^exact_east;
        case ^loc_north : $dir = ^exact_south;
        case ^loc_east : $dir = ^exact_west;
        case ^loc_south : $dir = ^exact_north;
    }
}
if (coord ! $start) {
    p_teleport($start);
    p_delay(1);
}
facesquare($end);
p_delay(0);
~agility_exactmove(human_walk_style, 0, 2, $start, $end, 0, 76, $dir, true);
p_teleport($end);"#,
        skill: None,
        worn: None,
        gated_when_check_axis: false,
    },
];

/// Scripted wall crossings: for each [`SCRIPTED_DOORS`] entry whose one
/// `[oploc1,<name>]` block is exactly the canonical body, every straight
/// wall placement gets both crossings; a guild door's gated one carries
/// `skill_req` + the any-one `worn_req`. The script moves the player across
/// and no open leaf ever stands, so none is packed. A body in any other
/// form, a second block, an unresolved loc or worn obj, or a non-straight
/// placement is counted and left out.
pub(super) fn scripted_door_edges(
    content_root: &Path,
    ids: &HashMap<String, i32>,
    positions: &HashMap<i32, Vec<Placement>>,
    graph: &mut TransportGraph,
    collision: &WorldCollision,
    skipped: &mut HashMap<&'static str, usize>,
) {
    let mut bodies: HashMap<String, Vec<String>> = HashMap::new();
    visit_rs2(&content_root.join("scripts"), &mut |text| {
        for (op, name, body) in script_blocks(text) {
            if op == "oploc1" && SCRIPTED_DOORS.iter().any(|g| g.name == name) {
                bodies.entry(name).or_default().push(normalized_body(&body));
            }
        }
    });
    let objs = obj_ids_by_name(content_root);
    for guild in &SCRIPTED_DOORS {
        let Some(&loc_id) = ids.get(guild.name) else {
            continue;
        };
        let canonical = bodies.get(guild.name).is_some_and(
            |b| matches!(b.as_slice(), [only] if *only == normalized_body(guild.body)),
        );
        let worn = match guild.worn {
            Some(name) => objs.get(name).map(|&id| vec![id]),
            None => Some(vec![]),
        };
        let Some(worn) = worn.filter(|_| canonical) else {
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
                let Some(to) = door_far_side(at, dir, collision) else {
                    continue;
                };
                let gated = guild.skill.is_some() && dir == gated_dir;
                graph.edges.push(TransportEdge {
                    kind: TransportKind::Door,
                    at,
                    to,
                    loc_id,
                    option: 1,
                    ticks: 1,
                    dir: Some(dir),
                    open_loc_id: None,
                    skill_req: guild.skill.filter(|_| gated).into_iter().collect(),
                    item_req: vec![],
                    quest_req: vec![],
                    varp_req: vec![],
                    worn_req: if gated { worn.clone() } else { vec![] },
                    members_req: false,
                    wildy_cap: None,
                });
            }
        }
    }
}
