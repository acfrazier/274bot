use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::wilderness::{
    TEST_LEVELS_CONSTANT, TEST_LEVELS_RS2, TEST_SPELL_TELEPORT_RS2, TEST_ZONES_DBROW,
};
use super::*;
use crate::collision::{bake_from_maps, WorldCollision};
use client::config::{Cache, LocType};
use client::io::JagFile;

const JOURNAL_GREEN_SOURCE: &str = "\
[proc,send_quest_progress_colour](component $component, int $progress, int $complete_progress)
if ($progress = 0) {
    if_setcolour($component, ^red_rgb);
} else if ($progress >= $complete_progress) {
    if_setcolour($component, ^green_rgb);
} else {
    if_setcolour($component, ^yellow_rgb);
}
";

/// The derived graph carries the Rune Mysteries essence-mine
/// entries — one `TransportKind::Npc` edge per wizard who knows the
/// teleport (Aubury, Sedridor, Distentor, Cromperty, Brimstail), each
/// carrying the Rune Mysteries quest name and landing on the mine pad
/// (m45_75, the walkable centre anchor of the enclosed mine; the real
/// landing is randomised among the `essence_mine_teleports` enum
/// coords, so the executor accepts any landing in the mine). The gate
/// is the script's `%runemysteries >= ^runemysteries_complete` — the
/// `teleport_to_essence_mine` proc refuses below it.
#[test]
fn derive_transports_emits_essence_mine_entries() {
    let graph = derive_static_routes();
    let ess: Vec<_> = graph
        .edges
        .iter()
        .filter(|e| {
            e.kind == TransportKind::Npc
                && e.quest_req.iter().any(|q| {
                    q.to_ascii_lowercase().contains("rune mysteries") || q == "runemysteries"
                })
        })
        .cloned()
        .collect();
    assert!(ess.len() >= 4, "Aubury+Sedridor+…, got {}", ess.len());
    // Each edge is the wizard NPC placement -> the enclosed mine pad.
    for e in &ess {
        assert_eq!(
            e.to,
            WorldTile {
                x: 2912,
                z: 4833,
                level: 0
            },
            "every wizard lands on the mine pad: {e:?}"
        );
        assert!(
            e.quest_req
                .iter()
                .any(|q| q.to_ascii_lowercase().contains("rune mysteries")),
            "Rune Mysteries on the entry: {e:?}"
        );
    }
    // The five known wizards pin their mined placement tiles.
    let wizards = [
        (
            553,
            WorldTile {
                x: 3253,
                z: 3402,
                level: 0,
            },
        ), // aubury (Varrock)
        (
            300,
            WorldTile {
                x: 3103,
                z: 9571,
                level: 0,
            },
        ), // head_wizard (tower cellar)
        (
            462,
            WorldTile {
                x: 2594,
                z: 3089,
                level: 0,
            },
        ), // guild_wizard (Yanille)
        (
            844,
            WorldTile {
                x: 2683,
                z: 3326,
                level: 0,
            },
        ), // ardounge_wizard (Cromperty)
        (
            171,
            WorldTile {
                x: 2390,
                z: 9810,
                level: 0,
            },
        ), // gnome_brimstail
    ];
    for (npc, at) in wizards {
        assert!(
            ess.iter().any(|e| e.loc_id == npc && e.at == at),
            "no entry edge from {at:?} (npc {npc})"
        );
    }
}

/// The derived graph carries Elkoy's two Tree Gnome Village maze
/// escorts (`elkoy_edges`): the maze-side Elkoy (npc 473) escorts into
/// the village (`p_telejump(^elkoy_maze_coord)` → (2515,3159)) and the
/// village Elkoy (npc 474) escorts out (`p_telejump(^elkoy_entrance_coord)`
/// → (2504,3192)), each `Talk-to` op 1 carrying the Tree Gnome Village
/// quest name.
#[test]
fn derive_transports_emits_elkoy_escort_both_ways() {
    let graph = derive_static_routes();
    let elk: Vec<_> = graph
        .edges
        .iter()
        .filter(|e| {
            e.kind == TransportKind::Npc
                && ((e.to.x == 2504 && e.to.z == 3192) || (e.to.x == 2515 && e.to.z == 3159))
        })
        .cloned()
        .collect();
    assert_eq!(
        elk.len(),
        2,
        "maze-side + village escort, got {}",
        elk.len()
    );
    // The maze-side Elkoy (npc 473) sits at the maze entrance
    // (m39_49 local (8,55) = (2504,3191)) and escorts into the village;
    // the village Elkoy (npc 474, local (18,23) = (2514,3159)) escorts
    // back out to the entrance. Both hops land on the script's own
    // `p_telejump` coords (the quest_tree.constant values), never a
    // snap.
    for e in &elk {
        assert_eq!(e.option, 1, "Talk-to: {e:?}");
        assert!(
            e.quest_req.iter().any(|q| q == "Tree Gnome Village"),
            "Tree Gnome Village on the escort: {e:?}"
        );
    }
    let into_maze = elk
        .iter()
        .find(|e| {
            e.at == WorldTile {
                x: 2504,
                z: 3191,
                level: 0,
            }
        })
        .expect("maze-side Elkoy placement");
    assert_eq!(into_maze.loc_id, 473);
    assert_eq!(
        into_maze.to,
        WorldTile {
            x: 2515,
            z: 3159,
            level: 0
        }
    );
    let out_maze = elk
        .iter()
        .find(|e| {
            e.at == WorldTile {
                x: 2514,
                z: 3159,
                level: 0,
            }
        })
        .expect("village Elkoy placement");
    assert_eq!(out_maze.loc_id, 474);
    assert_eq!(
        out_maze.to,
        WorldTile {
            x: 2504,
            z: 3192,
            level: 0
        }
    );
}

/// The Zanaris shed door (`quest_zanaris.rs2:89-100`
/// `[oploc1,zanarisdoor]`): its Open channel teleports through to Zanaris
/// (`0_50_149_20_56` = (3220,9592)) when the Dramen staff is worn on a
/// members world, so the edge carries both the staff's obj id as `worn_req`
/// and `members_req`, plus the Lost City quest name, and names the `loc_1532`
/// leaf it swings.
#[test]
fn derive_transports_emits_zanaris_shed_door_with_worn_dramen() {
    let fx = Fixture::new();
    fx.write("pack/loc.pack", "2406=zanarisdoor\n1532=loc_1532\n");
    fx.write("pack/obj.pack", "772=dramen_staff\n");
    fx.write(
        "scripts/quests/quest_zanaris/scripts/quest_zanaris.rs2",
        "\
[oploc1,zanarisdoor]
def_boolean $entering = ~check_axis(coord, loc_coord, loc_angle);
~open_and_close_door2(loc_1532, $entering, door_open);
if($entering = false) {
    if(inv_total(worn, dramen_staff) > 0 & map_members = ^true) {
        mes(\"The world starts to shimmer...\");
        if(%zanaris = ^zanaris_staff_made) {
            queue(zanaris_quest_complete, 0, 0);
        }
        p_delay(1);
        ~player_teleport_normal(0_50_149_20_56);
    }
}
",
    );
    fx.write(
        "maps/m50_49.jm2",
        "==== MAP ====\n0 20 56: h1 u50\n==== LOC ====\n0 20 56: 2406 0 0\n",
    );
    let defs = loc_defs(&[(2406, 1, 1)]);
    let wc = bake_collision(&fx, &defs, &HashSet::from([2406]));
    let graph = derive_transports(fx.path(), &defs, &wc);
    let shed: Vec<_> = graph.edges.iter().filter(|e| e.loc_id == 2406).collect();
    assert_eq!(shed.len(), 1, "{shed:?}");
    let e = shed[0];
    assert_eq!(
        e.at,
        WorldTile {
            x: 3220,
            z: 3192,
            level: 0
        }
    );
    assert_eq!(
        e.to,
        WorldTile {
            x: 3220,
            z: 9592,
            level: 0
        },
        "Zanaris landing, not Lumbridge swamp"
    );
    assert_eq!(e.worn_req, [772]);
    assert!(e.members_req, "Zanaris requires a members world");
    assert_eq!(e.quest_req, ["Lost City"]);
    assert_eq!(e.open_loc_id, Some(1532));
    assert_eq!(e.ticks, ZANARIS_DOOR_TICKS);
    let mut facts = crate::world_state::WorldState::empty();
    facts.worn.insert(772);
    facts.quests.insert("Lost City".into());
    assert!(
        !facts.allows(e),
        "the source's map_members check must block F2P even with staff and quest"
    );
    assert!(
        facts.with_map_members(true).allows(e),
        "the same Zanaris edge is usable on a members world"
    );
}
#[test]
fn entrana_boat_members_guard_controls_world_behavior() {
    let fx = Fixture::new();
    fx.write("pack/npc.pack", "657=shipmonk\n658=shipmonk2\n");
    fx.write(
        "scripts/areas/area_port_sarim/scripts/monk_of_entrana.rs2",
        "\
[opnpc1,shipmonk]
if(map_members = ^false) {
    mes(\"You need to be on a members' world to access this content.\");
    return;
}
",
    );
    // The return monk's handler exists and has no guard.
    fx.write(
        "scripts/areas/area_entrana/scripts/monk_of_entrana.rs2",
        "\
[opnpc1,shipmonk2]
~chatnpc(\"<p,neutral>Do you wish to leave holy Entrana?\");
",
    );
    let graph = derive_static_routes_for(&fx);
    let outbound = graph
        .edges
        .iter()
        .find(|edge| edge.kind == TransportKind::Boat && edge.loc_id == 657)
        .expect("Port Sarim -> Entrana boat");
    assert!(outbound.members_req, "shipmonk is members-gated");
    assert!(
        !crate::world_state::WorldState::empty().allows(outbound),
        "F2P cannot take the Entrana boat"
    );
    assert!(
        crate::world_state::WorldState::empty()
            .with_map_members(true)
            .allows(outbound),
        "members worlds can take the Entrana boat"
    );

    let return_boat = graph
        .edges
        .iter()
        .find(|edge| edge.kind == TransportKind::Boat && edge.loc_id == 658)
        .expect("Entrana -> Port Sarim boat");
    assert!(
        !return_boat.members_req,
        "the return script has no map_members guard"
    );
}

#[test]
fn castlecrumbly_members_guard_controls_world_behavior() {
    let fx = Fixture::new();
    fx.write("pack/loc.pack", "1947=castlecrumbly\n");
    fx.write(
        "scripts/skill_agility/scripts/shortcuts.rs2",
        "\
[oploc1,castlecrumbly]
if (map_members = ^false) {
    mes(^mes_members_feature);
    return;
}
if(stat(agility) < 5) {
    return;
}
",
    );
    fx.write(
        "maps/m45_52.jm2",
        "\
==== MAP ====
0 55 27: h1 u50
==== LOC ====
0 55 27: 1947 0 0
",
    );
    let defs = loc_defs(&[(1947, 1, 1)]);
    let wc = bake_collision(&fx, &defs, &HashSet::new());
    let graph = derive_transports(fx.path(), &defs, &wc);
    let edge = graph
        .edges
        .iter()
        .find(|edge| edge.kind == TransportKind::AgilityShortcut && edge.loc_id == 1947)
        .expect("Falador castlecrumbly shortcut");
    assert!(edge.members_req, "castlecrumbly is members-gated");
    assert!(
        !crate::world_state::WorldState::empty().allows(edge),
        "F2P cannot climb castlecrumbly"
    );
    let mut members = crate::world_state::WorldState::empty().with_map_members(true);
    members.stats.insert(SKILL_AGILITY, 5);
    assert!(
        members.allows(edge),
        "members worlds with Agility 5 can climb castlecrumbly"
    );
}

/// The Duel Arena gates (`loc_3197`/`loc_3198`) are plain `gates.loc`
/// fence gates whose own `[oploc1,…]` opens with the F2P refusal before
/// `~open_gate` (`game_duelarena/scripts/misc_locs.rs2`): both crossings
/// need a members world, like any other gate in that config would.
#[test]
fn gates_loc_gate_behind_members_refusal_requires_a_members_world() {
    let fx = Fixture::new();
    fx.write("pack/loc.pack", "3197=loc_3197\n");
    fx.write(
        "scripts/general_use/configs/gates.loc",
        "[loc_3197]\nname=Gate\nop1=Open\nactive=yes\ncategory=gate_main_closed\n",
    );
    fx.write(
        "scripts/minigames/game_duelarena/scripts/misc_locs.rs2",
        "\
[oploc1,loc_3197]
if (map_members = ^false) {
    mes(^mes_members_gate);
    return;
}
~open_gate;
",
    );
    fx.write(
        "maps/m44_53.jm2",
        "==== MAP ====\n0 0 0: h1\n==== LOC ====\n0 5 46: 3197 0 0\n",
    );
    let defs = loc_defs(&[(3197, 1, 1)]);
    let wc = bake_collision(&fx, &defs, &HashSet::from([3197]));
    let graph = derive_transports(fx.path(), &defs, &wc);
    let gate: Vec<_> = graph.edges.iter().filter(|e| e.loc_id == 3197).collect();
    assert_eq!(gate.len(), 2, "{gate:?}");
    let f2p = crate::world_state::WorldState::empty();
    for e in gate {
        assert!(e.members_req, "{e:?}");
        assert!(!f2p.allows(e), "F2P cannot open the Duel Arena gate");
        assert!(f2p.clone().with_map_members(true).allows(e));
    }
}

/// The Shantay henge doorway's `[oploc1,shantay_pass_henge_doorway]`
/// refuses F2P worlds before either branch (`shantay_pass.rs2`), so the
/// paid entry and the free exit both need a members world.
#[test]
fn shantay_henge_doorway_requires_a_members_world_both_ways() {
    let fx = Fixture::new();
    fx.write("pack/obj.pack", "995=coins\n1854=shantay_pass\n");
    fx.write(
        "pack/loc.pack",
        "4031=shantay_pass_henge_doorway\n2882=border_gate_toll_left\n1562=loc_1562\n",
    );
    fx.write(
        "scripts/areas/area_alkharid/configs/border_gate.loc",
        "[border_gate_toll_left]\nname=Gate\nop1=Open\ncategory=border_gate_toll_left\nparam=next_loc_stage,loc_1562\n",
    );
    fx.write(
        "scripts/areas/area_alkharid/scripts/shantay_pass.rs2",
        "\
[oploc1,shantay_pass_henge_doorway]
if (map_members = ^false) {
    mes(^mes_members_gate); // guess
    return;
}
if (coordz(coord) <= coordz(loc_coord)) {
    p_telejump(movecoord(coord, 0, 0, 3));
    return;
}
queue(shantay_pass_enter, 0, 0);
",
    );
    fx.write(
        "maps/m51_48.jm2",
        "==== MAP ====\n0 38 44: h1 u50\n==== LOC ====\n0 38 44: 4031 10 0\n",
    );
    let defs = loc_defs(&[(4031, 1, 1)]);
    let wc = bake_collision(&fx, &defs, &HashSet::from([4031]));
    let graph = derive_transports(fx.path(), &defs, &wc);
    let henge: Vec<_> = graph.edges.iter().filter(|e| e.loc_id == 4031).collect();
    assert_eq!(henge.len(), 2, "{henge:?}");
    let mut f2p = crate::world_state::WorldState::empty();
    f2p.inv.insert(1854, 1);
    for e in henge {
        assert!(e.members_req, "{e:?}");
        assert!(!f2p.allows(e), "F2P cannot pass the Shantay doorway");
        assert!(f2p.clone().with_map_members(true).allows(e));
    }
}

fn members_check_edge(kind: TransportKind, loc_id: i32, option: i32) -> TransportEdge {
    TransportEdge {
        kind,
        player_delta: None,
        at: WorldTile {
            x: 3200,
            z: 3200,
            level: 0,
        },
        to: WorldTile {
            x: 3201,
            z: 3200,
            level: 0,
        },
        loc_id,
        option,
        ticks: 1,
        dir: None,
        open_loc_id: None,
        skill_req: vec![],
        item_req: vec![],
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![],
        members_req: false,
        wildy_cap: None,
        quest_gates: None,
    }
}

/// The bake's two-way check: a packed edge whose source handler reads
/// `map_members` in a form the shared pass does not set (here a positive
/// `map_members = ^true` arm reached through the loc's category and a
/// label) fails while free, and still fails when marked members-only,
/// because that arm is not a pinned producer gate.
#[test]
fn members_check_fails_an_edge_whose_source_reads_map_members() {
    let fx = Fixture::new();
    fx.write("pack/loc.pack", "100=bridge\n");
    fx.write(
        "scripts/areas/configs/bridge.loc",
        "[bridge]\nname=Bridge\nop1=Cross\ncategory=rope_bridge\n",
    );
    fx.write(
        "scripts/areas/scripts/bridge.rs2",
        "\
[oploc1,_rope_bridge] @cross_bridge;

[label,cross_bridge]
if (%bridge_quest >= 3 & map_members = ^true) {
    p_telejump(movecoord(coord, 1, 0, 0));
}
",
    );
    let mut graph = TransportGraph::default();
    graph
        .edges
        .push(members_check_edge(TransportKind::Door, 100, 1));
    let err = require_members_guards(fx.path(), &graph).expect_err("ungated read");
    assert!(err.contains("[label,cross_bridge]"), "{err}");
    graph.edges[0].members_req = true;
    let err = require_members_guards(fx.path(), &graph).expect_err("unpinned members arm");
    assert!(
        err.contains("leading path of [oploc1,_rope_bridge] does not refuse F2P"),
        "{err}"
    );
}

/// The shared pass gates an edge whose source opens with the F2P refusal,
/// found through the NPC's category; a pinned non-gate read passes only
/// while its statement is unchanged; an edge with no handler, and a
/// members spell packed free, fail the check.
#[test]
fn members_guards_follow_the_handler_and_pin_non_gate_reads() {
    let fx = Fixture::new();
    fx.write("pack/npc.pack", "378=seaman_thresnor\n500=ferryman\n");
    fx.write(
        "scripts/areas/configs/sailors.npc",
        "[seaman_thresnor]\nname=Seaman\ncategory=sailor\n\n[ferryman]\nname=Ferryman\ncategory=ferry\n",
    );
    let sailor = "\
[opnpc1,_sailor]
if(map_members = ^true & npc_type = captain_tobias) {
    return;
}
@karamja_sailor_dialogue;

[opnpc1,_ferry]
if (map_members = ^false | %ferry < 2) { mes(\"No.\"); return; }
~set_sail;
";
    fx.write("scripts/areas/scripts/sailors.rs2", sailor);
    let guards = MembersGuards::from_content(fx.path());
    let mut graph = TransportGraph::default();
    graph.edges.extend([
        members_check_edge(TransportKind::Boat, 378, 1),
        members_check_edge(TransportKind::Boat, 500, 1),
    ]);
    apply_members_guards(&guards, &mut graph);
    assert!(!graph.edges[0].members_req, "the clue branch is no gate");
    assert!(graph.edges[1].members_req, "the ferry refuses F2P first");
    require_members_guards(fx.path(), &graph).expect("both edges match their source");

    // A different read in the pinned handler is no longer the exemption.
    fx.write(
        "scripts/areas/scripts/sailors.rs2",
        &sailor.replace("captain_tobias", "seaman_thresnor"),
    );
    let err = require_members_guards(fx.path(), &graph).expect_err("changed read");
    assert!(err.contains("[opnpc1,_sailor]"), "{err}");
    fx.write("scripts/areas/scripts/sailors.rs2", sailor);

    graph
        .edges
        .push(members_check_edge(TransportKind::Npc, 501, 1));
    let err = require_members_guards(fx.path(), &graph).expect_err("no handler");
    assert!(err.contains("has no source handler"), "{err}");
    graph.edges.pop();

    fx.write(
        "scripts/skill_magic/configs/magic_spells.dbrow",
        "[magic_spell_teleport_camelot]\ndata=members,true\ndata=tele_coord,0_43_54_5_62\n",
    );
    let mut camelot = members_check_edge(TransportKind::Teleport, 0, 0);
    camelot.to = WorldTile {
        x: 2757,
        z: 3518,
        level: 0,
    };
    graph.teleports.push(camelot);
    let err = require_members_guards(fx.path(), &graph).expect_err("free members spell");
    assert!(err.contains("members spell teleport"), "{err}");
}

/// The engine resolves an op through the type, then the category, then
/// the global `[<op>,_]` handler (`ScriptProvider.getByTrigger`). An edge
/// served only by the global handler reads it; a type or category handler
/// still wins over it; an op with no handler names every key tried.
#[test]
fn members_guards_fall_back_to_the_global_handler() {
    let fx = Fixture::new();
    fx.write(
        "pack/loc.pack",
        "100=plain_gate\n101=own_gate\n102=cat_gate\n",
    );
    fx.write(
        "scripts/areas/configs/gates.loc",
        "[plain_gate]\nname=Gate\n\n[own_gate]\nname=Gate\n\n[cat_gate]\nname=Gate\ncategory=free_gate\n",
    );
    fx.write(
        "scripts/areas/scripts/gates.rs2",
        "\
[oploc1,_]
if (map_members = ^false) {
    mes(^mes_members_gate);
    return;
}
~open_gate;

[oploc1,own_gate]
~open_gate;

[oploc1,_free_gate]
~open_gate;
",
    );
    let guards = MembersGuards::from_content(fx.path());
    let mut graph = TransportGraph::default();
    graph.edges.extend([
        members_check_edge(TransportKind::Door, 100, 1),
        members_check_edge(TransportKind::Door, 101, 1),
        members_check_edge(TransportKind::Door, 102, 1),
    ]);
    apply_members_guards(&guards, &mut graph);
    let gated: Vec<bool> = graph.edges.iter().map(|e| e.members_req).collect();
    assert_eq!(
        gated,
        [true, false, false],
        "global handler gates; type and category handlers win over it"
    );
    require_members_guards(fx.path(), &graph).expect("every edge matches its engine handler");

    graph
        .edges
        .push(members_check_edge(TransportKind::Door, 100, 2));
    let err = require_members_guards(fx.path(), &graph).expect_err("no op2 handler");
    assert!(
        err.contains(
            "has no source handler (tried [oploc2,plain_gate], [oploc2,loc_100], [oploc2,_])"
        ),
        "{err}"
    );
}

/// Only the leading path gates: the handler's first statement, through a
/// chain of unconditional leading `@label` jumps. A refusal behind a
/// conditional label, a `@multiN` option or a queue does not stop the
/// free choice beside it, so the edge stays free and the bake fails until
/// the read is reviewed; marking that edge members-only fails the bake too.
#[test]
fn members_guards_gate_only_the_leading_path() {
    let fx = Fixture::new();
    fx.write("pack/npc.pack", "500=ferryman\n501=keeper\n");
    fx.write(
        "scripts/areas/scripts/ferry.rs2",
        "\
[opnpc1,ferryman]
~chatnpc(\"<p,neutral>Where would you like to go?\");
if (%ferry_lore = 1) {
    @ferry_lore;
}
weakqueue(ferry_tale, 0);
@multi2(\"Tell me about the island.\", ferry_lore, \"Sail, please.\", ferry_sail);

[label,ferry_lore]
if (map_members = ^false) {
    mes(\"Only members may hear this tale.\");
    return;
}
~chatnpc(\"<p,neutral>The island is old.\");

[queue,ferry_tale]
if (map_members = ^false) {
    return;
}
~chatnpc(\"<p,neutral>Mind the waves.\");

[label,ferry_sail]
p_telejump(0_50_50_10_10);

[opnpc1,keeper] @keeper_greet;

[label,keeper_greet] @keeper_check(1);

[label,keeper_check](int $n)
if (map_members = ^false | $n = 0) {
    mes(^mes_members_feature);
    return;
}
p_telejump(0_50_50_20_20);
",
    );
    let guards = MembersGuards::from_content(fx.path());
    let mut graph = TransportGraph::default();
    graph.edges.extend([
        members_check_edge(TransportKind::Boat, 500, 1),
        members_check_edge(TransportKind::Boat, 501, 1),
    ]);
    apply_members_guards(&guards, &mut graph);
    assert!(
        !graph.edges[0].members_req,
        "the Sail choice is free on every world"
    );
    assert!(
        graph.edges[1].members_req,
        "the keeper's leading jump chain refuses F2P"
    );

    let err = require_members_guards(fx.path(), &graph).expect_err("optional refusal");
    assert!(err.contains("Boat 500"), "{err}");
    assert!(err.contains("[label,ferry_lore]"), "{err}");
    assert!(err.contains("[queue,ferry_tale]"), "{err}");
    assert!(!err.contains("Boat 501"), "{err}");

    graph.edges[0].members_req = true;
    let err = require_members_guards(fx.path(), &graph).expect_err("over-gate");
    assert!(err.contains("Boat 500 op1"), "{err}");
    assert!(
        err.contains(
            "packs members_req=true but the leading path of [opnpc1,ferryman] does not refuse F2P"
        ),
        "{err}"
    );
    assert!(!err.contains("Boat 501"), "{err}");
}

fn derive_static_routes_for(fx: &Fixture) -> TransportGraph {
    let defs = loc_defs(&[]);
    let wc = bake_collision(fx, &defs, &HashSet::new());
    derive_transports(fx.path(), &defs, &wc)
}

/// The graph derived from an empty content root: only the explicit route
/// tables (boats, carts, essence-mine wizards, Elkoy) that read no content.
fn derive_static_routes() -> TransportGraph {
    let fx = Fixture::new();
    let defs = loc_defs(&[]);
    let wc = bake_collision(&fx, &defs, &HashSet::new());
    derive_transports(fx.path(), &defs, &wc)
}

/// A throwaway content root written on demand, removed on drop.
struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("nav-transport-fixture-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        Fixture { root }
    }

    fn write(&self, rel: &str, text: &str) {
        let path = self.root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn path(&self) -> &Path {
        &self.root
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn loc_defs(entries: &[(i32, i32, i32)]) -> LocDefs {
    let locs: Vec<LocType> = entries
        .iter()
        .map(|&(id, width, length)| LocType {
            id,
            width,
            length,
            ..Default::default()
        })
        .collect();
    LocDefs::from_locs(&locs)
}

/// A collision bake over the fixture's maps, with the given door locs
/// stamped blocked-when-closed. Fixtures that write no maps get a
/// trivial single-square bake (their assertions never touch the
/// collision; `derive_transports` only needs one to walk door far
/// sides out on).
fn bake_collision(fx: &Fixture, defs: &LocDefs, door_ids: &HashSet<i32>) -> WorldCollision {
    if !fx.path().join("maps").is_dir() {
        fx.write("maps/m44_53.jm2", "==== MAP ====\n0 0 0: h1 u50\n");
    }
    bake_from_maps(&fx.path().join("maps"), defs, door_ids).unwrap()
}

/// `(at, dir, to)` of every door edge of `loc_id`, sorted, so a
/// directional door's exact crossings can be asserted.
type DoorCrossing = ((i32, i32), char, (i32, i32));
fn door_crossings(graph: &TransportGraph, loc_id: i32) -> Vec<DoorCrossing> {
    let mut out: Vec<_> = graph
        .edges
        .iter()
        .filter(|e| e.kind == TransportKind::Door && e.loc_id == loc_id)
        .map(|e| {
            let dir = match e.dir {
                Some(DoorDir::N) => 'N',
                Some(DoorDir::E) => 'E',
                Some(DoorDir::S) => 'S',
                Some(DoorDir::W) => 'W',
                None => panic!("door edge without a dir: {e:?}"),
            };
            ((e.at.x, e.at.z), dir, (e.to.x, e.to.z))
        })
        .collect();
    out.sort();
    out
}

fn write_brass_key_door_source(fx: &Fixture, handler: &str) {
    fx.write("pack/loc.pack", "1535=loc_1535\n1804=brasskeydoor\n");
    fx.write("pack/obj.pack", "983=edgevilledungeonkey\n");
    fx.write(
        "scripts/_unpack/225/all.loc",
        "\
[loc_1535]
name=Door

[brasskeydoor]
name=Door
desc=This door requires a key.
model=basic_wall
active=yes
op1=Open
param=next_loc_stage,loc_1535
",
    );
    fx.write(
        "scripts/areas/area_edgeville/scripts/edgeville_dungeon.rs2",
        handler,
    );
    write_blocked_square(
        fx,
        48,
        53,
        &[(3115, 3448), (3115, 3449), (3115, 3450), (3115, 3451)],
        "0 43 58: 1804 0 3\n",
    );
}

fn canonical_brass_key_door_handler() -> &'static str {
    "\
[oploc1,brasskeydoor]
mes(\"The door is locked.\");

[oplocu,brasskeydoor]
switch_obj(last_useitem) {
    case edgevilledungeonkey : @open_edgeville_dungeon_door;
    case default : ~displaymessage(^dm_default);
}

[label,open_edgeville_dungeon_door]
if (inv_total(inv, edgevilledungeonkey) > 0) {
    mes(\"You unlock the door.\");
    sound_synth(locked, 1, 0);
    def_coord $loc_coord = loc_coord;
    def_int $angle = loc_angle;
    def_locshape $shape = loc_shape;
    def_loc $replacement = loc_param(next_loc_stage);
    def_int $x;
    def_int $z;
    $x, $z = ~door_open($angle, loc_shape);
    def_boolean $entering = ~check_axis(coord, $loc_coord, $angle);
    def_coord $dest = $loc_coord;
    if ($entering = true) {
        if (coord ! $loc_coord) {
            p_delay(0);
            p_teleport($loc_coord);
            sound_synth(door_open, 1, 0);
            p_delay(1);
        } else {
            p_delay(0);
            sound_synth(door_open, 1, 0);
        }
        $dest = movecoord($loc_coord, $x, 0, $z);
    } else {
        p_delay(0);
        sound_synth(door_open, 1, 0);
    }
    p_teleport($dest);
    loc_add($loc_coord, inviswall, $angle, $shape, 3);
    loc_add(movecoord($loc_coord, $x, 0, $z), $replacement, modulo(add($angle, 1), 4), $shape, 3);
} else {
    mes(\"The door is locked.\");
}
"
}

#[test]
fn derive_transports_emits_source_shaped_brass_key_door() {
    let fx = Fixture::new();
    write_brass_key_door_source(&fx, canonical_brass_key_door_handler());
    let defs = loc_defs(&[(1535, 1, 1), (1804, 1, 1)]);
    let wc = bake_collision(&fx, &defs, &HashSet::from([1804]));
    let graph = derive_transports(fx.path(), &defs, &wc);
    let edges: Vec<_> = graph
        .edges
        .iter()
        .filter(|edge| edge.loc_id == 1804)
        .collect();
    assert_eq!(
        edges.len(),
        2,
        "one keyed crossing per direction: {edges:?}"
    );
    assert_eq!(
        door_crossings(&graph, 1804),
        vec![
            ((3115, 3449), 'N', (3115, 3450)),
            ((3115, 3450), 'S', (3115, 3449)),
        ],
        "reverse starts at the temporary replacement leaf; it is not a generic adjacent-door edge"
    );
    for edge in edges {
        assert_eq!(edge.option, 0, "oplocu, never the locked oploc1");
        assert_eq!(edge.item_req, vec![(983, 1)]);
        assert_eq!(edge.open_loc_id, Some(1535));
        assert!(edge.skill_req.is_empty());
        assert!(edge.quest_req.is_empty());
        assert!(edge.varp_req.is_empty());
        assert!(edge.worn_req.is_empty());
        assert!(!edge.members_req);
        assert!(wc.standable(edge.at), "standable take-off: {edge:?}");
        assert!(wc.standable(edge.to), "standable landing: {edge:?}");
    }
}

#[test]
fn brass_key_door_routes_both_ways_only_with_held_key() {
    use crate::router::{find_with, FindOptions, Leg, RouteError};

    let fx = Fixture::new();
    write_brass_key_door_source(&fx, canonical_brass_key_door_handler());
    let defs = loc_defs(&[(1535, 1, 1), (1804, 1, 1)]);
    let wc = bake_collision(&fx, &defs, &HashSet::from([1804]));
    let graph = derive_transports(fx.path(), &defs, &wc);
    let hut = WorldTile {
        x: 3115,
        z: 3451,
        level: 0,
    };
    let outside = WorldTile {
        x: 3115,
        z: 3448,
        level: 0,
    };
    let empty = crate::world_state::WorldState::empty();
    for (from, to) in [(hut, outside), (outside, hut)] {
        assert!(
            matches!(
                find_with(&wc, &graph, from, to, FindOptions::default(), &empty),
                Err(RouteError::NoPath)
            ),
            "the locked Open action must not become a free edge"
        );
    }

    let keyed = crate::world_state::WorldState {
        inv: HashMap::from([(983, 1)]),
        ..crate::world_state::WorldState::empty()
    };
    for (from, to, dir) in [(hut, outside, DoorDir::S), (outside, hut, DoorDir::N)] {
        let route = find_with(&wc, &graph, from, to, FindOptions::default(), &keyed)
            .expect("held brass key permits the short hut crossing");
        let hop = route
            .legs
            .iter()
            .find_map(|leg| match leg {
                Leg::Transport { edge } if edge.loc_id == 1804 => Some(edge),
                _ => None,
            })
            .expect("route uses the keyed brass-hut door");
        assert_eq!(hop.dir, Some(dir));
        assert_eq!(
            hop.option, 0,
            "traveller dispatches existing use-item-on-loc"
        );
        assert_eq!(hop.item_req, vec![(983, 1)]);
    }
    assert_eq!(
        keyed.inv.get(&983),
        Some(&1),
        "routing proves possession; the source never consumes the key"
    );
}

#[test]
fn brass_key_door_fails_closed_when_alias_or_handler_changes() {
    let defs = loc_defs(&[(1535, 1, 1), (1804, 1, 1)]);
    for (label, handler) in [
        (
            "wrong use item",
            canonical_brass_key_door_handler()
                .replace("case edgevilledungeonkey :", "case muddy_key :"),
        ),
        (
            "missing inventory guard",
            canonical_brass_key_door_handler()
                .replace("inv_total(inv, edgevilledungeonkey) > 0", "true"),
        ),
        (
            "consumed key",
            canonical_brass_key_door_handler().replace(
                "mes(\"You unlock the door.\");",
                "inv_del(inv, edgevilledungeonkey, 1);",
            ),
        ),
    ] {
        let fx = Fixture::new();
        write_brass_key_door_source(&fx, &handler);
        let wc = bake_collision(&fx, &defs, &HashSet::from([1804]));
        let graph = derive_transports(fx.path(), &defs, &wc);
        assert!(
            graph.edges.iter().all(|edge| edge.loc_id != 1804),
            "{label} must omit the family"
        );
    }

    let fx = Fixture::new();
    write_brass_key_door_source(&fx, canonical_brass_key_door_handler());
    fx.write("pack/obj.pack", "");
    let wc = bake_collision(&fx, &defs, &HashSet::from([1804]));
    let graph = derive_transports(fx.path(), &defs, &wc);
    assert!(
        graph.edges.iter().all(|edge| edge.loc_id != 1804),
        "missing selected key alias must omit the family"
    );
}

#[test]
#[ignore = "NAV_CONTENT_ROOT and NAV_CACHE required; absence fails"]
fn selected_274_and_289_content_derives_the_keyed_hut_crossing() {
    use crate::router::{find_with, FindOptions, Leg};

    fn assert_selected(label: &str, graph: &TransportGraph, wc: &WorldCollision) {
        let edges: Vec<_> = graph
            .edges
            .iter()
            .filter(|edge| edge.loc_id == 1804)
            .collect();
        assert_eq!(edges.len(), 2, "{label}: {edges:?}");
        assert_eq!(
            door_crossings(graph, 1804),
            vec![
                ((3115, 3449), 'N', (3115, 3450)),
                ((3115, 3450), 'S', (3115, 3449)),
            ],
            "{label}"
        );
        assert!(edges.iter().all(|edge| {
            edge.option == 0
                && edge.item_req == [(983, 1)]
                && edge.open_loc_id == Some(1535)
                && wc.standable(edge.at)
                && wc.standable(edge.to)
        }));
        assert!(
            graph
                .edges
                .iter()
                .all(|edge| edge.loc_id != 1804 || edge.option != 1),
            "{label}: locked Open must not become a free edge"
        );

        let inside = WorldTile {
            x: 3116,
            z: 3450,
            level: 0,
        };
        let outside = WorldTile {
            x: 3115,
            z: 3449,
            level: 0,
        };
        let unkeyed = find_with(
            wc,
            graph,
            inside,
            outside,
            FindOptions::default(),
            &crate::world_state::WorldState::empty(),
        );
        if let Ok(route) = &unkeyed {
            assert!(
                route.legs.iter().all(|leg| !matches!(
                    leg,
                    Leg::Transport { edge } if edge.loc_id == 1804
                )),
                "{label}: no held key must reject the keyed crossing"
            );
        }
        let keyed = crate::world_state::WorldState {
            inv: HashMap::from([(983, 1)]),
            ..crate::world_state::WorldState::empty()
        };
        for (from, to, dir) in [(inside, outside, DoorDir::S), (outside, inside, DoorDir::N)] {
            let route = find_with(wc, graph, from, to, FindOptions::default(), &keyed)
                .unwrap_or_else(|error| panic!("{label}: keyed short route: {error:?}"));
            assert!(
                route.ticks < 10.0,
                "{label}: keyed crossing must beat the long public-dungeon route: {route:?}"
            );
            assert!(route.legs.iter().any(|leg| matches!(
                leg,
                Leg::Transport { edge }
                    if edge.loc_id == 1804
                        && edge.option == 0
                        && edge.dir == Some(dir)
            )));
        }
        assert_eq!(
            keyed.inv.get(&983),
            Some(&1),
            "{label}: key is not consumed"
        );
    }

    for (root, graph, wc) in qualification_worlds() {
        let label = root.display().to_string();
        let ids = loc_ids_by_name(root);
        assert!(brass_key_handler_matches(root), "{label} handler shape");
        assert_eq!(
            brass_key_open_loc_id(root, &ids, ids[BRASS_KEY_DOOR_NAME]),
            Some(1535),
            "{label} replacement leaf"
        );
        assert_selected(&label, graph, wc);
    }
}

#[test]
fn straight_door_reverse_routes_out_when_behind_loc_is_scenery() {
    use crate::router::{find, Leg, RouteError};
    use client::dash3d::CollisionFlag;

    let fx = Fixture::new();
    fx.write("pack/loc.pack", "1530=loc_1530\n1531=loc_1531\n");
    fx.write(
        "scripts/doors/configs/doors.loc",
        "[loc_1530]\nname=Door\nop1=Open\ncategory=door_closed\nparam=next_loc_stage,loc_1531\n",
    );
    fx.write("maps/m44_53.jm2", "==== LOC ====\n0 1 1: 1530 0 2\n");
    let defs = loc_defs(&[(1530, 1, 1)]);
    let at = WorldTile {
        x: 2817,
        z: 3393,
        level: 0,
    };
    let inside = WorldTile { x: 2818, ..at };
    let outside = WorldTile { z: 3394, ..at };
    let mut flags = vec![CollisionFlag::WALK_SCENERY as u32; 3 * 3 * 4];
    flags[4] = CollisionFlag::W_E as u32;
    flags[5] = CollisionFlag::W_W as u32;
    flags[7] = 0;
    let (walk, blocked) = crate::collision::pack_walk(&flags);
    let collision = WorldCollision {
        origin: WorldTile {
            x: 2816,
            z: 3392,
            level: 0,
        },
        width: 3,
        height: 3,
        walk,
        blocked,
        flags: Some(flags),
    };
    assert!(
        matches!(
            find(&collision, &TransportGraph::default(), inside, outside),
            Err(RouteError::NoPath)
        ),
        "closed wall isolates the standable origin"
    );
    let graph = derive_transports(fx.path(), &defs, &collision);
    // Exercise the packed graph, not just the deriver's in-memory endpoints.
    let bytes = crate::pack::encode(&collision, &graph, &[]);
    let (collision, graph, _) = crate::pack::decode(&bytes).unwrap();
    for (from, to) in [(inside, outside), (outside, inside)] {
        let route = find(&collision, &graph, from, to).expect("door routes both ways");
        assert!(route.legs.iter().any(|leg| matches!(
            leg, Leg::Transport { edge } if edge.loc_id == 1530
        )));
    }
}

#[test]
#[ignore = "NAV_CONTENT_ROOT and NAV_CACHE required; absence fails"]
fn ardougne_market_pocket_routes_via_door_without_origin_snap() {
    use crate::router::{find_with, FindOptions, Leg};
    for (_, graph, collision) in qualification_worlds() {
        let from = WorldTile {
            x: 2659,
            z: 3292,
            level: 0,
        };
        assert!(collision.standable(from));
        for to in [
            WorldTile {
                x: 2661,
                z: 3301,
                level: 0,
            },
            WorldTile {
                x: 2724,
                z: 3484,
                level: 0,
            },
        ] {
            let route = find_with(
                collision,
                graph,
                from,
                to,
                FindOptions::default(),
                &crate::WorldState {
                    map_members: true,
                    ..crate::WorldState::empty()
                },
            )
            .expect("market pocket must have a packed exit");
            assert!(
                route.legs.iter().any(|leg| matches!(
                    leg, Leg::Transport { edge }
                        if edge.loc_id == 1530 && edge.to == edge.at
                )),
                "route must leave through the actual door"
            );
        }
    }
}

#[test]
fn door_far_side_does_not_skip_bank_return_obstacles() {
    // Captured at (2651..=2657,3292): counter/plant/bench footprints
    // separate Door1530 at2656 from the old bogus west landing2651.
    let mut flags = vec![0u32; 7 * 4];
    flags[..7].copy_from_slice(&[0x4020, 0x4120, 0x4120, 0x4120, 0x4120, 0x5028, 0x10080]);
    let (walk, blocked) = crate::collision::pack_walk(&flags);
    let collision = WorldCollision {
        origin: WorldTile {
            x: 2651,
            z: 3292,
            level: 0,
        },
        width: 7,
        height: 1,
        walk,
        blocked,
        flags: Some(flags),
    };
    let at = WorldTile {
        x: 2656,
        z: 3292,
        level: 0,
    };
    assert_eq!(door_far_side(at, DoorDir::W, &collision), None);
    assert_eq!(
        door_far_side(at, DoorDir::E, &collision),
        Some(WorldTile {
            x: 2657,
            z: 3292,
            level: 0
        })
    );
    assert_eq!(door_far_side(at, DoorDir::N, &collision), None);
    assert_eq!(door_far_side(at, DoorDir::S, &collision), None);
}

#[test]
fn web_far_side_preserves_multi_tile_footprint_crossing() {
    let mut flags = vec![0u32; 3 * 4];
    flags[1] = 0x100; // Adjacent scenery remains part of the web walk-out.
    let (walk, blocked) = crate::collision::pack_walk(&flags);
    let collision = WorldCollision {
        origin: WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
        width: 3,
        height: 1,
        walk,
        blocked,
        flags: Some(flags),
    };
    let at = WorldTile {
        x: 0,
        z: 0,
        level: 0,
    };
    assert_eq!(
        web_far_side(at, DoorDir::E, &collision),
        Some(WorldTile {
            x: 2,
            z: 0,
            level: 0
        })
    );
    assert_eq!(door_far_side(at, DoorDir::E, &collision), None);
    assert_eq!(web_far_side(at, DoorDir::W, &collision), None);
}

/// A revision's pack is a fixed point of its inputs: two derivations of
/// the same content encode identically. Door ids come out of a hash set,
/// so without a canonical edge order each run packs them differently.
#[test]
fn derive_transports_packs_identically_across_runs() {
    let fx = Fixture::new();
    let ids: Vec<i32> = (1530..1542).collect();
    let mut pack = String::new();
    let mut doors = String::new();
    let mut map = String::from("==== MAP ====\n");
    let mut locs = String::from("==== LOC ====\n");
    for (i, id) in ids.iter().enumerate() {
        let x = 2 * i;
        pack.push_str(&format!("{id}=loc_{id}\n"));
        doors.push_str(&format!(
            "[loc_{id}]\nname=Door\nop1=Open\ncategory=door_closed\n"
        ));
        for z in 45..=47 {
            map.push_str(&format!("0 {x} {z}: h1 o6 u50\n"));
        }
        locs.push_str(&format!("0 {x} 46: {id} 0 1\n"));
    }
    fx.write("pack/loc.pack", &pack);
    fx.write("scripts/doors/configs/doors.loc", &doors);
    fx.write("maps/m44_53.jm2", &(map + &locs));
    let defs = loc_defs(&ids.iter().map(|&id| (id, 1, 1)).collect::<Vec<_>>());
    let door_ids: HashSet<i32> = ids.iter().copied().collect();
    let wc = bake_from_maps(&fx.path().join("maps"), &defs, &door_ids).unwrap();

    let first = derive_transports(fx.path(), &defs, &wc);
    let door_edges = first
        .edges
        .iter()
        .filter(|e| e.kind == TransportKind::Door)
        .count();
    assert_eq!(door_edges, 2 * ids.len());
    let bytes = crate::pack::encode(&wc, &first, &[]);
    for _ in 0..4 {
        let again = derive_transports(fx.path(), &defs, &wc);
        assert!(crate::pack::encode(&wc, &again, &[]) == bytes);
    }
}

/// Fence gates declared in `scripts/general_use/configs/gates.loc` (the
/// Sinclair wooden gates `loc_1551`/`loc_1553`: `op1=Open` with the closed
/// gate categories) join the door set like the door configs, not only
/// `scripts/doors/configs/*.loc`: each placement crosses both ways.
#[test]
fn derive_transports_content_emits_sinclair_gate_edges() {
    let fx = Fixture::new();
    fx.write("pack/loc.pack", "1551=loc_1551\n1553=loc_1553\n");
    fx.write(
        "scripts/general_use/configs/gates.loc",
        "\
[loc_1551]
name=Gate
op1=Open
active=yes
category=gate_main_closed
param=next_loc_stage,loc_1552

[loc_1553]
name=Gate
mirror=yes
op1=Open
active=yes
category=gate_outer_closed
param=next_loc_stage,loc_1556
",
    );
    fx.write(
        "maps/m44_53.jm2",
        "==== MAP ====\n0 0 0: h1\n==== LOC ====\n0 5 46: 1551 0 0\n0 5 47: 1553 0 0\n",
    );
    let defs = loc_defs(&[(1551, 1, 1), (1553, 1, 1)]);
    let wc = bake_collision(&fx, &defs, &HashSet::from([1551, 1553]));
    let graph = derive_transports(fx.path(), &defs, &wc);
    assert_eq!(
        door_crossings(&graph, 1551),
        vec![
            ((2821, 3438), 'E', (2821, 3438)),
            ((2821, 3438), 'W', (2820, 3438)),
        ]
    );
    assert_eq!(
        door_crossings(&graph, 1553),
        vec![
            ((2821, 3439), 'E', (2821, 3439)),
            ((2821, 3439), 'W', (2820, 3439)),
        ]
    );
}

/// The spirit-tree network from `area_gnome/scripts/spirit_tree.rs2` (the
/// three `[oploc1]` blocks and `[label,spirit_tree_tele]`, verbatim but for
/// dialog lines) and `spirit_tree.constant`: the stronghold tree (ent)
/// flies to village/varrock/khazard, the village tree (stronghold_ent)
/// back to khazard/varrock/stronghold, and each young tree (loc_1317,
/// placed twice) to the village — 8 directed hops. Neither quest varp is
/// transmitted, so each hop carries its completed journal row, and the
/// helper's members abort puts `members_req` on all of them.
#[test]
fn derive_transports_emits_spirit_tree_edges() {
    let fx = Fixture::new();
    fx.write(
        "scripts/areas/area_gnome/configs/spirit_tree.constant",
        "\
^khazard_tree = 0_39_50_59_59
^varrock_tree = 0_49_54_43_51
^village_tree = 0_39_49_46_33
^stronghold_tree = 0_38_53_29_52
",
    );
    fx.write(
        "scripts/areas/area_gnome/scripts/spirit_tree.rs2",
        "\
[oploc1,ent] // gnome stronghold tree
if(%grandtree ! ^grandtree_complete) {
    ~mesbox(\"The tree doesn't feel like talking.\");
    return;
}
def_coord $end_pos = ^stronghold_tree;
switch_int(~p_choice3(\"Tree Gnome Village.\", 1, \"Forest north of Varrock.\", 2, \"Battlefield of Khazard.\", 3)) {
    case 1 : $end_pos = ^village_tree;
    case 2 : $end_pos = ^varrock_tree;
    case 3 : $end_pos = ^khazard_tree;
}
@spirit_tree_tele($end_pos);

[oploc1,stronghold_ent] // gnome village tree
if(%treequest ! ^tree_complete) {
    ~mesbox(\"The tree doesn't feel like talking.\");
    return;
}
def_coord $end_pos = ^village_tree;
switch_int(~p_choice3(\"Battlefield of Khazard.\", 1, \"Forest north of Varrock.\", 2, \"Gnome stronghold.\", 3)) {
    case 1 : $end_pos = ^khazard_tree;
    case 2 : $end_pos = ^varrock_tree;
    case 3 : $end_pos = ^stronghold_tree;
}
@spirit_tree_tele($end_pos);

[oploc1,loc_1317] // young tree
if(%treequest ! ^tree_complete) {
    ~mesbox(\"The tree doesn't feel like talking.\");
    return;
}
switch_int(~p_choice2(\"Yes please.\", 1, \"No thank you.\", 2)) {
    case 1 :
        ~chatplayer(\"<p,neutral>Yes please.\");
        @spirit_tree_tele(^village_tree);
    case 2 :
        ~chatplayer(\"<p,neutral>No thank you.\");
}

[label,spirit_tree_tele](coord $dest)
if(map_members = ^false) {
    if_close;
    mes(^mes_members_feature);
    return;
}
if_close;
anim(human_reachforladder, 0);
p_delay(0);
p_telejump($dest);
",
    );
    fx.write("pack/varp.pack", "150=grandtree\n111=treequest\n");
    fx.write(
        "scripts/general/scripts/quests.rs2",
        &format!("{JOURNAL_GREEN_SOURCE}~send_quest_progress_colour(questlist:grandtree, %grandtree, ^grandtree_complete);\n~send_quest_progress_colour(questlist:tree, %treequest, ^tree_complete);\n"),
    );
    fx.write(
        "scripts/general/configs/quest.constant",
        "^grandtree_complete = 160\n^tree_complete = 9\n",
    );
    fx.write(
        "scripts/player/interfaces/questlist.if",
        "[grandtree]\ntext=The Grand Tree\n[tree]\ntext=Tree Gnome Village\n",
    );
    let ids = HashMap::from([
        ("ent".to_string(), 1293),
        ("stronghold_ent".to_string(), 1294),
        ("loc_1317".to_string(), 1317),
    ]);
    let placement = |id, x, z| Placement {
        id,
        shape: 10,
        angle: 0,
        level: 0,
        x,
        z,
    };
    let positions = HashMap::from([
        (1293, vec![placement(1293, 2461, 3444)]),
        (1294, vec![placement(1294, 2542, 3169)]),
        (
            1317,
            vec![placement(1317, 3179, 3507), placement(1317, 2555, 3259)],
        ),
    ]);
    let mut graph = TransportGraph::default();
    let gates = ObservableGates::from_content(fx.path());
    spirit_tree_edges(
        fx.path(),
        &ids,
        &positions,
        &mut graph,
        &mut HashMap::new(),
        &gates,
        &mut VarpGateAudit::default(),
    );
    let trees: Vec<_> = graph
        .edges
        .iter()
        .filter(|e| e.kind == TransportKind::SpiritTree)
        .collect();
    assert_eq!(trees.len(), 8, "{trees:?}");
    for e in &trees {
        assert_eq!(e.option, 1, "Talk-to");
        assert_eq!(e.ticks, SPIRIT_TREE_TICKS);
        assert_eq!(e.dir, None);
        assert!(e.members_req);
        assert!(e.varp_req.is_empty());
        match e.loc_id {
            1293 => assert_eq!(e.quest_req, ["The Grand Tree"]),
            1294 | 1317 => assert_eq!(e.quest_req, ["Tree Gnome Village"]),
            other => panic!("unexpected spirit-tree loc id {other}"),
        }
    }
    // The young tree near Varrock stands in F2P-world geometry; its
    // completed journal row alone cannot override the members abort.
    let young_near_varrock = trees
        .iter()
        .find(|e| e.loc_id == 1317 && e.at.x > 3000)
        .expect("young tree near Varrock");
    let mut journal = crate::world_state::WorldState::empty();
    journal.quests.insert("Tree Gnome Village".into());
    assert!(!journal.allows(young_near_varrock));
    journal.map_members = true;
    assert!(journal.allows(young_near_varrock));
    let tile = |x, z| WorldTile { x, z, level: 0 };
    let dests = |loc_id: i32| -> Vec<WorldTile> {
        let mut v: Vec<WorldTile> = trees
            .iter()
            .filter(|e| e.loc_id == loc_id)
            .map(|e| e.to)
            .collect();
        v.sort_by_key(|t| (t.x, t.z));
        v.dedup();
        v
    };
    // ^village_tree, ^khazard_tree, ^varrock_tree / ^stronghold_tree.
    assert_eq!(
        dests(1293),
        vec![tile(2542, 3169), tile(2555, 3259), tile(3179, 3507)]
    );
    assert_eq!(
        dests(1294),
        vec![tile(2461, 3444), tile(2555, 3259), tile(3179, 3507)]
    );
    assert_eq!(dests(1317), vec![tile(2542, 3169)]);
    assert_eq!(trees.iter().filter(|e| e.loc_id == 1317).count(), 2);
}

#[test]
fn spirit_tree_without_members_abort_is_not_emitted() {
    let fx = Fixture::new();
    fx.write(
        "scripts/areas/area_gnome/configs/spirit_tree.constant",
        "^village_tree=0_39_49_30_33\n",
    );
    let script = "[oploc1,youngtree]\n@spirit_tree_tele(^village_tree);\n\
[label,spirit_tree_tele](coord $dest)\n\
if(map_members = ^false) {\n    if_close;\n    mes(^mes_members_feature);\n    return;\n}\n\
p_telejump($dest);\n";
    fx.write("scripts/areas/area_gnome/scripts/spirit_tree.rs2", script);
    let ids = HashMap::from([("youngtree".to_string(), 1317)]);
    let positions = HashMap::from([(
        1317,
        vec![Placement {
            id: 1317,
            shape: 10,
            angle: 0,
            level: 0,
            x: 3179,
            z: 3507,
        }],
    )]);
    let mut graph = TransportGraph::default();
    let mut skipped = HashMap::new();
    let mut audit = VarpGateAudit::default();
    let gates = ObservableGates::from_content(fx.path());
    spirit_tree_edges(
        fx.path(),
        &ids,
        &positions,
        &mut graph,
        &mut skipped,
        &gates,
        &mut audit,
    );
    assert_eq!(graph.edges.len(), 1);
    assert!(graph.edges[0].members_req);

    fx.write(
        "scripts/areas/area_gnome/scripts/spirit_tree.rs2",
        &script.replace("    return;\n", ""),
    );
    graph.edges.clear();
    spirit_tree_edges(
        fx.path(),
        &ids,
        &positions,
        &mut graph,
        &mut skipped,
        &gates,
        &mut audit,
    );
    assert!(
        graph.edges.is_empty(),
        "missing members abort supplies no edge"
    );
}

/// The derived graph carries the `TransportKind::Npc` edges
/// for the Shilo↔Brimhaven cart (`cart_edges`, the `hajedy.rs2` /
/// `vigroy.rs2` route pair): coins on the fare and the Shilo Village
/// journal name on the Brim→Shilo hop.
#[test]
fn derive_transports_emits_shilo_brimhaven_cart() {
    let graph = derive_static_routes();
    let carts: Vec<_> = graph
        .edges
        .iter()
        .filter(|e| e.kind == TransportKind::Npc)
        .cloned()
        .collect();
    assert!(
        carts.len() >= 2,
        "both cart directions derive, got {}",
        carts.len()
    );
    assert!(
        carts.iter().any(|e| !e.item_req.is_empty()),
        "coins on the fare"
    );
    assert!(
        carts.iter().any(|e| !e.quest_req.is_empty()),
        "Shilo complete on Brim→Shilo"
    );
}

/// Both wilderness levers (`wilderness_lever.rs2` `[oploc1,wildinlever]` /
/// `[oploc1,wildoutlever]`, locs 1814/1815) teleport through their
/// `wilderness_lever.constant` coords: the Ardougne lever lands inside the
/// wilderness zone and the wilderness lever outside it.
#[test]
fn derive_transports_emits_wildy_ardougne_levers() {
    let fx = Fixture::new();
    fx.write("pack/loc.pack", "1814=wildinlever\n1815=wildoutlever\n");
    fx.write(
        "scripts/areas/area_ardougne_east/configs/wilderness_lever.constant",
        "^ardougne_to_wilderness_coord = 0_49_61_18_20\n^wilderness_to_ardougne_coord = 0_40_51_2_47\n",
    );
    fx.write(
        "scripts/areas/area_ardougne_east/scripts/wilderness_lever.rs2",
        "\
[oploc1,wildinlever]
p_arrivedelay;
anim(human_leverdown, 0);
sound_synth(lever, 1, 0);
loc_change(hauntedleverdown, 7);
if_close;
p_delay(1);
mes(\"You pull the lever...\");
p_delay(0);
~player_teleport_normal(^ardougne_to_wilderness_coord);
mes(\"...And teleport into the wilderness.\");

[oploc1,wildoutlever]
p_arrivedelay;
anim(human_leverdown, 0);
sound_synth(lever, 1, 0);
loc_change(hauntedleverdown, 7);
if_close;
p_delay(0);
mes(\"You pull the lever...\");
p_delay(0);
~player_teleport_normal(^wilderness_to_ardougne_coord);
mes(\"...And teleport out of the wilderness.\");
",
    );
    // (2561,3311) in Ardougne and (3153,3923) in the wilderness.
    fx.write(
        "maps/m40_51.jm2",
        "==== MAP ====\n0 1 47: h1 o6 u50\n==== LOC ====\n0 1 47: 1814 4\n",
    );
    fx.write(
        "maps/m49_61.jm2",
        "==== MAP ====\n0 17 19: h1 o6 u50\n==== LOC ====\n0 17 19: 1815 4\n",
    );
    let defs = loc_defs(&[(1814, 1, 1), (1815, 1, 1)]);
    let wc = bake_collision(&fx, &defs, &HashSet::new());
    let mut graph = derive_transports(fx.path(), &defs, &wc);
    graph.wilderness = surface_wildy_rules();
    let lever = |id| {
        let found: Vec<_> = graph.edges.iter().filter(|e| e.loc_id == id).collect();
        assert_eq!(found.len(), 1, "{id}: {found:?}");
        found[0].clone()
    };
    let into = lever(1814);
    let out = lever(1815);
    assert_eq!(
        into.to,
        WorldTile {
            x: 3154,
            z: 3924,
            level: 0
        }
    );
    assert_eq!(
        out.to,
        WorldTile {
            x: 2562,
            z: 3311,
            level: 0
        }
    );
    assert!(
        graph.wilderness.contains(into.to),
        "the Ardougne→wildy lever lands inside the wilderness"
    );
    assert!(
        !graph.wilderness.contains(out.to),
        "the wildy→Ardougne lever lands outside the wilderness"
    );
}

/// The real content must derive the Al Kharid border toll and the
/// Shantay-pass edges as item-gated `TransportKind::Door` edges.
/// The toll gates (`border_gate_toll_left`/`_right`, loc 2882/2883 —
/// the m51_50 (4,27)/(4,28) placements = (3268,3227)/(3268,3228))
/// carry the 10-coin toll (`inv_del(inv, coins, 10)` in
/// border_gate.rs2's `pass_toll_gate`); the Shantay henge doorway
/// (loc 4031, m51_48 (38,44) = (3302,3116)) derives **two** edges,
/// one per `shantay_pass.rs2` `[oploc1,...]` branch — the gated hop
/// into the desert (`at` the placement, `to` (3304,3115), the
/// landing of the `[queue,shantay_pass_enter]` `p_teleport
/// (0_51_48_40_46)` + `p_telejump(movecoord(coord,0,0,-3))`,
/// `item_req` one Shantay pass (obj 1854)) and the free desert exit
/// (`at` (3302,3115) one tile south of the placement, `to`
/// (3303,3118), the `coordz(coord) <= coordz(loc_coord)`
/// `p_telejump(movecoord(coord,0,0,3))` landing, **no** `item_req`).
/// Only the northbound hop carries the pass.
#[test]
#[ignore = "NAV_CONTENT_ROOT and NAV_CACHE required; absence fails"]
fn derive_transports_emits_alkharid_toll_and_shantay_north() {
    for (root, graph, _wc) in qualification_worlds() {
        let _ = root;
        // Each gate has two crossing directions, each with paid and
        // journal-completed alternatives derived from the border guard script.
        let tolls: Vec<_> = graph
            .edges
            .iter()
            .filter(|e| e.kind == TransportKind::Door && (e.loc_id == 2882 || e.loc_id == 2883))
            .cloned()
            .collect();
        assert!(
            !tolls.is_empty(),
            "no Door edges for the Al Kharid toll gates (loc 2882/2883)"
        );
        assert_eq!(
            tolls.iter().filter(|e| e.loc_id == 2882).count(),
            4,
            "left toll gate derives both crossings"
        );
        assert_eq!(
            tolls.iter().filter(|e| e.loc_id == 2883).count(),
            4,
            "right toll gate derives both crossings"
        );
        for e in &tolls {
            assert_eq!(
                e.at,
                if e.loc_id == 2882 {
                    WorldTile {
                        x: 3268,
                        z: 3227,
                        level: 0,
                    }
                } else {
                    WorldTile {
                        x: 3268,
                        z: 3228,
                        level: 0,
                    }
                }
            );
            assert!(
                (e.item_req == vec![(995, 10)] && e.quest_req.is_empty())
                    || (e.item_req.is_empty() && e.quest_req == ["Prince Ali Rescue"]),
                "each crossing is either paid or waived by the completed quest journal: {e:?}"
            );
            assert!(e.varp_req.is_empty(), "princequest is not transmitted");
            assert_eq!(e.option, 1, "Open op");
            assert_eq!(
                e.open_loc_id,
                Some(if e.loc_id == 2882 { 1562 } else { 1563 })
            );
        }
        for gate in [2882, 2883] {
            for eastbound in [false, true] {
                let crossing: Vec<_> = tolls
                    .iter()
                    .filter(|e| e.loc_id == gate && (e.to.x > e.at.x) == eastbound)
                    .collect();
                assert_eq!(crossing.len(), 2);
                assert_eq!(crossing.iter().filter(|e| e.item_req.is_empty()).count(), 1);
                assert_eq!(
                    crossing.iter().filter(|e| e.quest_req.is_empty()).count(),
                    1
                );
            }
        }
        // The Shantay henge carries exactly two edges, one per
        // `[oploc1,shantay_pass_henge_doorway]` branch: the gated hop
        // into the desert and the free desert exit.
        let henge: Vec<_> = graph
            .edges
            .iter()
            .filter(|e| e.loc_id == 4031)
            .cloned()
            .collect();
        assert_eq!(
            henge.len(),
            2,
            "exactly two Shantay henge edges derive (the gated desert hop \
                 and the free desert exit), got {}",
            henge.len()
        );
        let gated = henge
            .iter()
            .find(|e| !e.item_req.is_empty())
            .expect("one Shantay henge edge carries the pass");
        let free = henge
            .iter()
            .find(|e| e.item_req.is_empty())
            .expect("one Shantay henge edge is free");
        assert_eq!(
            gated.at,
            WorldTile {
                x: 3302,
                z: 3116,
                level: 0,
            }
        );
        assert_eq!(
            gated.to,
            WorldTile {
                x: 3304,
                z: 3115,
                level: 0,
            }
        );
        assert!(
            gated.item_req.iter().any(|(id, n)| *id == 1854 && *n >= 1),
            "Shantay pass on the gated desert hop"
        );
        assert_eq!(gated.option, 1, "Go-through op");
        assert_eq!(gated.dir, None);
        assert_eq!(
            free.at,
            WorldTile {
                x: 3302,
                z: 3115,
                level: 0,
            }
        );
        assert_eq!(
            free.to,
            WorldTile {
                x: 3303,
                z: 3118,
                level: 0,
            }
        );
        assert!(free.item_req.is_empty(), "the desert exit is free");
        assert_eq!(free.option, 1, "Go-through op");
        assert_eq!(free.dir, None);
    }
}

/// The Ardougne→wilderness lever is an enter-wildy hop: default
/// [`crate::router::find`] must never relax it (its `to` is inside the
/// wilderness zone), and [`crate::router::find_with`] with
/// `allow_wilderness` must route through it. Fixture: an isolated
/// content root whose only lever is that one (same placement and
/// `p_teleport` destination constant the real content declares).
#[test]
fn default_find_skips_the_ardougne_to_wildy_lever() {
    use crate::router::{find, find_with, FindOptions, RouteError};

    let fx = Fixture::new();
    fx.write("pack/loc.pack", "1814=wildinlever\n");
    fx.write(
        "scripts/areas/area_ardougne_east/configs/wilderness_lever.constant",
        "^ardougne_to_wilderness_coord = 0_49_61_18_20\n",
    );
    fx.write(
            "scripts/areas/area_ardougne_east/scripts/wilderness_lever.rs2",
            "\
[oploc1,wildinlever]
p_arrivedelay;
if (%warning_wilderness_teleport_lever = ^false) {
    ~mesbox(\"Warning! Pulling the lever will teleport you deep into the wilderness.\");
    def_int $choice = ~p_choice3_header(\"Yes I'm brave.\", 1, \"Eep! The wilderness... No thank you.\", 2, \"Yes please, don't show this message again.\", 3, \"Are you sure you wish to pull it?\");
    if ($choice = 2) {
        return;
    }
    if ($choice = 3) {
        %warning_wilderness_teleport_lever = ^true;
    }
}
anim(human_leverdown, 0);
sound_synth(lever, 1, 0);
loc_change(hauntedleverdown, 7);
if_close;
p_delay(1);
mes(\"You pull the lever...\");
p_delay(0);
~player_teleport_normal(^ardougne_to_wilderness_coord);
mes(\"...And teleport into the wilderness.\");
",
        );
    // m40_51 local (1,47) = absolute (2561,3311,0); the constant
    // `0_49_61_18_20` = (3154,3924,0), inside the surface zone.
    fx.write(
        "maps/m40_51.jm2",
        "\
==== MAP ====
0 1 47: h1 o6 u50
==== LOC ====
0 1 47: 1814 4
",
    );
    let defs = loc_defs(&[(1814, 1, 1)]);
    let wc = bake_collision(&fx, &defs, &HashSet::new());
    let mut graph = derive_transports(fx.path(), &defs, &wc);
    graph.wilderness = surface_wildy_rules();

    let at = WorldTile {
        x: 2561,
        z: 3311,
        level: 0,
    };
    let to = WorldTile {
        x: 3154,
        z: 3924,
        level: 0,
    };
    let levers: Vec<_> = graph
        .edges
        .iter()
        .filter(|e| e.kind == TransportKind::Door && e.loc_id == 1814)
        .cloned()
        .collect();
    assert_eq!(levers.len(), 1, "one placement, one Pull edge");
    assert_eq!(levers[0].at, at);
    assert_eq!(levers[0].to, to);
    assert_eq!(levers[0].option, 1); // Pull (oploc1)
    assert_eq!(levers[0].dir, None);
    assert!(graph.wilderness.contains(to));

    // Default find: the enter-wildy hop is never relaxed, and no walk
    // path can reach the landing — NoPath.
    assert!(matches!(find(&wc, &graph, at, to), Err(RouteError::NoPath)));
    // find_with(allow_wilderness): the same hop routes through.
    let route = find_with(
        &wc,
        &graph,
        at,
        to,
        FindOptions {
            allow_teleports: false,
            allow_wilderness: true,
            allow_bank_fetch: false,
            ..FindOptions::default()
        },
        &crate::world_state::WorldState::empty(),
    )
    .expect("allow_wilderness routes the Ardougne→wildy lever");
    assert_eq!(route.dest, to);
}

fn surface_wildy_rules() -> WildernessRules {
    WildernessRules {
        zones: vec![WildernessZone {
            x1: 2944,
            z1: 3520,
            x2: 3391,
            z2: 6399,
            level1: 0,
            level2: 3,
            origin_z: 3520,
        }],
        divisor: 8,
        offset: 1,
    }
}

/// The gates seam: a route must now exist from Seers street
/// (2725,3485,0) to the rock-crab shore (2710,3720,0) once the fence
/// gates join the door set, on the real map.
#[test]
#[ignore = "NAV_CONTENT_ROOT and NAV_CACHE required; absence fails"]
fn seers_street_reaches_rock_crabs_after_gates() {
    use crate::router::find;

    let from = WorldTile {
        x: 2725,
        z: 3485,
        level: 0,
    };
    let to = WorldTile {
        x: 2710,
        z: 3720,
        level: 0,
    };
    for (_, graph, collision) in qualification_worlds() {
        let route = find(collision, graph, from, to).unwrap_or_else(|e| {
            panic!("Seers street -> rock crabs must route once gates join: {e:?}")
        });
        assert_eq!(route.dest, to);
    }
}

#[test]
fn derive_transports_pins_source_ladder() {
    let fx = Fixture::new();
    fx.write("pack/loc.pack", "1747=ladder\n");
    fx.write(
        "maps/m44_53.jm2",
        "\
==== MAP ====
0 10 10: h1 o6 u50
==== LOC ====
0 10 10: 1747 0 0
",
    );
    fx.write(
        "scripts/ladders+stairs/scripts/ladders.rs2",
        "\
[oploc1,ladder]
p_arrivedelay;
switch_coord (loc_coord) {
    case 0_44_53_10_10 : ~climb_ladder(1_44_54_10_12, true);
    case default : ~climb_ladder(movecoord(coord(), 0, 1, 0), true);
}
",
    );
    let defs = loc_defs(&[(1747, 1, 1)]);
    let wc = bake_collision(&fx, &defs, &HashSet::new());
    let graph = derive_transports(fx.path(), &defs, &wc);

    // One ladder placement (id 1747 @ 2826,3402,0) climbing to
    // (1,2826,3468): one edge per placement — `at` the loc tile
    // (blocked), `to` the same landing.
    let ladders: Vec<_> = graph
        .edges
        .iter()
        .filter(|e| e.kind == TransportKind::Ladder && e.loc_id == 1747)
        .collect();
    assert_eq!(ladders.len(), 1);
    let landing = WorldTile {
        x: 2826,
        z: 3468,
        level: 1,
    };
    let ladder = &ladders[0];
    assert_eq!(
        ladder.at,
        WorldTile {
            x: 2826,
            z: 3402,
            level: 0
        }
    );
    assert_eq!(ladder.to, landing);
    assert_eq!(ladder.dir, None);
    assert_eq!(ladder.open_loc_id, None);
    assert_eq!(ladder.option, 1);
    assert_eq!(ladder.ticks, 3); // op base 1 + ladder extra 2
    assert!(ladder.skill_req.is_empty());

    let ladder_at = WorldTile {
        x: 2826,
        z: 3402,
        level: 0,
    };
    assert_eq!(graph.at[&ladder_at].len(), 1);
    assert_eq!(graph.edges[graph.at[&ladder_at][0]].to, landing);
}

#[test]
fn derive_transports_emits_edgeville_trapdoor() {
    let fx = Fixture::new();
    fx.write("pack/loc.pack", "1568=trapdoor\n1570=trapdoor_open\n");
    fx.write(
        "maps/m48_54.jm2",
        "\
==== MAP ====
0 25 12: h1 o6 u50
0 24 12: h1 o6 u50
==== LOC ====
0 25 12: 1568 22 2
",
    );
    fx.write(
        "scripts/general_use/scripts/trapdoors.rs2",
        "\
[oploc1,trapdoor]
mes(\"The trapdoor opens...\");
loc_change(trapdoor_open, 500);

[oploc1,trapdoor_open]
mes(\"You climb down through the trapdoor...\");
p_telejump(movecoord(coord(), 0, 0, 6400));
",
    );
    let defs = loc_defs(&[(1568, 1, 1), (1570, 1, 1)]);
    let wc = bake_collision(&fx, &defs, &HashSet::new());
    let graph = derive_transports(fx.path(), &defs, &wc);
    let at = WorldTile {
        x: 3097,
        z: 3468,
        level: 0,
    };
    let edges: Vec<_> = graph
        .at
        .get(&at)
        .into_iter()
        .flatten()
        .map(|&i| &graph.edges[i])
        .collect();
    assert_eq!(edges.len(), 1, "got {edges:?}");
    let e = edges[0];
    assert_eq!(e.kind, TransportKind::Ladder);
    assert_eq!(e.loc_id, 1568);
    assert_eq!(e.open_loc_id, Some(1570));
    assert_eq!(e.option, 1);
    assert_eq!(
        e.to,
        WorldTile {
            x: 3097,
            z: 9868,
            level: 0
        }
    );
    assert_eq!(e.ticks, 3);
    assert!(!e.members_req);
}

#[test]
fn derive_transports_pins_watchshortcut_agility_req() {
    let fx = Fixture::new();
    fx.write("pack/loc.pack", "2298=watchshortcut\n");
    fx.write(
        "maps/m44_53.jm2",
        "\
==== MAP ====
0 5 5: h1 o6 u50
==== LOC ====
0 5 5: 2298 10 0
",
    );
    fx.write(
        "scripts/skill_agility/scripts/shortcuts.rs2",
        "\
[oploc1,watchshortcut]
if(stat(agility) < 5) {
    ~mesbox(\"You need an Agility level of 5 to climb the wall.\");
    return;
}
p_telejump(movecoord(loc_coord, 0, 0, 3));
",
    );
    let defs = loc_defs(&[(2298, 1, 1)]);
    let wc = bake_collision(&fx, &defs, &HashSet::new());
    let graph = derive_transports(fx.path(), &defs, &wc);

    let edges: Vec<_> = graph
        .edges
        .iter()
        .filter(|e| e.kind == TransportKind::AgilityShortcut && e.loc_id == 2298)
        .collect();
    assert_eq!(edges.len(), 1);
    let landing = WorldTile {
        x: 2821,
        z: 3400,
        level: 0,
    };
    let e = &edges[0];
    // One edge per placement: `at` the loc tile, `to` the shortcut dest.
    assert_eq!(
        e.at,
        WorldTile {
            x: 2821,
            z: 3397,
            level: 0
        }
    );
    assert_eq!(e.to, landing);
    assert_eq!(e.dir, None);
    assert_eq!(e.open_loc_id, None);
    assert_eq!(e.option, 1);
    assert_eq!(e.ticks, 1); // op base 1 + watchshortcut extra 0
    assert_eq!(e.skill_req, vec![(SKILL_AGILITY, 5)]);
}

const ISLAND_ROPE_LOC: &str = "\
[tree_ropeswing1]
name=Ropeswing
op1=Swing-on
length=6
blockwalk=no
category=island_rope_swing
param=start_coord,0_42_50_21_9
param=end_coord,0_42_50_16_9
param=dir,3

[tree_ropeswing2]
name=Ropeswing
op1=Swing-on
length=6
blockwalk=no
category=island_rope_swing
param=start_coord,0_42_50_17_5
param=end_coord,0_42_50_21_5
param=dir,1

[tree_ropeswing3]
name=Ropeswing
op1=Swing-on
length=6
blockwalk=no
category=island_rope_swing
param=start_coord,0_39_48_15_19
param=end_coord,0_39_48_15_24
param=dir,0

[zqrockjump1]
name=Stepping stones
op1=Cross
category=karamja_stepping_stone
";

fn island_rope_fixture(pack_ids: &[(i32, &str)]) -> Fixture {
    let fx = Fixture::new();
    let mut pack = String::new();
    for (id, name) in pack_ids {
        pack.push_str(&format!("{id}={name}\n"));
    }
    fx.write("pack/loc.pack", &pack);
    fx.write(
        "scripts/skill_agility/configs/shortcuts.loc",
        ISLAND_ROPE_LOC,
    );
    fx.write(
        "maps/m42_50.jm2",
        "\
==== MAP ====
0 17 9: h1 o6 u50
0 15 5: h1 o6 u50
==== LOC ====
0 17 9: 9001 10 1
0 15 5: 9002 10 3
",
    );
    fx.write(
        "maps/m39_48.jm2",
        "\
==== MAP ====
0 15 18: h1 o6 u50
==== LOC ====
0 15 18: 9003 10 2
",
    );
    fx.write(
        "maps/m45_73.jm2",
        "\
==== MAP ====
0 15 18: h1 o6 u50
==== LOC ====
0 15 18: 9003 10 2
",
    );
    fx.write(
        "maps/m45_46.jm2",
        "\
==== MAP ====
0 45 4: h1 o6 u50
==== LOC ====
0 45 4: 9333 10 0
",
    );
    fx
}

fn island_rope_pack() -> Vec<(i32, &'static str)> {
    vec![
        (9001, "tree_ropeswing1"),
        (9002, "tree_ropeswing2"),
        (9003, "tree_ropeswing3"),
        (9333, "zqrockjump1"),
    ]
}

fn island_rope_defs() -> LocDefs {
    loc_defs(&[(9001, 1, 6), (9002, 1, 6), (9003, 1, 6), (9333, 1, 1)])
}

fn island_rope_edge(graph: &TransportGraph, loc_id: i32) -> Option<&TransportEdge> {
    graph
        .edges
        .iter()
        .find(|e| e.kind == TransportKind::AgilityShortcut && e.loc_id == loc_id)
}

#[test]
fn derive_transports_emits_island_rope_swing_from_start_end_params() {
    let fx = island_rope_fixture(&island_rope_pack());
    let defs = island_rope_defs();
    let wc = bake_collision(&fx, &defs, &HashSet::new());
    let (graph, skipped) = derive_transports_with_skips(fx.path(), &defs, &wc);

    let inbound = island_rope_edge(&graph, 9001).expect("tree_ropeswing1");
    assert_eq!(
        inbound.at,
        WorldTile {
            x: 2709,
            z: 3209,
            level: 0
        }
    );
    assert_eq!(
        inbound.to,
        WorldTile {
            x: 2704,
            z: 3209,
            level: 0
        }
    );
    assert_eq!(inbound.option, 1);
    assert_eq!(inbound.dir, None);
    assert_eq!(inbound.ticks, 2);
    assert_eq!(inbound.skill_req, vec![(SKILL_AGILITY, 10)]);
    assert!(inbound.item_req.is_empty());
    assert!(inbound.quest_req.is_empty());
    assert!(inbound.varp_req.is_empty());
    assert!(inbound.worn_req.is_empty());
    assert!(!inbound.members_req);

    let ret = island_rope_edge(&graph, 9002).expect("tree_ropeswing2");
    assert_eq!(
        ret.at,
        WorldTile {
            x: 2705,
            z: 3205,
            level: 0
        }
    );
    assert_eq!(
        ret.to,
        WorldTile {
            x: 2709,
            z: 3205,
            level: 0
        }
    );
    assert!(ret.skill_req.is_empty(), "return has no agility gate");
    assert_eq!(ret.ticks, 2);
    assert_eq!(ret.option, 1);
    assert_eq!(ret.dir, None);

    let ogre = island_rope_edge(&graph, 9003).expect("tree_ropeswing3");
    assert_eq!(
        ogre.at,
        WorldTile {
            x: 2511,
            z: 3091,
            level: 0
        }
    );
    assert_eq!(
        ogre.to,
        WorldTile {
            x: 2511,
            z: 3096,
            level: 0
        }
    );
    assert_eq!(ogre.skill_req, vec![(SKILL_AGILITY, 10)]);

    assert!(
        graph.edges.iter().filter(|e| e.loc_id == 9003).count() == 1,
        "far m45_73 copy must not invent a second edge"
    );
    assert!(
        graph.edges.iter().all(|e| e.at
            != WorldTile {
                x: 2895,
                z: 4690,
                level: 0
            }),
        "absolute params must not emit from the far copy origin"
    );
    assert_eq!(skip_total(&skipped, SKIP_ISLAND_ROPE_JOIN), 1);
    assert_eq!(skip_total(&skipped, SKIP_UNPRICED), 0);
    assert!(
        island_rope_edge(&graph, 9333).is_none(),
        "zqrockjump1 is not island_rope_swing"
    );

    let inbound_ok = crate::world_state::WorldState {
        stats: HashMap::from([(SKILL_AGILITY, 10)]),
        ..crate::world_state::WorldState::empty()
    };
    let inbound_low = crate::world_state::WorldState {
        stats: HashMap::from([(SKILL_AGILITY, 9)]),
        ..crate::world_state::WorldState::empty()
    };
    assert!(inbound_ok.allows(inbound));
    assert!(!inbound_low.allows(inbound));
    assert!(crate::world_state::WorldState {
        stats: HashMap::from([(SKILL_AGILITY, 1)]),
        ..crate::world_state::WorldState::empty()
    }
    .allows(ret));
}

#[test]
fn derive_transports_skips_island_rope_missing_end_coord() {
    let fx = Fixture::new();
    fx.write("pack/loc.pack", "9001=tree_ropeswing1\n");
    fx.write(
        "scripts/skill_agility/configs/shortcuts.loc",
        "\
[tree_ropeswing1]
category=island_rope_swing
length=6
param=start_coord,0_42_50_21_9
",
    );
    fx.write(
        "maps/m42_50.jm2",
        "==== MAP ====\n0 17 9: h1 o6 u50\n==== LOC ====\n0 17 9: 9001 10 1\n",
    );
    let defs = loc_defs(&[(9001, 1, 6)]);
    let wc = bake_collision(&fx, &defs, &HashSet::new());
    let (graph, skipped) = derive_transports_with_skips(fx.path(), &defs, &wc);
    assert!(island_rope_edge(&graph, 9001).is_none());
    assert_eq!(skip_total(&skipped, SKIP_ISLAND_ROPE_PARAMS), 1);
}

#[test]
fn derive_transports_skips_island_rope_when_shortcuts_loc_missing() {
    let fx = Fixture::new();
    fx.write("pack/loc.pack", "9001=tree_ropeswing1\n");
    fx.write(
        "maps/m42_50.jm2",
        "==== MAP ====\n0 17 9: h1 o6 u50\n==== LOC ====\n0 17 9: 9001 10 1\n",
    );
    let defs = loc_defs(&[(9001, 1, 6)]);
    let wc = bake_collision(&fx, &defs, &HashSet::new());
    let (graph, skipped) = derive_transports_with_skips(fx.path(), &defs, &wc);
    assert!(island_rope_edge(&graph, 9001).is_none());
    assert_eq!(skip_total(&skipped, SKIP_ISLAND_ROPE_CONFIG), 1);
}

fn pack_id_by_name(content_root: &Path, name: &str) -> Option<i32> {
    loc_ids_by_name(content_root).get(name).copied()
}

fn assert_real_island_ropes(graph: &TransportGraph, content_root: &Path) {
    let id1 = pack_id_by_name(content_root, "tree_ropeswing1").expect("pack tree_ropeswing1");
    let id2 = pack_id_by_name(content_root, "tree_ropeswing2").expect("pack tree_ropeswing2");
    let id3 = pack_id_by_name(content_root, "tree_ropeswing3").expect("pack tree_ropeswing3");
    let inbound = island_rope_edge(graph, id1).expect("inbound island rope");
    assert_eq!(
        inbound.at,
        WorldTile {
            x: 2709,
            z: 3209,
            level: 0
        }
    );
    assert_eq!(
        inbound.to,
        WorldTile {
            x: 2704,
            z: 3209,
            level: 0
        }
    );
    assert_eq!(inbound.option, 1);
    assert_eq!(inbound.dir, None);
    assert_eq!(inbound.skill_req, vec![(SKILL_AGILITY, 10)]);
    let ret = island_rope_edge(graph, id2).expect("return island rope");
    assert_eq!(
        ret.at,
        WorldTile {
            x: 2705,
            z: 3205,
            level: 0
        }
    );
    assert_eq!(
        ret.to,
        WorldTile {
            x: 2709,
            z: 3205,
            level: 0
        }
    );
    assert!(ret.skill_req.is_empty());
    let ogre = island_rope_edge(graph, id3).expect("ogre island rope");
    assert_eq!(
        ogre.at,
        WorldTile {
            x: 2511,
            z: 3091,
            level: 0
        }
    );
    assert_eq!(
        ogre.to,
        WorldTile {
            x: 2511,
            z: 3096,
            level: 0
        }
    );
    assert_eq!(ogre.skill_req, vec![(SKILL_AGILITY, 10)]);
    if let Some(zq) = pack_id_by_name(content_root, "zqrockjump1") {
        assert!(
            graph
                .edges
                .iter()
                .filter(|e| e.kind == TransportKind::AgilityShortcut && e.loc_id == zq)
                .all(|e| e.at.x != 2698 && e.to.x != 2698),
            "zqrockjump1 must not close FIELD"
        );
    }
}

/// The real island rope swings sit where `shortcuts.loc` places them.
#[test]
#[ignore = "NAV_CONTENT_ROOT and NAV_CACHE required; absence fails"]
fn derive_transports_island_ropes_from_real_content() {
    for (root, graph, _) in qualification_worlds() {
        assert_real_island_ropes(graph, root);
    }
}

fn inspect_avoid() -> [crate::router::AvoidRect; 1] {
    [crate::router::AvoidRect {
        min_x: 2780,
        max_x: 3040,
        min_z: 3130,
        max_z: 3330,
        level: None,
    }]
}

fn inspect_opts() -> crate::router::FindOptions {
    crate::router::FindOptions {
        allow_teleports: false,
        allow_wilderness: false,
        allow_bank_fetch: false,
        essence: None,
        zones: crate::zones::ZoneExempt::all(),
    }
}

fn pier_field_state(coins: i32, agility: i32) -> crate::world_state::WorldState {
    crate::world_state::WorldState {
        inv: HashMap::from([(995, coins)]),
        stats: HashMap::from([(SKILL_AGILITY, agility)]),
        ..crate::world_state::WorldState::empty()
    }
    .with_map_members(true)
}

#[test]
#[ignore = "NAV_CONTENT_ROOT and NAV_CACHE required; absence fails"]
fn derived_graph_pier_reaches_field() {
    for (root, graph, collision) in qualification_worlds() {
        let swing1 = pack_id_by_name(root, "tree_ropeswing1").expect("pack tree_ropeswing1");
        let inbound = island_rope_edge(graph, swing1).expect("derived inbound swing");
        assert_eq!(
            inbound.at,
            WorldTile {
                x: 2709,
                z: 3209,
                level: 0
            }
        );
        let pier = WorldTile {
            x: 2683,
            z: 3272,
            level: 0,
        };
        let field = WorldTile {
            x: 2698,
            z: 3206,
            level: 0,
        };
        let opts = inspect_opts();
        let avoid = inspect_avoid();
        assert!(
            matches!(
                crate::router::find_with_avoid(
                    collision,
                    graph,
                    pier,
                    field,
                    opts,
                    &pier_field_state(0, 10),
                    &avoid,
                ),
                Err(crate::router::RouteError::NoPath)
            ),
            "missing coins stay NoPath"
        );
        assert!(
            matches!(
                crate::router::find_with_avoid(
                    collision,
                    graph,
                    pier,
                    field,
                    opts,
                    &pier_field_state(30, 9),
                    &avoid,
                ),
                Err(crate::router::RouteError::NoPath)
            ),
            "agility 9 stays NoPath"
        );
        let ok = crate::router::find_with_avoid(
            collision,
            graph,
            pier,
            field,
            opts,
            &pier_field_state(30, 10),
            &avoid,
        )
        .unwrap_or_else(|e| panic!("coins30 agility10 must reach FIELD ({e:?})"));
        assert!(
            ok.legs.iter().any(|l| matches!(
                l,
                crate::router::Leg::Transport { edge } if edge.loc_id == 381
            )),
            "Barnaby 381 must be on the route: {ok:?}"
        );
        assert!(
            ok.legs.iter().any(|l| matches!(
                l,
                crate::router::Leg::Transport { edge }
                    if edge.loc_id == swing1
                        && edge.at
                            == WorldTile {
                                x: 2709,
                                z: 3209,
                                level: 0
                            }
            )),
            "derived inbound swing must be on the route: {ok:?}"
        );

        let ret_id = pack_id_by_name(root, "tree_ropeswing2").expect("pack tree_ropeswing2");
        let back = crate::router::find_with_avoid(
            collision,
            graph,
            field,
            pier,
            opts,
            &pier_field_state(30, 1),
            &avoid,
        )
        .unwrap_or_else(|e| panic!("return must allow low agility ({e:?})"));
        assert!(
            back.legs.iter().any(|l| matches!(
                l,
                crate::router::Leg::Transport { edge } if edge.loc_id == ret_id
            )),
            "low-agility return uses tree_ropeswing2: {back:?}"
        );
    }
}

#[test]
fn parse_landing_handles_movecoord_forms() {
    assert!(matches!(
        parse_landing("0_48_49_32_26"),
        Outcome::Landing(Landing::Abs {
            level: 0,
            x: 3104,
            z: 3162
        })
    ));
    assert!(matches!(
        parse_landing("movecoord(coord(), 0, 1, 0)"),
        Outcome::Landing(Landing::FromLevel { d: 1 })
    ));
    assert!(matches!(
        parse_landing("movecoord(coord, 0, 0, 6400)"),
        Outcome::Landing(Landing::FromZ { d: 6400 })
    ));
    assert!(matches!(
        parse_landing("movecoord(loc_coord, 2, 1, 0)"),
        Outcome::Landing(Landing::LocDelta {
            dx: 2,
            d_level: 1,
            dz: 0
        })
    ));
    assert!(matches!(
        parse_landing("movecoord(coord, -549, 0, 756)"),
        Outcome::Landing(Landing::PlayerDelta {
            dx: -549,
            d_level: 0,
            dz: 756
        })
    ));
    assert!(matches!(
        parse_landing("movecoord(1_34_77_30_5, $randomX, 0, $randomZ)"),
        Outcome::Skipped(SKIP_RANDOM)
    ));
    assert!(matches!(
        parse_landing("movecoord(0_45_55_19_44, 0, 1, 0)"),
        Outcome::Landing(Landing::Abs { level: 1, .. })
    ));
}

#[test]
fn parse_statement_classifies_handoffs_and_dialogs() {
    assert!(matches!(
        parse_statement(
            "@ladder_options(movecoord(coord(), 0, 1, 0), movecoord(coord(), 0, -1, 0));"
        ),
        Some(Outcome::Skipped(SKIP_DIALOG))
    ));
    assert!(matches!(
        parse_statement("@stair_options(2_50_50_5_9, 0_50_50_5_9);"),
        Some(Outcome::Skipped(SKIP_DIALOG))
    ));
    assert!(parse_statement("@unhandled_stairs(loc_coord);").is_none());
    assert!(matches!(
        parse_statement("@ladder_to_dwarf_remains;"),
        Some(Outcome::Skipped(SKIP_HANDOFF))
    ));
    assert!(matches!(
            parse_statement("def_int $option = ~p_choice2_header(\"Climb Up.\", 1, \"Climb Down.\", 2, \"Climb up or down the ladder?\");"),
            Some(Outcome::Skipped(SKIP_DIALOG))
        ));
    assert!(parse_statement("p_arrivedelay;").is_none());
}

#[test]
fn parse_script_picks_out_coord_cases_and_fallbacks() {
    let mut rules = HashMap::new();
    parse_script(
        "\
[oploc1,laddertop]
p_arrivedelay;
switch_coord (loc_coord) {
    case 2_47_54_17_57 : ~climb_ladder(1_47_54_17_58, false); // black knights fortress ladder
    case default : ~climb_ladder(movecoord(coord(), 0, -1, 0), false);
}
",
        TransportKind::Ladder,
        &mut rules,
    );
    let (kind, rule) = rules.get(&("laddertop".to_string(), 1)).unwrap();
    assert_eq!(*kind, TransportKind::Ladder);
    // 2_47_54_17_57 -> level 2, x=47<<6|17=3025, z=54<<6|57=3513.
    let packed = pack_coord(2, 3025, 3513);
    match rule.by_loc_coord.get(&packed) {
        Some(Outcome::Landing(Landing::Abs { level: 1, x, z })) => {
            assert_eq!(*x, 3025);
            assert_eq!(*z, 3514);
        }
        other => panic!("unexpected case outcome: {other:?}"),
    }
    assert!(matches!(
        rule.fallback,
        Some(Outcome::Landing(Landing::FromLevel { d: -1 }))
    ));
}

#[test]
fn parse_script_records_unguarded_statements_as_fallback() {
    let mut rules = HashMap::new();
    parse_script(
        "\
[oploc1,ship_ladder]
p_arrivedelay;
~climb_ladder(movecoord(coord(), 0, 1, 0), true);
",
        TransportKind::Ladder,
        &mut rules,
    );
    let (_, rule) = rules.get(&("ship_ladder".to_string(), 1)).unwrap();
    assert!(matches!(
        rule.fallback,
        Some(Outcome::Landing(Landing::FromLevel { d: 1 }))
    ));
}

#[test]
fn parse_script_records_dialog_skip_as_fallback() {
    let mut rules = HashMap::new();
    parse_script(
        "\
[oploc1,laddermiddle]
p_arrivedelay;
@ladder_options(movecoord(coord(), 0, 1, 0), movecoord(coord(), 0, -1, 0));
",
        TransportKind::Ladder,
        &mut rules,
    );
    let (_, rule) = rules.get(&("laddermiddle".to_string(), 1)).unwrap();
    assert!(matches!(rule.fallback, Some(Outcome::Skipped(SKIP_DIALOG))));
}

#[test]
fn derive_transports_skips_script_names_missing_from_pack() {
    let fx = Fixture::new();
    fx.write("pack/loc.pack", "");
    fx.write(
        "maps/m44_53.jm2",
        "\
==== MAP ====
0 0 0: h1 o6 u50
==== LOC ====
0 0 0: 1747 0 0
",
    );
    let defs = loc_defs(&[(1747, 1, 1)]);
    let wc = bake_collision(&fx, &defs, &HashSet::new());
    let baseline = derive_transports(fx.path(), &defs, &wc);
    fx.write(
        "scripts/ladders+stairs/scripts/ladders.rs2",
        "\
[oploc1,some_unknown_ladder]
p_arrivedelay;
~climb_ladder(movecoord(coord(), 0, 1, 0), true);
",
    );
    let graph = derive_transports(fx.path(), &defs, &wc);
    assert_eq!(
        graph.edges, baseline.edges,
        "an unresolved ladder script must not add a route"
    );
    // Other ungated explicit hops remain, while Shanks and gliders require
    // absent source proofs.
    let explicit = graph
        .edges
        .iter()
        .filter(|e| {
            e.kind == TransportKind::Boat
                || e.kind == TransportKind::Glider
                || e.kind == TransportKind::Npc
                || e.kind == TransportKind::Ladder
        })
        .count();
    assert_eq!(explicit, graph.edges.len());
    // 2 carts + the 5 essence-mine wizard entries + the 2 Elkoy maze
    // escorts.
    assert_eq!(
        graph
            .edges
            .iter()
            .filter(|e| e.kind == TransportKind::Npc)
            .count(),
        9
    );
    assert_eq!(
        graph
            .edges
            .iter()
            .filter(|e| e.kind == TransportKind::Glider)
            .count(),
        0
    );
}

#[test]
fn derive_transports_without_door_configs_emits_no_door_edges() {
    let fx = Fixture::new();
    fx.write("pack/loc.pack", "1530=loc_1530\n");
    fx.write(
        "maps/m44_53.jm2",
        "\
==== MAP ====
0 0 46: h1 o6 u50
==== LOC ====
0 0 46: 1530 0 1
",
    );
    let defs = loc_defs(&[(1530, 1, 1)]);
    let wc = bake_collision(&fx, &defs, &HashSet::new());
    let graph = derive_transports(fx.path(), &defs, &wc);
    assert!(graph.edges.iter().all(|e| e.kind != TransportKind::Door));
}

#[test]
fn derive_transports_emits_boat_edges_from_npc_tile_to_dock_tile() {
    let fx = Fixture::new();
    fx.write("pack/varp.pack", "116=zombiequeen\n");
    fx.write(
        "scripts/quests/configs/quest.varp",
        "[zombiequeen]\ntransmit=yes\n",
    );
    let defs = loc_defs(&[]);
    let wc = bake_collision(&fx, &defs, &HashSet::new());
    let graph = derive_transports(fx.path(), &defs, &wc);

    let boats: Vec<_> = graph
        .edges
        .iter()
        .filter(|e| e.kind == TransportKind::Boat)
        .collect();
    assert_eq!(boats.len(), 8);

    let boat = |npc: i32, at: WorldTile| -> &TransportEdge {
        boats
            .iter()
            .find(|e| e.loc_id == npc && e.at == at)
            .unwrap_or_else(|| panic!("boat route npc {npc} at {at:?}"))
    };

    // Port Sarim → Musa: Talk-to lands on the Karamja ship deck;
    // `sarimshipplank_off` (loc 2082) is a separate loc hop off the boat.
    let ps_musa = boat(
        378,
        WorldTile {
            x: 3026,
            z: 3217,
            level: 0,
        },
    );
    assert_eq!(
        ps_musa.to,
        WorldTile {
            x: 2956,
            z: 3143,
            level: 1
        }
    );
    assert_eq!(ps_musa.option, 1); // Talk-to
    assert_eq!(ps_musa.ticks, 7); // set_sail delay only
    assert_eq!(ps_musa.item_req, vec![(995, 30)]); // 30-coin fare
    assert!(ps_musa.varp_req.is_empty());
    let musa_plank = graph
        .edges
        .iter()
        .find(|e| e.kind == TransportKind::Ladder && e.loc_id == 2082)
        .expect("sarimshipplank_off");
    assert_eq!(
        musa_plank.at,
        WorldTile {
            x: 2956,
            z: 3144,
            level: 1
        }
    );
    assert_eq!(
        musa_plank.to,
        WorldTile {
            x: 2956,
            z: 3146,
            level: 0
        }
    );
    assert_eq!(musa_plank.option, 1); // Cross
    assert_eq!(musa_plank.ticks, GANGPLANK_TICKS);

    // Musa → Port Sarim: deck landing, then karamjashipplank_off 2084.
    let musa_ps = boat(
        380,
        WorldTile {
            x: 2955,
            z: 3146,
            level: 0,
        },
    );
    assert_eq!(
        musa_ps.to,
        WorldTile {
            x: 3032,
            z: 3217,
            level: 1
        }
    );
    assert_eq!(musa_ps.ticks, 7);
    assert!(graph
        .edges
        .iter()
        .any(|e| e.kind == TransportKind::Ladder && e.loc_id == 2084));

    // Sail hops land on the ship; Shanks is the exception (direct dock).
    let interiors = [
        (2956, 3143, 1),
        (3032, 3217, 1),
        (2683, 3268, 1),
        (2775, 3234, 1),
        (2834, 3331, 1),
        (3048, 3231, 1),
    ];
    for b in boats.iter().filter(|b| b.loc_id != 518) {
        assert!(
            interiors.contains(&(b.to.x, b.to.z, b.to.level)),
            "sail hop should land on the ship deck, got {:?}",
            b.to
        );
    }

    // Shilo boats (Captain Shanks, npc 518) carry the Shilo Village gate
    // and land directly on the dock (`set_sail_cairn`, no plank).
    let shanks: Vec<_> = boats.iter().filter(|e| e.loc_id == 518).collect();
    assert_eq!(shanks.len(), 2);
    for s in &shanks {
        assert_eq!(s.varp_req, vec![(116, 15)]);
        assert_eq!(s.option, 1);
        assert_eq!(s.to.level, 0);
    }
    let khazard = boat(
        518,
        WorldTile {
            x: 2763,
            z: 2961,
            level: 1,
        },
    );
    assert_eq!(
        khazard.to,
        WorldTile {
            x: 2680,
            z: 3150,
            level: 0
        }
    );
    assert_eq!(khazard.ticks, 9);
    let shanks_sarim = shanks
        .iter()
        .find(|s| {
            s.to == WorldTile {
                x: 3047,
                z: 3235,
                level: 0,
            }
        })
        .expect("Shilo → Port Sarim boat");
    assert_eq!(shanks_sarim.ticks, 15);
}

/// Slashable webs (`bigweb_slashable`) pack two edges per crossing: knife
/// `oplocu` (`option` 0, `item_req` knife — the knife is unequippable) and
/// `oploc1` Slash (`option` 1, `worn_req` every obj whose `.obj` block sets a
/// `slashattack_anim` other than the unarmed punch). Wilderness placements
/// pack too; `find` still gates wildy.
#[test]
fn derive_transports_emits_slashable_web_knife_edges() {
    let fx = Fixture::new();
    fx.write(
        "pack/loc.pack",
        "733=bigweb_slashable\n734=bigweb_slashed\n",
    );
    fx.write(
        "pack/obj.pack",
        "946=knife\n1277=bronze_sword\n1321=bronze_scimitar\n1265=bronze_pickaxe\n",
    );
    fx.write(
        "scripts/_unpack/225/all.obj",
        "\
[knife]
param=slashattack_anim,human_stab
[bronze_sword]
param=slashattack_anim,human_sword_slash
[bronze_scimitar]
param=slashattack_anim,human_scimitar_slash
[bronze_pickaxe]
param=slashattack_anim,human_unarmedpunch
",
    );
    // A Yanille web (m40_48) and a wilderness one (m47_56).
    fx.write(
        "maps/m40_48.jm2",
        "==== MAP ====\n0 0 0: h1\n==== LOC ====\n0 10 50: 733 0 0\n",
    );
    fx.write(
        "maps/m47_56.jm2",
        "==== MAP ====\n0 0 0: h1\n==== LOC ====\n0 20 20: 733 0 1\n",
    );
    let defs = loc_defs(&[(733, 1, 1), (734, 1, 1)]);
    let wc = bake_collision(&fx, &defs, &HashSet::from([733]));
    let graph = derive_transports(fx.path(), &defs, &wc);
    let webs: Vec<_> = graph
        .edges
        .iter()
        .filter(|e| e.kind == TransportKind::Door && e.loc_id == 733)
        .collect();
    let knife: Vec<_> = webs.iter().filter(|w| w.option == 0).collect();
    let slash: Vec<_> = webs.iter().filter(|w| w.option == 1).collect();
    assert_eq!(knife.len(), 4, "two placements, two crossings each");
    assert_eq!(slash.len(), 4, "one Slash per knife crossing");
    for w in &knife {
        assert_eq!(w.item_req, vec![(946, 1)], "unequippable knife: {w:?}");
        assert!(w.worn_req.is_empty(), "{w:?}");
        assert_eq!(w.open_loc_id, Some(734), "{w:?}");
        assert_eq!(w.ticks, WEB_TICKS);
        assert!(w.dir.is_some());
    }
    for w in &slash {
        assert!(w.item_req.is_empty(), "Slash is worn-blade, not inv: {w:?}");
        assert_eq!(w.worn_req, vec![1277, 1321], "slash blades only: {w:?}");
        assert_eq!(w.open_loc_id, Some(734), "{w:?}");
        assert_eq!(w.ticks, WEB_TICKS);
        assert!(w.dir.is_some());
    }
    assert!(
        webs.iter().any(|e| e.at.z >= 3520),
        "wilderness webs pack too"
    );
}

/// The Yanille dungeon balancing ledge (`balancing_ledge3` / loc 2303,
/// `area_yanille/scripts/agility_dungeon.rs2:30-34` `if(stat(agility) <
/// 40)`) packs one Walk-across hop per placement: the northern placement
/// (`coordz(loc_coord) > 9518`) walks south to `0_40_148_20_40`
/// (2580,9512), the southern one north to `0_40_148_20_48` (2580,9520).
#[test]
fn derive_transports_emits_yanille_balancing_ledge() {
    let fx = Fixture::new();
    fx.write("pack/loc.pack", "2303=balancing_ledge3\n");
    fx.write(
        "scripts/areas/area_yanille/scripts/agility_dungeon.rs2",
        "\
[oploc1,balancing_ledge3]
if(stat(agility) < 40) {
    ~mesbox(\"You need an Agility level of 40 to cross the ledge.\");
    return;
}
",
    );
    fx.write(
        "maps/m40_148.jm2",
        "==== MAP ====\n0 0 0: h1\n==== LOC ====\n0 20 47: 2303 10 0\n0 20 42: 2303 10 2\n",
    );
    let defs = loc_defs(&[(2303, 1, 1)]);
    let wc = bake_collision(&fx, &defs, &HashSet::new());
    let graph = derive_transports(fx.path(), &defs, &wc);
    let ledges: Vec<_> = graph
        .edges
        .iter()
        .filter(|e| e.kind == TransportKind::AgilityShortcut && e.loc_id == 2303)
        .collect();
    assert_eq!(ledges.len(), 2, "one hop per placement: {ledges:?}");
    for e in &ledges {
        assert_eq!(e.option, 1, "Walk-across: {e:?}");
        assert_eq!(e.skill_req, vec![(SKILL_AGILITY, 40)], "{e:?}");
        assert_eq!((e.at.x, e.to.x), (2580, 2580));
        assert!(e.item_req.is_empty());
    }
    let hop = |at_z| ledges.iter().find(|e| e.at.z == at_z).map(|e| e.to.z);
    assert_eq!(hop(9514), Some(9520), "the southern placement walks north");
    assert_eq!(hop(9519), Some(9512), "the northern placement walks south");
}

/// Live step 27: Yanille bank → dungeon warriors. Knife (web) +
/// Agility 40 (ledge). Empty WorldState stays NoPath.
#[test]
#[ignore = "NAV_CONTENT_ROOT and NAV_CACHE required; absence fails"]
fn yanille_bank_reaches_dungeon_warriors_with_knife_and_agility() {
    for (root, graph, collision) in qualification_worlds() {
        let _ = root;
        let bank = WorldTile {
            x: 2612,
            z: 3092,
            level: 0,
        };
        let warriors = WorldTile {
            x: 2580,
            z: 9501,
            level: 0,
        };
        assert!(
            crate::router::find_with(
                collision,
                graph,
                bank,
                warriors,
                crate::router::FindOptions::default(),
                &crate::world_state::WorldState::empty(),
            )
            .is_err(),
            "empty WorldState cannot take the knife web or the Agility-40 ledge"
        );
        let mut state = crate::world_state::WorldState::empty();
        state.inv.insert(946, 1);
        state.stats.insert(SKILL_AGILITY, 40);
        let route = crate::router::find_with(
            collision,
            graph,
            bank,
            warriors,
            crate::router::FindOptions::default(),
            &state,
        )
        .expect("Yanille bank → dungeon warriors with knife + Agility 40");
        let hops: Vec<_> = route
            .legs
            .iter()
            .filter_map(|l| match l {
                crate::router::Leg::Transport { edge } => {
                    Some((edge.kind, edge.loc_id, edge.at, edge.to, edge.option))
                }
                crate::router::Leg::Walk { .. } => None,
            })
            .collect();
        assert!(
            hops.iter()
                .any(|(_, loc_id, _, _, option)| *loc_id == 733 && *option == 0),
            "knife in inv takes the oplocu hop: {hops:?}"
        );
        let mut worn = crate::world_state::WorldState::empty();
        worn.stats.insert(SKILL_AGILITY, 40);
        worn.worn.insert(1277);
        let worn_route = crate::router::find_with(
            collision,
            graph,
            bank,
            warriors,
            crate::router::FindOptions::default(),
            &worn,
        )
        .expect("Yanille bank → dungeon warriors with a worn bronze sword");
        assert!(
            worn_route.legs.iter().any(|l| match l {
                crate::router::Leg::Transport { edge } => {
                    edge.loc_id == 733 && edge.option == 1
                }
                _ => false,
            }),
            "worn slash blade takes oploc1 Slash"
        );
        let walked_web = route.legs.iter().any(|l| match l {
            crate::router::Leg::Walk { tiles } => tiles.iter().any(|t| {
                t.level == 0
                    && t.x >= 2568
                    && t.x <= 2578
                    && t.z >= 3120
                    && t.z <= 3128
                    && graph.edges.iter().any(|e| {
                        e.loc_id == 733 && e.at.x == t.x && e.at.z == t.z && e.at.level == 0
                    })
            }),
            _ => false,
        });
        assert!(
            !walked_web,
            "walk must not step onto the web loc tile: {hops:?}"
        );
    }
}

/// The Yanille cellar stairs pack (in-town and outside-town mouths).
#[test]
#[ignore = "NAV_CONTENT_ROOT and NAV_CACHE required; absence fails"]
fn yanille_cellar_stairs_pack_to_the_dungeon() {
    for (root, graph, _wc) in qualification_worlds() {
        let _ = root;
        assert!(
            graph.edges.iter().any(|e| {
                e.kind == TransportKind::Stairs
                    && e.at
                        == WorldTile {
                            x: 2569,
                            z: 3122,
                            level: 0,
                        }
                    && e.to.z >= 9472
            }),
            "outside-town cellar stairs 2569,3122 → dungeon"
        );
        assert!(
            graph.edges.iter().any(|e| {
                e.kind == TransportKind::Stairs
                    && e.at
                        == WorldTile {
                            x: 2603,
                            z: 3078,
                            level: 0,
                        }
                    && e.to.z >= 9472
            }),
            "in-town cellar stairs 2603,3078 → dungeon"
        );
    }
}

#[test]
fn derive_transports_carries_quest_door_varp_req() {
    let fx = Fixture::new();
    fx.write("pack/loc.pack", "2526=elenagateshut\n4=mcannondoor1\n");
    fx.write("pack/varp.pack", "165=elenaquest\n0=mcannon\n");
    fx.write(
        "scripts/quests/configs/quest.varp",
        "[elenaquest]\ntransmit=yes\n[mcannon]\ntransmit=yes\n",
    );
    fx.write(
        "scripts/quests/quest_elena/configs/doors.loc",
        "\
[elenagateshut]
name=Door
model=basic_wall
active=yes
op1=Open
category=door_open_and_close
param=next_loc_stage,elenagateopen
",
    );
    fx.write(
        "scripts/quests/quest_elena/configs/quest_elena.constant",
        "^quest_elena_freed_elena = 28\n^elena_complete = 29\n",
    );
    fx.write(
            "scripts/quests/quest_elena/scripts/plaguehouse.rs2",
            "\
[oploc1,elenagateshut] // elena door
switch_int(%elenaquest) {
    case ^quest_elena_freed_elena, ^elena_complete : ~open_and_close_door(loc_param(next_loc_stage), ~check_axis(coord, loc_coord, loc_angle), false);
    case default : mes(\"The door is locked.\");
}
",
        );
    fx.write(
        "scripts/quests/quest_mcannon/configs/mcannon_doors.loc",
        "\
[mcannondoor1]
name=Door
model=basic_wall
op1=Open
category=door_closed
",
    );
    fx.write(
        "scripts/quests/quest_mcannon/configs/quest_mcannon.constant",
        "^mcannon_tasked_with_fixing_cannon = 6\n",
    );
    fx.write(
        "scripts/quests/quest_mcannon/scripts/mcannon_doors.rs2",
        "\
[oploc1,mcannondoor1]
if (%mcannon >= ^mcannon_tasked_with_fixing_cannon) {
    @open_dwarf_cannon_door;
} else {
    mes(\"The door is locked.\");
}

[label,open_dwarf_cannon_door]
~open_and_close_door(loc_param(next_loc_stage), true, false);
",
    );
    fx.write(
        "maps/m44_53.jm2",
        "\
==== MAP ====
0 0 0: h1 o6 u50
0 0 2: h1 o6 u50
0 1 0: h1 o6 u50
0 3 0: h1 o6 u50
==== LOC ====
0 0 1: 2526 0 2
0 2 0: 4 0 0
",
    );
    let defs = loc_defs(&[(2526, 1, 1), (4, 1, 1)]);
    let mut door_ids = HashSet::new();
    door_ids.extend([2526, 4]);
    let wc = bake_collision(&fx, &defs, &door_ids);
    let graph = derive_transports(fx.path(), &defs, &wc);

    // The Elena door (Plague City) carries `%elenaquest >= 28`
    // on both crossings, including the reverse back onto the loc tile.
    let elena: Vec<_> = graph
        .edges
        .iter()
        .filter(|e| e.kind == TransportKind::Door && e.loc_id == 2526)
        .collect();
    assert_eq!(elena.len(), 2, "both Elena door crossings: {elena:?}");
    assert_eq!(
        elena.iter().filter(|d| d.to == d.at).count(),
        1,
        "exactly the reverse crossing lands on the loc tile: {elena:?}"
    );
    for d in &elena {
        assert_eq!(d.varp_req, vec![(165, 28)]);
        assert!(d.quest_req.is_empty());
    }
    // The dwarf-cannon door's `if (%mcannon >= ^…) { @label }` gate.
    let cannon: Vec<_> = graph
        .edges
        .iter()
        .filter(|e| e.kind == TransportKind::Door && e.loc_id == 4)
        .collect();
    assert_eq!(cannon.len(), 2);
    for d in &cannon {
        assert_eq!(d.varp_req, vec![(0, 6)]);
    }
}

/// Tenzing's hut doors prove one free crossing each from the exact
/// check-axis / proc-bitfield open shape: 3745's exit (dir E, free
/// under `$leaving = true`) and 3746's entry from the north (dir S,
/// free under `$leaving = false`). The gated reverse crossings carry
/// completed `Death Plateau` (front entry W min 2, garden exit N min 7)
/// and never a raw varp-315 gate. The direct-varp castle door keeps both
/// crossings and its gate, and the crossing the free arm lands on follows
/// the placement angle, not the door id.
#[test]
fn derive_transports_emits_tenzing_free_door_arms() {
    let fx = Fixture::new();
    fx.write(
        "pack/loc.pack",
        "3743=death_castledoor\n3745=death_sherpa_door\n3746=death_sherpa_backdoor\n",
    );
    fx.write("pack/varp.pack", "314=death_equiproom\n315=death_map\n");
    fx.write(
        "scripts/quests/configs/quest.varp",
        "[death_equiproom]\ntransmit=yes\n",
    );
    fx.write(
        "scripts/quests/quest_death/configs/quest_death.loc",
        "\
[death_sherpa_door]
name=Door
active=yes
op1=Open

[death_sherpa_backdoor]
name=Door
active=yes
op1=Open

[death_castledoor]
name=Door
active=yes
op1=Open
",
    );
    fx.write(
        "scripts/quests/quest_death/configs/quest_death.constant",
        "\
^death_spoken_saba = 1
^death_spoken_tenzing = 2
^death_got_map = 7
^death_unlocked_door = 70
^death_map_lower = 0
^death_map_upper = 3
",
    );
    // The three `[oploc1,…]` blocks and the `death_get_map` proc
    // verbatim from the 289 and 274 content roots.
    fx.write(
            "scripts/quests/quest_death/scripts/quest_death.rs2",
            "\
[oploc1,death_sherpa_door]
def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
if($leaving = true | ~death_get_map >= ^death_spoken_tenzing) {
    ~open_and_close_door2(loc_1532, $leaving, door_open); // open door loc isnt in this version?
    return;
}
sound_synth(knock_knock, 1, 0);
~mesbox(\"You knock on the door.\");
if(npc_find(coord, death_sherpa, 5, 0) = true) {
    ~chatnpc(\"<p,angry>No milk today! Thank you!\");
}

[oploc1,death_sherpa_backdoor]
def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
if($leaving = false | ~death_get_map >= ^death_got_map) {
    ~open_and_close_door2(loc_1532, $leaving, door_open); // open door loc isnt in this version?
    return;
}
if(npc_find(coord, death_sherpa, 5, 0) = true) {
    ~chatnpc(\"<p,angry>Where do you think you're going? This is private property!\");
}

[oploc1,death_castledoor]
if(%death_equiproom >= ^death_unlocked_door) {
    ~open_and_close_door2(castledoor_inactive, ~check_axis(coord, loc_coord, loc_angle), door_open); // nicedoor_open
    return;
}
mes(\"The door is locked.\");

[proc,death_get_map]()(int)
return (getbit_range(%death_map, ^death_map_lower, ^death_map_upper));
",
        );
    // The 289/274 placements: 3745 at (2822,3555) angle 2, 3746 at
    // (2820,3557) angle 1, plus a second 3745 placement at angle 0
    // (mirrored) and the castle door.
    fx.write(
        "maps/m44_55.jm2",
        "\
==== MAP ====
0 6 35: h98 f4 u64
0 4 37: h98 f4 u64

==== LOC ====
0 6 35: 3745 0 2
0 4 37: 3746 0 1
0 6 10: 3745 0 0
0 4 10: 3743 0 2
",
    );
    let defs = loc_defs(&[(3743, 1, 1), (3745, 1, 1), (3746, 1, 1)]);
    let wc = bake_collision(&fx, &defs, &HashSet::new());
    let graph = derive_transports(fx.path(), &defs, &wc);

    assert_eq!(
        door_crossings(&graph, 3745),
        vec![
            ((2822, 3530), 'E', (2822, 3530)),
            ((2822, 3530), 'W', (2821, 3530)),
            ((2822, 3555), 'E', (2823, 3555)),
            ((2822, 3555), 'W', (2822, 3555)),
        ],
        "3745 emits the free `$leaving = true` crossing along the \
             placement angle and the gated reverse"
    );
    assert_eq!(
        door_crossings(&graph, 3746),
        vec![
            ((2820, 3557), 'N', (2820, 3558)),
            ((2820, 3557), 'S', (2820, 3557)),
        ],
        "3746 emits the free `$leaving = false` crossing into the hut \
             and the gated garden exit"
    );
    let free_3745 = [((2822, 3530), DoorDir::W), ((2822, 3555), DoorDir::E)];
    let gated_3745 = [((2822, 3530), DoorDir::E), ((2822, 3555), DoorDir::W)];
    for e in graph
        .edges
        .iter()
        .filter(|e| e.kind == TransportKind::Door && matches!(e.loc_id, 3745 | 3746))
    {
        assert_eq!(e.option, 1, "Open op: {e:?}");
        assert!(
            e.varp_req.is_empty()
                && e.item_req.is_empty()
                && e.worn_req.is_empty()
                && e.skill_req.is_empty(),
            "Tenzing hut doors never carry a raw varp/item/wear/skill gate: {e:?}"
        );
        let at = (e.at.x, e.at.z);
        let free = match e.loc_id {
            3745 => free_3745.contains(&(at, e.dir.unwrap())),
            3746 => e.dir == Some(DoorDir::S),
            _ => false,
        };
        let gated = match e.loc_id {
            3745 => gated_3745.contains(&(at, e.dir.unwrap())),
            3746 => e.dir == Some(DoorDir::N),
            _ => false,
        };
        if free {
            assert!(
                e.quest_req.is_empty(),
                "a proven free crossing carries no requirement: {e:?}"
            );
        } else if gated {
            assert_eq!(
                e.quest_req,
                vec!["Death Plateau".to_string()],
                "gated reverse requires completed Death Plateau: {e:?}"
            );
        } else {
            panic!("unexpected Tenzing crossing: {e:?}");
        }
    }
    // High unrelated raw 315 bits do not satisfy the gated reverse:
    // the mapping is the completed journal row, not varp 315.
    let gated = graph
        .edges
        .iter()
        .find(|e| e.loc_id == 3745 && e.dir == Some(DoorDir::W) && e.at.z == 3555)
        .expect("3745 W front entry");
    let empty = crate::world_state::WorldState::empty();
    assert!(!empty.allows(gated), "absent quest rejects the gated entry");
    let high_bits = crate::world_state::WorldState {
        varps: HashMap::from([(315, i32::MAX)]),
        ..crate::world_state::WorldState::empty()
    };
    assert!(
        !high_bits.allows(gated),
        "high raw 315 bits do not bypass the completed-quest gate"
    );
    let incomplete = crate::world_state::WorldState {
        quests: HashSet::from(["Imp Catcher".to_string()]),
        varps: HashMap::from([(315, i32::MAX)]),
        ..crate::world_state::WorldState::empty()
    };
    assert!(
        !incomplete.allows(gated),
        "an unrelated completed quest does not open the gated entry"
    );
    let done = crate::world_state::WorldState {
        quests: HashSet::from(["Death Plateau".to_string()]),
        ..crate::world_state::WorldState::empty()
    };
    assert!(
        done.allows(gated),
        "completed Death Plateau allows the gated entry"
    );
    // The direct-varp castle door is untouched: both crossings, its gate.
    let castle: Vec<_> = graph
        .edges
        .iter()
        .filter(|e| e.kind == TransportKind::Door && e.loc_id == 3743)
        .collect();
    assert_eq!(castle.len(), 2);
    for e in &castle {
        assert_eq!(e.varp_req, vec![(314, 70)]);
        assert!(e.quest_req.is_empty());
        assert!(
            !empty.allows(e),
            "the castle door still fails closed without varp 314"
        );
        let with_varp = crate::world_state::WorldState {
            varps: HashMap::from([(314, 70)]),
            ..crate::world_state::WorldState::empty()
        };
        assert!(with_varp.allows(e), "direct-varp castle gate unchanged");
    }
}

/// Doors whose open script only *almost* matches the supported free-arm
/// shape prove nothing and stay out of the pack — never ungated: a
/// malformed proc body, an unreachable proc, a raw `%varp` in the OR
/// head, an unresolved varp or constant, an arm that does not open, a
/// `<` compare, and a dialogue-only block.
#[test]
fn derive_transports_omits_unproven_directional_door_arms() {
    let fx = Fixture::new();
    fx.write(
        "pack/loc.pack",
        "5001=bad_proc_body\n5002=no_proc\n5003=raw_varp_or\n5004=no_open_arm\n\
             5005=lt_compare\n5006=dialogue_only\n5007=unknown_varp\n5008=unknown_constant\n",
    );
    fx.write("pack/varp.pack", "315=death_map\n");
    fx.write(
        "scripts/quests/quest_death/configs/quest_death.loc",
        "\
[bad_proc_body]
op1=Open
[no_proc]
op1=Open
[raw_varp_or]
op1=Open
[no_open_arm]
op1=Open
[lt_compare]
op1=Open
[dialogue_only]
op1=Open
[unknown_varp]
op1=Open
[unknown_constant]
op1=Open
",
    );
    fx.write(
        "scripts/quests/quest_death/configs/quest_death.constant",
        "^death_spoken_tenzing = 2\n^death_map_lower = 0\n^death_map_upper = 3\n",
    );
    fx.write(
        "scripts/quests/quest_death/scripts/quest_death.rs2",
        "\
[oploc1,bad_proc_body]
def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
if($leaving = true | ~death_get_map >= ^death_spoken_tenzing) {
    ~open_and_close_door2(loc_1532, $leaving, door_open);
    return;
}

[oploc1,no_proc]
def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
if($leaving = true | ~death_missing_proc >= ^death_spoken_tenzing) {
    ~open_and_close_door2(loc_1532, $leaving, door_open);
    return;
}

[oploc1,raw_varp_or]
def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
if($leaving = true | %death_map >= ^death_spoken_tenzing) {
    ~open_and_close_door2(loc_1532, $leaving, door_open);
    return;
}

[oploc1,no_open_arm]
def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
if($leaving = true | ~death_get_map >= ^death_spoken_tenzing) {
    ~mesbox(\"The door is stuck.\");
    return;
}

[oploc1,lt_compare]
def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
if($leaving = true | ~death_get_map < ^death_spoken_tenzing) {
    ~open_and_close_door2(loc_1532, $leaving, door_open);
    return;
}

[oploc1,dialogue_only]
def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
if(npc_find(coord, death_sherpa, 5, 0) = true) {
    ~chatnpc(\"<p,angry>This is private property!\");
}

[oploc1,unknown_varp]
def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
if($leaving = true | ~death_get_unknown >= ^death_spoken_tenzing) {
    ~open_and_close_door2(loc_1532, $leaving, door_open);
    return;
}

[oploc1,unknown_constant]
def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
if($leaving = true | ~death_get_map >= ^death_unknown_stage) {
    ~open_and_close_door2(loc_1532, $leaving, door_open);
    return;
}

[proc,death_get_map]()(int)
return (getbit_range(%death_map, ^death_map_lower, ^death_map_upper) + 1);

[proc,death_get_unknown]()(int)
return (getbit_range(%death_unknown_varp, ^death_map_lower, ^death_map_upper));
",
    );
    fx.write(
        "maps/m44_55.jm2",
        "\
==== MAP ====
0 6 35: h98 f4 u64

==== LOC ====
0 6 35: 5001 0 2
0 6 36: 5002 0 2
0 6 37: 5003 0 2
0 6 38: 5004 0 2
0 6 39: 5005 0 2
0 6 40: 5006 0 2
0 6 41: 5007 0 2
0 6 42: 5008 0 2
",
    );
    let sizes = [(5001, 1, 1), (5002, 1, 1), (5003, 1, 1), (5004, 1, 1)]
        .into_iter()
        .chain([(5005, 1, 1), (5006, 1, 1), (5007, 1, 1), (5008, 1, 1)])
        .collect::<Vec<_>>();
    let defs = loc_defs(&sizes);
    let wc = bake_collision(&fx, &defs, &HashSet::new());
    let graph = derive_transports(fx.path(), &defs, &wc);

    for id in 5001..=5008 {
        assert!(
            door_crossings(&graph, id).is_empty(),
            "loc {id} proves no free arm and must not be packed"
        );
        assert!(
            graph.edges.iter().all(|e| e.loc_id != id),
            "loc {id} must not be reachable through any other edge kind"
        );
    }
}

/// Extra conditions on an otherwise-valid canonical block/proc must not
/// prove a free arm: a wrapping outer `if`, an earlier conditional
/// return, a reassigned check-axis boolean, and an open call nested
/// inside the opening arm. The control loc uses the same valid
/// `death_get_map` proc as Tenzing and must still emit its free crossing,
/// so each negative is a refusal of extra conditions, not a dead pipeline.
#[test]
fn derive_transports_omits_extra_condition_directional_door_arms() {
    let fx = Fixture::new();
    fx.write(
        "pack/loc.pack",
        "5010=good_door\n5011=nested_outer\n5012=earlier_return\n\
             5013=reassigned_axis\n5014=nested_open\n",
    );
    fx.write("pack/varp.pack", "315=death_map\n");
    fx.write(
        "scripts/quests/quest_death/configs/quest_death.loc",
        "\
[good_door]
op1=Open
[nested_outer]
op1=Open
[earlier_return]
op1=Open
[reassigned_axis]
op1=Open
[nested_open]
op1=Open
",
    );
    fx.write(
        "scripts/quests/quest_death/configs/quest_death.constant",
        "^death_spoken_tenzing = 2\n^death_map_lower = 0\n^death_map_upper = 3\n",
    );
    fx.write(
        "scripts/quests/quest_death/scripts/quest_death.rs2",
        "\
[oploc1,good_door]
def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
if($leaving = true | ~death_get_map >= ^death_spoken_tenzing) {
    ~open_and_close_door2(loc_1532, $leaving, door_open);
    return;
}

[oploc1,nested_outer]
if(inv_total(inv, coins) > 0) {
    def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
    if($leaving = true | ~death_get_map >= ^death_spoken_tenzing) {
        ~open_and_close_door2(loc_1532, $leaving, door_open);
        return;
    }
}

[oploc1,earlier_return]
if(inv_total(inv, coins) = 0) {
    return;
}
def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
if($leaving = true | ~death_get_map >= ^death_spoken_tenzing) {
    ~open_and_close_door2(loc_1532, $leaving, door_open);
    return;
}

[oploc1,reassigned_axis]
def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
$leaving = false;
if($leaving = true | ~death_get_map >= ^death_spoken_tenzing) {
    ~open_and_close_door2(loc_1532, $leaving, door_open);
    return;
}

[oploc1,nested_open]
def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
if($leaving = true | ~death_get_map >= ^death_spoken_tenzing) {
    if(inv_total(inv, coins) > 0) {
        ~open_and_close_door2(loc_1532, $leaving, door_open);
        return;
    }
}

[proc,death_get_map]()(int)
return (getbit_range(%death_map, ^death_map_lower, ^death_map_upper));
",
    );
    fx.write(
        "maps/m44_55.jm2",
        "\
==== MAP ====
0 6 35: h98 f4 u64

==== LOC ====
0 6 35: 5010 0 2
0 6 36: 5011 0 2
0 6 37: 5012 0 2
0 6 38: 5013 0 2
0 6 39: 5014 0 2
",
    );
    let defs = loc_defs(&[
        (5010, 1, 1),
        (5011, 1, 1),
        (5012, 1, 1),
        (5013, 1, 1),
        (5014, 1, 1),
    ]);
    let wc = bake_collision(&fx, &defs, &HashSet::new());
    let graph = derive_transports(fx.path(), &defs, &wc);

    assert_eq!(
        door_crossings(&graph, 5010),
        vec![((2822, 3555), 'E', (2823, 3555))],
        "the valid control must still prove its free arm from the same proc"
    );
    for (id, why) in [
        (
            5011,
            "a wrapping outer if must not prove the inner free arm",
        ),
        (
            5012,
            "an earlier conditional return must not prove a later free arm",
        ),
        (
            5013,
            "a reassigned check-axis boolean must not prove a free arm",
        ),
        (
            5014,
            "an open nested inside the opening arm must not prove a free arm",
        ),
    ] {
        assert!(
            door_crossings(&graph, id).is_empty(),
            "loc {id} must not be packed: {why}"
        );
    }
}

/// Quoted text, an unrelated `~open_` name, `~open_overlay`, and an
/// open after `return` must not prove a free arm. The control loc uses
/// the canonical `~open_and_close_door2(loc_1532, $leaving, door_open)`
/// sequence and must still emit.
#[test]
fn derive_transports_omits_noncanonical_directional_door_openers() {
    let fx = Fixture::new();
    fx.write(
        "pack/loc.pack",
        "5020=good_open\n5021=quoted_open\n5022=open_overlay\n\
             5023=unrelated_open\n5024=open_after_return\n",
    );
    fx.write("pack/varp.pack", "315=death_map\n");
    fx.write(
        "scripts/quests/quest_death/configs/quest_death.loc",
        "\
[good_open]
op1=Open
[quoted_open]
op1=Open
[open_overlay]
op1=Open
[unrelated_open]
op1=Open
[open_after_return]
op1=Open
",
    );
    fx.write(
        "scripts/quests/quest_death/configs/quest_death.constant",
        "^death_spoken_tenzing = 2\n^death_map_lower = 0\n^death_map_upper = 3\n",
    );
    fx.write(
        "scripts/quests/quest_death/scripts/quest_death.rs2",
        "\
[oploc1,good_open]
def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
if($leaving = true | ~death_get_map >= ^death_spoken_tenzing) {
    ~open_and_close_door2(loc_1532, $leaving, door_open);
    return;
}

[oploc1,quoted_open]
def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
if($leaving = true | ~death_get_map >= ^death_spoken_tenzing) {
    mes(\\\"try ~open_and_close_door2(loc_1532, $leaving, door_open)\\\");
    return;
}

[oploc1,open_overlay]
def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
if($leaving = true | ~death_get_map >= ^death_spoken_tenzing) {
    ~open_overlay(overlay_door);
    return;
}

[oploc1,unrelated_open]
def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
if($leaving = true | ~death_get_map >= ^death_spoken_tenzing) {
    ~open_gate();
    return;
}

[oploc1,open_after_return]
def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
if($leaving = true | ~death_get_map >= ^death_spoken_tenzing) {
    return;
    ~open_and_close_door2(loc_1532, $leaving, door_open);
}

[proc,death_get_map]()(int)
return (getbit_range(%death_map, ^death_map_lower, ^death_map_upper));
",
    );
    fx.write(
        "maps/m44_55.jm2",
        "\
==== MAP ====
0 6 35: h98 f4 u64

==== LOC ====
0 6 35: 5020 0 2
0 6 36: 5021 0 2
0 6 37: 5022 0 2
0 6 38: 5023 0 2
0 6 39: 5024 0 2
",
    );
    let defs = loc_defs(&[
        (5020, 1, 1),
        (5021, 1, 1),
        (5022, 1, 1),
        (5023, 1, 1),
        (5024, 1, 1),
    ]);
    let wc = bake_collision(&fx, &defs, &HashSet::new());
    let graph = derive_transports(fx.path(), &defs, &wc);

    assert_eq!(
        door_crossings(&graph, 5020),
        vec![((2822, 3555), 'E', (2823, 3555))],
        "the valid control must still prove its canonical opener"
    );
    for (id, why) in [
        (5021, "a quoted open call must not prove a free arm"),
        (5022, "~open_overlay must not prove a free arm"),
        (5023, "an unrelated ~open_ name must not prove a free arm"),
        (5024, "an open after return must not prove a free arm"),
    ] {
        assert!(
            door_crossings(&graph, id).is_empty(),
            "loc {id} must not be packed: {why}"
        );
    }
}

/// Completed Death Plateau attaches only to the canonical sherpa loc
/// with `%death_map` bits 0..3 and the source polarity/threshold. An
/// unrelated varp, a different bit range, or the wrong polarity still
/// proves a free arm when the bitfield shape is valid, but must not
/// borrow the reverse. A duplicated proc proves nothing.
#[test]
fn derive_transports_omits_borrowed_death_plateau_reverse() {
    fn graph_for(
        script: &str,
        loc_pack: &str,
        varp_pack: &str,
        constants: &str,
        loc_line: &str,
    ) -> TransportGraph {
        let fx = Fixture::new();
        fx.write("pack/loc.pack", loc_pack);
        fx.write("pack/varp.pack", varp_pack);
        fx.write(
            "scripts/quests/quest_death/configs/quest_death.loc",
            "\
[death_sherpa_door]
op1=Open
[death_sherpa_backdoor]
op1=Open
[good_door]
op1=Open
",
        );
        fx.write(
            "scripts/quests/quest_death/configs/quest_death.constant",
            constants,
        );
        fx.write("scripts/quests/quest_death/scripts/quest_death.rs2", script);
        fx.write(
            "maps/m44_55.jm2",
            &format!(
                "\
==== MAP ====
0 6 35: h98 f4 u64

==== LOC ====
{loc_line}
"
            ),
        );
        let defs = loc_defs(&[(3745, 1, 1), (3746, 1, 1), (5010, 1, 1)]);
        let wc = bake_collision(&fx, &defs, &HashSet::new());
        derive_transports(fx.path(), &defs, &wc)
    }
    let constants = "\
^death_spoken_tenzing = 2
^death_got_map = 7
^death_map_lower = 0
^death_map_upper = 3
";
    let loc_pack = "3745=death_sherpa_door\n3746=death_sherpa_backdoor\n5010=good_door\n";
    let varps = "314=death_equiproom\n315=death_map\n";
    let control_script = "\
[oploc1,death_sherpa_door]
def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
if($leaving = true | ~death_get_map >= ^death_spoken_tenzing) {
    ~open_and_close_door2(loc_1532, $leaving, door_open);
    return;
}

[oploc1,good_door]
def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
if($leaving = true | ~death_get_map >= ^death_spoken_tenzing) {
    ~open_and_close_door2(loc_1532, $leaving, door_open);
    return;
}

[proc,death_get_map]()(int)
return (getbit_range(%death_map, ^death_map_lower, ^death_map_upper));
";
    let control = graph_for(
        control_script,
        loc_pack,
        varps,
        constants,
        "0 6 35: 3745 0 2\n0 6 36: 5010 0 2\n",
    );
    assert_eq!(
        door_crossings(&control, 3745),
        vec![
            ((2822, 3555), 'E', (2823, 3555)),
            ((2822, 3555), 'W', (2822, 3555)),
        ],
        "the sherpa control keeps the free exit and gated reverse"
    );
    let gated = control
        .edges
        .iter()
        .find(|e| e.loc_id == 3745 && e.dir == Some(DoorDir::W))
        .expect("3745 W");
    assert_eq!(gated.quest_req, vec!["Death Plateau".to_string()]);
    assert_eq!(
        door_crossings(&control, 5010),
        vec![((2822, 3556), 'E', (2823, 3556))],
        "an unrelated loc with the same shape must not be a dead pipeline"
    );
    assert!(
        control
            .edges
            .iter()
            .find(|e| e.loc_id == 5010)
            .is_some_and(|e| e.quest_req.is_empty()),
        "a non-sherpa loc must not borrow Death Plateau"
    );

    let wrong_varp = graph_for(
        "\
[oploc1,death_sherpa_door]
def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
if($leaving = true | ~death_get_map >= ^death_spoken_tenzing) {
    ~open_and_close_door2(loc_1532, $leaving, door_open);
    return;
}

[oploc1,good_door]
def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
if($leaving = true | ~death_get_map >= ^death_spoken_tenzing) {
    ~open_and_close_door2(loc_1532, $leaving, door_open);
    return;
}

[proc,death_get_map]()(int)
return (getbit_range(%death_equiproom, ^death_map_lower, ^death_map_upper));
",
        loc_pack,
        varps,
        constants,
        "0 6 35: 3745 0 2\n0 6 36: 5010 0 2\n",
    );
    assert_eq!(
        door_crossings(&wrong_varp, 3745),
        vec![((2822, 3555), 'E', (2823, 3555))],
        "a valid bitfield on the wrong varp still proves the free arm"
    );
    assert!(
        wrong_varp
            .edges
            .iter()
            .filter(|e| e.loc_id == 3745)
            .all(|e| e.quest_req.is_empty() && e.dir == Some(DoorDir::E)),
        "an unrelated varp must not borrow the Death Plateau reverse"
    );
    assert_eq!(
        door_crossings(&wrong_varp, 5010),
        vec![((2822, 3556), 'E', (2823, 3556))],
        "the control loc in the wrong-varp fixture must still emit"
    );

    let wrong_range = graph_for(
        "\
[oploc1,death_sherpa_door]
def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
if($leaving = true | ~death_get_map >= ^death_spoken_tenzing) {
    ~open_and_close_door2(loc_1532, $leaving, door_open);
    return;
}

[oploc1,good_door]
def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
if($leaving = true | ~good_get_map >= ^death_spoken_tenzing) {
    ~open_and_close_door2(loc_1532, $leaving, door_open);
    return;
}

[proc,death_get_map]()(int)
return (getbit_range(%death_map, ^death_map_lower, ^death_got_map));

[proc,good_get_map]()(int)
return (getbit_range(%death_map, ^death_map_lower, ^death_map_upper));
",
        loc_pack,
        varps,
        constants,
        "0 6 35: 3745 0 2\n0 6 36: 5010 0 2\n",
    );
    assert_eq!(
        door_crossings(&wrong_range, 3745),
        vec![((2822, 3555), 'E', (2823, 3555))],
        "bits 0..7 still prove a free arm"
    );
    assert!(
        wrong_range
            .edges
            .iter()
            .filter(|e| e.loc_id == 3745)
            .all(|e| e.quest_req.is_empty()),
        "a different bit range must not borrow Death Plateau"
    );
    assert_eq!(
        door_crossings(&wrong_range, 5010),
        vec![((2822, 3556), 'E', (2823, 3556))],
        "the 0..3 control must still emit beside the range negative"
    );

    let wrong_polarity = graph_for(
        "\
[oploc1,death_sherpa_door]
def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
if($leaving = false | ~death_get_map >= ^death_spoken_tenzing) {
    ~open_and_close_door2(loc_1532, $leaving, door_open);
    return;
}

[oploc1,good_door]
def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
if($leaving = true | ~death_get_map >= ^death_spoken_tenzing) {
    ~open_and_close_door2(loc_1532, $leaving, door_open);
    return;
}

[proc,death_get_map]()(int)
return (getbit_range(%death_map, ^death_map_lower, ^death_map_upper));
",
        loc_pack,
        varps,
        constants,
        "0 6 35: 3745 0 2\n0 6 36: 5010 0 2\n",
    );
    assert_eq!(
        door_crossings(&wrong_polarity, 3745),
        vec![((2822, 3555), 'W', (2822, 3555))],
        "the flipped polarity still proves its free crossing"
    );
    assert!(
        wrong_polarity
            .edges
            .iter()
            .filter(|e| e.loc_id == 3745)
            .all(|e| e.quest_req.is_empty()),
        "the wrong polarity must not borrow Death Plateau"
    );
    assert_eq!(
        door_crossings(&wrong_polarity, 5010),
        vec![((2822, 3556), 'E', (2823, 3556))],
        "the polarity control must still emit"
    );

    let duplicate = graph_for(
        "\
[oploc1,death_sherpa_door]
def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
if($leaving = true | ~death_get_map >= ^death_spoken_tenzing) {
    ~open_and_close_door2(loc_1532, $leaving, door_open);
    return;
}

[oploc1,good_door]
def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
if($leaving = true | ~good_get_map >= ^death_spoken_tenzing) {
    ~open_and_close_door2(loc_1532, $leaving, door_open);
    return;
}

[proc,death_get_map]()(int)
return (getbit_range(%death_map, ^death_map_lower, ^death_map_upper));

[proc,death_get_map]()(int)
return (getbit_range(%death_map, ^death_map_lower, ^death_map_upper));

[proc,good_get_map]()(int)
return (getbit_range(%death_map, ^death_map_lower, ^death_map_upper));
",
        loc_pack,
        varps,
        constants,
        "0 6 35: 3745 0 2\n0 6 36: 5010 0 2\n",
    );
    assert!(
        door_crossings(&duplicate, 3745).is_empty(),
        "a duplicated proc must not prove a free arm or borrow completion"
    );
    assert_eq!(
        door_crossings(&duplicate, 5010),
        vec![((2822, 3556), 'E', (2823, 3556))],
        "the unique-proc control must still emit beside the duplicate"
    );
}

/// The real Server content carries Tenzing's four crossings and the
/// castle's non-transmitted threshold is implied by the completed
/// Death Plateau journal row.
#[test]
#[ignore = "NAV_CONTENT_ROOT and NAV_CACHE required; absence fails"]
fn derive_transports_tenzing_free_arms_from_real_content() {
    for (root, graph, _wc) in qualification_worlds() {
        let _ = root;
        assert_eq!(
            door_crossings(graph, 3745),
            vec![
                ((2822, 3555), 'E', (2823, 3555)),
                ((2822, 3555), 'W', (2821, 3555)),
            ],
            "3745 free exit E and gated front entry W"
        );
        assert_eq!(
            door_crossings(graph, 3746),
            vec![
                ((2820, 3557), 'N', (2820, 3558)),
                ((2820, 3557), 'S', (2820, 3556)),
            ],
            "3746 gated garden exit N and free garden-to-hut S"
        );
        for e in graph.edges.iter().filter(|e| e.loc_id == 3745) {
            match e.dir {
                Some(DoorDir::E) => assert!(e.quest_req.is_empty() && e.varp_req.is_empty()),
                Some(DoorDir::W) => {
                    assert_eq!(e.quest_req, vec!["Death Plateau".to_string()]);
                    assert!(e.varp_req.is_empty());
                }
                other => panic!("unexpected 3745 dir {other:?}"),
            }
        }
        for e in graph.edges.iter().filter(|e| e.loc_id == 3746) {
            match e.dir {
                Some(DoorDir::S) => assert!(e.quest_req.is_empty() && e.varp_req.is_empty()),
                Some(DoorDir::N) => {
                    assert_eq!(e.quest_req, vec!["Death Plateau".to_string()]);
                    assert!(e.varp_req.is_empty());
                }
                other => panic!("unexpected 3746 dir {other:?}"),
            }
        }
        let castle: Vec<_> = graph
            .edges
            .iter()
            .filter(|e| e.kind == TransportKind::Door && e.loc_id == 3743)
            .collect();
        assert_eq!(castle.len(), 2);
        assert!(castle
            .iter()
            .all(|e| { e.varp_req.is_empty() && e.quest_req == ["Death Plateau"] }));
    }
}

#[test]
fn derive_transports_emits_glider_edges_from_platform_to_platform() {
    let fx = Fixture::new();
    fx.write(
        "scripts/areas/area_gnome/scripts/gnome_glider.rs2",
        "[opnpc1,gnomepilot]\nif(%grandtree = ^grandtree_complete & map_members = ^true) {\n    @multi3(\"Can you take me on the glider?\", gnome_pilot_glider);\n}\n",
    );
    fx.write(
        "scripts/general/scripts/quests.rs2",
        &format!("{JOURNAL_GREEN_SOURCE}~send_quest_progress_colour(questlist:grandtree, %grandtree, ^grandtree_complete);\n"),
    );
    fx.write(
        "scripts/general/configs/quest.constant",
        "^grandtree_complete=160\n",
    );
    fx.write(
        "scripts/player/interfaces/questlist.if",
        "[grandtree]\ntext=The Grand Tree\n",
    );
    let defs = loc_defs(&[]);
    let wc = bake_collision(&fx, &defs, &HashSet::new());
    let graph = derive_transports(fx.path(), &defs, &wc);

    let gliders: Vec<_> = graph
        .edges
        .iter()
        .filter(|e| e.kind == TransportKind::Glider)
        .collect();
    // The Grand Tree hub flies to all four pads and back from three of
    // them (`calc_glidervar` has no lemanto_andra → hub pair): 7
    // journal-gated flights, no non-transmitted varp alternative.
    assert_eq!(gliders.len(), 7);
    let hub = WorldTile {
        x: 2465,
        z: 3501,
        level: 3,
    };
    let sindarpos = WorldTile {
        x: 2850,
        z: 3497,
        level: 0,
    };
    let gandius = WorldTile {
        x: 2971,
        z: 2969,
        level: 0,
    };
    let lemanto_andra = WorldTile {
        x: 3320,
        z: 3430,
        level: 0,
    };
    let hub_edges: Vec<_> = gliders.iter().filter(|e| e.at == hub).collect();
    assert_eq!(hub_edges.len(), 4);
    assert!(hub_edges.iter().any(|e| e.to == sindarpos));
    assert!(hub_edges.iter().any(|e| e.to == gandius));
    assert!(hub_edges.iter().any(|e| e.to == lemanto_andra));
    let sindarpos_edges: Vec<_> = gliders.iter().filter(|e| e.at == sindarpos).collect();
    assert_eq!(sindarpos_edges.len(), 1);
    assert_eq!(sindarpos_edges[0].to, hub);
    // Lemanto Andra is one-way: no pad → hub flight exists in
    // gnome_glider.rs2.
    assert!(gliders.iter().all(|e| e.at != lemanto_andra));
    for g in &gliders {
        assert_eq!(g.option, 1, "Talk-to the Gnome pilot");
        assert_eq!(g.loc_id, 170);
        assert!(
            g.varp_req.is_empty(),
            "the server never transmits grandtree"
        );
        assert_eq!(g.quest_req, ["The Grand Tree"]);
        assert!(g.members_req, "the pilot's map_members guard is required");
    }
    fx.write(
        "scripts/areas/area_gnome/scripts/gnome_glider.rs2",
        "[opnpc1,gnomepilot]\nif(%grandtree = ^grandtree_complete) {\n    @multi3(\"Can you take me on the glider?\", gnome_pilot_glider);\n}\n",
    );
    let no_proof = derive_transports(fx.path(), &defs, &wc);
    assert!(
        no_proof
            .edges
            .iter()
            .all(|e| e.kind != TransportKind::Glider),
        "pilot with missing members proof emits no glider"
    );
}

/// Gandius pad → Grand Tree hub admits the visible quest journal row,
/// not the untransmitted `%grandtree` varp.
#[test]
#[ignore = "NAV_CONTENT_ROOT and NAV_CACHE required; absence fails"]
fn gandius_glider_reaches_grand_tree_hub_from_journal() {
    for (root, graph, collision) in qualification_worlds() {
        let _ = root;
        let pad = WorldTile {
            x: 2971,
            z: 2969,
            level: 0,
        };
        let hub = WorldTile {
            x: 2465,
            z: 3501,
            level: 3,
        };
        assert!(
            crate::router::find_with(
                collision,
                graph,
                pad,
                hub,
                crate::router::FindOptions::default(),
                &crate::world_state::WorldState::empty(),
            )
            .is_err(),
            "empty WorldState cannot take the Grand Tree glider"
        );
        let mut state = crate::world_state::WorldState::empty();
        state.varps.insert(150, 160);
        assert!(
            crate::router::find_with(
                collision,
                graph,
                pad,
                hub,
                crate::router::FindOptions::default(),
                &state,
            )
            .is_err(),
            "non-transmitted grandtree varp is not a live route proof"
        );
        let mut journal = crate::world_state::WorldState::empty();
        journal.quests.insert("The Grand Tree".into());
        assert!(
            crate::router::find_with(
                collision,
                graph,
                pad,
                hub,
                crate::router::FindOptions::default(),
                &journal,
            )
            .is_err(),
            "Gandius is F2P-world geometry, but the pilot still denies F2P"
        );
        journal.map_members = true;
        crate::router::find_with(
            collision,
            graph,
            pad,
            hub,
            crate::router::FindOptions::default(),
            &journal,
        )
        .expect("Gandius → Grand Tree hub with members and journal complete");
    }
}

#[test]
fn derive_transports_derives_spell_teleports_as_any_tile_edges() {
    let fx = Fixture::new();
    fx.write("pack/obj.pack", "554=firerune\n556=airrune\n563=lawrune\n");
    fx.write(
        "scripts/skill_magic/configs/magic_spells.dbrow",
        "\
[magic_spell_teleport_varrock]
table=magic_spell_table
data=spell,^varrock_teleport
data=members,false
data=levelrequired,25
data=runesrequired,firerune,1,airrune,3,lawrune,1
data=experience,350
data=tele_coord,0_50_53_13_32

[magic_spell_teleport_trollheim]
table=magic_spell_table
data=spell,^trollheim_teleport
data=members,true
data=levelrequired,61
data=runesrequired,firerune,2,lawrune,2,null,null
data=experience,680
data=tele_coord,0_45_57_10_31
",
    );
    let defs = loc_defs(&[]);
    let wc = bake_collision(&fx, &defs, &HashSet::new());
    let graph = derive_transports(fx.path(), &defs, &wc);

    assert_eq!(graph.teleports.len(), 2);
    // Teleports never join the `at`-indexed edge set.
    assert!(graph
        .edges
        .iter()
        .all(|e| e.kind != TransportKind::Teleport));
    assert!(!graph.at.contains_key(&TELEPORT_PLACEHOLDER_AT));

    let varrock = graph
        .teleports
        .iter()
        .find(|e| {
            e.to == WorldTile {
                x: 3213,
                z: 3424,
                level: 0,
            }
        })
        .expect("Varrock teleport");
    assert_eq!(varrock.kind, TransportKind::Teleport);
    assert_eq!(varrock.skill_req, vec![(SKILL_MAGIC, 25)]);
    assert_eq!(varrock.item_req, vec![(554, 1), (556, 3), (563, 1)]);
    assert_eq!(varrock.ticks, SPELL_TELEPORT_TICKS);

    let trollheim = graph
        .teleports
        .iter()
        .find(|e| {
            e.to == WorldTile {
                x: 2890,
                z: 3679,
                level: 0,
            }
        })
        .expect("Trollheim teleport");
    // The trailing `null,null` rune-slot padding is dropped.
    assert_eq!(trollheim.item_req, vec![(554, 2), (563, 2)]);
    assert_eq!(trollheim.skill_req, vec![(SKILL_MAGIC, 61)]);
    assert_eq!(trollheim.ticks, SPELL_TELEPORT_TICKS);
    // `data=members,true` is refused on an F2P world (`check_spell_requirements`).
    assert!(!varrock.members_req, "Varrock is a free spell");
    assert!(trollheim.members_req, "Trollheim is a members spell");
    let mut f2p = crate::world_state::WorldState::empty();
    f2p.stats.insert(SKILL_MAGIC, 99);
    f2p.inv.extend([(554, 10), (556, 10), (563, 10)]);
    assert!(f2p.allows(varrock));
    assert!(!f2p.allows(trollheim), "F2P cannot cast Trollheim");
    assert!(f2p.with_map_members(true).allows(trollheim));
}

#[test]
fn derive_transports_derives_jewellery_teleports_with_item_reqs() {
    let fx = Fixture::new();
    fx.write(
        "pack/obj.pack",
        "1712=amulet_of_glory_4\n2552=ring_of_dueling_8\n",
    );
    fx.write(
        "scripts/skill_magic/configs/enchanted_jewelry.obj",
        "\
[ring_of_dueling_8]
name=Ring of dueling(8)
iop4=Rub
category=category_136
param=charges,8
",
    );
    fx.write(
        "scripts/general/scripts/enchanted_jewellry/amulet_of_glory.rs2",
        "\
[opheld4,amulet_of_glory_4] @amulet_of_glory_interface(\"Your amulet has three charges left.\");
[label,amulet_of_glory_interface](string $message)
def_obj $item = last_item;
switch_int($choice) {
    case 1 : ~player_teleport_normal(0_48_54_15_40);
    case 2 : ~player_teleport_normal(0_45_49_38_40);
    case 3 : ~player_teleport_normal(0_48_50_33_51);
    case 4 : ~player_teleport_normal(0_51_49_29_27);
}
",
    );
    fx.write(
        "scripts/general/scripts/enchanted_jewellry/ring_of_dueling.rs2",
        "\
[opheld4,_category_136]
mes(\"You rub the ring...\");
p_delay(1);
~player_teleport_normal(map_findsquare(0_51_50_51_35, 0, 2, ^map_findsquare_lineofwalk));
",
    );
    let defs = loc_defs(&[]);
    let wc = bake_collision(&fx, &defs, &HashSet::new());
    let graph = derive_transports(fx.path(), &defs, &wc);

    // Glory: the charged `_4` stage forwards to the interface label,
    // whose four cases are the four destinations; each carries the
    // charged item as its requirement.
    let glory: Vec<_> = graph
        .teleports
        .iter()
        .filter(|e| e.loc_id == 1712)
        .collect();
    assert_eq!(glory.len(), 4);
    let dests: HashSet<WorldTile> = glory.iter().map(|e| e.to).collect();
    assert_eq!(dests.len(), 4);
    for e in &glory {
        assert_eq!(e.item_req, vec![(1712, 1)]);
        assert_eq!(e.ticks, JEWELLERY_TELEPORT_TICKS);
        assert_eq!(e.option, 4); // Rub (opheld4)
        assert!(e.skill_req.is_empty());
    }
    assert!(glory.iter().any(|e| e.to
        == WorldTile {
            x: 3087,
            z: 3496,
            level: 0
        })); // Edgeville
    assert!(glory.iter().any(|e| e.to
        == WorldTile {
            x: 3293,
            z: 3163,
            level: 0
        })); // Al Kharid
             // Packed sibling order is the dialog's choice order: the traveller
             // answers choice N with the N-th same-item rub (`dest_dialog_choice`),
             // so canonical ordering must not sort them by destination.
    let order: Vec<(i32, i32)> = glory.iter().map(|e| (e.to.x, e.to.z)).collect();
    assert_eq!(
        order,
        vec![(3087, 3496), (2918, 3176), (3105, 3251), (3293, 3163)]
    );

    // Dueling: the `_category_136` script applies to every
    // `category=category_136` obj in enchanted_jewelry.obj.
    let duel = graph
        .teleports
        .iter()
        .find(|e| e.loc_id == 2552)
        .expect("ring of dueling teleport");
    assert_eq!(
        duel.to,
        WorldTile {
            x: 3315,
            z: 3235,
            level: 0
        }
    );
    assert_eq!(duel.item_req, vec![(2552, 1)]);
    assert_eq!(duel.ticks, JEWELLERY_TELEPORT_TICKS);

    // The placeholder `at` never enters the `at` index.
    assert!(!graph.at.contains_key(&TELEPORT_PLACEHOLDER_AT));
}

/// Explicit real-content qualification inputs: `NAV_CONTENT_ROOT` names
/// one or more content roots (colon-separated) and `NAV_CACHE` the client
/// config jag of each (one entry for all roots, or one per root in the
/// same order). Ordinary `cargo test -p nav` never reads them; the ignored
/// tests below fail closed when the env is missing or a named path is
/// absent. Does not scan default HOME/experiments layouts or unrelated
/// worktrees.
fn required_qualification_roots() -> Vec<(PathBuf, LocDefs)> {
    let roots_raw = std::env::var("NAV_CONTENT_ROOT").unwrap_or_else(|_| {
        panic!(
            "NAV_CONTENT_ROOT is required (colon-separated content roots); \
                 this ignored qualification must not skip"
        )
    });
    let cache_raw = std::env::var("NAV_CACHE").unwrap_or_else(|_| {
        panic!(
            "NAV_CACHE is required (client config jag per root); \
                 this ignored qualification must not skip"
        )
    });
    let roots: Vec<PathBuf> = roots_raw
        .split(':')
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .collect();
    let caches: Vec<PathBuf> = cache_raw
        .split(':')
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .collect();
    assert!(
        !roots.is_empty(),
        "NAV_CONTENT_ROOT did not name any content root"
    );
    assert!(
        caches.len() == 1 || caches.len() == roots.len(),
        "NAV_CACHE names {} jags for {} roots",
        caches.len(),
        roots.len()
    );
    roots
        .into_iter()
        .enumerate()
        .map(|(k, root)| {
            assert!(
                root.join("maps").is_dir() && root.join("pack").join("loc.pack").is_file(),
                "NAV_CONTENT_ROOT entry {} is missing maps/ or pack/loc.pack",
                root.display()
            );
            let cache_path = &caches[k.min(caches.len() - 1)];
            let bytes = std::fs::read(cache_path).unwrap_or_else(|e| {
                panic!("NAV_CACHE {} is unreadable: {e}", cache_path.display());
            });
            let cache = Cache::unpack(&JagFile::new(bytes));
            (root, LocDefs::from_locs(&cache.locs))
        })
        .collect()
}

/// Every qualification root derived once with its loc defs, for the
/// `#[ignore]` tests that pin real-map geometry. They qualify the
/// production 289 tree (`NAV_CONTENT_ROOT=<lostcity-289>/content`,
/// `NAV_CACHE=<lostcity-289>/engine/data/pack/client/config`); the Tenzing,
/// membergate and keyed-hut ones also hold on the 274 tree.
type QualificationWorld = (PathBuf, TransportGraph, WorldCollision);
fn qualification_worlds() -> &'static [QualificationWorld] {
    static WORLDS: std::sync::OnceLock<Vec<QualificationWorld>> = std::sync::OnceLock::new();
    WORLDS.get_or_init(|| {
        required_qualification_roots()
            .into_iter()
            .map(|(root, defs)| {
                let (graph, wc) = derive_from_root_with(&root, &defs);
                (root, graph, wc)
            })
            .collect()
    })
}

fn derive_from_root_with(root: &Path, defs: &LocDefs) -> (TransportGraph, WorldCollision) {
    let wc = bake_from_maps(&root.join("maps"), defs, &HashSet::new())
        .unwrap_or_else(|e| panic!("qualification content bakes ({e:?}) {}", root.display()));
    let graph = derive_transports(root, defs, &wc);
    (graph, wc)
}

/// The Wilderness wolf pit on the real `m46_61` mapsquare alone: the pit
/// (3001,3923) walks round the east railings to the ridge approach
/// (2998,3916), while the ridge corridor (2998,3924) cannot walk south
/// through the `loc_2309` railings (recovery is from the pit after the
/// fall).
#[test]
#[ignore = "NAV_CONTENT_ROOT and NAV_CACHE required; absence fails"]
fn packed_wildy_wolf_pit_reaches_ridge_approach() {
    use crate::router::{find_with, FindOptions, RouteError};
    use crate::world_state::WorldState;
    for (root, defs) in required_qualification_roots() {
        let tmp = std::env::temp_dir().join(format!("wildy-pit-maps-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::copy(root.join("maps/m46_61.jm2"), tmp.join("m46_61.jm2")).unwrap();
        let collision = bake_from_maps(&tmp, &defs, &HashSet::new()).expect("bake m46_61");
        let _ = std::fs::remove_dir_all(&tmp);
        let graph = derive_transports(&root, &defs, &collision);
        let pit = WorldTile {
            x: 3001,
            z: 3923,
            level: 0,
        };
        let ridge = WorldTile {
            x: 2998,
            z: 3924,
            level: 0,
        };
        let approach = WorldTile {
            x: 2998,
            z: 3916,
            level: 0,
        };
        let opts = FindOptions {
            allow_teleports: false,
            allow_wilderness: true,
            allow_bank_fetch: true,
            ..FindOptions::default()
        };
        let state = WorldState::empty().with_map_members(true);
        find_with(&collision, &graph, pit, approach, opts, &state).unwrap_or_else(|e| {
            panic!("wolf pit (3001,3923) -> ridge approach (2998,3916) must walk around the east railings: {e:?}")
        });
        assert!(
            matches!(
                find_with(&collision, &graph, ridge, approach, opts, &state),
                Err(RouteError::NoPath)
            ),
            "the ridge corridor cannot walk south through loc_2309; recovery is from the pit after the fall"
        );
    }
}

/// The real 289 and 274 content must derive the closed fence-gate pair
/// behind Tenzing's passage (3725 `death_fencegate_l` at (2824,3555),
/// 3726 `death_fencegate_r` at (2824,3554)) as ordinary closed-gate
/// crossings: both members declare `category=gate_main_closed` /
/// `gate_outer_closed` with `op1=Open` in
/// `scripts/quests/quest_death/configs/quest_death.loc`, have no
/// loc-specific `[oploc1,…]` block anywhere, and inherit
/// `[oploc1,_gate_main_closed] ~open_gate;` /
/// `[oploc1,_gate_outer_closed] ~open_outer_gate;` from
/// `scripts/general_use/scripts/gates.rs2`. The Paterdomus pair
/// (memberfencegate_l/_r, loc 1598/1599) carries the same categories and
/// `op1=Open` but has loc-specific open scripts
/// (`scripts/areas/area_paterdomus/scripts/paterdomus_members_gate.rs2`,
/// the members gate), so it is never inherited as a free gate: only the
/// canonical members-check override admits it, members-only. The
/// previously supported `gates.loc` members keep their crossings.
#[test]
#[ignore = "NAV_CONTENT_ROOT and NAV_CACHE required; absence fails"]
fn derive_transports_tenzing_gate_pair_from_real_content() {
    for (root, graph, _) in qualification_worlds() {
        let doors = graph
            .edges
            .iter()
            .filter(|e| e.kind == TransportKind::Door)
            .count();
        eprintln!(
            "qualification {} edges={} doors={}",
            root.display(),
            graph.edges.len(),
            doors
        );
        assert_eq!(
            door_crossings(graph, 3725),
            vec![
                ((2824, 3555), 'E', (2825, 3555)),
                ((2824, 3555), 'W', (2823, 3555)),
            ],
            "3725 must cross both ways ({})",
            root.display()
        );
        assert_eq!(
            door_crossings(graph, 3726),
            vec![((2824, 3554), 'E', (2825, 3554))],
            "3726 keeps only its standable east crossing ({})",
            root.display()
        );
        let gates: Vec<_> = graph
            .edges
            .iter()
            .filter(|e| matches!(e.loc_id, 3725 | 3726))
            .collect();
        assert_eq!(gates.len(), 3, "({})", root.display());
        for e in gates {
            assert_eq!(e.option, 1, "{e:?} ({})", root.display());
            assert_eq!(
                e.open_loc_id,
                Some(if e.loc_id == 3725 { 3727 } else { 3728 }),
                "the stage leaf of {e:?} ({})",
                root.display()
            );
            assert!(
                e.varp_req.is_empty()
                    && e.quest_req.is_empty()
                    && e.item_req.is_empty()
                    && e.worn_req.is_empty()
                    && e.skill_req.is_empty(),
                "an inherited generic gate carries no requirement: {e:?} ({})",
                root.display()
            );
        }
        // The named-override members are not inherited as free gates: their
        // canonical members-check override admits them members-only.
        for id in [1598, 1599] {
            assert!(
                graph
                    .edges
                    .iter()
                    .filter(|e| e.loc_id == id)
                    .all(|e| e.members_req),
                "Paterdomus loc {id} crosses only on a members world ({})",
                root.display()
            );
        }
        // The generic `gates.loc` members keep their existing crossings.
        for id in [1551, 1553] {
            assert!(
                !door_crossings(graph, id).is_empty(),
                "generic fence gate loc {id} lost its crossings ({})",
                root.display()
            );
        }
    }
}

/// The derived fence-gate crossing must route the recorded start
/// (2823,3555, the hut's front-door passage) out to the road and back:
/// both legs hop loc 3725 through the fence. Incomplete / empty state
/// still cannot enter the hut (3745 W requires completed Death Plateau);
/// with that quest the recorded ClimbingBoots walk routes through 3745 W.
/// Hut → road stays free via 3745 E. 3746 S remains the garden return,
/// not a road entry.
#[test]
#[ignore = "NAV_CONTENT_ROOT and NAV_CACHE required; absence fails"]
fn tenzing_passage_and_road_route_through_the_inherited_gate() {
    use crate::router::{find_with, FindOptions, Leg};
    for (root, graph, wc) in qualification_worlds() {
        let state = crate::world_state::WorldState::empty();
        let passage = WorldTile {
            x: 2823,
            z: 3555,
            level: 0,
        };
        let road = WorldTile {
            x: 2826,
            z: 3556,
            level: 0,
        };
        for (label, from, to, dir) in [
            ("passage -> road", passage, road, DoorDir::E),
            ("road -> passage", road, passage, DoorDir::W),
        ] {
            let route = find_with(wc, graph, from, to, FindOptions::default(), &state)
                .unwrap_or_else(|e| panic!("{label} must route ({e:?})"));
            assert_eq!(route.dest, to, "{label} ({})", root.display());
            let hop = route
                .legs
                .iter()
                .find_map(|l| match l {
                    Leg::Transport { edge } if edge.loc_id == 3725 => Some(edge.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("{label} must hop the fence gate ({})", root.display()));
            assert_eq!(hop.dir, Some(dir), "{label} ({})", root.display());
            assert_eq!(
                (hop.at.x, hop.at.z),
                (2824, 3555),
                "{label} ({})",
                root.display()
            );
            assert_eq!(
                (hop.to.x, hop.to.z),
                if dir == DoorDir::E {
                    (2825, 3555)
                } else {
                    (2823, 3555)
                },
                "{label}: the landing on the crossing's far side ({})",
                root.display()
            );
        }
        // Incomplete state: the hut's front room stays sealed.
        let hut = WorldTile {
            x: 2820,
            z: 3556,
            level: 0,
        };
        assert!(
            find_with(wc, graph, passage, hut, FindOptions::default(), &state).is_err(),
            "passage -> hut stays NoPath without completed Death Plateau ({})",
            root.display()
        );
        let high_bits = crate::world_state::WorldState {
            varps: HashMap::from([(315, i32::MAX)]),
            ..crate::world_state::WorldState::empty()
        };
        assert!(
            find_with(wc, graph, passage, hut, FindOptions::default(), &high_bits).is_err(),
            "high raw 315 bits do not open 3745 W ({})",
            root.display()
        );
        let done = crate::world_state::WorldState {
            quests: HashSet::from(["Death Plateau".to_string()]),
            ..crate::world_state::WorldState::empty()
        };
        let entry = find_with(wc, graph, passage, hut, FindOptions::default(), &done)
            .unwrap_or_else(|e| {
                panic!(
                    "passage -> hut must route with completed Death Plateau ({e:?}) ({})",
                    root.display()
                )
            });
        let hop_3745 = entry
            .legs
            .iter()
            .find_map(|l| match l {
                Leg::Transport { edge } if edge.loc_id == 3745 => Some(edge.clone()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("passage -> hut must hop 3745 W ({})", root.display()));
        assert_eq!(hop_3745.dir, Some(DoorDir::W), "{}", root.display());
        assert_eq!(
            (hop_3745.at.x, hop_3745.at.z, hop_3745.to.x, hop_3745.to.z),
            (2822, 3555, 2821, 3555),
            "{}",
            root.display()
        );
        assert_eq!(
            hop_3745.quest_req,
            vec!["Death Plateau".to_string()],
            "{}",
            root.display()
        );
        // Hut -> road does not need the quest: free 3745 E then 3725 E.
        let exit =
            find_with(wc, graph, hut, road, FindOptions::default(), &state).unwrap_or_else(|e| {
                panic!(
                    "hut -> road must route without a quest ({e:?}) ({})",
                    root.display()
                )
            });
        assert!(
            exit.legs.iter().any(|l| matches!(
                l,
                Leg::Transport { edge } if edge.loc_id == 3745 && edge.dir == Some(DoorDir::E)
            )),
            "hut -> road hops 3745 E ({})",
            root.display()
        );
        assert!(
            exit.legs.iter().any(|l| matches!(
                l,
                Leg::Transport { edge } if edge.loc_id == 3725 && edge.dir == Some(DoorDir::E)
            )),
            "hut -> road hops the fence ({})",
            root.display()
        );
        // Taverley is south of the compound gates on the road toward
        // Falador; empty state can leave the passage. Falador's interior
        // pin is a separate city-wall problem — the fence is not the seal
        // once passage -> road routes.
        let taverley = WorldTile {
            x: 2895,
            z: 3435,
            level: 0,
        };
        find_with(
                wc,
                graph,
                passage,
                taverley,
                FindOptions::default(),
                &state,
            )
            .unwrap_or_else(|e| {
                panic!(
                    "passage -> Taverley must no longer be sealed by the missing compound gates ({e:?}) ({})",
                    root.display()
                )
            });
        find_with(wc, graph, taverley, hut, FindOptions::default(), &done).unwrap_or_else(|e| {
            panic!(
                "Taverley -> hut with completed Death Plateau ({e:?}) ({})",
                root.display()
            )
        });
        assert!(
            find_with(wc, graph, taverley, hut, FindOptions::default(), &state).is_err(),
            "Taverley -> hut stays NoPath without the quest ({})",
            root.display()
        );
        // 3746 S remains the garden return; N is the gated reverse.
        assert_eq!(
            door_crossings(graph, 3746),
            vec![
                ((2820, 3557), 'N', (2820, 3558)),
                ((2820, 3557), 'S', (2820, 3556)),
            ],
            "3746 keeps garden -> hut free and the gated reverse ({})",
            root.display()
        );
        let bank = WorldTile {
            x: 2946,
            z: 3369,
            level: 0,
        };
        for (label, from) in [("passage", passage), ("Taverley", taverley)] {
            match find_with(wc, graph, from, bank, FindOptions::default(), &state) {
                Ok(route) => eprintln!(
                    "{label} -> BANK_STAND(2946,3369) ok dest=({},{},{}) legs={} ({})",
                    route.dest.x,
                    route.dest.z,
                    route.dest.level,
                    route.legs.len(),
                    root.display()
                ),
                Err(e) => eprintln!(
                    "{label} -> BANK_STAND(2946,3369) {e:?} ({})",
                    root.display()
                ),
            }
        }
    }
}

fn membergate_pack() -> &'static str {
    "\
1596=membergatel
1597=membergater
1560=loc_1560
1561=loc_1561
"
}

fn membergate_loc_blocks() -> &'static str {
    "\
[membergatel]
name=Gate
desc=A wrought iron gate.
model=outdoorfurniture_metalgateclosedl
op1=Open
active=yes
blockrange=no
raiseobject=no
category=door_left_closed
param=next_loc_stage,loc_1560
param=open_sound,grate_open

[membergater]
name=Gate
desc=A wrought iron gate.
model=outdoorfurniture_metalgateclosedl
op1=Open
mirror=yes
active=yes
blockrange=no
raiseobject=no
category=door_right_closed
param=next_loc_stage,loc_1561
param=open_sound,grate_open

[loc_1560]
name=Gate
desc=A wrought iron gate.
model=outdoorfurniture_metalgateclosedl
op1=Close
active=yes
raiseobject=no
category=door_left_opened
param=next_loc_stage,loc_1557
param=close_sound,grate_close

[loc_1561]
name=Gate
desc=A wrought iron gate.
model=outdoorfurniture_metalgateclosedl
op1=Close
mirror=yes
active=yes
raiseobject=no
category=door_right_opened
param=next_loc_stage,loc_1558
param=close_sound,grate_close
"
}

fn membergate_handlers_text() -> &'static str {
    "\
[oploc1,membergatel]
if (map_members = ^false) {
    mes(^mes_members_gate);
    return;
}
~open_double_doors_left(500, door_right_closed, loc_param(open_sound));

[oploc1,membergater]
if (map_members = ^false) {
    mes(^mes_members_gate);
    return;
}
~open_double_doors_right(500, door_left_closed, loc_param(open_sound));

[proc,open_double_doors_left](int $duration, category $category, synth $sound)
return;

[proc,open_double_doors_right](int $duration, category $category, synth $sound)
return;
"
}

fn write_membergate_family(fx: &Fixture) {
    fx.write("pack/loc.pack", membergate_pack());
    fx.write(
        "scripts/doors/configs/doubledoors.loc",
        membergate_loc_blocks(),
    );
    fx.write(
        "scripts/doors/scripts/doubledoors.rs2",
        membergate_handlers_text(),
    );
}

fn parse_membergate_defs(
    text: &str,
) -> (
    HashMap<i32, MembergateDef>,
    HashMap<i32, MembergateOpenLeaf>,
    HashSet<i32>,
) {
    let fx = Fixture::new();
    fx.write("scripts/doors/configs/doubledoors.loc", text);
    membergate_loc_defs(
        fx.path(),
        &HashMap::from([
            (MEMBERGATE_LEFT.to_string(), 1596),
            (MEMBERGATE_RIGHT.to_string(), 1597),
            ("loc_1560".to_string(), 1560),
            ("loc_1561".to_string(), 1561),
        ]),
    )
}

#[test]
fn membergate_open_leaf_conflicts_are_order_independent_and_permanent() {
    let valid = "[loc_1560]\nop1=Close\ncategory=door_left_opened\n";
    let invalid = "[loc_1560]\nop1=Close\ncategory=other\n";
    for text in [
        format!("{valid}{invalid}"),
        format!("{invalid}{valid}"),
        format!("{valid}{invalid}{valid}"),
    ] {
        let (_, leaves, conflicted) = parse_membergate_defs(&text);
        assert!(conflicted.contains(&1560), "{text:?}");
        assert!(!leaves.contains_key(&1560), "{text:?}");
    }
}

#[test]
fn membergate_numeric_closed_alias_conflicts_with_named_definition() {
    let valid =
        "[membergatel]\nop1=Open\ncategory=door_left_closed\nparam=next_loc_stage,loc_1560\n";
    let invalid = "[loc_1596]\nop1=Close\ncategory=other\n";
    for text in [format!("{valid}{invalid}"), format!("{invalid}{valid}")] {
        let (defs, _, conflicted) = parse_membergate_defs(&text);
        assert!(conflicted.contains(&1596), "{text:?}");
        assert!(!defs.contains_key(&1596), "{text:?}");
    }
}

#[test]
fn membergate_relevant_keys_cannot_promote_an_invalid_block() {
    let text = "\
[membergatel]
op1=Close
op1=Open
category=other
category=door_left_closed
param=next_loc_stage,missing_leaf
param=next_loc_stage,loc_1560
";
    let (defs, _, _) = parse_membergate_defs(text);
    assert!(
        defs.get(&1596)
            .is_none_or(|def| !def.op_open || !def.category_ok || def.open.is_none()),
        "contradictory repeated keys must not produce a valid definition: {defs:?}"
    );
}

#[test]
fn membergate_identical_duplicate_definitions_remain_valid() {
    let text = format!("{}{}", membergate_loc_blocks(), membergate_loc_blocks());
    let (defs, leaves, conflicted) = parse_membergate_defs(&text);
    assert!(conflicted.is_empty(), "{conflicted:?}");
    assert_eq!(defs.len(), 2, "{defs:?}");
    assert_eq!(leaves.len(), 2, "{leaves:?}");
}

fn write_blocked_square(fx: &Fixture, mx: i32, mz: i32, walk: &[(i32, i32)], locs: &str) {
    let mut map = String::from("==== MAP ====\n");
    let ox = mx * 64;
    let oz = mz * 64;
    let walk: HashSet<(i32, i32)> = walk.iter().copied().collect();
    for lz in 0..64i32 {
        for lx in 0..64i32 {
            if !walk.contains(&(ox + lx, oz + lz)) {
                map.push_str(&format!("0 {lx} {lz}: f1 u48\n"));
            }
        }
    }
    map.push_str("\n==== LOC ====\n");
    map.push_str(locs);
    fx.write(&format!("maps/m{mx}_{mz}.jm2"), &map);
}

/// The Taverley pair keeps member-only crossings and rejects blocked landings.
#[test]
fn derive_transports_emits_membergate_family_crossings() {
    let fx = Fixture::new();
    write_membergate_family(&fx);
    fx.write(
        "maps/m45_53.jm2",
        "\
==== MAP ====
0 55 58: f1 u48

==== LOC ====
0 55 58: 1597 0 2
0 55 59: 1596 0 2
",
    );
    let defs = loc_defs(&[(1596, 1, 1), (1597, 1, 1), (1560, 1, 1), (1561, 1, 1)]);
    let wc = bake_collision(&fx, &defs, &HashSet::from([1596, 1597]));
    let graph = derive_transports(fx.path(), &defs, &wc);
    assert_eq!(
        door_crossings(&graph, 1596),
        vec![
            ((2935, 3451), 'E', (2936, 3451)),
            ((2935, 3451), 'W', (2935, 3451)),
        ],
        "1596 crosses both ways"
    );
    assert_eq!(
        door_crossings(&graph, 1597),
        vec![((2935, 3450), 'E', (2936, 3450))],
        "1597 cannot land back on its MAP-blocked loc tile"
    );
    for id in [1560, 1561] {
        assert!(
            door_crossings(&graph, id).is_empty(),
            "open leaf {id} is not a crossing"
        );
    }
    for e in graph
        .edges
        .iter()
        .filter(|e| matches!(e.loc_id, 1596 | 1597))
    {
        assert_eq!(e.option, 1, "{e:?}");
        assert!(e.members_req, "{e:?}");
        assert_eq!(
            e.open_loc_id,
            Some(if e.loc_id == 1596 { 1560 } else { 1561 }),
            "{e:?}"
        );
        assert!(
            e.skill_req.is_empty()
                && e.item_req.is_empty()
                && e.quest_req.is_empty()
                && e.varp_req.is_empty()
                && e.worn_req.is_empty(),
            "{e:?}"
        );
    }
    assert!(
        !crate::pack::parse_door_config(membergate_loc_blocks()).contains(&1596),
        "parse_door_config still ignores named membergate blocks"
    );
}

/// Empty WorldState cannot walk Taverley to BANK_STAND; map_members opens
/// the membergate hop. The corridor is sealed except through 1596/1597.
#[test]
fn membergate_routes_taverley_bank_only_when_map_members() {
    use crate::router::{find_with, FindOptions, Leg, RouteError};
    let fx = Fixture::new();
    write_membergate_family(&fx);
    let mut walk = Vec::new();
    for x in 2895..=2934 {
        walk.push((x, 3435));
    }
    for z in 3435..=3451 {
        walk.push((2934, z));
    }
    for z in 3450..=3451 {
        walk.push((2935, z));
        walk.push((2936, z));
    }
    for z in 3369..=3451 {
        walk.push((2936, z));
    }
    for x in 2936..=2946 {
        walk.push((x, 3369));
    }
    write_blocked_square(&fx, 45, 53, &walk, "0 55 58: 1597 0 2\n0 55 59: 1596 0 2\n");
    write_blocked_square(&fx, 45, 52, &walk, "");
    write_blocked_square(&fx, 46, 52, &walk, "");
    let defs = loc_defs(&[(1596, 1, 1), (1597, 1, 1), (1560, 1, 1), (1561, 1, 1)]);
    let wc = bake_collision(&fx, &defs, &HashSet::from([1596, 1597]));
    let graph = derive_transports(fx.path(), &defs, &wc);
    let taverley = WorldTile {
        x: 2895,
        z: 3435,
        level: 0,
    };
    let bank = WorldTile {
        x: 2946,
        z: 3369,
        level: 0,
    };
    let empty = crate::world_state::WorldState::empty();
    assert!(
        matches!(
            find_with(&wc, &graph, taverley, bank, FindOptions::default(), &empty),
            Err(RouteError::NoPath)
        ),
        "empty state cannot open the members gate"
    );
    assert!(
        matches!(
            find_with(&wc, &graph, bank, taverley, FindOptions::default(), &empty),
            Err(RouteError::NoPath)
        ),
        "empty reverse is also NoPath"
    );
    let members = empty.clone().with_map_members(true);
    let there = find_with(
        &wc,
        &graph,
        taverley,
        bank,
        FindOptions::default(),
        &members,
    )
    .expect("members world routes Taverley to bank");
    assert!(
        there.legs.iter().any(|l| matches!(
            l,
            Leg::Transport { edge } if matches!(edge.loc_id, 1596 | 1597) && edge.members_req
        )),
        "the hop is a membergate: {there:?}"
    );
    let back = find_with(
        &wc,
        &graph,
        bank,
        taverley,
        FindOptions::default(),
        &members,
    )
    .expect("members world routes bank to Taverley");
    assert!(back.legs.iter().any(|l| matches!(
        l,
        Leg::Transport { edge } if matches!(edge.loc_id, 1596 | 1597)
    )));
}

/// Near-misses stay out; a control pair in the same fixture is admitted.
#[test]
fn derive_transports_omits_unproven_membergate_members() {
    let fx = Fixture::new();
    fx.write(
        "pack/loc.pack",
        "\
1596=membergatel
1597=membergater
1560=loc_1560
1561=loc_1561
1598=memberfencegate_l
1599=memberfencegate_r
5001=plainopen
5002=unpaired_left
5003=unpaired_right
",
    );
    fx.write(
        "scripts/doors/configs/doubledoors.loc",
        &format!(
            "{}
[plainopen]
op1=Open
category=door_left_closed
param=next_loc_stage,loc_1560

[memberfencegate_l]
op1=Open
category=gate_main_closed
param=next_loc_stage,loc_1560

[memberfencegate_r]
op1=Open
category=gate_outer_closed
param=next_loc_stage,loc_1561
",
            membergate_loc_blocks()
        ),
    );
    fx.write(
        "scripts/doors/scripts/doubledoors.rs2",
        &format!(
            "{}
[oploc1,plainopen]
~open_double_doors_left(500, door_right_closed, loc_param(open_sound));

[oploc1,memberfencegate_l]
if(map_members = ^false) {{
    mes(^mes_members_gate);
    return;
}}
~open_gate;
",
            membergate_handlers_text()
        ),
    );
    fx.write(
        "maps/m45_53.jm2",
        "\
==== MAP ====
0 0 0: f1 u48

==== LOC ====
0 55 58: 1597 0 2
0 55 59: 1596 0 2
0 10 10: 5002 0 2
0 20 20: 1598 0 2
0 20 21: 1599 0 2
0 30 30: 5001 0 2
",
    );
    let defs = loc_defs(&[
        (1596, 1, 1),
        (1597, 1, 1),
        (1560, 1, 1),
        (1561, 1, 1),
        (1598, 1, 1),
        (1599, 1, 1),
        (5001, 1, 1),
        (5002, 1, 1),
    ]);
    let wc = bake_collision(&fx, &defs, &HashSet::new());
    let graph = derive_transports(fx.path(), &defs, &wc);
    assert!(
        !door_crossings(&graph, 1596).is_empty() && !door_crossings(&graph, 1597).is_empty(),
        "control Taverley pair is admitted"
    );
    for id in [1598, 1599, 5001, 5002] {
        assert!(
            graph.edges.iter().all(|e| e.loc_id != id),
            "unproven loc {id} must not emit"
        );
    }
}

/// A second named handler with a different body fail-closes the family.
#[test]
fn derive_transports_omits_membergate_handler_conflicts() {
    let fx = Fixture::new();
    write_membergate_family(&fx);
    fx.write(
        "scripts/areas/area_extra/scripts/extra.rs2",
        "[oploc1,membergatel] mes(^mes_members_gate);\n",
    );
    fx.write(
        "maps/m45_53.jm2",
        "\
==== MAP ====
0 55 58: f1 u48

==== LOC ====
0 55 58: 1597 0 2
0 55 59: 1596 0 2
",
    );
    let defs = loc_defs(&[(1596, 1, 1), (1597, 1, 1), (1560, 1, 1), (1561, 1, 1)]);
    let wc = bake_collision(&fx, &defs, &HashSet::new());
    let graph = derive_transports(fx.path(), &defs, &wc);
    assert!(
        door_crossings(&graph, 1596).is_empty() && door_crossings(&graph, 1597).is_empty(),
        "a conflicting extra handler fail-closes the family"
    );
}

fn find_radius3(
    wc: &WorldCollision,
    graph: &TransportGraph,
    from: WorldTile,
    to: WorldTile,
    state: &crate::world_state::WorldState,
) -> Result<crate::router::Route, crate::router::RouteError> {
    use crate::router::{find_with, local_step_component, FindOptions, RouteError};
    let mut dests: Vec<_> = local_step_component(wc, to, 3).into_iter().collect();
    dests.sort_by_key(|t| ((t.x - from.x).abs().max((t.z - from.z).abs()), t.x, t.z));
    let mut last = Err(RouteError::NoPath);
    for dest in dests {
        match find_with(wc, graph, from, dest, FindOptions::default(), state) {
            Ok(route) => return Ok(route),
            Err(e) => last = Err(e),
        }
    }
    last
}

/// Real 274+289 content: Taverley 1596/1597 crossings, empty-state
/// radius-3 bank remains NoPath, members world routes through the family.
/// 274config probes of 289 content are not fresh 289 qualification.
#[test]
#[ignore = "NAV_CONTENT_ROOT and NAV_CACHE required; absence fails"]
fn membergate_taverley_bank_from_real_content() {
    use crate::router::{Leg, RouteError};
    let taverley = WorldTile {
        x: 2895,
        z: 3435,
        level: 0,
    };
    let passage = WorldTile {
        x: 2823,
        z: 3555,
        level: 0,
    };
    let bank = WorldTile {
        x: 2946,
        z: 3369,
        level: 0,
    };
    for (root, graph, wc) in qualification_worlds() {
        let crossings_at = |id, x, z| {
            door_crossings(graph, id)
                .into_iter()
                .filter(|((at_x, at_z), _, _)| *at_x == x && *at_z == z)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            crossings_at(1596, 2935, 3451),
            vec![
                ((2935, 3451), 'E', (2936, 3451)),
                ((2935, 3451), 'W', (2934, 3451)),
            ],
            "1596 ({})",
            root.display()
        );
        assert_eq!(
            crossings_at(1597, 2935, 3450),
            vec![
                ((2935, 3450), 'E', (2936, 3450)),
                ((2935, 3450), 'W', (2934, 3450)),
            ],
            "1597 ({})",
            root.display()
        );
        let family: Vec<_> = graph
            .edges
            .iter()
            .filter(|e| matches!(e.loc_id, 1596 | 1597))
            .collect();
        let family_placements: HashSet<_> =
            family.iter().map(|edge| (edge.loc_id, edge.at)).collect();
        eprintln!(
            "membergate proof {}: family_edges={} family_placements={}",
            root.display(),
            family.len(),
            family_placements.len()
        );
        for e in &family {
            assert!(e.members_req, "{e:?} ({})", root.display());
            assert!(
                family.iter().any(|pair| {
                    pair.loc_id != e.loc_id
                        && pair.at.level == e.at.level
                        && pair.dir == e.dir
                        && (pair.at.x - e.at.x).abs() + (pair.at.z - e.at.z).abs() == 1
                }),
                "admitted family placement has no complementary pair: {e:?} ({})",
                root.display()
            );
        }
        for id in [1598, 1599] {
            assert!(
                graph
                    .edges
                    .iter()
                    .filter(|e| e.loc_id == id)
                    .all(|e| e.members_req),
                "Paterdomus {id} crosses only on a members world ({})",
                root.display()
            );
        }
        let empty = crate::world_state::WorldState::empty();
        for (label, from, to) in [
            ("Taverley -> bank", taverley, bank),
            ("bank -> Taverley", bank, taverley),
            ("passage -> bank", passage, bank),
            ("bank -> passage", bank, passage),
        ] {
            assert!(
                matches!(
                    find_radius3(wc, graph, from, to, &empty),
                    Err(RouteError::NoPath)
                ),
                "{label} unknown/false membership remains NoPath ({})",
                root.display()
            );
        }
        let members = empty.clone().with_map_members(true);
        let agility1 = crate::world_state::WorldState {
            stats: std::collections::HashMap::from([(16, 1)]),
            map_members: true,
            ..crate::world_state::WorldState::default()
        };
        let there = find_radius3(wc, graph, taverley, bank, &agility1)
            .unwrap_or_else(|e| panic!("members Taverley -> bank ({e:?}) ({})", root.display()));
        assert!(
            there.legs.iter().any(|l| matches!(
                l,
                Leg::Transport { edge } if matches!(edge.loc_id, 1596 | 1597)
            )),
            "Taverley -> bank hops membergate ({})",
            root.display()
        );
        let route_fact = |label: &str, route: &crate::router::Route| {
            let transports: Vec<_> = route
                .legs
                .iter()
                .filter_map(|leg| match leg {
                    Leg::Transport { edge } => Some(edge.loc_id),
                    _ => None,
                })
                .collect();
            eprintln!(
                "{label}: dest=({},{},{}) legs={} transports={transports:?} ({})",
                route.dest.x,
                route.dest.z,
                route.dest.level,
                route.legs.len(),
                root.display()
            );
        };
        route_fact("members Taverley -> bank radius3", &there);
        let back = find_radius3(wc, graph, bank, taverley, &members)
            .unwrap_or_else(|e| panic!("members bank -> Taverley ({e:?}) ({})", root.display()));
        route_fact("members bank -> Taverley radius3", &back);
        let done = crate::world_state::WorldState {
            quests: ["Death Plateau".to_string()].into(),
            stats: std::collections::HashMap::from([(16, 1)]),
            map_members: true,
            ..crate::world_state::WorldState::default()
        };
        let passage_to_bank = find_radius3(wc, graph, passage, bank, &done).unwrap_or_else(|e| {
            panic!(
                "passage -> bank with Death Plateau ({e:?}) ({})",
                root.display()
            )
        });
        route_fact(
            "members passage -> bank radius3 Death Plateau",
            &passage_to_bank,
        );
        let bank_to_passage = find_radius3(wc, graph, bank, passage, &done).unwrap_or_else(|e| {
            panic!(
                "bank -> passage with Death Plateau ({e:?}) ({})",
                root.display()
            )
        });
        route_fact(
            "members bank -> passage radius3 Death Plateau",
            &bank_to_passage,
        );
    }
}

/// The closed-gate inheritance is admitted from a quest config with the
/// real source shapes: the two category members, their `op1=Open`, the
/// named `next_loc_stage` leaves and the adjacent pair all resolve, and
/// the members cross both ways with no requirement (the generic
/// category handler opens them).
#[test]
fn derive_transports_emits_inherited_closed_gate_crossings() {
    let fx = Fixture::new();
    fx.write(
        "pack/loc.pack",
        "\
3725=death_fencegate_l
3726=death_fencegate_r
3727=death_openfencegate_l
3728=death_openfencegate_r
",
    );
    fx.write(
        "scripts/general_use/scripts/gates.rs2",
        "\
[proc,open_gate]
def_coord $main_open = ~movecoord_loc_return(~gate_set_close(loc_angle, 1));
return;

[proc,open_outer_gate]
loc_findallzone(~get_pair_coord(loc_coord, loc_angle, true));
return;

[oploc1,_gate_main_closed] ~open_gate;
[oploc1,_gate_outer_closed] ~open_outer_gate;
",
    );
    // Both closed members and both open leaves, verbatim shapes from
    // `scripts/quests/quest_death/configs/quest_death.loc`.
    fx.write(
        "scripts/quests/quest_death/configs/quest_death.loc",
        "\
[death_fencegate_l]
name=Gate
op1=Open
active=yes
blockrange=no
category=gate_main_closed
param=next_loc_stage,death_openfencegate_l

[death_fencegate_r]
name=Gate
op1=Open
active=yes
blockrange=no
mirror=yes
category=gate_outer_closed
param=next_loc_stage,death_openfencegate_r

[death_openfencegate_l]
name=Gate
op1=Close
active=yes
blockrange=no
category=gate_main_open
param=next_loc_stage,death_fencegate_l

[death_openfencegate_r]
name=Gate
op1=Close
active=yes
blockrange=no
mirror=yes
category=gate_outer_open
param=next_loc_stage,death_fencegate_r
",
    );
    // The m44_55 placements: 3726 at (2824,3554), 3725 at (2824,3555),
    // both angle 2, with (2823,3554) blocked exactly as the real map is.
    fx.write(
        "maps/m44_55.jm2",
        "\
==== MAP ====
0 7 34: f1 u48

==== LOC ====
0 8 34: 3726 0 2
0 8 35: 3725 0 2
",
    );
    let defs = loc_defs(&[(3725, 1, 1), (3726, 1, 1), (3727, 1, 1), (3728, 1, 1)]);
    let wc = bake_collision(&fx, &defs, &HashSet::new());
    let graph = derive_transports(fx.path(), &defs, &wc);

    assert_eq!(
        door_crossings(&graph, 3725),
        vec![
            ((2824, 3555), 'E', (2825, 3555)),
            ((2824, 3555), 'W', (2824, 3555)),
        ],
        "the main member crosses both ways"
    );
    assert_eq!(
        door_crossings(&graph, 3726),
        vec![
            ((2824, 3554), 'E', (2825, 3554)),
            ((2824, 3554), 'W', (2824, 3554)),
        ],
        "scenery behind the loc does not block the reverse onto the loc tile"
    );
    for id in [3727, 3728] {
        assert!(
            door_crossings(&graph, id).is_empty(),
            "the open leaf {id} is not a crossing"
        );
    }
    let mut leaves = HashSet::new();
    for e in graph
        .edges
        .iter()
        .filter(|e| matches!(e.loc_id, 3725 | 3726))
    {
        assert_eq!(e.option, 1, "Open op: {e:?}");
        assert!(
            e.varp_req.is_empty()
                && e.quest_req.is_empty()
                && e.item_req.is_empty()
                && e.worn_req.is_empty()
                && e.skill_req.is_empty(),
            "the inherited category handler carries no requirement: {e:?}"
        );
        leaves.insert(e.open_loc_id);
    }
    assert_eq!(
        leaves,
        HashSet::from([Some(3727), Some(3728)]),
        "each member carries its own stage leaf"
    );
}

/// Every near-miss below shares the closed categories with the admitted
/// pair but breaks one part of the inheritance: a loc-specific open
/// script (named override), a category with no verified handler, a
/// member without the `Open` op, an unresolvable `next_loc_stage`, a
/// member with no adjacent paired placement, and a member defined twice
/// with disagreeing data. None may be promoted, and an unrelated
/// `op1=Open` quest door is not admitted either. The valid control pair
/// (5071/5072) is admitted from the same fixture, so every negative
/// below is a refusal, not a dead pipeline. Only the main category
/// handler is verified here: the outer handler is missing entirely, so
/// the control's outer member (the pair the main needs) is exactly the
/// unsupported-handler case.
#[test]
fn derive_transports_omits_unproven_inherited_gate_members() {
    let fx = Fixture::new();
    fx.write(
        "pack/loc.pack",
        "\
5001=death_gate_override
5002=death_gate_override_outer
5003=death_gate_override_open
5004=death_gate_override_outer_open
5021=death_gate_noop
5025=death_gate_noop_open
5031=death_gate_nostage
5041=death_gate_unpaired
5045=death_gate_unpaired_open
5051=death_gate_conflict
5059=death_gate_conflict_open
5061=death_plainopen
5071=death_gate_control_main
5072=death_gate_control_outer
5073=death_gate_control_open
",
    );
    // Only the main category handler is verified here: the outer handler
    // is missing entirely, so `gate_outer_closed` has nothing to
    // inherit.
    fx.write(
        "scripts/general_use/scripts/gates.rs2",
        "\
[proc,open_gate]
return;

[oploc1,_gate_main_closed] ~open_gate;
",
    );
    fx.write(
        "scripts/quests/quest_neg/configs/neg.loc",
        "\
[death_gate_override]
op1=Open
category=gate_main_closed
param=next_loc_stage,death_gate_override_open

[death_gate_override_outer]
op1=Open
category=gate_outer_closed
param=next_loc_stage,death_gate_override_outer_open

[death_gate_noop]
op1=Climb
category=gate_main_closed
param=next_loc_stage,death_gate_noop_open

[death_gate_nostage]
op1=Open
category=gate_main_closed
param=next_loc_stage,death_gate_absent_leaf

[death_gate_unpaired]
op1=Open
category=gate_main_closed
param=next_loc_stage,death_gate_unpaired_open

[death_gate_conflict]
op1=Open
category=gate_main_closed
param=next_loc_stage,death_gate_conflict_open

[death_plainopen]
op1=Open

[death_gate_control_main]
op1=Open
category=gate_main_closed
param=next_loc_stage,death_gate_control_open

[death_gate_control_outer]
op1=Open
category=gate_outer_closed
param=next_loc_stage,death_gate_control_open

[death_gate_control_open]
op1=Close
category=gate_main_open
",
    );
    // The same member again, with a different category and stage.
    fx.write(
        "scripts/quests/quest_neg/configs/neg_again.loc",
        "\
[death_gate_conflict]
op1=Open
category=gate_outer_closed
param=next_loc_stage,death_gate_conflict_open
",
    );
    // The loc-specific open scripts the resolver prefers, and an
    // unrelated free-standing `Open` door.
    fx.write(
        "scripts/quests/quest_neg/scripts/neg.rs2",
        "\
[oploc1,death_gate_override]
mes(^mes_members_gate);
return;

[oploc1,death_gate_override_outer]
mes(^mes_members_gate);
return;

[oploc1,death_plainopen]
~open_and_close_door2(loc_1532, true, door_open);
return;
",
    );
    // Pairs sit at (x, z) + (x, z+1) for angle 2 (outer at z, main at
    // z+1, the `get_pair_coord` rule); the unpaired member has no
    // partner placement at all.
    fx.write(
        "maps/m44_53.jm2",
        "\
==== MAP ====
0 0 0: f0 u48

==== LOC ====
0 1 3: 5002 0 2
0 1 4: 5001 0 2
0 2 3: 5072 0 2
0 2 4: 5071 0 2
0 4 4: 5021 0 2
0 5 4: 5031 0 2
0 6 4: 5041 0 2
0 7 4: 5051 0 2
0 8 4: 5061 0 2
",
    );
    let defs = loc_defs(&[
        (5001, 1, 1),
        (5002, 1, 1),
        (5021, 1, 1),
        (5031, 1, 1),
        (5041, 1, 1),
        (5051, 1, 1),
        (5061, 1, 1),
        (5071, 1, 1),
        (5072, 1, 1),
    ]);
    let wc = bake_collision(&fx, &defs, &HashSet::new());
    let graph = derive_transports(fx.path(), &defs, &wc);

    // The control: the same shapes without any flaw do cross.
    assert_eq!(
        door_crossings(&graph, 5071),
        vec![
            ((2818, 3396), 'E', (2819, 3396)),
            ((2818, 3396), 'W', (2818, 3396)),
        ],
        "the valid control member must be admitted from the same fixture"
    );
    for (id, why) in [
        (5001, "the main member has a loc-specific open script"),
        (5002, "the outer member has a loc-specific open script"),
        (5072, "the outer category handler is missing"),
        (5021, "the member has no Open op"),
        (5031, "the next_loc_stage leaf does not resolve"),
        (5041, "the member has no paired placement"),
        (5051, "the member is defined twice with different data"),
        (5061, "an unrelated op1=Open door without a gate category"),
    ] {
        assert!(
            door_crossings(&graph, id).is_empty(),
            "loc {id} must not be promoted: {why}"
        );
    }
}

/// Unresolvable headers must end the previous block, `loc_N` aliases
/// must resolve through loc.pack, and a missing or mismatched open-leaf
/// category must refuse the member. The control pair in the same
/// fixture still crosses.
#[test]
fn derive_transports_omits_malformed_inherited_gate_blocks() {
    let fx = Fixture::new();
    fx.write(
        "pack/loc.pack",
        "\
5081=scan_control_main
5082=scan_control_outer
5083=scan_control_open
5084=scan_control_outer_open
5085=scan_leak_main
5086=scan_leak_outer
5087=scan_leak_open
5088=scan_leak_outer_open
5089=scan_missing_main
5090=scan_missing_outer
5091=scan_missing_open
5092=scan_mismatch_main
5093=scan_mismatch_outer
5094=scan_mismatch_open
5095=scan_bogus_main
5096=scan_bogus_outer
5097=scan_bogus_open
",
    );
    fx.write(
        "scripts/general_use/scripts/gates.rs2",
        "\
[proc,open_gate]
return;

[proc,open_outer_gate]
return;

[oploc1,_gate_main_closed] ~open_gate;
[oploc1,_gate_outer_closed] ~open_outer_gate;
",
    );
    fx.write(
        "scripts/quests/quest_scan/configs/scan.loc",
        "\
[scan_control_main]
op1=Open
category=gate_main_closed
param=next_loc_stage,scan_control_open

[scan_control_outer]
op1=Open
category=gate_outer_closed
param=next_loc_stage,scan_control_outer_open

[scan_control_open]
op1=Close
category=gate_main_open

[scan_control_outer_open]
op1=Close
category=gate_outer_open

[scan_leak_main]
op1=Open
category=gate_main_closed
param=next_loc_stage,scan_leak_open

[not_in_the_pack]
op1=Open
category=gate_outer_closed
param=next_loc_stage,scan_leak_outer_open

[scan_leak_outer]
op1=Open
category=gate_outer_closed
param=next_loc_stage,scan_leak_outer_open

[scan_leak_open]
op1=Close
category=gate_main_open

[scan_leak_outer_open]
op1=Close
category=gate_outer_open

[scan_missing_main]
op1=Open
category=gate_main_closed
param=next_loc_stage,scan_missing_open

[scan_missing_outer]
op1=Open
category=gate_outer_closed
param=next_loc_stage,scan_missing_open

[scan_mismatch_main]
op1=Open
category=gate_main_closed
param=next_loc_stage,scan_mismatch_open

[scan_mismatch_outer]
op1=Open
category=gate_outer_closed
param=next_loc_stage,scan_mismatch_open

[scan_mismatch_open]
op1=Close
category=gate_outer_open

[scan_bogus_main]
op1=Open
category=gate_main_closed
param=next_loc_stage,loc_9999

[scan_bogus_outer]
op1=Open
category=gate_outer_closed
param=next_loc_stage,scan_bogus_open

[scan_bogus_open]
op1=Close
category=gate_main_open
",
    );
    fx.write(
        "maps/m44_53.jm2",
        "\
==== MAP ====
0 0 0: f0 u48

==== LOC ====
0 1 3: 5082 0 2
0 1 4: 5081 0 2
0 2 3: 5086 0 2
0 2 4: 5085 0 2
0 3 3: 5090 0 2
0 3 4: 5089 0 2
0 4 3: 5093 0 2
0 4 4: 5092 0 2
0 5 3: 5096 0 2
0 5 4: 5095 0 2
",
    );
    let defs = loc_defs(&[
        (5081, 1, 1),
        (5082, 1, 1),
        (5085, 1, 1),
        (5086, 1, 1),
        (5089, 1, 1),
        (5090, 1, 1),
        (5092, 1, 1),
        (5093, 1, 1),
        (5095, 1, 1),
        (5096, 1, 1),
    ]);
    let wc = bake_collision(&fx, &defs, &HashSet::new());
    let graph = derive_transports(fx.path(), &defs, &wc);

    assert_eq!(
        door_crossings(&graph, 5081),
        vec![
            ((2817, 3396), 'E', (2818, 3396)),
            ((2817, 3396), 'W', (2817, 3396)),
        ],
        "the valid control must still be admitted"
    );
    assert_eq!(
        door_crossings(&graph, 5085),
        vec![
            ((2818, 3396), 'E', (2819, 3396)),
            ((2818, 3396), 'W', (2818, 3396)),
        ],
        "an unresolvable header must not steal the previous member"
    );
    for (id, why) in [
        (
            5089,
            "a missing open-leaf category config must not be admitted",
        ),
        (5092, "a mismatched open-leaf category must not be admitted"),
        (
            5095,
            "a loc_N alias that is not in loc.pack must not be admitted",
        ),
    ] {
        assert!(
            door_crossings(&graph, id).is_empty(),
            "loc {id} must not be promoted: {why}"
        );
    }
}

#[test]
fn inherited_gate_open_leaf_categories_fail_closed_on_conflicts_and_aliases() {
    let fx = Fixture::new();
    let ids = HashMap::from([
        ("control_main".to_string(), 6001),
        ("control_open".to_string(), 6002),
        ("duplicate_main".to_string(), 6011),
        ("duplicate_open".to_string(), 6012),
        ("conflict_main".to_string(), 6021),
        ("conflict_open".to_string(), 6022),
        ("alias_main".to_string(), 6031),
        ("alias_open".to_string(), 6032),
    ]);
    fx.write(
        "scripts/general_use/scripts/gates.rs2",
        "\
[proc,open_gate]
return;

[oploc1,_gate_main_closed] ~open_gate;
",
    );
    fx.write(
        "scripts/quests/quest_scan/configs/categories.loc",
        "\
[control_main]
op1=Open
category=gate_main_closed
param=next_loc_stage,control_open

[control_open]
category=gate_main_open

[duplicate_main]
op1=Open
category=gate_main_closed
param=next_loc_stage,duplicate_open

[duplicate_open]
category=gate_main_open

[loc_6012]
category=gate_main_open

[conflict_main]
op1=Open
category=gate_main_closed
param=next_loc_stage,conflict_open

[conflict_open]
category=gate_main_open

[conflict_open]
category=gate_outer_open

[conflict_open]
category=gate_main_open

[alias_main]
op1=Open
category=gate_main_closed
param=next_loc_stage,alias_open

[alias_open]
category=gate_main_open

[loc_6032]
category=gate_outer_open

[alias_open]
category=gate_main_open
",
    );

    let supported = generic_gate_handlers(fx.path());
    let mut skipped = HashMap::new();
    let inherited = inherited_closed_gates(
        fx.path(),
        &ids,
        &HashMap::new(),
        &supported,
        &HashMap::new(),
        &mut skipped,
    );

    assert_eq!(inherited.get(&6001), Some(&6002), "valid control");
    assert_eq!(
        inherited.get(&6011),
        Some(&6012),
        "repeating the same category, including through loc_N, remains valid"
    );
    assert_eq!(
        inherited.get(&6021),
        None,
        "a differing category permanently conflicts the named open leaf"
    );
    assert_eq!(
        inherited.get(&6031),
        None,
        "a differing loc_N alias permanently conflicts the same open leaf id"
    );
}

fn write_magicguild_source(fx: &Fixture, script: &str) {
    fx.write(
        "pack/loc.pack",
        "\
1600=magicguild_door_l
1601=magicguild_door_r
1522=loc_1522
1523=loc_1523
",
    );
    fx.write(
        "scripts/areas/area_yanille/configs/magic_guild/magic_guild.loc",
        "\
[magicguild_door_l]
name=Magic guild door
desc=The doors to the Magic Guild.
model=castle_doubledoorl
op1=Open
raiseobject=no
category=double_door_open_and_close_left
param=next_loc_stage,loc_1522
param=open_sound,null

[magicguild_door_r]
name=Magic guild door
desc=The doors to the Magic guild.
model=castle_doubledoorl
op1=Open
mirror=yes
raiseobject=no
category=double_door_open_and_close_right
param=next_loc_stage,loc_1523
param=open_sound,null
",
    );
    fx.write("scripts/areas/area_yanille/scripts/magic_guild.rs2", script);
}

fn magicguild_opener_script() -> &'static str {
    "\
[oploc1,magicguild_door_l] @open_mageguild_door(^left);
[oploc1,magicguild_door_r] @open_mageguild_door(^right);

[label,open_mageguild_door](int $side)
def_boolean $entering = ~check_axis_locactive(coord);
if($entering = true & stat(magic) < 66) {
    if(npc_find(coord, guild_wizard, 14, 0) = true) {
        ~chatnpc(\"<p,neutral>You need a magic level of 66.|The magical energy in here is unsafe for those below that level.\");
    }
    return;
}
~open_and_close_double_door2($entering, $side, door_open);
"
}

fn write_magicguild_yanille_placements(fx: &Fixture) {
    fx.write(
        "maps/m40_48.jm2",
        "\
==== MAP ====
0 23 15: h1 o6 u50
0 23 16: h1 o6 u50
0 24 15: h1 o6 u50
0 24 16: h1 o6 u50
0 25 15: h1 o6 u50
0 25 16: h1 o6 u50
0 36 15: h1 o6 u50
0 36 16: h1 o6 u50
0 37 15: h1 o6 u50
0 37 16: h1 o6 u50
0 38 15: h1 o6 u50
0 38 16: h1 o6 u50

==== LOC ====
0 24 15: 1601 0 2
0 24 16: 1600 0 2
0 37 15: 1600 0
0 37 16: 1601 0
",
    );
}

fn magic_state(level: i32) -> crate::world_state::WorldState {
    crate::world_state::WorldState {
        stats: HashMap::from([(SKILL_MAGIC, level)]),
        ..crate::world_state::WorldState::empty()
    }
}

/// Wizard Guild stairs from `stairs.rs2` + `m40_48.jm2` must already be
/// packed. If this fails, the Magic shop→bank hole is not door-only.
#[test]
#[ignore = "NAV_CONTENT_ROOT and NAV_CACHE required; absence fails"]
fn yanille_wizard_guild_stair_pairs_exist() {
    for (root, graph, _wc) in qualification_worlds() {
        let _ = root;
        let down = graph
            .edges
            .iter()
            .find(|e| {
                e.kind == TransportKind::Stairs
                    && e.loc_id == 1723
                    && e.at
                        == WorldTile {
                            x: 2590,
                            z: 3090,
                            level: 1,
                        }
            })
            .expect("1723 Climb-down at 2590,3090,1");
        assert_eq!(
            down.to,
            WorldTile {
                x: 2590,
                z: 3088,
                level: 0
            },
            "0_40_48_30_16 landing"
        );
        let up = graph
            .edges
            .iter()
            .find(|e| {
                e.kind == TransportKind::Stairs
                    && e.loc_id == 1722
                    && e.at
                        == WorldTile {
                            x: 2590,
                            z: 3089,
                            level: 0,
                        }
            })
            .expect("1722 Climb-up at 2590,3089,0");
        assert_eq!(
            up.to,
            WorldTile {
                x: 2590,
                z: 3092,
                level: 1
            },
            "1_40_48_30_20 landing"
        );
    }
}

/// Named-override guild doors are not inherited gates. The opener in
/// `magic_guild.rs2` admits `Open` hops at the four map tiles, and
/// `stat(magic) < 66` applies only on the entering (`check_axis` /
/// `door_open`) crossing.
#[test]
fn derive_transports_emits_magicguild_door_crossings() {
    let fx = Fixture::new();
    write_magicguild_source(&fx, magicguild_opener_script());
    write_magicguild_yanille_placements(&fx);
    let defs = loc_defs(&[(1600, 1, 1), (1601, 1, 1), (1522, 1, 1), (1523, 1, 1)]);
    let wc = bake_collision(&fx, &defs, &HashSet::from([1600, 1601]));
    let graph = derive_transports(fx.path(), &defs, &wc);
    assert_eq!(
        door_crossings(&graph, 1600),
        vec![
            ((2584, 3088), 'E', (2585, 3088)),
            ((2584, 3088), 'W', (2584, 3088)),
            ((2597, 3087), 'E', (2597, 3087)),
            ((2597, 3087), 'W', (2596, 3087)),
        ],
        "1600 at the west and east guild doors"
    );
    assert_eq!(
        door_crossings(&graph, 1601),
        vec![
            ((2584, 3087), 'E', (2585, 3087)),
            ((2584, 3087), 'W', (2584, 3087)),
            ((2597, 3088), 'E', (2597, 3088)),
            ((2597, 3088), 'W', (2596, 3088)),
        ],
        "1601 paired with 1600"
    );
    for e in graph
        .edges
        .iter()
        .filter(|e| matches!(e.loc_id, 1600 | 1601))
    {
        assert_eq!(e.kind, TransportKind::Door, "{e:?}");
        assert_eq!(e.option, 1, "stock Open {e:?}");
        assert_eq!(
            e.open_loc_id,
            Some(if e.loc_id == 1600 { 1522 } else { 1523 }),
            "{e:?}"
        );
        assert!(
            e.item_req.is_empty()
                && e.quest_req.is_empty()
                && e.varp_req.is_empty()
                && e.worn_req.is_empty()
                && !e.members_req,
            "{e:?}"
        );
        let entering = match (e.at.x, e.dir) {
            (2584, Some(DoorDir::E)) | (2597, Some(DoorDir::W)) => true,
            (2584, Some(DoorDir::W)) | (2597, Some(DoorDir::E)) => false,
            other => panic!("unexpected magicguild crossing {other:?}"),
        };
        if entering {
            assert_eq!(e.skill_req, vec![(SKILL_MAGIC, 66)], "enter {e:?}");
        } else {
            assert!(e.skill_req.is_empty(), "exit must stay ungated {e:?}");
        }
    }
}

/// A missing named opener must not invent skill-gated hops.
#[test]
fn derive_transports_omits_magicguild_doors_without_named_opener() {
    let fx = Fixture::new();
    write_magicguild_source(&fx, "");
    write_magicguild_yanille_placements(&fx);
    let defs = loc_defs(&[(1600, 1, 1), (1601, 1, 1), (1522, 1, 1), (1523, 1, 1)]);
    let wc = bake_collision(&fx, &defs, &HashSet::from([1600, 1601]));
    let graph = derive_transports(fx.path(), &defs, &wc);
    assert!(
        door_crossings(&graph, 1600).is_empty() && door_crossings(&graph, 1601).is_empty(),
        "no named override → no magicguild edges"
    );
}

/// WorldState magic gates entering only. Exiting a sealed corridor is
/// free; entering with magic 65 stays NoPath.
#[test]
fn magicguild_door_eligibility_follows_entering_axis() {
    use crate::router::{find_with, FindOptions, Leg, RouteError};
    let fx = Fixture::new();
    write_magicguild_source(&fx, magicguild_opener_script());
    let mut walk = Vec::new();
    for x in 2580..=2584 {
        walk.push((x, 3088));
    }
    for x in 2585..=2588 {
        walk.push((x, 3088));
    }
    write_blocked_square(&fx, 40, 48, &walk, "0 24 16: 1600 0 2\n0 24 15: 1601 0 2\n");
    let defs = loc_defs(&[(1600, 1, 1), (1601, 1, 1), (1522, 1, 1), (1523, 1, 1)]);
    let wc = bake_collision(&fx, &defs, &HashSet::from([1600, 1601]));
    let graph = derive_transports(fx.path(), &defs, &wc);
    let outside = WorldTile {
        x: 2583,
        z: 3088,
        level: 0,
    };
    let inside = WorldTile {
        x: 2585,
        z: 3088,
        level: 0,
    };
    let empty = crate::world_state::WorldState::empty();
    let low = magic_state(65);
    let ok = magic_state(66);
    assert!(
        matches!(
            find_with(&wc, &graph, outside, inside, FindOptions::default(), &empty),
            Err(RouteError::NoPath)
        ),
        "empty stats cannot enter"
    );
    assert!(
        matches!(
            find_with(&wc, &graph, outside, inside, FindOptions::default(), &low),
            Err(RouteError::NoPath)
        ),
        "magic 65 cannot enter"
    );
    let enter = find_with(&wc, &graph, outside, inside, FindOptions::default(), &ok)
        .expect("magic 66 enters");
    assert!(
        enter.legs.iter().any(|l| matches!(
            l,
            Leg::Transport { edge }
                if edge.loc_id == 1600
                    && edge.dir == Some(DoorDir::E)
                    && edge.skill_req == vec![(SKILL_MAGIC, 66)]
        )),
        "enter hops 1600 E with the parsed magic gate: {enter:?}"
    );
    let exit = find_with(&wc, &graph, inside, outside, FindOptions::default(), &empty)
        .expect("exit does not need magic");
    assert!(
        exit.legs.iter().any(|l| matches!(
            l,
            Leg::Transport { edge }
                if edge.loc_id == 1600
                    && edge.dir == Some(DoorDir::W)
                    && edge.skill_req.is_empty()
        )),
        "exit hops 1600 W ungated: {exit:?}"
    );
}

/// Live hole: floor-1 shop → Yanille bank is NoPath until the guild
/// doors join. After import, exiting stays eligible without magic;
/// entering the shop from the bank requires magic 66.
#[test]
#[ignore = "NAV_CONTENT_ROOT and NAV_CACHE required; absence fails"]
fn magic_guild_shop_bank_route_uses_derived_doors() {
    use crate::router::{find_with, FindOptions, Leg, RouteError};
    for (root, graph, wc) in qualification_worlds() {
        let _ = root;
        let shop = WorldTile {
            x: 2594,
            z: 3090,
            level: 1,
        };
        let bank = WorldTile {
            x: 2613,
            z: 3092,
            level: 0,
        };
        let doors: Vec<_> = graph
            .edges
            .iter()
            .filter(|e| e.kind == TransportKind::Door && matches!(e.loc_id, 1600 | 1601))
            .collect();
        assert_eq!(
            doors.len(),
            8,
            "four guild door tiles × two crossings, got {doors:?}"
        );
        let empty = crate::world_state::WorldState::empty();
        let low = magic_state(65);
        let ok = magic_state(66);
        let leave = find_with(wc, graph, shop, bank, FindOptions::default(), &empty)
            .unwrap_or_else(|e| panic!("shop → bank must exit without a magic gate ({e:?})"));
        assert_eq!(leave.dest, bank);
        assert!(
            leave.legs.iter().any(|l| matches!(
                l,
                Leg::Transport { edge } if edge.loc_id == 1723
            )),
            "shop → bank climbs down 1723: {leave:?}"
        );
        assert!(
            leave.legs.iter().any(|l| matches!(
                l,
                Leg::Transport { edge }
                    if matches!(edge.loc_id, 1600 | 1601) && edge.skill_req.is_empty()
            )),
            "shop → bank exits an ungated guild door: {leave:?}"
        );
        assert!(
            matches!(
                find_with(wc, graph, bank, shop, FindOptions::default(), &empty),
                Err(RouteError::NoPath)
            ),
            "empty stats cannot enter the guild"
        );
        assert!(
            matches!(
                find_with(wc, graph, bank, shop, FindOptions::default(), &low),
                Err(RouteError::NoPath)
            ),
            "magic 65 cannot enter the guild"
        );
        let enter = find_with(wc, graph, bank, shop, FindOptions::default(), &ok)
            .unwrap_or_else(|e| panic!("bank → shop with magic 66 ({e:?})"));
        assert_eq!(enter.dest, shop);
        assert!(
            enter.legs.iter().any(|l| matches!(
                l,
                Leg::Transport { edge }
                    if matches!(edge.loc_id, 1600 | 1601)
                        && edge.skill_req == vec![(SKILL_MAGIC, 66)]
            )),
            "bank → shop enters a magic-gated door: {enter:?}"
        );
        assert!(
            enter.legs.iter().any(|l| matches!(
                l,
                Leg::Transport { edge } if edge.loc_id == 1722
            )),
            "bank → shop climbs 1722: {enter:?}"
        );
    }
}

const RANGINGGUILD_OUTSIDE: WorldTile = WorldTile {
    x: 2657,
    z: 3439,
    level: 0,
};
const RANGINGGUILD_INSIDE: WorldTile = WorldTile {
    x: 2659,
    z: 3437,
    level: 0,
};
const RANGINGGUILD_LOC: WorldTile = WorldTile {
    x: 2658,
    z: 3438,
    level: 0,
};

fn write_rangingguild_source(fx: &Fixture, script: &str) {
    fx.write(
        "pack/loc.pack",
        "\
2514=ranging_guild_door
1532=loc_1532
",
    );
    fx.write(
        "scripts/minigames/game_ranging/configs/ranging.loc",
        "\
[ranging_guild_door]
name=Guild door
desc=The door to the Ranging Guild.
model=basic_wall
active=yes
op1=Open
",
    );
    fx.write(
        "scripts/minigames/game_ranging/scripts/ranging_guild_door.rs2",
        script,
    );
}

fn write_rangingguild_placement(fx: &Fixture, loc_lines: &str) {
    fx.write(
        "maps/m41_53.jm2",
        &format!(
            "\
==== MAP ====
0 33 47: h1 u50
0 34 46: h1 u50
0 35 45: h1 u50

==== LOC ====
{loc_lines}
"
        ),
    );
}

fn rangingguild_opener_script() -> &'static str {
    "\
[oploc1,ranging_guild_door]
if(coordx(coord) > coordx(loc_coord) | coordz(coord) < coordz(loc_coord)) {
    sound_synth(door_open, 1, 0);
    ~forcemove(movecoord(loc_coord, 1, 0, -1));
    loc_change(inviswall, 3);
    loc_add(movecoord($loc_coord, $x, 0, $z), loc_1532, modulo(add($angle, 1), 4), $shape, 3);
    p_teleport(movecoord(coord, -2, 0, 2));
    return;
}
if(stat(ranged) < 40) {
    return;
}
sound_synth(door_open, 1, 0);
~forcemove(movecoord(loc_coord, -1, 0, 1));
loc_change(inviswall, 3);
loc_add(movecoord($loc_coord, $x, 0, $z), loc_1532, modulo(add($angle, 1), 4), $shape, 3);
p_teleport(movecoord(coord, 2, 0, -2));
"
}

fn ranging_state(level: i32) -> crate::world_state::WorldState {
    crate::world_state::WorldState {
        stats: HashMap::from([(SKILL_RANGED, level)]),
        ..crate::world_state::WorldState::empty()
    }
}

fn rangingguild_doors(graph: &TransportGraph) -> Vec<&TransportEdge> {
    graph
        .edges
        .iter()
        .filter(|e| e.kind == TransportKind::Door && e.loc_id == 2514)
        .collect()
}

fn rangingguild_usable_from(graph: &TransportGraph, stand: WorldTile) -> Vec<&TransportEdge> {
    rangingguild_doors(graph)
        .into_iter()
        .filter(|e| {
            e.at.level == stand.level && (e.at.x - stand.x).abs().max((e.at.z - stand.z).abs()) <= 1
        })
        .collect()
}

fn assert_rangingguild_records(graph: &TransportGraph) {
    let doors = rangingguild_doors(graph);
    assert_eq!(doors.len(), 2, "exactly the reciprocal pair: {doors:?}");
    let enter = doors
        .iter()
        .find(|e| e.at == RANGINGGUILD_OUTSIDE && e.to == RANGINGGUILD_INSIDE)
        .unwrap_or_else(|| panic!("enter stand→landing missing: {doors:?}"));
    let exit = doors
        .iter()
        .find(|e| e.at == RANGINGGUILD_INSIDE && e.to == RANGINGGUILD_OUTSIDE)
        .unwrap_or_else(|| panic!("exit stand→landing missing: {doors:?}"));
    for (e, skill) in [
        (*enter, vec![(SKILL_RANGED, 40)]),
        (*exit, Vec::<(i32, i32)>::new()),
    ] {
        assert_eq!(e.option, 1, "{e:?}");
        assert_eq!(e.ticks, 1, "{e:?}");
        assert_eq!(e.dir, None, "{e:?}");
        assert_eq!(e.open_loc_id, None, "visual loc_1532 is not a leaf {e:?}");
        assert_eq!(e.skill_req, skill, "{e:?}");
        assert!(
            e.item_req.is_empty()
                && e.quest_req.is_empty()
                && e.varp_req.is_empty()
                && e.worn_req.is_empty()
                && !e.members_req,
            "{e:?}"
        );
    }
    assert_ne!(
        enter.at, RANGINGGUILD_LOC,
        "at must be the origin stand, not the loc tile"
    );
    assert_ne!(exit.at, RANGINGGUILD_LOC);
}

fn derive_rangingguild_fixture(script: &str, loc_lines: &str) -> TransportGraph {
    let fx = Fixture::new();
    write_rangingguild_source(&fx, script);
    write_rangingguild_placement(&fx, loc_lines);
    let defs = loc_defs(&[(2514, 1, 1), (1532, 1, 1)]);
    let wc = bake_collision(&fx, &defs, &HashSet::from([2514]));
    derive_transports(fx.path(), &defs, &wc)
}

/// Named-override diagonal wall: origin stands are the script
/// forcemove tiles, landings are the coord-relative teleports, enter
/// only carries Ranged 40, and loc_1532 stays a visual.
#[test]
fn derive_transports_emits_rangingguild_door_stand_teleport_pair() {
    let graph = derive_rangingguild_fixture(rangingguild_opener_script(), "0 34 46: 2514 9\n");
    assert_rangingguild_records(&graph);
}

/// Missing opener, missing/moved 40-check, inverted half-plane,
/// changed or swapped offset pairs, commented-out required forms,
/// extra contradictory calls, swapped forcemove/teleport order,
/// early exit return, a bare enter 40-check, a return between the
/// enter calls, shape 0, angle drift, or a second level-0 2514 must
/// emit nothing. Player-relative `p_teleport` cannot invent a `to`
/// the way [`parse_landing`] skips it.
#[test]
fn derive_transports_omits_unproven_rangingguild_door_forms() {
    let canonical = rangingguild_opener_script();
    let placement = "0 34 46: 2514 9\n";
    let cases = [
            ("missing opener", "", placement),
            (
                "missing ranged gate",
                &canonical.replace("if(stat(ranged) < 40) {\n    return;\n}\n", ""),
                placement,
            ),
            (
                "gate on exit",
                "\
[oploc1,ranging_guild_door]
if(coordx(coord) > coordx(loc_coord) | coordz(coord) < coordz(loc_coord)) {
    if(stat(ranged) < 40) {
        return;
    }
    ~forcemove(movecoord(loc_coord, 1, 0, -1));
    p_teleport(movecoord(coord, -2, 0, 2));
    return;
}
~forcemove(movecoord(loc_coord, -1, 0, 1));
p_teleport(movecoord(coord, 2, 0, -2));
",
                placement,
            ),
            (
                "inverted half-plane",
                &canonical.replace(
                    "coordx(coord) > coordx(loc_coord) | coordz(coord) < coordz(loc_coord)",
                    "coordx(coord) < coordx(loc_coord) | coordz(coord) > coordz(loc_coord)",
                ),
                placement,
            ),
            (
                "swapped forcemove offsets",
                &canonical
                    .replace("movecoord(loc_coord, 1, 0, -1)", "TMP_EXIT_FORCEMOVE")
                    .replace(
                        "movecoord(loc_coord, -1, 0, 1)",
                        "movecoord(loc_coord, 1, 0, -1)",
                    )
                    .replace("TMP_EXIT_FORCEMOVE", "movecoord(loc_coord, -1, 0, 1)"),
                placement,
            ),
            (
                "changed teleport offsets",
                &canonical.replace(
                    "p_teleport(movecoord(coord, -2, 0, 2))",
                    "p_teleport(movecoord(coord, 2, 0, -2))",
                ),
                placement,
            ),
            (
                "commented forcemove",
                &canonical.replace(
                    "~forcemove(movecoord(loc_coord, 1, 0, -1));",
                    "// ~forcemove(movecoord(loc_coord, 1, 0, -1));",
                ),
                placement,
            ),
            (
                "commented ranged gate",
                &canonical.replace(
                    "if(stat(ranged) < 40) {\n    return;\n}",
                    "// if(stat(ranged) < 40) {\n//     return;\n// }",
                ),
                placement,
            ),
            (
                "additional contradictory teleport",
                &canonical.replace(
                    "p_teleport(movecoord(coord, -2, 0, 2));",
                    "p_teleport(movecoord(coord, -2, 0, 2));\n    p_teleport(movecoord(coord, 2, 0, -2));",
                ),
                placement,
            ),
            (
                "player-relative landing without loc forcemove",
                "\
[oploc1,ranging_guild_door]
if(coordx(coord) > coordx(loc_coord) | coordz(coord) < coordz(loc_coord)) {
    p_teleport(movecoord(coord, -2, 0, 2));
    return;
}
if(stat(ranged) < 40) {
    return;
}
p_teleport(movecoord(coord, 2, 0, -2));
",
                placement,
            ),
            (
                "exit teleport before forcemove",
                "\
[oploc1,ranging_guild_door]
if(coordx(coord) > coordx(loc_coord) | coordz(coord) < coordz(loc_coord)) {
    p_teleport(movecoord(coord, -2, 0, 2));
    ~forcemove(movecoord(loc_coord, 1, 0, -1));
    return;
}
if(stat(ranged) < 40) {
    return;
}
~forcemove(movecoord(loc_coord, -1, 0, 1));
p_teleport(movecoord(coord, 2, 0, -2));
",
                placement,
            ),
            (
                "enter teleport before forcemove",
                "\
[oploc1,ranging_guild_door]
if(coordx(coord) > coordx(loc_coord) | coordz(coord) < coordz(loc_coord)) {
    ~forcemove(movecoord(loc_coord, 1, 0, -1));
    p_teleport(movecoord(coord, -2, 0, 2));
    return;
}
if(stat(ranged) < 40) {
    return;
}
p_teleport(movecoord(coord, 2, 0, -2));
~forcemove(movecoord(loc_coord, -1, 0, 1));
",
                placement,
            ),
            (
                "exit return before crossing",
                "\
[oploc1,ranging_guild_door]
if(coordx(coord) > coordx(loc_coord) | coordz(coord) < coordz(loc_coord)) {
    return;
    ~forcemove(movecoord(loc_coord, 1, 0, -1));
    p_teleport(movecoord(coord, -2, 0, 2));
    return;
}
if(stat(ranged) < 40) {
    return;
}
~forcemove(movecoord(loc_coord, -1, 0, 1));
p_teleport(movecoord(coord, 2, 0, -2));
",
                placement,
            ),
            (
                "enter ranged check without if-return gate",
                "\
[oploc1,ranging_guild_door]
if(coordx(coord) > coordx(loc_coord) | coordz(coord) < coordz(loc_coord)) {
    ~forcemove(movecoord(loc_coord, 1, 0, -1));
    p_teleport(movecoord(coord, -2, 0, 2));
    return;
}
stat(ranged)<40
~forcemove(movecoord(loc_coord, -1, 0, 1));
p_teleport(movecoord(coord, 2, 0, -2));
",
                placement,
            ),
            (
                "enter return between forcemove and teleport",
                "\
[oploc1,ranging_guild_door]
if(coordx(coord) > coordx(loc_coord) | coordz(coord) < coordz(loc_coord)) {
    ~forcemove(movecoord(loc_coord, 1, 0, -1));
    p_teleport(movecoord(coord, -2, 0, 2));
    return;
}
if(stat(ranged) < 40) {
    return;
}
~forcemove(movecoord(loc_coord, -1, 0, 1));
return;
p_teleport(movecoord(coord, 2, 0, -2));
",
                placement,
            ),
            ("shape 0", canonical, "0 34 46: 2514 0\n"),
            ("angle change", canonical, "0 34 46: 2514 9 1\n"),
            (
                "duplicate level-0 placement",
                canonical,
                "0 34 46: 2514 9\n0 35 45: 2514 9\n",
            ),
        ];
    for (label, script, loc_lines) in cases {
        let graph = derive_rangingguild_fixture(script, loc_lines);
        assert!(
            rangingguild_doors(&graph).is_empty(),
            "{label} must omit 2514, got {:?}",
            rangingguild_doors(&graph)
        );
    }
}

/// Reciprocal stands are Chebyshev 2 apart, so radius 1 admits only
/// the hop whose `at` is this stand. A shared `at=loc` pair would
/// expose both hops from either landing.
#[test]
fn rangingguild_door_radius1_admits_only_the_reciprocal_stand() {
    let graph = derive_rangingguild_fixture(rangingguild_opener_script(), "0 34 46: 2514 9\n");
    assert_rangingguild_records(&graph);
    let from_outside = rangingguild_usable_from(&graph, RANGINGGUILD_OUTSIDE);
    assert_eq!(from_outside.len(), 1, "{from_outside:?}");
    assert_eq!(from_outside[0].at, RANGINGGUILD_OUTSIDE);
    assert_eq!(from_outside[0].to, RANGINGGUILD_INSIDE);
    assert_eq!(from_outside[0].skill_req, vec![(SKILL_RANGED, 40)]);
    let from_inside = rangingguild_usable_from(&graph, RANGINGGUILD_INSIDE);
    assert_eq!(from_inside.len(), 1, "{from_inside:?}");
    assert_eq!(from_inside[0].at, RANGINGGUILD_INSIDE);
    assert_eq!(from_inside[0].to, RANGINGGUILD_OUTSIDE);
    assert!(from_inside[0].skill_req.is_empty());
    assert_eq!(
        (RANGINGGUILD_OUTSIDE.x - RANGINGGUILD_INSIDE.x)
            .abs()
            .max((RANGINGGUILD_OUTSIDE.z - RANGINGGUILD_INSIDE.z).abs()),
        2,
        "stands must stay outside INTERACT_RADIUS 1 of each other"
    );
}

#[test]
#[ignore = "NAV_CONTENT_ROOT and NAV_CACHE required; absence fails"]
fn derive_transports_rangingguild_door_pair_from_real_content() {
    for (root, graph, _wc) in qualification_worlds() {
        let _ = root;
        assert_rangingguild_records(graph);
        let from_outside = rangingguild_usable_from(graph, RANGINGGUILD_OUTSIDE);
        assert_eq!(from_outside.len(), 1, "{from_outside:?}");
        assert_eq!(from_outside[0].skill_req, vec![(SKILL_RANGED, 40)]);
        let from_inside = rangingguild_usable_from(graph, RANGINGGUILD_INSIDE);
        assert_eq!(from_inside.len(), 1, "{from_inside:?}");
        assert!(from_inside[0].skill_req.is_empty());
    }
}

/// Graph evidence only: Seers → JUDGE_STAND enters through 2514 at
/// the outside stand when Ranged is 70; empty / 39 stay NoPath.
#[test]
#[ignore = "NAV_CONTENT_ROOT and NAV_CACHE required; absence fails"]
fn ranging_guild_seers_judge_route_uses_derived_door() {
    use crate::router::{find_with, FindOptions, Leg, RouteError};
    for (root, graph, wc) in qualification_worlds() {
        let _ = root;
        let seers = WorldTile {
            x: 2722,
            z: 3493,
            level: 0,
        };
        let judge = WorldTile {
            x: 2670,
            z: 3418,
            level: 0,
        };
        let opts = FindOptions {
            allow_wilderness: true,
            allow_teleports: false,
            ..FindOptions::default()
        };
        let empty = crate::world_state::WorldState::empty();
        let low = ranging_state(39);
        let ok = ranging_state(70);
        assert!(
            matches!(
                find_with(wc, graph, seers, judge, opts, &empty),
                Err(RouteError::NoPath)
            ),
            "empty stats cannot enter"
        );
        assert!(
            matches!(
                find_with(wc, graph, seers, judge, opts, &low),
                Err(RouteError::NoPath)
            ),
            "ranged 39 cannot enter"
        );
        let enter = find_with(wc, graph, seers, judge, opts, &ok)
            .unwrap_or_else(|e| panic!("ranged 70 Seers → judge ({e:?})"));
        assert_eq!(enter.dest, judge);
        assert!(
            enter.legs.iter().any(|l| matches!(
                l,
                Leg::Transport { edge }
                    if edge.loc_id == 2514
                        && edge.at == RANGINGGUILD_OUTSIDE
                        && edge.to == RANGINGGUILD_INSIDE
                        && edge.skill_req == vec![(SKILL_RANGED, 40)]
                        && edge.dir.is_none()
                        && edge.open_loc_id.is_none()
            )),
            "enter hops 2514 at the outside stand: {enter:?}"
        );
    }
}

fn skip_total(skipped: &HashMap<&'static str, usize>, reason: &str) -> usize {
    *skipped.get(reason).unwrap_or(&0)
}

/// N1: missing required `ranging.loc` must bump, not silently omit 2514.
#[test]
fn n1_ranging_missing_loc_increments_skip_without_2514_edges() {
    let fx = Fixture::new();
    fx.write("pack/loc.pack", "2514=ranging_guild_door\n1532=loc_1532\n");
    fx.write(
        "scripts/minigames/game_ranging/scripts/ranging_guild_door.rs2",
        rangingguild_opener_script(),
    );
    write_rangingguild_placement(&fx, "0 34 46: 2514 9\n");
    let defs = loc_defs(&[(2514, 1, 1), (1532, 1, 1)]);
    let wc = bake_collision(&fx, &defs, &HashSet::from([2514]));
    let (graph, skipped) = derive_transports_with_skips(fx.path(), &defs, &wc);
    assert!(rangingguild_doors(&graph).is_empty());
    assert_eq!(
        skip_total(&skipped, SKIP_RANGINGGUILD_CONFIG),
        RANGINGGUILD_DECLARED_PAIR,
        "one skip unit is the enter/exit pair: {skipped:?}"
    );
}

#[test]
fn n1_valid_ranging_fixture_has_no_ranging_skip_reasons() {
    let fx = Fixture::new();
    write_rangingguild_source(&fx, rangingguild_opener_script());
    write_rangingguild_placement(&fx, "0 34 46: 2514 9\n");
    let defs = loc_defs(&[(2514, 1, 1), (1532, 1, 1)]);
    let wc = bake_collision(&fx, &defs, &HashSet::from([2514]));
    let (graph, skipped) = derive_transports_with_skips(fx.path(), &defs, &wc);
    assert_rangingguild_records(&graph);
    assert_eq!(skip_total(&skipped, SKIP_RANGINGGUILD_CONFIG), 0);
    assert_eq!(skip_total(&skipped, SKIP_RANGINGGUILD_SCRIPT), 0);
    assert_eq!(skip_total(&skipped, SKIP_RANGINGGUILD_PLACEMENT), 0);
    assert_eq!(skip_total(&skipped, SKIP_RANGINGGUILD_PACK), 0);
}

/// Pack-gated applicability: unrelated/empty fixtures must not pollute the
/// five-producer skip ledger; partial selection counts only packed names.
#[test]
fn n1_producer_skips_follow_packed_applicability() {
    let empty = Fixture::new();
    let defs = loc_defs(&[]);
    let wc = bake_collision(&empty, &defs, &HashSet::new());
    let (_, skipped) = derive_transports_with_skips(empty.path(), &defs, &wc);
    assert_eq!(skip_total(&skipped, SKIP_LEVER_SOURCE), 0);
    assert_eq!(skip_total(&skipped, SKIP_TOLL_OBJ_PACK), 0);
    assert_eq!(skip_total(&skipped, SKIP_TOLL_CONFIG), 0);
    assert_eq!(skip_total(&skipped, SKIP_MAGICGUILD_SCRIPT), 0);

    let fx = Fixture::new();
    fx.write("pack/loc.pack", "1814=wildinlever\n");
    let defs = loc_defs(&[(1814, 1, 1)]);
    let wc = bake_collision(&fx, &defs, &HashSet::new());
    let (graph, skipped) = derive_transports_with_skips(fx.path(), &defs, &wc);
    assert!(graph.edges.iter().all(|e| e.loc_id != 1814));
    assert_eq!(skip_total(&skipped, SKIP_LEVER_SOURCE), 1);

    let fx = Fixture::new();
    fx.write("pack/loc.pack", "1814=wildinlever\n1815=wildoutlever\n");
    let defs = loc_defs(&[(1814, 1, 1), (1815, 1, 1)]);
    let wc = bake_collision(&fx, &defs, &HashSet::new());
    let (_, skipped) = derive_transports_with_skips(fx.path(), &defs, &wc);
    assert_eq!(skip_total(&skipped, SKIP_LEVER_SOURCE), 2);

    let fx = Fixture::new();
    write_magicguild_source(&fx, "");
    fx.write(
        "pack/loc.pack",
        "1600=magicguild_door_l\n1601=magicguild_door_r\n1522=loc_1522\n1523=loc_1523\n",
    );
    fx.write(
        "scripts/areas/area_yanille/configs/magic_guild/magic_guild.loc",
        "\
[magicguild_door_l]
op1=Open
param=next_loc_stage,loc_1522

[magicguild_door_r]
op1=Open
param=next_loc_stage,loc_1523
",
    );
    write_magicguild_yanille_placements(&fx);
    let defs = loc_defs(&[(1600, 1, 1), (1601, 1, 1), (1522, 1, 1), (1523, 1, 1)]);
    let wc = bake_collision(&fx, &defs, &HashSet::from([1600, 1601]));
    let (graph, skipped) = derive_transports_with_skips(fx.path(), &defs, &wc);
    assert!(door_crossings(&graph, 1600).is_empty() && door_crossings(&graph, 1601).is_empty());
    assert_eq!(skip_total(&skipped, SKIP_MAGICGUILD_SCRIPT), 2);

    let fx = Fixture::new();
    write_magicguild_source(&fx, "");
    fx.write(
        "pack/loc.pack",
        "1600=magicguild_door_l\n1522=loc_1522\n1523=loc_1523\n",
    );
    fx.write(
        "scripts/areas/area_yanille/configs/magic_guild/magic_guild.loc",
        "\
[magicguild_door_l]
op1=Open
param=next_loc_stage,loc_1522
",
    );
    write_magicguild_yanille_placements(&fx);
    let defs = loc_defs(&[(1600, 1, 1), (1522, 1, 1), (1523, 1, 1)]);
    let wc = bake_collision(&fx, &defs, &HashSet::from([1600]));
    let (_, skipped) = derive_transports_with_skips(fx.path(), &defs, &wc);
    assert_eq!(skip_total(&skipped, SKIP_MAGICGUILD_SCRIPT), 1);
}

#[test]
fn n1_zanaris_missing_script_increments_skip_without_shed_edge() {
    let fx = Fixture::new();
    fx.write("pack/loc.pack", "2409=zanarisdoor\n");
    fx.write("pack/obj.pack", "772=dramen_staff\n");
    fx.write(
        "maps/m50_49.jm2",
        "\
==== MAP ====
0 20 56: h1 u50
==== LOC ====
0 20 56: 2409 0 0
",
    );
    let defs = loc_defs(&[(2409, 1, 1)]);
    let wc = bake_collision(&fx, &defs, &HashSet::from([2409]));
    let (graph, skipped) = derive_transports_with_skips(fx.path(), &defs, &wc);
    assert!(
        graph.edges.iter().all(|e| e.worn_req != vec![772]),
        "no dramen-gated shed hop"
    );
    assert_eq!(
        skip_total(&skipped, SKIP_ZANARIS_SOURCE),
        ZANARIS_DECLARED_ROUTES
    );
}

/// The Shantay henge doorway (loc 4031, the `shantay_pass.rs2`
/// `[oploc1,shantay_pass_henge_doorway]` branches) packs exactly two edges
/// per placement next to the Al Kharid toll gates: the gated hop into the
/// desert `at` the placement (one Shantay pass) and the free desert exit
/// `at` one tile south of it.
#[test]
fn shantay_henge_packs_the_paid_entry_and_the_free_exit() {
    let fx = Fixture::new();
    fx.write("pack/obj.pack", "995=coins\n1854=shantay_pass\n");
    fx.write(
        "pack/loc.pack",
        "4031=shantay_pass_henge_doorway\n2882=border_gate_toll_left\n1562=loc_1562\n",
    );
    fx.write(
        "scripts/areas/area_alkharid/configs/border_gate.loc",
        "[border_gate_toll_left]\nname=Gate\nop1=Open\ncategory=border_gate_toll_left\nparam=next_loc_stage,loc_1562\n",
    );
    fx.write(
        "maps/m51_48.jm2",
        "==== MAP ====\n0 38 44: h1 u50\n==== LOC ====\n0 38 44: 4031 10 0\n",
    );
    let defs = loc_defs(&[(4031, 1, 1), (2882, 1, 1)]);
    let wc = bake_collision(&fx, &defs, &HashSet::from([4031]));
    let graph = derive_transports(fx.path(), &defs, &wc);
    let henge: Vec<_> = graph.edges.iter().filter(|e| e.loc_id == 4031).collect();
    assert_eq!(henge.len(), 2, "{henge:?}");
    let gated = henge
        .iter()
        .find(|e| !e.item_req.is_empty())
        .expect("one Shantay henge edge carries the pass");
    let free = henge
        .iter()
        .find(|e| e.item_req.is_empty())
        .expect("one Shantay henge edge is free");
    assert_eq!(
        (gated.at, gated.to),
        (
            WorldTile {
                x: 3302,
                z: 3116,
                level: 0
            },
            SHANTAY_NORTH_TO
        )
    );
    assert_eq!(gated.item_req, [(1854, 1)]);
    assert_eq!(
        (free.at, free.to),
        (
            WorldTile {
                x: 3302,
                z: 3115,
                level: 0
            },
            SHANTAY_SOUTH_TO
        )
    );
    for e in henge {
        assert_eq!(e.option, 1, "Go-through op");
        assert_eq!(e.dir, None);
    }
}

#[test]
fn n1_toll_skips_config_henge_and_gate_placements_without_emitting() {
    let fx = Fixture::new();
    fx.write("pack/obj.pack", "995=coins\n1854=shantay_pass\n");
    fx.write("pack/loc.pack", "4031=shantay_pass_henge_doorway\n");
    fx.write(
        "maps/m51_48.jm2",
        "\
==== MAP ====
0 38 44: h1 u50
==== LOC ====
0 38 44: 4031 10 0
",
    );
    let defs = loc_defs(&[(4031, 1, 1)]);
    let wc = bake_collision(&fx, &defs, &HashSet::from([4031]));
    let (graph, skipped) = derive_transports_with_skips(fx.path(), &defs, &wc);
    assert!(
        graph
            .edges
            .iter()
            .all(|e| e.loc_id != 4031 && e.item_req != vec![(995, 10)]),
        "no toll or henge hops without border_gate.loc"
    );
    assert_eq!(skip_total(&skipped, SKIP_TOLL_CONFIG), 0);
    assert_eq!(skip_total(&skipped, SKIP_TOLL_HENGE), 1);

    let fx = Fixture::new();
    fx.write("pack/obj.pack", "995=coins\n1854=shantay_pass\n");
    fx.write(
        "pack/loc.pack",
        "2882=border_gate_toll_left\n2883=border_gate_toll_right\n1562=loc_1562\n1563=loc_1563\n",
    );
    fx.write(
        "scripts/areas/area_alkharid/configs/border_gate.loc",
        "\
[border_gate_toll_left]
name=Gate
op1=Open
category=border_gate_toll_left
param=next_loc_stage,loc_1562

[border_gate_toll_right]
name=Gate
op1=Open
category=border_gate_toll_right
param=next_loc_stage,loc_1563
",
    );
    let defs = loc_defs(&[(2882, 1, 1), (2883, 1, 1), (1562, 1, 1), (1563, 1, 1)]);
    let wc = bake_collision(&fx, &defs, &HashSet::from([2882, 2883]));
    let (graph, skipped) = derive_transports_with_skips(fx.path(), &defs, &wc);
    assert!(
        graph.edges.iter().all(|e| !matches!(e.loc_id, 2882 | 2883)),
        "no toll crossings without jm2 placements"
    );
    assert_eq!(skip_total(&skipped, SKIP_TOLL_GATE), 2);

    fx.write(
        "maps/m51_50.jm2",
        "\
==== MAP ====
0 4 27: h1 u50
==== LOC ====
0 4 27: 2882 0 0
",
    );
    let wc = bake_collision(&fx, &defs, &HashSet::from([2882, 2883]));
    let (graph, skipped) = derive_transports_with_skips(fx.path(), &defs, &wc);
    assert!(
        graph.edges.iter().any(|e| e.loc_id == 2882),
        "left gate emits when placed"
    );
    assert_eq!(
        skip_total(&skipped, SKIP_TOLL_GATE),
        1,
        "right gate still missing"
    );
    assert!(
        graph
            .edges
            .iter()
            .filter(|e| e.loc_id == 2882)
            .all(|e| { e.item_req == vec![(995, 10)] && e.varp_req.is_empty() }),
        "a missing waiver script must leave only paid crossings"
    );
    fx.write("pack/varp.pack", "419=princequest\n");
    fx.write(
        "scripts/quests/quest_prince/configs/quest_prince.constant",
        "^prince_saved = 73\n",
    );
    fx.write(
        "scripts/areas/area_alkharid/scripts/border_gate.rs2",
        "[label,talk_to_border_guard](coord $loc_coord)\nif (%princequest >= ^prince_saved) {\n    @pass_toll_gate($loc_coord);\n}\n",
    );
    fx.write(
        "scripts/general/scripts/quests.rs2",
        &format!("{JOURNAL_GREEN_SOURCE}~send_quest_progress_colour(questlist:prince, %princequest, ^prince_complete);\n"),
    );
    fx.write(
        "scripts/general/configs/quest.constant",
        "^prince_complete = 83\n",
    );
    fx.write(
        "scripts/player/interfaces/questlist.if",
        "[prince]\ntype=text\ntext=Prince Ali Rescue\n",
    );
    let (graph, _) = derive_transports_with_skips(fx.path(), &defs, &wc);
    let free: Vec<_> = graph
        .edges
        .iter()
        .filter(|e| e.loc_id == 2882 && e.item_req.is_empty())
        .collect();
    assert!(
        !free.is_empty(),
        "the source-backed waiver adds a parallel crossing"
    );
    assert!(free
        .iter()
        .all(|e| e.quest_req == ["Prince Ali Rescue"] && e.varp_req.is_empty()));
    fx.write(
        "scripts/general/configs/quest.constant",
        "^prince_complete = 72\n",
    );
    let (graph, _) = derive_transports_with_skips(fx.path(), &defs, &wc);
    assert!(
        graph
            .edges
            .iter()
            .filter(|e| e.loc_id == 2882)
            .all(|e| !e.item_req.is_empty()),
        "journal green below the free-branch threshold cannot prove the waiver"
    );
    fx.write(
        "scripts/general/configs/quest.constant",
        "^prince_complete = 83\n",
    );
    fx.write(
        "scripts/areas/area_alkharid/scripts/border_gate.rs2",
        "[label,talk_to_border_guard](coord $loc_coord)\nif (%princequest >= ^prince_saved) {\n    return;\n}\n",
    );
    let (graph, _) = derive_transports_with_skips(fx.path(), &defs, &wc);
    assert!(
        graph
            .edges
            .iter()
            .filter(|e| e.loc_id == 2882)
            .all(|e| !e.item_req.is_empty()),
        "a changed guard without the pass call must not waive coins"
    );
}

#[test]
fn producers_require_transmission_or_a_unique_completed_journal_proof() {
    let fx = Fixture::new();
    fx.write(
        "pack/varp.pack",
        "145=blackarmgang\n146=phoenixgang\n150=grandtree\n500=visible\n",
    );
    fx.write(
        "scripts/quests/configs/quest.varp",
        "[blackarmgang]\nscope=perm\n[phoenixgang]\nscope=perm\n[grandtree]\nscope=perm\n[visible]\ntransmit=yes\n",
    );
    fx.write(
        "scripts/general/scripts/quests.rs2",
        &format!("{JOURNAL_GREEN_SOURCE}~send_quest_progress_colour(questlist:grandtree, %grandtree, ^grandtree_complete);\n~send_quest_progress_colour(questlist:blackarmgang, %blackarmgang, ^blackarmgang_complete);\n~send_quest_progress_colour(questlist:blackarmgang, %phoenixgang, ^phoenixgang_complete);\n"),
    );
    fx.write(
        "scripts/general/configs/quest.constant",
        "^grandtree_complete=160\n^blackarmgang_complete=4\n^phoenixgang_complete=10\n",
    );
    fx.write(
        "scripts/player/interfaces/questlist.if",
        "[grandtree]\ntext=The Grand Tree\n[blackarmgang]\ntext=Shield of Arrav\n",
    );
    let edge = |loc_id, id, min| TransportEdge {
        kind: TransportKind::Door,
        player_delta: None,
        at: WorldTile {
            x: 100,
            z: 100,
            level: 0,
        },
        to: WorldTile {
            x: 101,
            z: 100,
            level: 0,
        },
        loc_id,
        option: 1,
        ticks: 1,
        dir: None,
        open_loc_id: None,
        skill_req: vec![],
        item_req: vec![],
        quest_req: vec![],
        varp_req: vec![(id, min)],
        worn_req: vec![],
        members_req: false,
        wildy_cap: None,
        quest_gates: None,
    };
    let raw = [
        edge(10, 150, 160),
        edge(11, 145, 4),
        edge(12, 146, 10),
        edge(13, 500, 3),
        edge(14, 150, 161),
    ];
    let gates = ObservableGates::from_content(fx.path());
    let mut graph = TransportGraph::default();
    let mut audit = VarpGateAudit::default();
    assert!(
        std::panic::catch_unwind(|| {
            let mut invalid = TransportGraph::default();
            invalid.edges.push(raw[0].clone());
            assert_transmitted_varp_reqs(fx.path(), &invalid);
        })
        .is_err(),
        "the pack assertion rejects raw producer output before any filtering"
    );
    for edge in raw {
        gates.admit_edge(&mut graph, edge, &mut audit);
    }
    assert_transmitted_varp_reqs(fx.path(), &graph);
    assert_eq!(audit.converted, 1);
    assert_eq!(audit.omitted, HashMap::from([(145, 1), (146, 1), (150, 1)]));
    assert_eq!(graph.edges.len(), 2);
    assert_eq!(graph.edges[0].quest_req, ["The Grand Tree"]);
    assert!(graph.edges[0].varp_req.is_empty());
    assert_eq!(
        graph.edges[1].varp_req,
        [(500, 3)],
        "transmitted varp stays"
    );
}

/// Original-base `toll_edges` emits every `parse_door_config_ids` openable
/// block, including a packed name that is not in `TOLL_GATE_LOC_NAMES`.
/// Applicability must not gate that derivation.
#[test]
fn n1_toll_emits_openable_alternate_border_gate_name() {
    let fx = Fixture::new();
    fx.write("pack/obj.pack", "995=coins\n1854=shantay_pass\n");
    fx.write("pack/loc.pack", "4242=border_gate_extra\n1564=loc_1564\n");
    fx.write(
        "scripts/areas/area_alkharid/configs/border_gate.loc",
        "\
[border_gate_extra]
name=Gate
op1=Open
param=next_loc_stage,loc_1564
",
    );
    fx.write(
        "maps/m51_50.jm2",
        "\
==== MAP ====
0 3 27: h1 u50
0 4 27: h1 u50
0 5 27: h1 u50
==== LOC ====
0 4 27: 4242 0 0
",
    );
    let defs = loc_defs(&[(4242, 1, 1), (1564, 1, 1)]);
    let wc = bake_collision(&fx, &defs, &HashSet::from([4242]));
    let (graph, skipped) = derive_transports_with_skips(fx.path(), &defs, &wc);
    let extras: Vec<_> = graph
        .edges
        .iter()
        .filter(|e| e.loc_id == 4242)
        .cloned()
        .collect();
    assert_eq!(
        extras.len(),
        2,
        "alternate openable name must emit both crossings: {extras:?}"
    );
    assert_eq!(
        extras[0].at,
        WorldTile {
            x: 3268,
            z: 3227,
            level: 0,
        }
    );
    assert_eq!(
        extras[0].to,
        WorldTile {
            x: 3267,
            z: 3227,
            level: 0
        }
    );
    assert_eq!(extras[0].dir, Some(DoorDir::W));
    assert_eq!(extras[1].dir, Some(DoorDir::E));
    assert_eq!(
        extras[1].to, extras[1].at,
        "the toll's reverse crossing lands back on the gate tile"
    );
    for e in &extras {
        assert_eq!(e.kind, TransportKind::Door, "{e:?}");
        assert_eq!(e.option, 1, "Open {e:?}");
        assert_eq!(e.ticks, 1, "{e:?}");
        assert_eq!(e.open_loc_id, Some(1564), "{e:?}");
        assert_eq!(e.item_req, vec![(995, 10)], "{e:?}");
        assert_eq!(e.at, extras[0].at);
        assert!(e.skill_req.is_empty() && e.quest_req.is_empty(), "{e:?}");
    }
    assert_eq!(skip_total(&skipped, SKIP_TOLL_OBJ_PACK), 0);
    assert_eq!(skip_total(&skipped, SKIP_TOLL_CONFIG), 0);
    assert_eq!(skip_total(&skipped, SKIP_TOLL_GATE), 0);
}

/// After source/config load, a packed declared name whose block is absent
/// or not admitted counts once. Emitted routes stay off the skip ledger.
#[test]
fn n1_packed_named_block_omissions_count_declared_routes() {
    let fx = Fixture::new();
    fx.write("pack/obj.pack", "995=coins\n1854=shantay_pass\n");
    fx.write(
        "pack/loc.pack",
        "2882=border_gate_toll_left\n2883=border_gate_toll_right\n1562=loc_1562\n1563=loc_1563\n",
    );
    fx.write(
        "scripts/areas/area_alkharid/configs/border_gate.loc",
        "\
[border_gate_toll_left]
name=Gate
op1=Open
param=next_loc_stage,loc_1562
",
    );
    fx.write(
        "maps/m51_50.jm2",
        "\
==== MAP ====
0 3 27: h1 u50
0 4 27: h1 u50
0 5 27: h1 u50
==== LOC ====
0 4 27: 2882 0 0
",
    );
    let defs = loc_defs(&[(2882, 1, 1), (2883, 1, 1), (1562, 1, 1), (1563, 1, 1)]);
    let wc = bake_collision(&fx, &defs, &HashSet::from([2882, 2883]));
    let (graph, skipped) = derive_transports_with_skips(fx.path(), &defs, &wc);
    let left: Vec<_> = graph
        .edges
        .iter()
        .filter(|e| e.loc_id == 2882)
        .cloned()
        .collect();
    assert_eq!(left.len(), 2, "left gate still emits: {left:?}");
    assert_eq!(left[0].item_req, vec![(995, 10)]);
    assert_eq!(left[0].dir, Some(DoorDir::W));
    assert_eq!(left[1].dir, Some(DoorDir::E));
    assert!(
        graph.edges.iter().all(|e| e.loc_id != 2883),
        "omitted right block must not emit"
    );
    assert_eq!(skip_total(&skipped, SKIP_TOLL_CONFIG), 1);
    assert_eq!(
        skip_total(&skipped, SKIP_TOLL_GATE),
        0,
        "emitted left must not also take SKIP_TOLL_GATE: {skipped:?}"
    );

    let fx = Fixture::new();
    fx.write("pack/loc.pack", "1814=wildinlever\n");
    fx.write(
        "scripts/areas/area_ardougne_east/configs/wilderness_lever.constant",
        "^ardougne_to_wilderness_coord = 0_49_61_18_20\n",
    );
    fx.write(
        "scripts/areas/area_ardougne_east/scripts/wilderness_lever.rs2",
        "[oploc1,unrelatedlever]\n~player_teleport_normal(^ardougne_to_wilderness_coord);\n",
    );
    let defs = loc_defs(&[(1814, 1, 1)]);
    let wc = bake_collision(&fx, &defs, &HashSet::new());
    let (graph, skipped) = derive_transports_with_skips(fx.path(), &defs, &wc);
    assert!(graph.edges.iter().all(|e| e.loc_id != 1814));
    assert_eq!(skip_total(&skipped, SKIP_LEVER_SOURCE), 0);
    assert_eq!(skip_total(&skipped, SKIP_LEVER_ROUTE), 1);

    let fx = Fixture::new();
    write_magicguild_source(&fx, magicguild_opener_script());
    fx.write(
        "scripts/areas/area_yanille/configs/magic_guild/magic_guild.loc",
        "\
[magicguild_door_l]
op1=Open
param=next_loc_stage,loc_1522
",
    );
    write_magicguild_yanille_placements(&fx);
    let defs = loc_defs(&[(1600, 1, 1), (1601, 1, 1), (1522, 1, 1), (1523, 1, 1)]);
    let wc = bake_collision(&fx, &defs, &HashSet::from([1600, 1601]));
    let (graph, skipped) = derive_transports_with_skips(fx.path(), &defs, &wc);
    assert_eq!(
        door_crossings(&graph, 1600),
        vec![
            ((2584, 3088), 'E', (2585, 3088)),
            ((2584, 3088), 'W', (2584, 3088)),
            ((2597, 3087), 'E', (2597, 3087)),
            ((2597, 3087), 'W', (2596, 3087)),
        ],
        "admitted left door still emits"
    );
    assert!(
        door_crossings(&graph, 1601).is_empty(),
        "omitted right door must not emit"
    );
    assert_eq!(skip_total(&skipped, SKIP_MAGICGUILD_CONFIG), 1);
    assert_eq!(
        skip_total(&skipped, SKIP_MAGICGUILD_DOOR),
        0,
        "emitted left must not count as a door skip: {skipped:?}"
    );
}

#[test]
fn wilderness_level_formula_and_caps_derive_from_fixture_content() {
    let fx = Fixture::new();
    fx.write(
        TEST_LEVELS_RS2,
        "\
[proc,wilderness_level](coord $coord)(int)
def_coord $coord1;
def_coord $coord2;
def_int $i = 0;
while($i < db_getfieldcount(wilderness_zones, coord_pair_table:coord_pair)) {
    $coord1, $coord2 = db_getfield(wilderness_zones, coord_pair_table:coord_pair, $i);
    if(inzone($coord1, $coord2, $coord) = true) {
        return (calc((coordz($coord) - coordz($coord1))/8 + 1));
    } else {
        $i = add($i, 1);
    }
}
return(0);
",
    );
    fx.write(
        TEST_ZONES_DBROW,
        "\
[wilderness_zones]
table=coord_pair_table
data=coord_pair,0_46_55_0_0,3_52_99_63_63
data=coord_pair,0_46_155_0_0,0_52_199_63_63
",
    );
    fx.write(
        TEST_LEVELS_CONSTANT,
        "^wilderness_starting_z = 3520\n^wilderness_starting_underground_z = 9920\n",
    );
    fx.write(
        TEST_SPELL_TELEPORT_RS2,
        "if (~wilderness_level(coord) > 20) {\n    return;\n}\n",
    );
    fx.write(
        "scripts/general/scripts/enchanted_jewellry/ring_of_dueling.rs2",
        "[opheld4,_category_136]\nif (~wilderness_level(coord) > 20) {\n    return;\n}\n~player_teleport_normal(0_51_50_51_35);\n",
    );
    fx.write(
        "scripts/general/scripts/enchanted_jewellry/necklace_of_minigames.rs2",
        "[opheld4,_necklace_of_minigames]\nif (~wilderness_level(coord) > 20) {\n    return;\n}\n~player_teleport_normal(0_34_77_31_12);\n",
    );
    fx.write(
        "scripts/general/scripts/enchanted_jewellry/amulet_of_glory.rs2",
        "[opheld4,amulet_of_glory_4] @amulet_of_glory_interface(\"x\");\n[label,amulet_of_glory_interface](string $message)\nif (~wilderness_level(coord) > 30) {\n    return;\n}\n~player_teleport_normal(0_48_54_15_40);\n",
    );
    fx.write(
        "pack/obj.pack",
        "554=firerune\n556=airrune\n563=lawrune\n1712=amulet_of_glory_4\n2552=ring_of_dueling_8\n3853=necklace_of_minigames_8\n",
    );
    fx.write(
        "scripts/skill_magic/configs/enchanted_jewelry.obj",
        "[ring_of_dueling_8]\ncategory=category_136\n[necklace_of_minigames_8]\ncategory=necklace_of_minigames\n[amulet_of_glory_4]\n",
    );
    fx.write(
        "scripts/skill_magic/configs/magic_spells.dbrow",
        "[magic_spell_teleport_varrock]\ndata=levelrequired,25\ndata=runesrequired,firerune,1,airrune,3,lawrune,1\ndata=tele_coord,0_50_53_13_32\n",
    );
    let defs = loc_defs(&[]);
    let wc = bake_collision(&fx, &defs, &HashSet::new());
    let graph = derive_transports(fx.path(), &defs, &wc);
    require_wilderness_teleport_legality(fx.path(), &graph).expect("caps derive");
    assert_eq!(graph.wilderness.divisor, 8);
    assert_eq!(graph.wilderness.offset, 1);
    assert_eq!(
        graph.wilderness.level(WorldTile {
            x: 3100,
            z: 3679,
            level: 0
        }),
        20
    );
    assert_eq!(
        graph.wilderness.level(WorldTile {
            x: 3100,
            z: 3680,
            level: 0
        }),
        21
    );
    let spells: Vec<_> = graph.teleports.iter().filter(|e| e.loc_id == 0).collect();
    assert!(!spells.is_empty());
    assert!(spells.iter().all(|e| e.wildy_cap == Some(20)));
    assert!(graph
        .teleports
        .iter()
        .any(|e| e.loc_id == 1712 && e.wildy_cap == Some(30)));
    assert!(graph
        .teleports
        .iter()
        .any(|e| e.loc_id == 2552 && e.wildy_cap == Some(20)));
}

#[test]
fn bake_fails_when_wilderness_teleport_caps_cannot_be_derived() {
    let fx = Fixture::new();
    fx.write(
        TEST_LEVELS_RS2,
        "[proc,wilderness_level](coord $coord)(int)\nreturn(99);\n",
    );
    fx.write(
        TEST_ZONES_DBROW,
        "[wilderness_zones]\ntable=coord_pair_table\ndata=coord_pair,0_46_55_0_0,3_52_99_63_63\n",
    );
    fx.write(
        TEST_SPELL_TELEPORT_RS2,
        "[label,magic_teleport](int $spell)\nreturn;\n",
    );
    fx.write(
        "scripts/general/scripts/enchanted_jewellry/ring_of_dueling.rs2",
        "",
    );
    fx.write(
        "scripts/general/scripts/enchanted_jewellry/necklace_of_minigames.rs2",
        "",
    );
    fx.write(
        "scripts/general/scripts/enchanted_jewellry/amulet_of_glory.rs2",
        "",
    );
    let defs = loc_defs(&[]);
    let wc = bake_collision(&fx, &defs, &HashSet::new());
    let graph = derive_transports(fx.path(), &defs, &wc);
    let err = require_wilderness_teleport_legality(fx.path(), &graph)
        .expect_err("unparseable formula must fail the bake check");
    assert!(
        err.contains("cannot be derived")
            || err.contains("does not return")
            || err.contains("missing")
            || err.contains("wilderness_zones")
            || err.contains("coord_pair"),
        "{err}"
    );
}

#[test]
fn bake_fails_when_only_one_wilderness_source_is_present() {
    let fx = Fixture::new();
    fx.write(
        TEST_LEVELS_RS2,
        "[proc,wilderness_level](coord $coord)(int)\nreturn(0);\n",
    );
    let defs = loc_defs(&[]);
    let wc = bake_collision(&fx, &defs, &HashSet::new());
    let graph = derive_transports(fx.path(), &defs, &wc);
    let err = require_wilderness_teleport_legality(fx.path(), &graph)
        .expect_err("exactly one wilderness source must fail the bake");
    assert!(
        err.contains("wilderness_zones") || err.contains("missing"),
        "{err}"
    );
}

#[test]
fn bake_fails_when_teleport_gates_exist_without_wilderness_zones() {
    let fx = Fixture::new();
    fx.write(
        TEST_SPELL_TELEPORT_RS2,
        "if (~wilderness_level(coord) > 20) {\n    return;\n}\n",
    );
    let defs = loc_defs(&[]);
    let wc = bake_collision(&fx, &defs, &HashSet::new());
    let graph = derive_transports(fx.path(), &defs, &wc);
    let err = require_wilderness_teleport_legality(fx.path(), &graph)
        .expect_err("gates without zones must fail the bake");
    assert!(
        err.contains("without") || err.contains("wilderness_levels") || err.contains("missing"),
        "{err}"
    );
}

#[test]
#[ignore = "NAV_CONTENT_ROOT and NAV_CACHE required; absence fails"]
fn real_289_content_derives_wilderness_teleport_caps() {
    for (root, graph, _wc) in qualification_worlds() {
        let _ = root;
        require_wilderness_teleport_legality(root, graph)
            .expect("289 content must derive wilderness teleport caps");
        assert_eq!(graph.wilderness.divisor, 8);
        assert_eq!(graph.wilderness.offset, 1);
        let t = |z| WorldTile {
            x: 3100,
            z,
            level: 0,
        };
        assert_eq!(graph.wilderness.level(t(3679)), 20);
        assert_eq!(graph.wilderness.level(t(3680)), 21);
        assert_eq!(graph.wilderness.level(t(3759)), 30);
        assert_eq!(graph.wilderness.level(t(3760)), 31);
        let spell = graph
            .teleports
            .iter()
            .find(|e| e.loc_id == 0)
            .and_then(|e| e.wildy_cap)
            .expect("spell cap");
        assert_eq!(spell, 20);
        assert!(graph
            .teleports
            .iter()
            .any(|e| e.loc_id > 0 && e.wildy_cap == Some(20)));
        let glory = graph
            .teleports
            .iter()
            .filter(|e| e.loc_id > 0)
            .filter_map(|e| e.wildy_cap)
            .max()
            .unwrap();
        assert_eq!(glory, 30);
    }
}

#[test]
#[ignore = "NAV_CONTENT_ROOT and NAV_CACHE required; absence fails"]
fn real_289_routes_respect_wilderness_teleport_caps() {
    use crate::router::{find_with, FindOptions, Leg};
    for (root, graph, wc) in qualification_worlds() {
        let _ = root;
        require_wilderness_teleport_legality(root, graph).expect("caps");
        let spell = graph
            .teleports
            .iter()
            .find(|e| e.loc_id == 0 && e.wildy_cap.is_some())
            .expect("spell");
        let spell_cap = spell.wildy_cap.unwrap();
        assert_eq!(spell_cap, 20);
        let dest = spell.to;
        let mut inv = HashMap::new();
        for &(id, n) in &spell.item_req {
            inv.insert(id, n.max(5));
        }
        let magic = spell
            .skill_req
            .iter()
            .find(|(id, _)| *id == 6)
            .map(|(_, lvl)| *lvl)
            .unwrap_or(99);
        let state = crate::world_state::WorldState {
            stats: HashMap::from([(6, magic)]),
            inv,
            ..crate::world_state::WorldState::default()
        };
        let opts = FindOptions {
            allow_teleports: true,
            allow_wilderness: true,
            ..FindOptions::default()
        };
        let below =
            standable_wildy_on(wc, graph, 20, true).expect("standable last tile of level 20");
        let above =
            standable_wildy_on(wc, graph, 21, false).expect("standable first tile of level 21");
        assert_eq!(below.z, 3679, "level 20 ends at z 3679");
        assert_eq!(above.z, 3680, "level 21 starts at z 3680");
        let r_below = find_with(wc, graph, below, dest, opts, &state)
            .unwrap_or_else(|e| panic!("from level {spell_cap} {below:?} -> {dest:?}: {e:?}"));
        assert!(
            r_below.legs.iter().any(
                |l| matches!(l, Leg::Transport { edge } if edge.kind == TransportKind::Teleport)
            ),
            "from below the cap the route still teleports"
        );
        assert_legal_teleports(graph, below, &r_below);
        if let Ok(route) = find_with(wc, graph, above, dest, opts, &state) {
            assert_legal_teleports(graph, above, &route);
            for (from, _) in teleport_takeoffs(above, &route) {
                assert_ne!(
                    from, above,
                    "must not teleport from above-cap origin {above:?}"
                );
            }
        }
        let glory_cap = graph
            .teleports
            .iter()
            .filter(|e| e.loc_id > 0)
            .filter_map(|e| e.wildy_cap)
            .max()
            .expect("jewellery cap");
        assert_eq!(glory_cap, 30);
        let glory = graph
            .teleports
            .iter()
            .find(|e| e.wildy_cap == Some(glory_cap))
            .unwrap();
        let gstate = crate::world_state::WorldState {
            inv: HashMap::from([(glory.item_req[0].0, 1)]),
            ..crate::world_state::WorldState::default()
        };
        let g_below =
            standable_wildy_on(wc, graph, 30, true).expect("standable last tile of level 30");
        let g_above =
            standable_wildy_on(wc, graph, 31, false).expect("standable first tile of level 31");
        assert_eq!(g_below.z, 3759, "level 30 ends at z 3759");
        assert_eq!(g_above.z, 3760, "level 31 starts at z 3760");
        let ok = find_with(wc, graph, g_below, glory.to, opts, &gstate)
            .unwrap_or_else(|e| panic!("glory from level {glory_cap}: {e:?}"));
        assert!(ok
            .legs
            .iter()
            .any(|l| matches!(l, Leg::Transport { edge } if edge.kind == TransportKind::Teleport)));
        assert_legal_teleports(graph, g_below, &ok);
        if let Ok(route) = find_with(wc, graph, g_above, glory.to, opts, &gstate) {
            assert_legal_teleports(graph, g_above, &route);
            for (from, _) in teleport_takeoffs(g_above, &route) {
                assert_ne!(from, g_above);
            }
        }
    }
}

fn standable_wildy_on(
    wc: &WorldCollision,
    graph: &TransportGraph,
    want: i32,
    north_edge: bool,
) -> Option<WorldTile> {
    let rules = &graph.wilderness;
    if rules.divisor <= 0 {
        return None;
    }
    for zone in &rules.zones {
        if zone.level1 > 0 {
            continue;
        }
        let z0 = zone.origin_z + (want - rules.offset) * rules.divisor;
        let z = if north_edge {
            z0 + rules.divisor - 1
        } else {
            z0
        };
        for x in zone.x1..=zone.x2 {
            let t = WorldTile { x, z, level: 0 };
            if zone.contains(t) && wc.standable(t) {
                return Some(t);
            }
        }
    }
    None
}

fn teleport_takeoffs(start: WorldTile, route: &crate::router::Route) -> Vec<(WorldTile, i32)> {
    let mut cur = start;
    let mut out = Vec::new();
    for leg in &route.legs {
        match leg {
            crate::router::Leg::Walk { tiles } => {
                if let Some(last) = tiles.last() {
                    cur = *last;
                }
            }
            crate::router::Leg::Transport { edge } => {
                if edge.kind == TransportKind::Teleport {
                    if let Some(cap) = edge.wildy_cap {
                        out.push((cur, cap));
                    }
                }
                cur = edge.to;
            }
        }
    }
    out
}

fn assert_legal_teleports(graph: &TransportGraph, start: WorldTile, route: &crate::router::Route) {
    for (from, cap) in teleport_takeoffs(start, route) {
        let level = graph.wilderness.level(from);
        assert!(
            level <= cap,
            "teleport from {from:?} wilderness level {level} > cap {cap}"
        );
    }
}

/// The generic `[oploc1,_door_closed]` handler and the proc it forwards to,
/// as `scripts/doors/scripts/doors.rs2` writes them (same-line body).
const GENERIC_DOOR_RS2: &str = "\
[oploc1,_door_closed] ~open_door(500);

[proc,open_door](int $duration)
loc_del($duration);
";

/// A `loc_1530`/`loc_1531` generic door fixture: the door config, the
/// category handler, and a mapsquare (m44_53, origin (2816,3392)) whose
/// MAP rows block every listed local tile with `f1` ground.
fn write_generic_door_fixture(fx: &Fixture, blocked: &[(i32, i32)], locs: &str) {
    fx.write("pack/loc.pack", "1530=loc_1530\n1531=loc_1531\n");
    fx.write(
        "scripts/doors/configs/doors.loc",
        "[loc_1530]\nname=Door\nop1=Open\ncategory=door_closed\nparam=next_loc_stage,loc_1531\n",
    );
    fx.write("scripts/doors/scripts/doors.rs2", GENERIC_DOOR_RS2);
    let mut map = String::from("==== MAP ====\n0 0 0: h1\n");
    for (x, z) in blocked {
        map.push_str(&format!("0 {x} {z}: f1\n"));
    }
    fx.write("maps/m44_53.jm2", &(map + "==== LOC ====\n" + locs));
}

fn derive_generic_door_fixture(fx: &Fixture) -> (TransportGraph, WorldCollision) {
    let defs = loc_defs(&[(1530, 1, 1), (1531, 1, 1)]);
    let wc = bake_collision(fx, &defs, &HashSet::from([1530]));
    let graph = derive_transports(fx.path(), &defs, &wc);
    (graph, wc)
}

/// A closed diagonal (`wall_diagonal`, shape 9) generic door blocks its own
/// tile, and `~open_door` frees it while swinging the leaf onto
/// `door_dir(angle + 1)`. In the open, each angle crosses to
/// `door_dir(angle)` on its own side and to the opposite cardinal on the
/// other — the old extractor skipped every non-straight door.
#[test]
fn diagonal_door_crosses_its_tile_for_every_angle() {
    let door = (2821, 3438);
    for (angle, a, b) in [
        (0, ('W', (2820, 3438)), ('E', (2822, 3438))),
        (1, ('N', (2821, 3439)), ('S', (2821, 3437))),
        (2, ('E', (2822, 3438)), ('W', (2820, 3438))),
        (3, ('S', (2821, 3437)), ('N', (2821, 3439))),
    ] {
        let fx = Fixture::new();
        write_generic_door_fixture(&fx, &[], &format!("0 5 46: 1530 9 {angle}\n"));
        let (graph, wc) = derive_generic_door_fixture(&fx);
        assert!(
            !wc.standable(WorldTile {
                x: door.0,
                z: door.1,
                level: 0
            }),
            "the closed diagonal door blocks its own tile"
        );
        let mut expected = vec![(door, a.0, a.1), (door, b.0, b.1)];
        expected.sort();
        assert_eq!(door_crossings(&graph, 1530), expected, "angle {angle}");
        assert!(graph
            .edges
            .iter()
            .filter(|e| e.loc_id == 1530)
            .all(|e| e.open_loc_id == Some(1531)));
    }
}

/// The swung leaf's tile is never a landing: with an angle-1 door's north
/// and south cardinals blocked, the only crossing into the north-east side
/// would be the east tile the leaf now covers, so none is emitted; the
/// south-west side still lands on its free west cardinal.
#[test]
fn diagonal_door_never_lands_on_its_swung_leaf() {
    let fx = Fixture::new();
    write_generic_door_fixture(&fx, &[(5, 47), (5, 45)], "0 5 46: 1530 9 1\n");
    let (graph, _) = derive_generic_door_fixture(&fx);
    assert_eq!(
        door_crossings(&graph, 1530),
        vec![((2821, 3438), 'W', (2820, 3438))]
    );
}

/// A one-wide corridor sealed by a closed diagonal door routes through it
/// (the East Ardougne house room shape); before the fix it was `NoPath`.
#[test]
fn diagonal_door_opens_a_sealed_corridor() {
    use crate::router::{find, Leg};
    let fx = Fixture::new();
    let walls: Vec<(i32, i32)> = (0..12)
        .flat_map(|x| (38..56).map(move |z| (x, z)))
        .filter(|&(x, _)| x != 5)
        .collect();
    write_generic_door_fixture(&fx, &walls, "0 5 46: 1530 9 1\n");
    let (graph, wc) = derive_generic_door_fixture(&fx);
    let tile = |z| WorldTile {
        x: 2821,
        z,
        level: 0,
    };
    for (from, to) in [(tile(3432), tile(3444)), (tile(3444), tile(3432))] {
        let route = find(&wc, &graph, from, to)
            .unwrap_or_else(|e| panic!("{from:?} -> {to:?} must cross the door: {e:?}"));
        assert!(route.legs.iter().any(|l| matches!(
            l,
            Leg::Transport { edge } if edge.loc_id == 1530 && edge.at == tile(3438)
        )));
    }
}

/// Door placements sit on the engine's game plane on every level: a door
/// on raw plane 2 crosses on level 2, and one on raw plane 1 over a
/// LINK_BELOW tile (the server's `loadLocations` bridge rule) crosses on
/// level 0. The old extractor kept only raw level 0.
#[test]
fn door_edges_sit_on_the_game_plane_of_upper_levels() {
    let fx = Fixture::new();
    write_generic_door_fixture(&fx, &[], "");
    fx.write(
        "maps/m44_53.jm2",
        "\
==== MAP ====
0 0 0: h1
1 9 46: f2
==== LOC ====
2 5 46: 1530 0 1
1 9 46: 1530 0 0
",
    );
    let (graph, _) = derive_generic_door_fixture(&fx);
    let mut levels: Vec<(i32, i32, i32, i32)> = graph
        .edges
        .iter()
        .filter(|e| e.loc_id == 1530)
        .map(|e| (e.at.x, e.at.z, e.at.level, e.to.level))
        .collect();
    levels.sort();
    assert_eq!(
        levels,
        vec![
            (2821, 3438, 2, 2),
            (2821, 3438, 2, 2),
            (2825, 3438, 0, 0),
            (2825, 3438, 0, 0),
        ]
    );
}

/// A closed door declared outside the door configs (West Ardougne's
/// `loc_2997` lives in `areas/area_ardougne_west/configs/doors.loc`) runs
/// the verified generic category handler and crosses like any door; a
/// member with its own `[oploc1,<name>]` script does not inherit it.
#[test]
fn generic_door_members_outside_the_door_configs_cross() {
    let fx = Fixture::new();
    fx.write(
        "pack/loc.pack",
        "2997=loc_2997\n2998=loc_2998\n3000=locked_door\n3001=locked_door_open\n",
    );
    fx.write("scripts/doors/scripts/doors.rs2", GENERIC_DOOR_RS2);
    fx.write(
        "scripts/areas/area_ardougne_west/configs/doors.loc",
        "\
[loc_2997]
op1=Open
category=door_closed
param=next_loc_stage,loc_2998

[locked_door]
op1=Open
category=door_closed
param=next_loc_stage,locked_door_open
",
    );
    fx.write(
        "scripts/areas/area_ardougne_west/scripts/locked.rs2",
        "[oploc1,locked_door]\nmes(\"The door is locked.\");\n",
    );
    fx.write(
        "maps/m44_53.jm2",
        "==== MAP ====\n0 0 0: h1\n==== LOC ====\n0 5 46: 2997 0 1\n0 9 46: 3000 0 1\n",
    );
    let defs = loc_defs(&[(2997, 1, 1), (3000, 1, 1)]);
    let wc = bake_collision(&fx, &defs, &HashSet::new());
    let graph = derive_transports(fx.path(), &defs, &wc);
    assert_eq!(
        door_crossings(&graph, 2997),
        vec![
            ((2821, 3438), 'N', (2821, 3439)),
            ((2821, 3438), 'S', (2821, 3438)),
        ]
    );
    assert!(door_crossings(&graph, 3000).is_empty());
    assert!(graph
        .edges
        .iter()
        .filter(|e| e.loc_id == 2997)
        .all(|e| e.open_loc_id == Some(2998)));
}

/// A curtain whose own open script only `loc_change`s it into a loc that
/// does not block walk (Al Kharid `loc_1528` → `loc_1529`) crosses like a
/// straight door; the same script into a still-blocking loc does not.
#[test]
fn swap_door_crosses_only_when_the_open_loc_frees_the_wall() {
    for (open_blocks, expected) in [(false, 2), (true, 0)] {
        let fx = Fixture::new();
        fx.write("pack/loc.pack", "1528=loc_1528\n1529=loc_1529\n");
        fx.write(
            "scripts/areas/area_alkharid/configs/curtains.loc",
            "[loc_1528]\nname=Curtain\nop1=Open\n\n[loc_1529]\nname=Curtain\nop1=Close\n",
        );
        fx.write(
            "scripts/areas/area_alkharid/scripts/misc_locs.rs2",
            "[oploc1,loc_1528]\n// Temp note\nloc_change(loc_1529, 200);\n",
        );
        fx.write(
            "maps/m44_53.jm2",
            "==== MAP ====\n0 0 0: h1\n==== LOC ====\n0 5 46: 1528 0 0\n",
        );
        let defs = LocDefs::from_locs(&[
            LocType {
                id: 1528,
                width: 1,
                length: 1,
                ..Default::default()
            },
            LocType {
                id: 1529,
                width: 1,
                length: 1,
                blockwalk: open_blocks,
                ..Default::default()
            },
        ]);
        let wc = bake_collision(&fx, &defs, &HashSet::new());
        let graph = derive_transports(fx.path(), &defs, &wc);
        let curtains: Vec<_> = graph.edges.iter().filter(|e| e.loc_id == 1528).collect();
        assert_eq!(curtains.len(), expected, "open loc blocks: {open_blocks}");
        assert!(curtains.iter().all(|e| e.open_loc_id == Some(1529)));
    }
}

/// The 0.1.8.1 Thiever trap: the East Ardougne house room (2668–2674,
/// 3315–3319) is sealed by the closed diagonal door `loc_1530` at
/// (2669,3316). The pack had no crossing, so the bank walk was `NoPath`
/// both ways; the real 289 content now crosses it.
#[test]
#[ignore = "NAV_CONTENT_ROOT and NAV_CACHE required; absence fails"]
fn ardougne_diagonal_door_room_routes_to_the_south_bank() {
    use crate::router::{find, Leg};
    for (root, graph, wc) in qualification_worlds() {
        let _ = root;
        let room = WorldTile {
            x: 2671,
            z: 3316,
            level: 0,
        };
        let bank = WorldTile {
            x: 2655,
            z: 3286,
            level: 0,
        };
        let door = WorldTile {
            x: 2669,
            z: 3316,
            level: 0,
        };
        for (from, to) in [(room, bank), (bank, room)] {
            let route = find(wc, graph, from, to)
                .unwrap_or_else(|e| panic!("{from:?} -> {to:?} must route: {e:?}"));
            assert!(
                route.legs.iter().any(|l| matches!(
                    l,
                    Leg::Transport { edge } if edge.loc_id == 1530 && edge.at == door
                )),
                "{from:?} -> {to:?} crosses the diagonal door: {route:?}"
            );
        }
    }
}

/// A `laddertop` placed on raw plane 2 over a LINK_BELOW tile (the
/// Brimhaven laddertop at (2784,3286): `m43_51` MAP row `1 32 22: … f2`) is
/// loaded on game plane 1, so its Climb-down (`ladders.rs2:42-47`,
/// `~climb_ladder(movecoord(coord(), 0, -1, 0), false)`) starts on plane 1
/// and lands on plane 0; the raw plane 2 is unreachable.
#[test]
fn bridged_laddertop_sits_on_its_game_plane() {
    let fx = Fixture::new();
    fx.write("pack/loc.pack", "1746=laddertop\n");
    fx.write(
        "scripts/ladders+stairs/scripts/ladders.rs2",
        "\
[oploc1,laddertop]
p_arrivedelay;
switch_coord (loc_coord) {
    case 2_47_54_17_57 : ~climb_ladder(1_47_54_17_58, false); // black knights fortress ladder
    case default : ~climb_ladder(movecoord(coord(), 0, -1, 0), false);
}
",
    );
    fx.write(
        "maps/m43_51.jm2",
        "\
==== MAP ====
0 32 22: h1 o6 f5 u48
1 32 22: h20 o42 f2
2 32 22: o42
==== LOC ====
2 32 22: 1746 10
",
    );
    let defs = loc_defs(&[(1746, 1, 1)]);
    let wc = bake_collision(&fx, &defs, &HashSet::new());
    let graph = derive_transports(fx.path(), &defs, &wc);
    let edges: Vec<_> = graph
        .edges
        .iter()
        .filter(|e| e.loc_id == 1746)
        .map(|e| (e.at, e.to.level))
        .collect();
    assert_eq!(
        edges,
        vec![(
            WorldTile {
                x: 2784,
                z: 3286,
                level: 1
            },
            0
        )]
    );
}

/// Paterdomus' fence gate (`memberfencegate_l/_r`,
/// `area_paterdomus/scripts/paterdomus_members_gate.rs2`) runs its own
/// `[oploc1]` block: the `map_members` refusal, then the generic
/// `~open_gate` / `~open_outer_gate`. It crosses on a members world and
/// never on a free one; a named override used to hide it entirely.
#[test]
fn paterdomus_members_fence_gate_crosses_only_on_a_members_world() {
    use crate::router::{find_with, FindOptions, Leg, RouteError};
    use crate::world_state::WorldState;
    let fx = Fixture::new();
    fx.write(
        "pack/loc.pack",
        "1598=memberfencegate_l\n1599=memberfencegate_r\n1552=loc_1552\n1556=loc_1556\n",
    );
    fx.write(
        "scripts/general_use/scripts/gates.rs2",
        "\
[proc,open_gate]
def_coord $main_open = ~movecoord_loc_return(~gate_set_close(loc_angle, 1));
return;

[proc,open_outer_gate]
loc_findallzone(~get_pair_coord(loc_coord, loc_angle, true));
return;

[oploc1,_gate_main_closed] ~open_gate;
[oploc1,_gate_outer_closed] ~open_outer_gate;
",
    );
    fx.write(
        "scripts/areas/area_paterdomus/configs/paterdomus.loc",
        "\
[memberfencegate_l]
name=Gate
op1=Open
active=yes
category=gate_main_closed
// todo: what is the intended next state?
param=next_loc_stage,loc_1552

[memberfencegate_r]
name=Gate
mirror=yes
op1=Open
active=yes
category=gate_outer_closed
param=next_loc_stage,loc_1556
",
    );
    fx.write(
        "scripts/general_use/configs/gates.loc",
        "[loc_1552]\nname=Gate\nop1=Close\ncategory=gate_main_open\n\n[loc_1556]\nname=Gate\nop1=Close\ncategory=gate_outer_open\n",
    );
    fx.write(
        "scripts/areas/area_paterdomus/scripts/paterdomus_members_gate.rs2",
        "\
[oploc1,memberfencegate_l]
if(map_members = ^false) {
    mes(^mes_members_gate);
    return;
}
~open_gate;

[oploc1,memberfencegate_r]
if(map_members = ^false) {
    mes(^mes_members_gate);
    return;
}
~open_outer_gate;
",
    );
    // The m51_54 pair (3320,3467)/(3320,3468), angle 0, in a fence column
    // that is solid everywhere else.
    let mut map = String::from("==== MAP ====\n0 0 0: h1\n");
    for z in 0..64 {
        if z != 11 && z != 12 {
            map.push_str(&format!("0 55 {z}: f1\n"));
        }
    }
    fx.write(
        "maps/m51_54.jm2",
        &(map + "==== LOC ====\n0 56 11: 1598 0 0\n0 56 12: 1599 0 0\n"),
    );
    let defs = loc_defs(&[(1598, 1, 1), (1599, 1, 1), (1552, 1, 1), (1556, 1, 1)]);
    let wc = bake_collision(&fx, &defs, &HashSet::from([1598, 1599]));
    let graph = derive_transports(fx.path(), &defs, &wc);
    let west = WorldTile {
        x: 3317,
        z: 3467,
        level: 0,
    };
    let east = WorldTile {
        x: 3322,
        z: 3467,
        level: 0,
    };
    let members = WorldState {
        map_members: true,
        ..WorldState::empty()
    };
    for (from, to) in [(west, east), (east, west)] {
        let route = find_with(&wc, &graph, from, to, FindOptions::default(), &members)
            .unwrap_or_else(|e| panic!("{from:?} -> {to:?} on a members world: {e:?}"));
        assert!(route.legs.iter().any(|l| matches!(
            l,
            Leg::Transport { edge } if matches!(edge.loc_id, 1598 | 1599) && edge.members_req
        )));
        assert_eq!(
            find_with(
                &wc,
                &graph,
                from,
                to,
                FindOptions::default(),
                &WorldState::empty()
            )
            .err(),
            Some(RouteError::NoPath),
            "{from:?} -> {to:?} on a free world"
        );
    }
}

/// The Cooking Guild door (`chefdoor`, `skill_cooking/scripts/
/// cooking_guild.rs2`) runs its own `~check_axis` opener: from outside it
/// refuses below cooking 32 or without a worn chef's hat, from inside it
/// lets anyone out. The guild had no edge at all before (a named opener).
#[test]
fn cooking_guild_door_gates_entry_on_level_and_worn_hat() {
    use crate::router::{find_with, FindOptions, Leg, RouteError};
    use crate::world_state::WorldState;
    let fx = Fixture::new();
    fx.write("pack/loc.pack", "2712=chefdoor\n1535=loc_1535\n");
    fx.write("pack/obj.pack", "1949=chefs_hat\n");
    fx.write(
        "scripts/skill_cooking/configs/cooking_guild.loc",
        "[chefdoor]\nname=Door\nop1=Open\nparam=next_loc_stage,loc_1535\n",
    );
    fx.write(
        "scripts/skill_cooking/scripts/cooking_guild.rs2",
        "\
[oploc1,chefdoor]
def_boolean $is_inside = ~check_axis(coord, loc_coord, loc_angle);
if ($is_inside = false) {
    if (stat(cooking) < 32) {
        mes(\"You need a cooking level of 32 to enter the Chef's Guild.\");
        def_string $fail_message = \"<p,neutral>Sorry. Only the finest chefs are allowed in here. Get your cooking level up to 32\";
        if (inv_total(worn, chefs_hat) < 1) {
            $fail_message = append($fail_message, \" and come back wearing a chef's hat\");
        }
        $fail_message = append($fail_message, \".\");
        ~chatnpc_specific(\"Head chef\", head_chef, $fail_message);
        return;
    }
    if (inv_total(worn, chefs_hat) < 1) {
        ~chatnpc_specific(\"Head chef\", head_chef, \"<p,neutral>You can't come in here unless you're wearing a chef's hat, or something like that.\");
        return;
    }
}
~open_and_close_door(loc_param(next_loc_stage), $is_inside, false);
",
    );
    // The guild's south wall door (angle 3) in a row that is solid
    // everywhere else: the guild is the north side, the loc's own tile.
    let mut map = String::from("==== MAP ====\n0 0 0: h1\n");
    for x in 0..64 {
        if x != 5 {
            map.push_str(&format!("0 {x} 45: f1\n"));
        }
    }
    fx.write(
        "maps/m44_53.jm2",
        &(map + "==== LOC ====\n0 5 46: 2712 0 3\n"),
    );
    let defs = loc_defs(&[(2712, 1, 1)]);
    let wc = bake_collision(&fx, &defs, &HashSet::from([2712]));
    let graph = derive_transports(fx.path(), &defs, &wc);
    let outside = WorldTile {
        x: 2821,
        z: 3434,
        level: 0,
    };
    let inside = WorldTile {
        x: 2821,
        z: 3442,
        level: 0,
    };
    let chef = WorldState {
        stats: HashMap::from([(SKILL_COOKING, 32)]),
        worn: HashSet::from([1949]),
        ..WorldState::empty()
    };
    let enter = find_with(&wc, &graph, outside, inside, FindOptions::default(), &chef)
        .unwrap_or_else(|e| panic!("a level-32 chef in a hat enters: {e:?}"));
    assert!(enter.legs.iter().any(|l| matches!(
        l,
        Leg::Transport { edge }
            if edge.loc_id == 2712
                && edge.skill_req == vec![(SKILL_COOKING, 32)]
                && edge.worn_req == vec![1949]
    )));
    for short in [
        WorldState {
            stats: HashMap::from([(SKILL_COOKING, 31)]),
            ..chef.clone()
        },
        WorldState {
            worn: HashSet::new(),
            ..chef.clone()
        },
    ] {
        assert_eq!(
            find_with(&wc, &graph, outside, inside, FindOptions::default(), &short).err(),
            Some(RouteError::NoPath)
        );
    }
    find_with(
        &wc,
        &graph,
        inside,
        outside,
        FindOptions::default(),
        &WorldState::empty(),
    )
    .unwrap_or_else(|e| panic!("anyone leaves the guild: {e:?}"));
}

/// The West Ardougne fence (`mournerstewfence`, `general_use/scripts/
/// fence.rs2`) climbs over with `~agility_exactmove` either way and no
/// check, so both crossings are ungated edges with no open leaf; it had
/// none before (a named opener).
#[test]
fn west_ardougne_fence_climbs_both_ways() {
    let fx = Fixture::new();
    fx.write("pack/loc.pack", "2068=mournerstewfence\n");
    fx.write(
        "scripts/general_use/scripts/fence.rs2",
        "\
[oploc1,mournerstewfence]
def_coord $start = loc_coord;
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
p_teleport($end);
",
    );
    // (2541,3331): m39_52 local (45,3), angle 0.
    fx.write(
        "maps/m39_52.jm2",
        "==== MAP ====\n0 0 0: h1\n==== LOC ====\n0 45 3: 2068 0 0\n",
    );
    let defs = loc_defs(&[(2068, 1, 1)]);
    let wc = bake_collision(&fx, &defs, &HashSet::from([2068]));
    let graph = derive_transports(fx.path(), &defs, &wc);
    assert_eq!(
        door_crossings(&graph, 2068),
        vec![
            ((2541, 3331), 'E', (2541, 3331)),
            ((2541, 3331), 'W', (2540, 3331)),
        ]
    );
    assert!(graph
        .edges
        .iter()
        .filter(|e| e.loc_id == 2068)
        .all(|e| e.skill_req.is_empty() && e.worn_req.is_empty() && e.open_loc_id.is_none()));
}

#[test]
fn scripted_climb_pair_uses_the_shared_ladder_tick_price() {
    let fx = Fixture::new();
    fx.write(
        "pack/loc.pack",
        "4187=viking_warrior_ladder\n4189=viking_warrior_ladder_down\n",
    );
    fx.write(
        "maps/m48_61.jm2",
        "==== MAP ====\n0 0 0: h1\n==== LOC ====\n0 19 53: 4187 10\n1 22 53: 4189 10\n",
    );
    fx.write(
        "scripts/ladders+stairs/scripts/ladders.rs2",
        "\
[proc,climb_ladder](coord $coord, boolean $up)
if ($up = true) { anim(human_reachforladder, 0); } else { anim(human_pickupfloor, 0); }
p_delay(0);
p_telejump($coord);
",
    );
    fx.write(
        "scripts/quests/quest_viking/scripts/viking_thorvald.rs2",
        "\
[oploc3,viking_warrior_ladder]
~climb_ladder(movecoord(coord, 0, 1, 0), true);
[oploc1,viking_warrior_ladder_down]
~climb_ladder(movecoord(coord, 0, -1, 0), true);
",
    );
    let defs = LocDefs::from_locs(&[
        LocType {
            id: 4187,
            op: vec![None, None, Some("Climb-up".into())],
            ..Default::default()
        },
        LocType {
            id: 4189,
            op: vec![Some("Climb-down".into())],
            ..Default::default()
        },
    ]);
    let collision = bake_collision(&fx, &defs, &HashSet::new());
    let graph = derive_transports(fx.path(), &defs, &collision);
    let ticks = |loc_id, option| {
        graph
            .edges
            .iter()
            .find(|edge| edge.loc_id == loc_id && edge.option == option)
            .unwrap_or_else(|| panic!("missing scripted ladder {loc_id}/{option}"))
            .ticks
    };

    assert_eq!(ticks(4187, 3), 3);
    assert_eq!(ticks(4189, 1), 3);
}

#[test]
fn scripted_climb_requires_the_handled_op_to_be_a_climb() {
    let fx = Fixture::new();
    fx.write(
        "pack/loc.pack",
        "9001=open_with_later_climb\n9002=selected_climb\n",
    );
    fx.write(
        "maps/m48_61.jm2",
        "==== MAP ====\n0 0 0: h1\n==== LOC ====\n0 19 53: 9001 10\n0 22 53: 9002 10\n",
    );
    fx.write(
        "scripts/areas/area_example/scripts/climbs.rs2",
        "\
[oploc1,open_with_later_climb]
p_telejump(movecoord(coord, 0, 1, 0));
[oploc1,selected_climb]
p_telejump(movecoord(coord, 0, 1, 0));
",
    );
    let defs = LocDefs::from_locs(&[
        LocType {
            id: 9001,
            op: vec![Some("Open".into()), Some("Climb-up".into())],
            ..Default::default()
        },
        LocType {
            id: 9002,
            op: vec![Some("Climb-up".into())],
            ..Default::default()
        },
    ]);
    let collision = bake_collision(&fx, &defs, &HashSet::new());
    let graph = derive_transports(fx.path(), &defs, &collision);

    assert!(
        !graph.edges.iter().any(|edge| edge.loc_id == 9001),
        "an unrelated Climb option cannot authorize Open op 1"
    );
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| edge.loc_id == 9002 && edge.option == 1),
        "the actual Climb option remains admitted"
    );
}

#[test]
fn relative_landing_returns_none_on_coordinate_overflow() {
    let mut edge = members_check_edge(TransportKind::Ladder, 2871, 1);
    for (takeoff, delta) in [
        (
            WorldTile {
                x: 1,
                z: 0,
                level: 0,
            },
            WorldTile {
                x: i32::MAX,
                z: 0,
                level: 0,
            },
        ),
        (
            WorldTile {
                x: 0,
                z: 1,
                level: 0,
            },
            WorldTile {
                x: 0,
                z: i32::MAX,
                level: 0,
            },
        ),
        (
            WorldTile {
                x: 0,
                z: 0,
                level: 1,
            },
            WorldTile {
                x: 0,
                z: 0,
                level: i32::MAX,
            },
        ),
    ] {
        edge.player_delta = Some(delta);
        assert_eq!(edge.landing_from(takeoff), None);
    }
    edge.player_delta = Some(WorldTile {
        x: 2,
        z: -3,
        level: 1,
    });
    assert_eq!(
        edge.landing_from(WorldTile {
            x: 10,
            z: 20,
            level: 0
        }),
        Some(WorldTile {
            x: 12,
            z: 17,
            level: 1
        })
    );
}

#[test]
fn direct_relative_climbs_scan_area_scripts_without_erasing_gates() {
    let fx = Fixture::new();
    fx.write(
        "pack/loc.pack",
        "9001=area_descent\n9002=area_ascent\n9003=guarded_climb\n",
    );
    fx.write("maps/m48_61.jm2", "==== MAP ====\n0 0 0: h1\n==== LOC ====\n0 19 53: 9001 10\n0 20 53: 9002 10\n0 21 53: 9003 10\n");
    fx.write(
        "scripts/ladders+stairs/scripts/ladders.rs2",
        "\
[proc,climb_ladder](coord $coord, boolean $up)
if ($up = true) { anim(human_reachforladder, 0); } else { anim(human_pickupfloor, 0); }
p_delay(2);
p_telejump($coord);
",
    );
    fx.write(
        "scripts/areas/area_example/scripts/climbs.rs2",
        "\
[oploc1,area_descent]
p_arrivedelay;
anim(human_reachforladder, 0);
p_delay(1);
p_telejump(movecoord(coord, -549, 0, 756));
mes(\"You climb down.\");
[oploc1,area_ascent] ~climb_ladder(movecoord(coord(), 549, 0, -756), true);
[label,not_an_op](coord $dest)
p_telejump($dest);
[oploc1,guarded_climb]
if (%quest = 2) { p_telejump(movecoord(coord, -549, 0, 756)); }
",
    );
    let defs = LocDefs::from_locs(&[9001, 9002, 9003].map(|id| LocType {
        id,
        op: vec![Some("Climb-down".into())],
        ..Default::default()
    }));
    let graph = derive_transports(
        fx.path(),
        &defs,
        &bake_collision(&fx, &defs, &HashSet::new()),
    );
    let down = graph
        .edges
        .iter()
        .find(|edge| edge.loc_id == 9001)
        .expect("area descent");
    assert_eq!(
        down.to,
        WorldTile {
            x: 2542,
            z: 4713,
            level: 0
        }
    );
    assert_eq!(
        down.ticks, 4,
        "interaction, shared arrival-delay tick, and source p_delay(1)"
    );
    let up = graph
        .edges
        .iter()
        .find(|edge| edge.loc_id == 9002)
        .expect("inline area ascent");
    assert_eq!(
        up.to,
        WorldTile {
            x: 3641,
            z: 3201,
            level: 0
        }
    );
    assert_eq!(
        up.ticks, 5,
        "interaction, shared arrival-delay tick, and helper p_delay(2)"
    );
    assert!(
        !graph.edges.iter().any(|edge| edge.loc_id == 9003),
        "a conditional offset is not an unconditional crossing"
    );
}

#[path = "transport/stage_doors_tests.rs"]
mod stage_doors;
