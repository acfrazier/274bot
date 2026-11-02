use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use api::query::loc_approach::LocApproach;
use api::selected::{FactKey, QuestGate};
use nav_planner::quest_gates::{QuestFamilyId, QuestGates};
use nav_planner::transport::{DoorDir, TransportEdge, TransportGraph, TransportKind};

use super::*;

const SKIP_GATE_SOURCE: &str = "Legends gate source did not match its pinned handler";
const SKIP_GATE_CONFIG: &str = "Legends gate config, pack id or placement was not unique";
const SKIP_GATE_QUEST: &str = "Legends gate quest progress identity was not verified";
const SKIP_JUNGLE_SOURCE: &str =
    "Legends jungle, axe or coordinate source did not match its pinned handler";
const SKIP_JUNGLE_CONFIG: &str = "Legends jungle loc config, pack id or placement was not verified";
const SKIP_JUNGLE_GEOMETRY: &str = "Legends jungle crossing had no verified standable landing";
const SKIP_JUNGLE_TOOLS: &str = "Legends jungle item or tool ids were not verified";
const SKIP_JUNGLE_QUEST: &str = "Legends jungle completion identity was not verified";

const LEGENDS_GATE_LOCATIONS: [GateLoc; 2] = [
    GateLoc {
        name: "legendsguildgatel",
        id: 2391,
        category: "double_door_open_and_close_left",
        next_stage: "loc_1562",
        next_stage_id: 1562,
        x: 2728,
        z: 3349,
    },
    GateLoc {
        name: "legendsguildgater",
        id: 2392,
        category: "double_door_open_and_close_right",
        next_stage: "loc_1563",
        next_stage_id: 1563,
        x: 2729,
        z: 3349,
    },
];

// jungle_tree.rs2:61-69 delays the first roll by 3 ticks for bushes and 5 for
// trees; 115-118 permits that roll only at the selected action-delay tick.
const JUNGLE_LOCS: [JungleLoc; 5] = [
    JungleLoc {
        name: "kharazi_jungle_tree_logs",
        id: 2887,
        category: "jungle_tree",
        next_stage: "junglestump_kharazi",
        next_stage_id: 2891,
        ticks: 5,
    },
    JungleLoc {
        name: "kharazi_jungle_tree1",
        id: 2889,
        category: "jungle_tree",
        next_stage: "junglestump_kharazi",
        next_stage_id: 2891,
        ticks: 5,
    },
    JungleLoc {
        name: "kharazi_jungle_tree2",
        id: 2890,
        category: "jungle_tree",
        next_stage: "junglestump_kharazi",
        next_stage_id: 2891,
        ticks: 5,
    },
    JungleLoc {
        name: "kharazi_jungle_plant1",
        id: 2892,
        category: "jungle_bush",
        next_stage: "kharazi_jungle_plant1_slashed",
        next_stage_id: 2894,
        ticks: 3,
    },
    JungleLoc {
        name: "kharazi_jungle_plant2",
        id: 2893,
        category: "jungle_bush",
        next_stage: "kharazi_jungle_plant1_slashed",
        next_stage_id: 2894,
        ticks: 3,
    },
];

const JUNGLE_MAPS: [(&str, i32); 2] = [("thkaramjamap", 714), ("thkaramjamapcomp", 715)];
const AXES: [(&str, i32); 7] = [
    ("rune_axe", 1359),
    ("adamant_axe", 1357),
    ("mithril_axe", 1355),
    ("black_axe", 1361),
    ("steel_axe", 1353),
    ("iron_axe", 1349),
    ("bronze_axe", 1351),
];
const MACHETTE: (&str, i32) = ("machette", 975);
const MAP_BLOCKED_MASK: u32 = 0x240100;
const LEGENDS_CONFIG: &str = "quests/quest_legends/configs/quest_legends.loc";
const LEGENDS_GATE_SCRIPT: &str = "quests/quest_legends/scripts/legends_gate.rs2";
const JUNGLE_SCRIPT: &str = "quests/quest_legends/scripts/jungle_tree.rs2";
const WOODCUT_SCRIPT: &str = "skill_woodcutting/scripts/woodcut.rs2";
const COORD_SCRIPT: &str = "general/scripts/misc/coord_procs.rs2";

#[derive(Clone, Copy)]
struct GateLoc {
    name: &'static str,
    id: i32,
    category: &'static str,
    next_stage: &'static str,
    next_stage_id: i32,
    x: i32,
    z: i32,
}

#[derive(Clone, Copy)]
struct JungleLoc {
    name: &'static str,
    id: i32,
    category: &'static str,
    next_stage: &'static str,
    next_stage_id: i32,
    ticks: i32,
}
#[derive(Clone, Copy)]
struct JungleCrossing {
    loc_id: i32,
    at: WorldTile,
    takeoff: WorldTile,
    to: WorldTile,
    ticks: i32,
}

#[derive(Clone, Copy)]
struct JungleToolOptions<'a> {
    machette: i32,
    machette_wearable: bool,
    axes: &'a [i32],
    worn_axes: &'a [i32],
}

#[derive(Clone, Copy)]
struct JungleToolUse {
    machette_worn: bool,
    axe: JungleAxeUse,
}

#[derive(Clone, Copy)]
enum JungleAxeUse {
    Carried(i32),
    Worn,
}

struct LegendsQuest {
    family: QuestFamilyId,
    quest: FactKey,
    signal: FactKey,
    started_min: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SourceBlock {
    header_tail: String,
    params: Vec<String>,
    body: Vec<super::rs2_syntax::Stmt>,
}

type SourceBlocks = HashMap<(String, String), Vec<SourceBlock>>;

#[derive(Default)]
struct LocConfigBuilder {
    op1: Vec<String>,
    category: Vec<String>,
    active: Vec<String>,
    blockwalk: Vec<String>,
    next_stage: Vec<String>,
}

#[derive(Debug)]
struct LocConfig {
    op1: String,
    category: String,
    active: Option<String>,
    blockwalk: Option<String>,
    next_stage: String,
}

pub(super) fn legends_edges(
    content_root: &Path,
    positions: &HashMap<i32, Vec<Placement>>,
    loc_defs: &LocDefs,
    graph: &mut TransportGraph,
    collision: &WorldCollision,
    skipped: &mut HashMap<&'static str, usize>,
    quests: Option<&QuestSource<'_>>,
) {
    if !source_matches_file(
        content_root,
        LEGENDS_GATE_SCRIPT,
        include_str!("legends_gate_template.rs2"),
    ) || !pinned_engine_proc_closure_matches(
        content_root,
        &["open_and_close_double_door2", "open_and_close_double_door3"],
    ) {
        bump(skipped, SKIP_GATE_SOURCE, 1);
    } else if let Some(quest) = legends_quest(content_root, quests) {
        gate_edges(content_root, positions, graph, collision, skipped, &quest);
    } else {
        bump(skipped, SKIP_GATE_QUEST, 1);
    }

    let jungle_sources_match = source_matches_file(
        content_root,
        JUNGLE_SCRIPT,
        include_str!("legends_jungle_template.rs2"),
    ) && source_matches_file(
        content_root,
        WOODCUT_SCRIPT,
        include_str!("legends_axe_template.rs2"),
    ) && source_matches_file(
        content_root,
        COORD_SCRIPT,
        include_str!("legends_coords_template.rs2"),
    );
    if !jungle_sources_match {
        bump(skipped, SKIP_JUNGLE_SOURCE, 1);
        return;
    }
    jungle_edges(
        content_root,
        positions,
        loc_defs,
        graph,
        collision,
        skipped,
        quests,
    );
}

fn gate_edges(
    content_root: &Path,
    positions: &HashMap<i32, Vec<Placement>>,
    graph: &mut TransportGraph,
    collision: &WorldCollision,
    skipped: &mut HashMap<&'static str, usize>,
    quest: &LegendsQuest,
) {
    let mut gates = Vec::with_capacity(LEGENDS_GATE_LOCATIONS.len());
    for gate in LEGENDS_GATE_LOCATIONS {
        let Some(id) = pinned_pack_id(content_root, "loc.pack", gate.name, gate.id) else {
            bump(skipped, SKIP_GATE_CONFIG, 1);
            return;
        };
        let Some(config) = unique_loc_config(content_root, LEGENDS_CONFIG, gate.name) else {
            bump(skipped, SKIP_GATE_CONFIG, 1);
            return;
        };
        let Some(open_loc_id) = pinned_pack_id(
            content_root,
            "loc.pack",
            gate.next_stage,
            gate.next_stage_id,
        ) else {
            bump(skipped, SKIP_GATE_CONFIG, 1);
            return;
        };
        if config.op1 != "Open"
            || config.category != gate.category
            || config.active.as_deref() != Some("yes")
            || config.next_stage != gate.next_stage
        {
            bump(skipped, SKIP_GATE_CONFIG, 1);
            return;
        }
        let Some([placement]) = positions.get(&id).map(Vec::as_slice) else {
            bump(skipped, SKIP_GATE_CONFIG, 1);
            return;
        };
        if placement.x != gate.x
            || placement.z != gate.z
            || placement.level != 0
            || placement.shape != LocShape::WALL_STRAIGHT
            || placement.angle != 1
        {
            bump(skipped, SKIP_GATE_CONFIG, 1);
            return;
        }
        let Some(angle_dir) = door_dir(placement.angle) else {
            bump(skipped, SKIP_GATE_CONFIG, 1);
            return;
        };
        gates.push((gate, id, open_loc_id, angle_dir));
    }

    // Varp 139 is not sent to the live client. Require typed progress evidence.
    let Ok(quest_gates) = QuestGates::new(
        quest.family,
        [QuestGate::Window(api::selected::StageWindow {
            quest: quest.quest.clone(),
            signal: quest.signal.clone(),
            values: api::selected::InclusiveRange {
                min: Some(quest.started_min),
                max: None,
            },
        })],
    ) else {
        bump(skipped, SKIP_GATE_QUEST, 1);
        return;
    };
    let mut directions = 0;
    for (gate, id, open_loc_id, angle_dir) in gates {
        let at = WorldTile {
            x: gate.x,
            z: gate.z,
            level: 0,
        };
        for dir in [angle_dir, opposite(angle_dir)] {
            let Some(to) = straight_door_landing(at, angle_dir, dir, collision) else {
                continue;
            };
            graph.edges.push(TransportEdge {
                kind: TransportKind::Door,
                at,
                to,
                takeoff: None,
                player_delta: None,
                loc_id: id,
                option: 1,
                ticks: 1,
                dir: Some(dir),
                open_loc_id: Some(open_loc_id),
                skill_req: vec![],
                item_req: vec![],
                consumed_req: vec![],
                item_returns: vec![],
                quest_req: vec![],
                varp_req: vec![],
                worn_req: vec![],
                worn_all_req: vec![],
                forbidden_held: vec![],
                forbidden_worn: vec![],
                members_req: false,
                wildy_cap: None,
                quest_gates: Some(quest_gates.clone()),
            });
            directions += 1;
        }
    }
    if directions != LEGENDS_GATE_LOCATIONS.len() * 2 {
        bump(
            skipped,
            SKIP_GATE_CONFIG,
            LEGENDS_GATE_LOCATIONS.len() * 2 - directions,
        );
    }
}

fn jungle_edges(
    content_root: &Path,
    positions: &HashMap<i32, Vec<Placement>>,
    loc_defs: &LocDefs,
    graph: &mut TransportGraph,
    collision: &WorldCollision,
    skipped: &mut HashMap<&'static str, usize>,
    quests: Option<&QuestSource<'_>>,
) {
    if collision.flags.is_none() {
        bump(skipped, SKIP_JUNGLE_GEOMETRY, 1);
        return;
    }
    let map_ids = JUNGLE_MAPS
        .iter()
        .filter_map(|&(name, expected)| pinned_pack_id(content_root, "obj.pack", name, expected))
        .collect::<Vec<_>>();
    let machette_id = pinned_pack_id(content_root, "obj.pack", MACHETTE.0, MACHETTE.1);
    let resolved_axes = AXES
        .iter()
        .filter_map(|&(name, expected)| {
            pinned_pack_id(content_root, "obj.pack", name, expected).map(|id| (name, id))
        })
        .collect::<Vec<_>>();
    if map_ids.is_empty() || machette_id.is_none() || resolved_axes.is_empty() {
        bump(skipped, SKIP_JUNGLE_TOOLS, 1);
        return;
    }
    let Some(machette_id) = machette_id else {
        return;
    };
    let constants = script_constants(content_root);
    let objects = std::iter::once((MACHETTE.0.to_string(), machette_id))
        .chain(
            resolved_axes
                .iter()
                .map(|&(name, id)| (name.to_string(), id)),
        )
        .collect::<HashMap<_, _>>();
    let worn_slots = stage_doors::worn_slot_facts(content_root, &objects, &constants);
    let right_hand = constants.get("wearpos_rhand").copied();
    let machette_wearable =
        right_hand.is_some_and(|slot| worn_slots.get(&machette_id) == Some(&slot));
    let mut axes = resolved_axes.iter().map(|&(_, id)| id).collect::<Vec<_>>();
    let mut worn_axes = axes
        .iter()
        .copied()
        .filter(|id| right_hand.is_some_and(|slot| worn_slots.get(id) == Some(&slot)))
        .collect::<Vec<_>>();
    worn_axes.sort_unstable();
    axes.sort_unstable();
    let mut map_ids = map_ids;
    map_ids.sort_unstable();
    let quest = legends_quest(content_root, quests);
    let tools = JungleToolOptions {
        machette: machette_id,
        machette_wearable,
        axes: &axes,
        worn_axes: &worn_axes,
    };
    if quest.is_none() {
        bump(skipped, SKIP_JUNGLE_QUEST, 1);
    }

    let loc_ids = JUNGLE_LOCS
        .iter()
        .filter_map(|loc| {
            pinned_pack_id(content_root, "loc.pack", loc.name, loc.id)
                .filter(|&id| id == loc.id)
                .map(|id| (*loc, id))
        })
        .collect::<Vec<_>>();
    for loc in JUNGLE_LOCS {
        let Some((_, id)) = loc_ids
            .iter()
            .find(|(candidate, _)| candidate.name == loc.name)
        else {
            bump(skipped, SKIP_JUNGLE_CONFIG, 1);
            continue;
        };
        let Some(config) = unique_loc_config(content_root, LEGENDS_CONFIG, loc.name) else {
            bump(skipped, SKIP_JUNGLE_CONFIG, 1);
            continue;
        };
        let Some(next_stage_id) =
            pinned_pack_id(content_root, "loc.pack", loc.next_stage, loc.next_stage_id)
        else {
            bump(skipped, SKIP_JUNGLE_CONFIG, 1);
            continue;
        };
        if config.op1 != "Chop-down"
            || config.category != loc.category
            || config.blockwalk.as_deref() != Some("no")
            || config.next_stage != loc.next_stage
            || next_stage_id != loc.next_stage_id
        {
            bump(skipped, SKIP_JUNGLE_CONFIG, 1);
            continue;
        }
        let Some(def) = loc_defs.loc(*id) else {
            bump(skipped, SKIP_JUNGLE_CONFIG, 1);
            continue;
        };
        for placement in positions.get(id).into_iter().flatten() {
            if placement.shape != LocShape::CENTREPIECE_STRAIGHT {
                bump(skipped, SKIP_JUNGLE_GEOMETRY, 1);
                continue;
            }
            let at = WorldTile {
                x: placement.x,
                z: placement.z,
                level: placement.level,
            };
            let Some(approach) = LocApproach::from_loc_def(
                def.width,
                def.length,
                placement.angle,
                def.force_approach,
            ) else {
                bump(skipped, SKIP_JUNGLE_GEOMETRY, 1);
                continue;
            };
            let mut stands = Vec::new();
            for x in at.x - 1..=at.x + i32::from(approach.width) {
                for z in at.z - 1..=at.z + i32::from(approach.length) {
                    let stand = WorldTile {
                        x,
                        z,
                        level: at.level,
                    };
                    if stand != at
                        && collision.standable(stand)
                        && approach.can_operate(
                            at,
                            stand,
                            collision.walkable_word(x, z, stand.level) as i32,
                        )
                    {
                        stands.push(stand);
                    }
                }
            }
            stands.sort_unstable_by_key(|stand| (stand.level, stand.x, stand.z));
            stands.dedup();
            let mut emitted = false;
            for takeoff in stands {
                let Some(to) = jungle_landing(takeoff, at) else {
                    continue;
                };
                let map_blocked = collision.flag(to.x, to.z, to.level) & MAP_BLOCKED_MASK != 0;
                let destination_is_jungle = JUNGLE_LOCS.iter().any(|candidate| {
                    positions
                        .get(&candidate.id)
                        .into_iter()
                        .flatten()
                        .any(|p| p.x == to.x && p.z == to.z && p.level == to.level)
                });
                // The handler permits a blocked landing only when another
                // active jungle loc occupies it; model that successful roll.
                if map_blocked && !destination_is_jungle {
                    continue;
                }
                if !collision.standable(to) && !destination_is_jungle {
                    continue;
                }
                let before = graph.edges.len();
                let crossing = JungleCrossing {
                    loc_id: loc.id,
                    at,
                    takeoff,
                    to,
                    ticks: loc.ticks,
                };
                emit_jungle_variants(graph, crossing, tools, &map_ids, quest.as_ref());
                emitted |= graph.edges.len() > before;
            }
            if !emitted {
                bump(skipped, SKIP_JUNGLE_GEOMETRY, 1);
            }
        }
    }
}

fn emit_jungle_variants(
    graph: &mut TransportGraph,
    crossing: JungleCrossing,
    tools: JungleToolOptions<'_>,
    map_ids: &[i32],
    quest: Option<&LegendsQuest>,
) {
    for &map_id in map_ids {
        emit_jungle_unlock_variants(graph, crossing, tools, JungleUnlock::Item(map_id));
    }
    if let Some(quest) = quest {
        let completed_gate =
            QuestGates::new(quest.family, [QuestGate::Complete(quest.quest.clone())]).ok();
        if let Some(completed_gate) = completed_gate.as_ref() {
            emit_jungle_unlock_variants(
                graph,
                crossing,
                tools,
                JungleUnlock::Complete(completed_gate),
            );
        }
    }
}

fn emit_jungle_unlock_variants(
    graph: &mut TransportGraph,
    crossing: JungleCrossing,
    tools: JungleToolOptions<'_>,
    unlock: JungleUnlock<'_>,
) {
    for machette_worn in [false, true] {
        if machette_worn && !tools.machette_wearable {
            continue;
        }
        for &axe in tools.axes {
            push_jungle_edge(
                graph,
                crossing,
                tools,
                JungleToolUse {
                    machette_worn,
                    axe: JungleAxeUse::Carried(axe),
                },
                unlock,
            );
        }
        if !tools.worn_axes.is_empty() && !machette_worn {
            push_jungle_edge(
                graph,
                crossing,
                tools,
                JungleToolUse {
                    machette_worn,
                    axe: JungleAxeUse::Worn,
                },
                unlock,
            );
        }
    }
}

fn push_jungle_edge(
    graph: &mut TransportGraph,
    crossing: JungleCrossing,
    tools: JungleToolOptions<'_>,
    tool_use: JungleToolUse,
    unlock: JungleUnlock<'_>,
) {
    let mut item_req = Vec::with_capacity(3);
    let worn_req = match tool_use.axe {
        JungleAxeUse::Carried(axe) => {
            item_req.push((axe, 1));
            Vec::new()
        }
        JungleAxeUse::Worn => tools.worn_axes.to_vec(),
    };
    if !tool_use.machette_worn {
        item_req.push((tools.machette, 1));
    }
    let quest_gates = match unlock {
        JungleUnlock::Item(id) => {
            item_req.push((id, 1));
            None
        }
        JungleUnlock::Complete(gates) => Some(gates.clone()),
    };
    item_req.sort_unstable();
    let worn_all_req = if tool_use.machette_worn {
        vec![tools.machette]
    } else {
        Vec::new()
    };
    graph.edges.push(TransportEdge {
        kind: TransportKind::AgilityShortcut,
        at: crossing.at,
        to: crossing.to,
        takeoff: Some(crossing.takeoff),
        player_delta: None,
        loc_id: crossing.loc_id,
        option: 1,
        ticks: crossing.ticks,
        dir: None,
        open_loc_id: None,
        skill_req: vec![],
        item_req,
        consumed_req: vec![],
        item_returns: vec![],
        quest_req: vec![],
        varp_req: vec![],
        worn_req,
        worn_all_req,
        forbidden_held: vec![],
        forbidden_worn: vec![],
        members_req: false,
        wildy_cap: None,
        quest_gates,
    });
}

#[derive(Clone, Copy)]
enum JungleUnlock<'a> {
    Item(i32),
    Complete(&'a QuestGates),
}

fn jungle_landing(takeoff: WorldTile, at: WorldTile) -> Option<WorldTile> {
    let dx = at.x.checked_sub(takeoff.x)?;
    let dz = at.z.checked_sub(takeoff.z)?;
    let dir = if dx > 0 {
        if dz.abs() > dx.abs() {
            if dz > 0 {
                DoorDir::N
            } else {
                DoorDir::S
            }
        } else {
            DoorDir::E
        }
    } else if dz.abs() > dx.abs() {
        if dz > 0 {
            DoorDir::N
        } else {
            DoorDir::S
        }
    } else {
        DoorDir::W
    };
    let (x, z) = dir_delta(dir);
    Some(WorldTile {
        x: takeoff.x.checked_add(x.checked_mul(2)?)?,
        z: takeoff.z.checked_add(z.checked_mul(2)?)?,
        level: takeoff.level,
    })
}

fn legends_quest(content_root: &Path, quests: Option<&QuestSource<'_>>) -> Option<LegendsQuest> {
    let quests = quests?;
    let started = unique_constant(
        content_root,
        "quests/quest_legends/configs/quest_legends.constant",
        "legends_not_started",
    )?;
    let complete = unique_constant(
        content_root,
        "general/configs/quest.constant",
        "legends_complete",
    )?;
    let varp_id = pinned_pack_id(content_root, "varp.pack", "legendsquest", 139)?;
    if started != 0 || complete != 75 {
        return None;
    }
    let matching = quests
        .facts
        .rows
        .iter()
        .filter(|row| row.id == "legends" || row.varp == "legendsquest" || row.varp_id == varp_id)
        .collect::<Vec<_>>();
    let [row] = matching.as_slice() else {
        return None;
    };
    if row.id != "legends"
        || row.varp != "legendsquest"
        || row.varp_id != varp_id
        || row.complete != complete
        || !row.unknown_sides.is_empty()
    {
        return None;
    }
    let started_min = started.checked_add(1)?;
    Some(LegendsQuest {
        family: quests.family,
        quest: FactKey::new(&row.id),
        signal: FactKey::new(&row.varp),
        started_min,
    })
}

fn pinned_pack_id(content_root: &Path, table: &str, name: &str, expected_id: i32) -> Option<i32> {
    let id = unique_pack_id(content_root, table, name)?;
    (id == expected_id).then_some(id)
}

fn unique_pack_id(content_root: &Path, table: &str, name: &str) -> Option<i32> {
    let text = fs::read_to_string(content_root.join("pack").join(table)).ok()?;
    let mut ids = Vec::new();
    let mut counts = HashMap::<i32, usize>::new();
    for raw in text.lines() {
        let line = raw.split("//").next()?.trim();
        let Some((id, value)) = line.split_once('=') else {
            continue;
        };
        let Ok(id) = id.trim().parse::<i32>() else {
            continue;
        };
        *counts.entry(id).or_default() += 1;
        if value.trim() == name {
            ids.push(id);
        }
    }
    let [id] = ids.as_slice() else {
        return None;
    };
    (counts.get(id) == Some(&1)).then_some(*id)
}

fn unique_constant(content_root: &Path, relative: &str, name: &str) -> Option<i32> {
    let text = fs::read_to_string(content_root.join("scripts").join(relative)).ok()?;
    let mut values = Vec::new();
    for raw in text.lines() {
        let line = raw.split("//").next()?.trim();
        let Some(constant) = line.strip_prefix('^') else {
            continue;
        };
        let Some((key, value)) = constant.split_once('=') else {
            continue;
        };
        if key.trim() == name {
            values.push(value.trim().parse::<i32>().ok()?);
        }
    }
    let [value] = values.as_slice() else {
        return None;
    };
    Some(*value)
}

fn unique_loc_config(content_root: &Path, relative: &str, name: &str) -> Option<LocConfig> {
    let text = fs::read_to_string(content_root.join("scripts").join(relative)).ok()?;
    let mut count = 0;
    let mut current = false;
    let mut builder = LocConfigBuilder::default();
    for raw in text.lines() {
        let line = raw.split("//").next()?.trim();
        if let Some(config_name) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            if current {
                count += 1;
            }
            current = config_name == name;
            if current {
                builder = LocConfigBuilder::default();
            }
            continue;
        }
        if !current {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        match key {
            "op1" => builder.op1.push(value.to_string()),
            "category" => builder.category.push(value.to_string()),
            "active" => builder.active.push(value.to_string()),
            "blockwalk" => builder.blockwalk.push(value.to_string()),
            "param" => {
                if let Some((param, value)) = value.split_once(',') {
                    if param.trim() == "next_loc_stage" {
                        builder.next_stage.push(value.trim().to_string());
                    }
                } else if value.starts_with("next_loc_stage") {
                    builder.next_stage.push(String::new());
                }
            }
            _ => {}
        }
    }
    if current {
        count += 1;
    }
    if count != 1 {
        return None;
    }
    Some(LocConfig {
        op1: one_config_value(builder.op1)?,
        category: one_config_value(builder.category)?,
        active: optional_config_value(builder.active)?,
        blockwalk: optional_config_value(builder.blockwalk)?,
        next_stage: one_config_value(builder.next_stage)?,
    })
}

fn one_config_value(values: Vec<String>) -> Option<String> {
    let [value] = values.as_slice() else {
        return None;
    };
    Some(value.clone())
}

fn optional_config_value(values: Vec<String>) -> Option<Option<String>> {
    match values.as_slice() {
        [] => Some(None),
        [value] => Some(Some(value.clone())),
        _ => None,
    }
}

fn source_matches_file(content_root: &Path, relative: &str, template: &str) -> bool {
    let Ok(actual) = fs::read_to_string(content_root.join("scripts").join(relative)) else {
        return false;
    };
    source_matches(&actual, template)
}

fn source_matches(actual: &str, template: &str) -> bool {
    let Some(expected) = source_blocks(template, None) else {
        return false;
    };
    let wanted = expected.keys().cloned().collect::<HashSet<_>>();
    let Some(actual) = source_blocks(actual, Some(&wanted)) else {
        return false;
    };
    expected.iter().all(|(key, blocks)| {
        blocks.len() == 1 && actual.get(key).is_some_and(|found| found == blocks)
    })
}

fn source_blocks(text: &str, wanted: Option<&HashSet<(String, String)>>) -> Option<SourceBlocks> {
    let headers = text
        .lines()
        .filter_map(|line| {
            let header = super::script_text::parse_script_header(
                line,
                super::script_text::ScriptHeaderStyle::Secondary,
            )?;
            Some((
                header.kind.to_string(),
                header.name.to_string(),
                super::script_text::normalized_body(header.tail),
            ))
        })
        .collect::<Vec<_>>();
    let blocks = stage_doors::header_blocks(text);
    if headers.len() != blocks.len() {
        return None;
    }
    let mut out: SourceBlocks = HashMap::new();
    for ((header_kind, header_name, header_tail), (kind, name, params, body)) in
        headers.into_iter().zip(blocks)
    {
        if header_kind != kind || header_name != name {
            return None;
        }
        let key = (kind, name);
        if wanted.is_some_and(|wanted| !wanted.contains(&key)) {
            continue;
        }
        let body = super::rs2_syntax::parse_body(&body)?;
        out.entry(key).or_default().push(SourceBlock {
            header_tail,
            params,
            body,
        });
    }
    Some(out)
}

fn pinned_engine_proc_closure_matches(content_root: &Path, roots: &[&str]) -> bool {
    let Some(template) = source_blocks(stage_doors::ENGINE_DOOR_PROCS, None) else {
        return false;
    };
    let mut closure = HashSet::new();
    let mut pending = roots
        .iter()
        .map(|name| (*name).to_string())
        .collect::<Vec<_>>();
    while let Some(name) = pending.pop() {
        if !closure.insert(name.clone()) {
            continue;
        }
        let key = ("proc".to_string(), name);
        let Some([block]) = template.get(&key).map(Vec::as_slice) else {
            return false;
        };
        let mut calls = Vec::new();
        for statement in &block.body {
            statement.calls(&mut calls);
        }
        pending.extend(
            calls
                .iter()
                .filter_map(|call| call.strip_prefix('~').map(str::to_string)),
        );
    }
    let wanted = closure
        .into_iter()
        .map(|name| ("proc".to_string(), name))
        .collect::<HashSet<_>>();
    let Some(expected) = source_blocks(stage_doors::ENGINE_DOOR_PROCS, Some(&wanted)) else {
        return false;
    };
    let scripts = content_root.join("scripts");
    let mut actual = SourceBlocks::new();
    let mut failed = false;
    if !visit_rs2(&scripts, &mut |text| {
        let Some(blocks) = source_blocks(text, Some(&wanted)) else {
            failed = true;
            return;
        };
        for (key, mut definitions) in blocks {
            actual.entry(key).or_default().append(&mut definitions);
        }
    }) {
        return false;
    }
    !failed && actual == expected
}

fn script_constants(content_root: &Path) -> HashMap<String, i32> {
    super::script_constants(content_root)
}
