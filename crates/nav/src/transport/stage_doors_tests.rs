//! Quest-stage/guild doors and guarded ladders ([`super::super::stage_doors`]):
//! synthetic content shaped like the 289 openers each test cites.

use super::*;
use crate::router::{find_first_with, find_with, FindOptions, Leg, RouteError};
use crate::world_state::WorldState;
use client::dash3d::CollisionFlag;

/// The shared engine facts every stage-door fixture needs: the pinned
/// engine door and ladder procs ([`ENGINE_DOOR_PROCS`], verbatim),
/// `^left`/`^right`, `^true`, the journal colour rule and
/// one quest row (`%heroquest`, "Hero's Quest", complete at 15), a
/// transmitted `%tbwt_main`, an untransmitted `%plain_flag` with no
/// journal row, and the `ladder_cellar` block the climb price follows.
fn write_stage_engine(fx: &Fixture) {
    // The engine procs verbatim, exactly as the 289 content defines them.
    fx.write(
        "scripts/doors/scripts/engine_door_procs.rs2",
        ENGINE_DOOR_PROCS,
    );
    fx.write(
        "scripts/ladders+stairs/scripts/ladders.rs2",
        "[oploc1,ladder_cellar]\np_arrivedelay;\n~climb_ladder(movecoord(coord(), 0, 0, 6400), false);\n",
    );
    fx.write(
        "scripts/doors/configs/doubledoors.constant",
        "^left = 0\n^right = 1\n",
    );
    fx.write("scripts/engine.constant", "^true = 1\n^false = 0\n");
    fx.write(
        "pack/varp.pack",
        "188=heroquest\n320=tbwt_main\n400=plain_flag\n",
    );
    fx.write(
        "scripts/quests/quest_hero/configs/quest_hero.varp",
        "[heroquest]\nscope=perm\n[plain_flag]\nscope=perm\n",
    );
    fx.write(
        "scripts/quests/quest_tbwt/configs/quest_tbwt.varp",
        "[tbwt_main]\nscope=perm\ntransmit=yes\n",
    );
    fx.write(
        "scripts/general/scripts/quests.rs2",
        &format!(
            "{JOURNAL_GREEN_SOURCE}~send_quest_progress_colour(questlist:hero, %heroquest, ^hero_complete);\n"
        ),
    );
    fx.write(
        "scripts/general/configs/quest.constant",
        "^hero_complete = 15\n^hero_phoenix_talked_charlie = 6\n^tbwt_complete = 6\n",
    );
    fx.write(
        "scripts/player/interfaces/questlist.if",
        "[hero]\ntext=Hero's Quest\n",
    );
}

/// A mapsquare (m44_53, origin (2816,3392)) whose column x = 2820 is solid
/// ground except the listed rows, plus the given LOC lines.
fn write_barrier_square(fx: &Fixture, open_rows: &[i32], locs: &str) {
    let mut map = String::from("==== MAP ====\n0 0 0: h1\n");
    for z in 0..64 {
        if !open_rows.contains(&z) {
            map.push_str(&format!("0 4 {z}: f1\n"));
        }
    }
    fx.write("maps/m44_53.jm2", &(map + "==== LOC ====\n" + locs));
}

/// A mapsquare with every tile walkable, plus the given LOC lines.
fn write_open_square(fx: &Fixture, locs: &str) {
    fx.write(
        "maps/m44_53.jm2",
        &format!("==== MAP ====\n0 0 0: h1\n==== LOC ====\n{locs}"),
    );
}

fn derive_stage(fx: &Fixture, locs: &[i32]) -> (TransportGraph, WorldCollision) {
    let entries: Vec<(i32, i32, i32)> = locs.iter().map(|&id| (id, 1, 1)).collect();
    let defs = loc_defs(&entries);
    let wc = bake_collision(fx, &defs, &locs.iter().copied().collect());
    let graph = derive_transports(fx.path(), &defs, &wc);
    (graph, wc)
}

fn tile(x: i32, z: i32) -> WorldTile {
    WorldTile { x, z, level: 0 }
}

fn crosses(
    graph: &TransportGraph,
    wc: &WorldCollision,
    from: WorldTile,
    to: WorldTile,
    state: &WorldState,
    loc: i32,
) -> Result<bool, RouteError> {
    find_with(wc, graph, from, to, FindOptions::default(), state).map(|route| {
        route
            .legs
            .iter()
            .any(|l| matches!(l, Leg::Transport { edge } if edge.loc_id == loc))
    })
}

/// The Heroes' Guild doors (`areas_heroes_guild/scripts/heroes_entrance.rs2:1-15`):
/// each leaf jumps to a label with its side, which opens the double door
/// for `%heroquest >= ^hero_complete` and otherwise talks. `%heroquest` is
/// not transmitted, so the gate becomes the completed "Hero's Quest"
/// journal row: the walled guild is entered and left only with the quest
/// complete. Both leaves cross both ways and swing their config leaf.
#[test]
fn quest_stage_double_doors_cross_only_with_the_completed_quest() {
    let fx = Fixture::new();
    write_stage_engine(&fx);
    fx.write(
        "pack/loc.pack",
        "2624=herodoor_l\n2625=herodoor_r\n1522=loc_1522\n1523=loc_1523\n",
    );
    fx.write(
        "scripts/areas/areas_heroes_guild/configs/heroes_guild.loc",
        "[herodoor_l]\nop1=Open\ncategory=double_door_open_and_close_left\nparam=next_loc_stage,loc_1522\n\n[herodoor_r]\nop1=Open\ncategory=double_door_open_and_close_right\nparam=next_loc_stage,loc_1523\n",
    );
    fx.write(
        "scripts/areas/areas_heroes_guild/scripts/heroes_entrance.rs2",
        "\
[oploc1,herodoor_l] @open_heroes_guild(^left);
[oploc1,herodoor_r] @open_heroes_guild(^right);

[label,open_heroes_guild](int $side) {
  if (%heroquest >= ^hero_complete) {
    ~open_and_close_double_door2(~check_axis_locactive(coord), $side, door_open);
    return;
  }
  if (npc_find(coord, achietties, 5, 0) = true) {
    facesquare(npc_coord);
    @speak_achietties;
    return;
  }
  mes(\"You need to speak to Achietties to get through this door.\"); // backup
}
",
    );
    write_barrier_square(&fx, &[46, 47], "0 5 46: 2624 0 0\n0 5 47: 2625 0 0\n");
    let (graph, wc) = derive_stage(&fx, &[2624, 2625]);
    assert_eq!(
        door_crossings(&graph, 2624),
        vec![
            ((2821, 3438), 'E', (2821, 3438)),
            ((2821, 3438), 'W', (2820, 3438)),
        ]
    );
    assert_eq!(door_crossings(&graph, 2625).len(), 2);
    for e in graph
        .edges
        .iter()
        .filter(|e| matches!(e.loc_id, 2624 | 2625))
    {
        assert_eq!(e.quest_req, ["Hero's Quest"], "{e:?}");
        assert!(e.varp_req.is_empty(), "{e:?}");
        assert_eq!(e.open_loc_id, Some(e.loc_id - 1102), "{e:?}");
    }
    let hero = WorldState {
        quests: HashSet::from(["Hero's Quest".to_string()]),
        ..WorldState::empty()
    };
    let (west, east) = (tile(2818, 3438), tile(2824, 3438));
    for (from, to) in [(west, east), (east, west)] {
        let used = crosses(&graph, &wc, from, to, &hero, 2624).unwrap()
            || crosses(&graph, &wc, from, to, &hero, 2625).unwrap();
        assert!(used, "{from:?} -> {to:?} crosses a guild door");
        assert_eq!(
            crosses(&graph, &wc, from, to, &WorldState::empty(), 2624).err(),
            Some(RouteError::NoPath),
            "{from:?} -> {to:?} without the quest"
        );
    }
}

/// A gate the live client cannot observe never becomes an edge: the same
/// opener gated on an untransmitted varp with no journal row
/// (`%plain_flag`) is dropped, while a transmitted varp (`%tbwt_main`,
/// `quest_tbwt.rs2:68-73` `if (%tbwt_main < ^tbwt_complete) { mes; return; }`)
/// stays a raw `varp_req` at the minimum the path reads.
#[test]
fn stage_door_gates_follow_what_the_client_observes() {
    let fx = Fixture::new();
    write_stage_engine(&fx);
    fx.write(
        "pack/loc.pack",
        "779=tbwt_bamboo_door\n780=tbwt_bamboo_door_open\n781=flag_door\n1535=loc_1535\n",
    );
    fx.write(
        "scripts/quests/quest_tbwt/configs/quest_tbwt.loc",
        "[tbwt_bamboo_door]\nop1=Open\nparam=next_loc_stage,tbwt_bamboo_door_open\n\n[flag_door]\nop1=Open\n",
    );
    fx.write(
        "scripts/quests/quest_tbwt/scripts/quest_tbwt.rs2",
        "\
[oploc1,tbwt_bamboo_door]
if (%tbwt_main < ^tbwt_complete) {
    mes(\"You do not have permission to enter here\");
    return;
}
~open_and_close_door(loc_param(next_loc_stage), ~check_axis(coord, loc_coord, loc_angle), false);

[oploc1,flag_door]
if (%plain_flag >= 1) {
    ~open_and_close_door(loc_1535, ~check_axis(coord, loc_coord, loc_angle), false);
}
",
    );
    write_open_square(&fx, "0 5 40: 779 0 0\n0 5 50: 781 0 0\n");
    let (graph, _) = derive_stage(&fx, &[779, 781]);
    let tbwt: Vec<_> = graph.edges.iter().filter(|e| e.loc_id == 779).collect();
    assert_eq!(tbwt.len(), 2);
    for e in tbwt {
        assert_eq!(e.varp_req, [(320, 6)], "{e:?}");
        assert!(e.quest_req.is_empty(), "{e:?}");
        assert_eq!(e.open_loc_id, Some(780), "{e:?}");
    }
    assert!(door_crossings(&graph, 781).is_empty());
}

/// The crossing direction is the one the opener admits from that side of
/// the wall, read from the placement:
/// - `coordz(coord) > coordz(loc_coord)` (Khazard's stronghold,
///   `quest_tree.rs2:32-41`): out from the north only;
/// - `~check_axis(…) = true` then `open(…, true, …)` (the fight arena,
///   `quest_arena.rs2:32-36`): from the loc's own tile only;
/// - a free leaving side plus a key-gated entry, where the disjunction keeps
///   the cheaper proof (the witch's house, `quest_ball.rs2:10-31`: `key > 0
///   | $leaving = true`): leaving is free, entering needs the key.
#[test]
fn stage_door_crosses_only_the_side_its_opener_admits() {
    let fx = Fixture::new();
    write_stage_engine(&fx);
    fx.write(
        "pack/loc.pack",
        "81=fightarena_door1\n2861=witchhousedoor\n1532=loc_1532\n1535=loc_1535\n",
    );
    fx.write("pack/obj.pack", "2409=witches_doorkey\n");
    fx.write(
        "scripts/quests/quest_misc/scripts/doors.rs2",
        "\
[oploc1,fightarena_door1]
if(~check_axis(coord, loc_coord, loc_angle) = true) {
    ~open_and_close_door(loc_1532, true, false);
    return;
}
~chatplayer(\"<p,neutral>This door appears to be locked.\");

[oploc1,witchhousedoor]
def_boolean $leaving = ~check_axis(coord, loc_coord, loc_angle);
if(inv_total(inv, witches_doorkey) > 0 | $leaving = true) {
    @open_witch_house_door($leaving);
}
mes(\"The door is locked.\");

[label,open_witch_house_door](boolean $leaving)
~open_and_close_door2(loc_1535, $leaving, door_open);
",
    );
    // The arena door at angle 0 (west wall), the witch's door at angle 2
    // (east wall).
    write_open_square(&fx, "0 5 44: 81 0 0\n0 5 48: 2861 0 2\n");
    let (graph, _) = derive_stage(&fx, &[81, 2861]);
    assert_eq!(
        door_crossings(&graph, 81),
        vec![((2821, 3436), 'W', (2820, 3436))],
        "the arena door lets its inside out only"
    );
    let witch: Vec<_> = graph.edges.iter().filter(|e| e.loc_id == 2861).collect();
    assert_eq!(witch.len(), 2);
    for e in witch {
        match e.dir {
            Some(DoorDir::E) => assert!(e.item_req.is_empty(), "leaving is free: {e:?}"),
            Some(DoorDir::W) => {
                assert_eq!(e.item_req, [(2409, 1)], "entering needs the key: {e:?}")
            }
            _ => panic!("{e:?}"),
        }
    }

    // The stronghold's north wall (angle 1, as placed at (2502,3250)) opens
    // from the north side only: on the loc's own tile `coordz` is equal.
    let fx = Fixture::new();
    write_stage_engine(&fx);
    fx.write(
        "pack/loc.pack",
        "2184=khazard_stronghold_door\n1532=loc_1532\n",
    );
    fx.write(
        "scripts/quests/quest_tree/scripts/quest_tree.rs2",
        "\
[oploc1,khazard_stronghold_door]
if(coordz(coord) > coordz(loc_coord)) {
    ~open_and_close_door(loc_1532, ~check_axis(coord, loc_coord, loc_angle), false);
    return;
}
mes(\"The door is locked from the inside.\");
",
    );
    write_open_square(&fx, "0 5 40: 2184 0 1\n");
    let (graph, _) = derive_stage(&fx, &[2184]);
    assert_eq!(
        door_crossings(&graph, 2184),
        vec![((2821, 3432), 'S', (2821, 3432))]
    );
}

/// Openers the evaluator cannot prove never cross: talk before the door, an
/// NPC lookup, a key taken, a varp written, a `switch`, a movement after
/// the crossing, a wrong `$entering` constant, and an in-progress window
/// (`>= a & < complete`: Hazeel Cult's wall, `quest_hazeelcult.rs2:72-78`).
/// A plain unconditional opener in the same content does cross.
#[test]
fn stage_door_refuses_openers_it_cannot_prove() {
    let fx = Fixture::new();
    write_stage_engine(&fx);
    let bodies = [
        "~chatnpc(\"<p,neutral>Come in.\");\n~open_and_close_door(loc_1535, ~check_axis(coord, loc_coord, loc_angle), false);",
        "if (npc_find(coord, guard, 5, 0) = true) {\n    ~open_and_close_door(loc_1535, ~check_axis(coord, loc_coord, loc_angle), false);\n}",
        "inv_del(inv, key, 1);\n~open_and_close_door(loc_1535, ~check_axis(coord, loc_coord, loc_angle), false);",
        "%heroquest = ^hero_complete;\n~open_and_close_door(loc_1535, ~check_axis(coord, loc_coord, loc_angle), false);",
        "switch_int(%heroquest) {\n    case default : ~open_and_close_door(loc_1535, ~check_axis(coord, loc_coord, loc_angle), false);\n}",
        "~open_and_close_door(loc_1535, ~check_axis(coord, loc_coord, loc_angle), false);\np_teleport(0_44_53_0_0);",
        "~open_and_close_door(loc_1535, true, false);\n~open_and_close_door(loc_1535, false, false);",
        "if (%heroquest >= ^hero_phoenix_talked_charlie & %heroquest < ^hero_complete) {\n    ~open_and_close_door2(loc_1535, ~check_axis(coord, loc_coord, loc_angle), door_open);\n}",
    ];
    let mut pack = String::from("1535=loc_1535\n900=plain_door\n");
    let mut script = String::from(
        "[oploc1,plain_door]\n~open_and_close_door(loc_1535, ~check_axis(coord, loc_coord, loc_angle), false);\n\n",
    );
    let mut locs = String::from("0 5 2: 900 0 0\n");
    for (k, body) in bodies.iter().enumerate() {
        let id = 901 + k as i32;
        pack.push_str(&format!("{id}=refused_{k}\n"));
        script.push_str(&format!("[oploc1,refused_{k}]\n{body}\n\n"));
        locs.push_str(&format!("0 5 {}: {id} 0 0\n", 6 + 4 * k));
    }
    fx.write("pack/loc.pack", &pack);
    fx.write("pack/obj.pack", "1=key\n2=priest_gown\n3=priest_robe\n");
    fx.write("scripts/quests/quest_misc/scripts/refused.rs2", &script);
    write_open_square(&fx, &locs);
    let ids: Vec<i32> = (900..901 + bodies.len() as i32).collect();
    let (graph, _) = derive_stage(&fx, &ids);
    assert_eq!(
        door_crossings(&graph, 900).len(),
        2,
        "the plain opener crosses"
    );
    for (k, body) in bodies.iter().enumerate() {
        let id = 901 + k as i32;
        let got = graph
            .edges
            .iter()
            .filter(|e| e.loc_id == id)
            .collect::<Vec<_>>();
        assert!(
            got.is_empty(),
            "refused_{k} must not cross:\n{body}\n{got:?}"
        );
    }
}

/// A disguise is a conjunction, never an alternative blade list. The
/// fortress's exact slot comparisons and Guidor's inventory-count checks
/// derive the same strict worn gate from their content, with a free exit.
#[test]
fn content_disguises_require_every_piece_currently_worn() {
    for disguise in [
        "((inv_getobj(worn, ^wearpos_hat) ! bronze_med_helm) | (inv_getobj(worn, ^wearpos_torso) ! iron_chainbody))",
        "(inv_total(worn, bronze_med_helm) < 1 | inv_total(worn, iron_chainbody) < 1)",
    ] {
        let fx = Fixture::new();
        write_stage_engine(&fx);
        fx.write("pack/loc.pack", "900=disguise_door\n1535=loc_1535\n");
        fx.write("pack/obj.pack", "1139=bronze_med_helm\n1101=iron_chainbody\n");
        fx.write("scripts/player/configs/equip.constant", "^wearpos_hat = 0\n^wearpos_torso = 4\n");
        fx.write("scripts/combat/configs/disguise.obj", "[bronze_med_helm]\nwearpos=hat\n[iron_chainbody]\nwearpos=torso\n");
        fx.write(
            "scripts/quests/quest_fixture/scripts/disguise.rs2",
            &format!(
                "[oploc1,disguise_door]\ndef_boolean $entering = ~check_axis(coord, loc_coord, loc_angle);\nif ($entering = true & {disguise}) {{\n    mes(\"You are not wearing the uniform.\");\n    return;\n}}\n~open_and_close_door(loc_1535, $entering, false);\n"
            ),
        );
        write_barrier_square(&fx, &[46], "0 4 46: 900 0 2\n");
        let (graph, wc) = derive_stage(&fx, &[900]);
        let ingress = graph.edges.iter().find(|e| e.loc_id == 900 && e.dir == Some(DoorDir::E)).expect("content-derived ingress");
        assert_eq!(ingress.worn_all_req, [1101, 1139]);
        assert!(ingress.worn_req.is_empty(), "not an ANY-of gate");
        let (outside, inside) = (tile(2818, 3438), tile(2824, 3438));
        for worn in [HashSet::new(), HashSet::from([1139]), HashSet::from([1101])] {
            let state = WorldState {
                worn,
                inv: HashMap::from([(1139, 1), (1101, 1)]),
                ..WorldState::empty()
            };
            assert_eq!(crosses(&graph, &wc, outside, inside, &state, 900), Err(RouteError::NoPath));
            assert_eq!(crosses(&graph, &wc, inside, outside, &state, 900), Ok(true), "exit remains free");
        }
        let state = WorldState {
            worn: HashSet::from([1139, 1101]),
            ..WorldState::empty()
        };
        assert_eq!(crosses(&graph, &wc, outside, inside, &state, 900), Ok(true));
    }
}

#[test]
fn exact_worn_slot_gate_refuses_unknown_or_incompatible_slots() {
    for (slot, declaration) in [
        ("^wearpos_torso", "wearpos=hat"),
        ("^unknown_slot", "wearpos=hat"),
        ("^wearpos_hat", ""),
        ("^wearpos_hat", "wearpos=hat\nwearpos=torso"),
    ] {
        let fx = plain_opener_fixture(&format!(
            "[oploc1,plain_door]\nif (inv_getobj(worn, {slot}) = bronze_med_helm) {{\n~open_and_close_door(loc_1535, ~check_axis(coord, loc_coord, loc_angle), false);\n}}\n"
        ));
        fx.write("pack/obj.pack", "1139=bronze_med_helm\n");
        fx.write(
            "scripts/player/configs/equip.constant",
            "^wearpos_hat = 0\n^wearpos_torso = 4\n",
        );
        fx.write(
            "scripts/combat/configs/disguise.obj",
            &format!("[bronze_med_helm]\n{declaration}\n"),
        );
        let (graph, _) = derive_stage(&fx, &[900]);
        assert!(
            door_crossings(&graph, 900).is_empty(),
            "{slot}: {declaration}"
        );
    }
}

#[test]
fn exact_worn_hand_slots_use_content_enum_spellings() {
    for (config_slot, script_slot, slot_id) in [("righthand", "rhand", 3), ("lefthand", "lhand", 5)]
    {
        let fx = plain_opener_fixture(&format!(
            "[oploc1,plain_door]\nif (inv_getobj(worn, ^wearpos_{script_slot}) = gate_item) {{\n~open_and_close_door(loc_1535, ~check_axis(coord, loc_coord, loc_angle), false);\n}}\n"
        ));
        fx.write("pack/obj.pack", "1=gate_item\n");
        fx.write(
            "scripts/player/configs/equip.constant",
            &format!("^wearpos_{script_slot} = {slot_id}\n"),
        );
        fx.write(
            "scripts/combat/configs/gate_item.obj",
            &format!("[gate_item]\nwearpos={config_slot}\n"),
        );
        let (graph, _) = derive_stage(&fx, &[900]);
        assert_eq!(door_crossings(&graph, 900).len(), 2, "{config_slot}");
        for edge in graph.edges.iter().filter(|edge| edge.loc_id == 900) {
            assert_eq!(edge.worn_all_req, [1]);
            assert!(!WorldState::empty().allows(edge));
            assert!(WorldState {
                worn: HashSet::from([1]),
                ..WorldState::empty()
            }
            .allows(edge));
        }
    }
}

/// Push-wall is an op label, not a geometry class. Its verified door proc
/// proves both crossings even when the loc config does not say `Open`.
#[test]
fn secret_push_wall_uses_the_verified_content_crossing() {
    let fx = Fixture::new();
    write_stage_engine(&fx);
    fx.write("pack/loc.pack", "900=secret_wall\n1535=loc_1535\n");
    fx.write(
        "scripts/quests/quest_fixture/configs/wall.loc",
        "[secret_wall]\nop1=Push\nparam=next_loc_stage,loc_1535\n",
    );
    fx.write("scripts/quests/quest_fixture/scripts/wall.rs2", "[oploc1,secret_wall]\nmes(\"You push against the wall. You find a secret passage.\");\n~open_and_close_door2(loc_param(next_loc_stage), ~check_axis_locactive(coord), coffin_open);\n");
    write_barrier_square(&fx, &[46], "0 5 46: 900 0 2\n");
    let (graph, wc) = derive_stage(&fx, &[900]);
    assert_eq!(door_crossings(&graph, 900).len(), 2);
    for (from, to) in [
        (tile(2818, 3438), tile(2824, 3438)),
        (tile(2824, 3438), tile(2818, 3438)),
    ] {
        assert_eq!(
            crosses(&graph, &wc, from, to, &WorldState::empty(), 900),
            Ok(true)
        );
    }
}

/// Requirements are the minimums the taken path reads, never more: a
/// branch that only prints or drops items does not gate (Merlin's whistle
/// door, `quest_grail.rs2:30-35`, crosses free); a `stat()` refusal is a
/// skill gate both ways, and the Mining Guild's door and surface ladder
/// (`area_falador/scripts/mining_guild.rs2:1-16`) need mining 60: the
/// ladder climbs from its loc tile down the cellar shift like
/// `ladder_cellar`, priced the same.
#[test]
fn mining_guild_door_and_ladder_gate_on_mining_60() {
    let fx = Fixture::new();
    write_stage_engine(&fx);
    fx.write(
        "pack/loc.pack",
        "2112=miningguilddoor\n2113=miningguildladder\n22=whistledoor\n1535=loc_1535\n1532=loc_1532\n1755=ladder_cellar\n",
    );
    fx.write("pack/obj.pack", "16=magic_whistle\n17=holy_table_napkin\n");
    fx.write(
        "scripts/areas/area_falador/configs/mining_guild.loc",
        "[miningguilddoor]\nop1=Open\nparam=next_loc_stage,loc_1532\n",
    );
    fx.write(
        "scripts/areas/area_falador/scripts/mining_guild.rs2",
        "\
[oploc1,miningguildladder]
if (stat(mining) < 60) {
    ~chatnpc_specific(\"Dwarf\", guilddwarf, \"<p,neutral>Sorry, but you're not experienced enough to go in there.\");
    ~mesbox(\"You need a Mining level of 60 to access the Mining Guild.\");
    return;
}
p_arrivedelay;
~climb_ladder(movecoord(coord, 0, 0, 6400), true);

[oploc1,miningguilddoor]
if (stat(mining) < 60) {
    ~chatnpc_specific(\"Dwarf\", guilddwarf, \"<p,neutral>Sorry, but you're not experienced enough to go in there.\");
    ~mesbox(\"You need a Mining level of 60 to access the Mining Guild.\");
    return;
}
~open_and_close_door(loc_param(next_loc_stage), ~check_axis(coord, loc_coord, loc_angle), false);
",
    );
    fx.write(
        "scripts/quests/quest_grail/scripts/quest_grail.rs2",
        "\
[oploc1,whistledoor]
if(map_members = ^true & inv_total(inv, holy_table_napkin) > 0 & inv_total(inv, magic_whistle) < 2) { // doesn't check bank for either
    obj_add(2_48_52_35_31, magic_whistle, 1, calc(^lootdrop_duration / 2));
}
~open_and_close_door(loc_1535, ~check_axis(coord, loc_coord, loc_angle), false);
",
    );
    write_barrier_square(
        &fx,
        &[46],
        "0 5 46: 2112 0 0\n0 10 20: 2113 10 0\n0 10 30: 22 0 1\n",
    );
    let (graph, wc) = derive_stage(&fx, &[2112, 2113, 22]);
    let door: Vec<_> = graph.edges.iter().filter(|e| e.loc_id == 2112).collect();
    assert_eq!(door.len(), 2);
    for e in &door {
        assert_eq!(e.skill_req, [(14, 60)], "{e:?}");
        assert_eq!(e.open_loc_id, Some(1532), "{e:?}");
    }
    let miner = |level| WorldState {
        stats: HashMap::from([(14, level)]),
        ..WorldState::empty()
    };
    let (west, east) = (tile(2818, 3438), tile(2824, 3438));
    for (from, to) in [(west, east), (east, west)] {
        assert_eq!(crosses(&graph, &wc, from, to, &miner(60), 2112), Ok(true));
        assert_eq!(
            crosses(&graph, &wc, from, to, &miner(59), 2112).err(),
            Some(RouteError::NoPath)
        );
    }
    let ladder: Vec<_> = graph.edges.iter().filter(|e| e.loc_id == 2113).collect();
    assert_eq!(ladder.len(), 1);
    assert_eq!(ladder[0].kind, TransportKind::Ladder);
    assert_eq!(ladder[0].at, tile(2826, 3412));
    assert_eq!(ladder[0].to, tile(2826, 3412 + CELLAR_SHIFT));
    assert_eq!(ladder[0].skill_req, [(14, 60)]);
    assert_eq!(
        ladder[0].player_delta, None,
        "the Mining 60 guard remains anchor-absolute"
    );
    assert_eq!(ladder[0].ticks, 1 + extra_ticks("ladder_cellar").unwrap());
    let whistle: Vec<_> = graph.edges.iter().filter(|e| e.loc_id == 22).collect();
    assert_eq!(whistle.len(), 2);
    assert!(whistle
        .iter()
        .all(|e| e.item_req.is_empty() && e.skill_req.is_empty() && !e.members_req));
}

/// The other engine crossing procs cross the same way: Prince Ali's jail
/// door (`quest_prince.rs2:46-51`, `~open_and_close_metal_gate2`-style
/// metal gate opened with a literal `true` from the cell side only), the
/// Mourner HQ gates (`quest_biohazard.rs2:118-144`: the loc handed to
/// `~open_and_close_double_door3` explicitly; leaving is free, entering
/// needs the key), and the Taverley jail door
/// (`jail_doors.rs2:8-35`: a string bound before the gate; leaving only).
/// A double door handed some other coord is not this loc's crossing.
#[test]
fn stage_door_crosses_through_every_modelled_open_proc() {
    let fx = Fixture::new();
    write_stage_engine(&fx);
    fx.write(
        "pack/loc.pack",
        "2881=alidoor\n1541=loc_1541\n2058=mournerquaters_gatel\n2059=mournerquaters_gatel_open\n2623=deepdungeondoor\n37=ctratgatea\n950=stray_gate\n",
    );
    fx.write("pack/obj.pack", "423=mournerkeytw\n");
    fx.write(
        "scripts/quests/quest_biohazard/configs/quest_biohazard.loc",
        "[mournerquaters_gatel]\nop1=Open\nparam=next_loc_stage,mournerquaters_gatel_open\n\n[stray_gate]\nop1=Open\nparam=next_loc_stage,mournerquaters_gatel_open\n",
    );
    fx.write(
        "scripts/quests/quest_misc/scripts/procs_doors.rs2",
        "\
[oploc1,alidoor]
if(coordz(coord) > coordz(loc_coord)) {
    mes(\"The door is locked.\");
    sound_synth(locked, 1, 0);
    return;
}
~open_and_close_metal_gate(loc_1541, true, false);

[oploc1,mournerquaters_gatel]
@open_mournerhq_gate(^left, false);

[label,open_mournerhq_gate](int $side, boolean $used_key)
def_loc $loc_type = loc_type;
def_locshape $loc_shape = loc_shape;
def_int $loc_angle = loc_angle;
def_coord $loc_coord = loc_coord;
def_boolean $entering = ~check_axis(coord, loc_coord, loc_angle);
if($entering = true & inv_total(inv, mournerkeytw) = 0 & $used_key = false) {
    mes(\"The gate is locked.\");
    p_delay(3);
    mes(\"You need a key.\");
    return;
}
if($used_key = true) {
    mes(\"The key fits the gate.\");
    p_delay(3);
}
~open_and_close_double_door3($entering, $loc_type, $loc_shape, $loc_angle, $loc_coord, $side, grate_open);

[oploc1,deepdungeondoor]
@unlock_taverley_jaildoor(false);

[label,unlock_taverley_jaildoor](boolean $key_used)
def_boolean $entering = ~check_axis(coord, loc_coord, loc_angle);
def_string $name = lowercase(loc_name);
if($entering = true & $key_used = false) {
    mes(\"This <$name> is locked.\");
    return;
}
if($key_used = true) {
    mes(\"You unlock the <$name>.\");
} else {
    mes(\"The <$name> locks shut behind you.\");
}
~open_and_close_door2(ctratgatea, $entering, grate_open);

[oploc1,stray_gate]
~open_and_close_double_door3(~check_axis(coord, loc_coord, loc_angle), loc_type, loc_shape, loc_angle, movecoord(loc_coord, 0, 0, 1), ^left, grate_open);
",
    );
    // Ali's cell door on its loc's south face (angle 3), the Mourner HQ
    // gate on its east face (angle 2, the real gate's), the others west.
    write_open_square(
        &fx,
        "0 5 20: 2881 0 3\n0 5 30: 2058 0 2\n0 5 40: 2623 0 0\n0 5 50: 950 0 0\n",
    );
    let (graph, _) = derive_stage(&fx, &[2881, 2058, 2623, 950]);
    assert_eq!(
        door_crossings(&graph, 2881),
        vec![((2821, 3412), 'S', (2821, 3411))],
        "out of the cell from the loc's own tile only"
    );
    let gate: Vec<_> = graph.edges.iter().filter(|e| e.loc_id == 2058).collect();
    assert_eq!(gate.len(), 2);
    for e in gate {
        assert_eq!(e.open_loc_id, Some(2059), "{e:?}");
        match e.dir {
            Some(DoorDir::W) => assert!(e.item_req.is_empty(), "leaving is free: {e:?}"),
            Some(DoorDir::E) => assert_eq!(e.item_req, [(423, 1)], "entering needs the key: {e:?}"),
            _ => panic!("{e:?}"),
        }
    }
    assert_eq!(
        door_crossings(&graph, 2623),
        vec![((2821, 3432), 'E', (2821, 3432))],
        "the jail door lets its prisoner out only"
    );
    assert!(door_crossings(&graph, 950).is_empty());
}

/// Writes a plain opener (`plain_door`, loc 900, west wall at (2821,3438))
/// over the pinned engine procs, with `extra` script text added.
fn plain_opener_fixture(extra: &str) -> Fixture {
    let fx = Fixture::new();
    write_stage_engine(&fx);
    fx.write("pack/loc.pack", "900=plain_door\n1535=loc_1535\n");
    fx.write("scripts/quests/quest_misc/scripts/plain.rs2", extra);
    write_open_square(&fx, "0 5 46: 900 0 0\n");
    fx
}

const PLAIN_OPENER: &str = "[oploc1,plain_door]\n~open_and_close_door(loc_1535, ~check_axis(coord, loc_coord, loc_angle), false);\n";

/// A crossing proc is modelled only while the content's body of it (and of
/// every proc it calls) is the pinned engine body: the verbatim 289 procs
/// cross, while an `~open_and_close_door` whose landing calculation drifts,
/// or that moves the player again, or a drifted `~door_open` it calls,
/// yields no edge and is counted in the skip report — even though every
/// line the old marker check looked for is still there.
#[test]
fn stage_door_crosses_only_through_the_pinned_engine_procs() {
    let fx = plain_opener_fixture(PLAIN_OPENER);
    let (graph, _) = derive_stage(&fx, &[900]);
    assert_eq!(
        door_crossings(&graph, 900).len(),
        2,
        "the pinned procs cross"
    );

    let door = "$dest = movecoord($loc_coord, $x, 0, $z);\n}\np_teleport($dest);";
    let door_open_west = "case ^loc_west : return(-1, 0);";
    for (label, from, to) in [
        (
            "landing drift",
            door,
            "$dest = movecoord($loc_coord, $x, 0, add($z, 1));\n}\np_teleport($dest);",
        ),
        (
            "second movement",
            door,
            "$dest = movecoord($loc_coord, $x, 0, $z);\n}\np_teleport($dest);\np_teleport(movecoord($dest, 0, 0, 3));",
        ),
        ("callee drift", door_open_west, "case ^loc_west : return(-2, 0);"),
    ] {
        assert!(ENGINE_DOOR_PROCS.contains(from), "{label}");
        // Drift only the first occurrence: `~open_and_close_door`'s body,
        // or the wall-straight arm of `~door_open`.
        let drifted = ENGINE_DOOR_PROCS.replacen(from, to, 1);
        let fx = plain_opener_fixture(PLAIN_OPENER);
        fx.write("scripts/doors/scripts/engine_door_procs.rs2", &drifted);
        let defs = loc_defs(&[(900, 1, 1)]);
        let wc = bake_collision(&fx, &defs, &HashSet::from([900]));
        let (graph, skipped) = derive_transports_with_skips(fx.path(), &defs, &wc);
        assert!(
            door_crossings(&graph, 900).is_empty(),
            "{label}: a drifted proc is not a crossing"
        );
        assert_eq!(
            skip_total(&skipped, SKIP_STAGE_DOOR_PROC_DRIFT),
            1,
            "{label}: the drifted proc is reported"
        );
    }
}

/// After the crossing, an opener may only call procs proven never to move
/// the player: a proc that only prints keeps the edge, while a wrapper that
/// teleports (directly or through another proc) or an undefined proc drops
/// it.
#[test]
fn stage_door_refuses_an_unproven_proc_after_the_crossing() {
    let opener = |call: &str| {
        format!(
            "[oploc1,plain_door]\n~open_and_close_door(loc_1535, ~check_axis(coord, loc_coord, loc_angle), false);\n{call}\n\n\
[proc,say_goodbye]\nmes(\"The door shuts.\");\n\n\
[proc,relocate_after_open]\np_delay(0);\n~nudge;\n\n\
[proc,nudge]\np_teleport(movecoord(coord, 0, 0, 5));\nreturn(true);\n\n\
[proc,returns_nudge](boolean)\nreturn(~nudge);\n\n\
[proc,returns_flag](boolean)\nreturn(true);\n"
        )
    };
    for (call, crosses) in [
        ("~say_goodbye;", true),
        ("~relocate_after_open;", false),
        ("~nudge;", false),
        ("~missing_proc;", false),
        // `traiborn.rs2:36` shape: the movement hides in a return value.
        ("def_boolean $moved = ~returns_nudge;", false),
        ("return(~nudge);", false),
        ("def_boolean $flag = ~returns_flag;", true),
        (
            "if (%heroquest >= ^hero_complete) {\n    ~relocate_after_open;\n}",
            false,
        ),
    ] {
        let fx = plain_opener_fixture(&opener(call));
        let (graph, _) = derive_stage(&fx, &[900]);
        assert_eq!(!door_crossings(&graph, 900).is_empty(), crosses, "{call}");
    }
}

/// Every call an opener makes is accounted for, wherever it hides: in a
/// string interpolation, in `calc(…)` arithmetic, in a starred queue, behind
/// `gosub`, as a targeted `p_op*t`, or in the condition or arguments of an
/// `if` whose branches only print. Each form below calls a proc that
/// teleports (or hands control elsewhere) and must leave the door without an
/// edge, before or after the crossing; the controls beside them, which only
/// format text or compute values, keep it.
#[test]
fn stage_door_certifies_every_call_an_opener_makes() {
    const DOOR: &str =
        "~open_and_close_door(loc_1535, ~check_axis(coord, loc_coord, loc_angle), false);";
    const PROCS: &str = "\
[proc,nudge_string]()(string)
p_teleport(movecoord(coord, 0, 0, 5));
return(\"x\");

[proc,nudge_int]()(int)
p_teleport(movecoord(coord, 0, 0, 5));
return(1);

[proc,nudge_bool]()(boolean)
p_teleport(movecoord(coord, 0, 0, 5));
return(true);

[proc,quiet_string]()(string)
return(\"x\");

[queue,nudge_queue]
p_teleport(movecoord(coord, 0, 0, 5));
";
    let after = [
        ("return(\"<~nudge_string>\");", false),
        ("mes(\"<~nudge_string>\");", false),
        ("def_string $s = \"<~nudge_string>\";", false),
        ("def_int $n = calc(~nudge_int + 1);", false),
        ("if (calc(~nudge_int + 1) > 0) {\n    mes(\"x\");\n}", false),
        ("queue*(nudge_queue, 0)(1);", false),
        ("strongqueue*(nudge_queue, 0)(1);", false),
        ("gosub(nudge_int);", false),
        ("def_proc $p = nudge_int;\ngosub($p);", false),
        ("p_opnpct(1);", false),
        ("p_opplayert(1);", false),
        // Commands that dispatch engine scripts (review round 4).
        ("if_close;", false),
        ("npc_add(coord, man, 100);", false),
        ("npc_del;", false),
        ("npc_setmode(opplayer2);", false),
        // Controls.
        (
            "mes(\"<p,neutral>Hello <text_gender(\"Sir\", \"Madam\")>, <tostring(1)>.\");",
            true,
        ),
        ("mes(\"<~quiet_string>\");", true),
        ("def_int $n = calc(1 + 2);", true),
    ];
    let before = [
        ("mes(\"<~nudge_string>\");", false),
        ("def_string $s = \"<~nudge_string>\";", false),
        ("if (~nudge_bool = true) {\n    mes(\"x\");\n}", false),
        (
            "if (%heroquest >= ^hero_complete) {\n    mes(~nudge_string);\n}",
            false,
        ),
        // Controls.
        (
            "mes(\"<p,neutral>Hello <text_gender(\"Sir\", \"Madam\")>.\");",
            true,
        ),
        (
            "if (%heroquest >= ^hero_complete) {\n    mes(\"x\");\n}",
            true,
        ),
    ];
    let cases = after
        .iter()
        .map(|(stmt, crosses)| (format!("{DOOR}\n{stmt}"), *crosses))
        .chain(
            before
                .iter()
                .map(|(stmt, crosses)| (format!("{stmt}\n{DOOR}"), *crosses)),
        );
    for (body, crosses) in cases {
        let fx = plain_opener_fixture(&format!("[oploc1,plain_door]\n{body}\n\n{PROCS}"));
        let (graph, _) = derive_stage(&fx, &[900]);
        assert_eq!(!door_crossings(&graph, 900).is_empty(), crosses, "{body}");
    }
}
/// `find_first_with` exercises both forward routing and the reverse landing
/// proof. Only the non-anchor take-off and its relative destination are
/// standable, so an anchor-absolute landing cannot reach the target.
fn assert_relative_climb_from_non_anchor(
    loc_id: i32,
    loc_name: &str,
    at: WorldTile,
    delta: WorldTile,
    source: &str,
    handler: &str,
) {
    let fx = Fixture::new();
    write_stage_engine(&fx);
    fx.write("pack/loc.pack", &format!("{loc_id}={loc_name}\n"));
    let map_path = format!("maps/m{}_{}.jm2", at.x.div_euclid(64), at.z.div_euclid(64));
    fx.write(
        &map_path,
        &format!(
            "==== MAP ====\n0 0 0: h1\n==== LOC ====\n{} {} {}: {loc_id} 10 0\n",
            at.level,
            at.x.rem_euclid(64),
            at.z.rem_euclid(64)
        ),
    );
    fx.write(source, handler);

    let takeoff = WorldTile { x: at.x + 1, ..at };
    let landing = WorldTile {
        x: takeoff.x + delta.x,
        z: takeoff.z + delta.z,
        level: takeoff.level + delta.level,
    };
    let nominal = WorldTile {
        x: at.x + delta.x,
        z: at.z + delta.z,
        level: at.level + delta.level,
    };
    let min_x = at.x.min(takeoff.x).min(landing.x).min(nominal.x) - 1;
    let max_x = at.x.max(takeoff.x).max(landing.x).max(nominal.x) + 1;
    let min_z = at.z.min(takeoff.z).min(landing.z).min(nominal.z) - 1;
    let max_z = at.z.max(takeoff.z).max(landing.z).max(nominal.z) + 1;
    let origin = WorldTile {
        x: min_x,
        z: min_z,
        level: 0,
    };
    let width = (max_x - min_x + 1) as usize;
    let height = (max_z - min_z + 1) as usize;
    let cells = width * height * 4;
    let mut blocked = vec![u64::MAX; cells.div_ceil(64)];
    let mut walk = vec![0; cells];
    for tile in [takeoff, landing] {
        let index = tile.level as usize * width * height
            + (tile.z - origin.z) as usize * width
            + (tile.x - origin.x) as usize;
        blocked[index / 64] &= !(1u64 << (index % 64));
    }
    // The anchor-relative landing is blocked, and even a transport to it
    // cannot walk east onto the non-anchor target from the anchor.
    let landing_index = landing.level as usize * width * height
        + (landing.z - origin.z) as usize * width
        + (landing.x - origin.x) as usize;
    walk[landing_index] = CollisionFlag::W_W as u8;
    let collision = WorldCollision {
        origin,
        width,
        height,
        walk,
        blocked,
        flags: None,
    };
    assert!(!collision.standable(nominal));
    let defs = loc_defs(&[(loc_id, 1, 1)]);
    let graph = derive_transports(fx.path(), &defs, &collision);
    let edge = graph
        .edges
        .iter()
        .find(|edge| edge.loc_id == loc_id)
        .unwrap_or_else(|| panic!("missing {loc_name} climb"));
    assert_eq!(edge.at, at);
    assert_eq!(edge.to, nominal, "wire `to` remains canonical at the loc");
    assert_ne!(takeoff, edge.at, "the proof must use a non-anchor stand");

    let search = find_first_with(
        &collision,
        &graph,
        takeoff,
        &[landing],
        FindOptions::default(),
        &WorldState::empty(),
    );
    let route = search.route().unwrap_or_else(|error| {
        panic!("{loc_name}: non-anchor take-off should land at {landing:?}; {error:?}")
    });
    assert_eq!(route.dest, landing);
    let planned = route
        .legs
        .iter()
        .find_map(|leg| match leg {
            Leg::Transport { edge } if edge.loc_id == loc_id => Some(edge),
            _ => None,
        })
        .unwrap_or_else(|| panic!("{loc_name}: route did not use the climb edge"));
    assert_eq!(planned.to, landing);
    assert_eq!(planned.landing_from(takeoff), Some(landing));
    assert_eq!(edge.player_delta, Some(delta));
}

#[test]
fn viking_seer_ladder_routes_and_reverse_proof_use_the_actual_takeoff() {
    assert_relative_climb_from_non_anchor(
        4163,
        "viking_seer_up_ladder",
        WorldTile {
            x: 2631,
            z: 3663,
            level: 0,
        },
        WorldTile {
            x: 0,
            z: 0,
            level: 2,
        },
        "scripts/quests/quest_viking/scripts/viking_peer.rs2",
        "[oploc1,viking_seer_up_ladder]\np_arrivedelay;\n~climb_ladder(movecoord(coord, 0, 2, 0), true);\n",
    );
}

#[test]
fn tutorial_cellar_ladder_routes_and_reverse_proof_use_the_actual_takeoff() {
    assert_relative_climb_from_non_anchor(
        3028,
        "newbieladder1",
        WorldTile {
            x: 3088,
            z: 9519,
            level: 0,
        },
        WorldTile {
            x: 0,
            z: -6400,
            level: 0,
        },
        "scripts/tutorial/scripts/tut_doors_and_gates.rs2",
        "[oploc1,newbieladder1]\np_arrivedelay;\n~climb_ladder(movecoord(coord, 0, 0, -6400), true);\n",
    );
}

#[test]
fn tutorial_cellar_ladder_up_routes_from_non_anchor_stand() {
    assert_relative_climb_from_non_anchor(
        3031,
        "newbieladdertop2",
        WorldTile {
            x: 3111,
            z: 3126,
            level: 0,
        },
        WorldTile {
            x: 0,
            z: 6400,
            level: 0,
        },
        "scripts/tutorial/scripts/tut_doors_and_gates.rs2",
        "[oploc1,newbieladdertop2]\np_arrivedelay;\n~climb_ladder(movecoord(coord, 0, 0, 6400), false);\n",
    );
}

#[test]
fn boardgames_rank_guard_stays_anchor_absolute_but_unconditional_down_climbs_are_relative() {
    let fx = Fixture::new();
    write_stage_engine(&fx);
    fx.write(
        "pack/loc.pack",
        "4643=boardgames_runelink_ladderup_experienced\n\
         4644=boardgames_runelink_ladderdown_experienced\n\
         4645=boardgames_draughts_ladderup_experienced\n\
         4646=boardgames_draughts_ladderdown_experienced\n",
    );
    fx.write(
        "pack/varp.pack",
        "353=boardgames_runelink_rank\n354=boardgames_draughts_rank\n",
    );
    fx.write(
        "scripts/minigames/game_gamesroom/general/configs/boardgames.varp",
        "[boardgames_runelink_rank]\nprotect=no\ntransmit=yes\nscope=perm\n\
         [boardgames_draughts_rank]\nprotect=no\ntransmit=yes\nscope=perm\n",
    );
    fx.write(
        "scripts/minigames/game_gamesroom/boardgames_runelink/scripts/boardgames_runelink.rs2",
        "[oploc1,boardgames_runelink_ladderup_experienced]\n\
         if(%boardgames_runelink_rank < 1500) {\n\
         mes(\"You need a Runelink rank of at least 1500 to climb this ladder.\");\n\
         return;\n\
         }\n\
         ~climb_ladder(movecoord(coord, 0, 1, 0), true);\n\
         [oploc1,boardgames_runelink_ladderdown_experienced]\n\
         ~climb_ladder(movecoord(coord, 0, -1, 0), false);\n",
    );
    fx.write(
        "scripts/minigames/game_gamesroom/boardgames_draughts/scripts/boardgames_draughts.rs2",
        "[oploc1,boardgames_draughts_ladderup_experienced]\n\
         if(%boardgames_draughts_rank < 1500) {\n\
         mes(\"You need a Draughts rank of at least 1500 to climb this ladder.\");\n\
         return;\n\
         }\n\
         ~climb_ladder(movecoord(coord, 0, 1, 0), true);\n\
         [oploc1,boardgames_draughts_ladderdown_experienced]\n\
         ~climb_ladder(movecoord(coord, 0, -1, 0), false);\n",
    );
    fx.write(
        "maps/m34_77.jm2",
        "==== MAP ====\n0 0 0: h1\n==== LOC ====\n\
         0 40 8: 4643 10 0\n1 40 8: 4644 10 0\n\
         0 23 8: 4645 10 0\n1 23 8: 4646 10 0\n",
    );
    let defs = loc_defs(&[(4643, 1, 1), (4644, 1, 1), (4645, 1, 1), (4646, 1, 1)]);
    let collision = bake_collision(&fx, &defs, &HashSet::new());
    let graph = derive_transports(fx.path(), &defs, &collision);

    for (id, varp) in [(4643, 353), (4645, 354)] {
        let edge = graph.edges.iter().find(|edge| edge.loc_id == id).unwrap();
        assert_eq!(
            edge.player_delta, None,
            "rank/dialog handler {id} stays absolute"
        );
        assert_eq!(edge.varp_req, [(varp, 1500)]);
    }
    for id in [4644, 4646] {
        let edge = graph.edges.iter().find(|edge| edge.loc_id == id).unwrap();
        assert_eq!(
            edge.player_delta,
            Some(WorldTile {
                x: 0,
                z: 0,
                level: -1,
            })
        );
        assert!(edge.varp_req.is_empty());
    }
}

#[test]
fn sound_effectful_ladder_handler_keeps_anchor_landing() {
    let fx = Fixture::new();
    write_stage_engine(&fx);
    fx.write("pack/loc.pack", "2605=funladdertop\n");
    fx.write(
        "scripts/quests/quest_dragon/scripts/melzars_maze.rs2",
        "[oploc1,funladdertop]\nsound_synth(door_open, 1, 0);\n\
         ~climb_ladder(movecoord(coord, 0, 0, 6400), true);\n",
    );
    write_open_square(&fx, "0 5 40: 2605 10 0\n");
    let defs = loc_defs(&[(2605, 1, 1)]);
    let collision = bake_collision(&fx, &defs, &HashSet::new());
    let graph = derive_transports(fx.path(), &defs, &collision);
    let edge = graph.edges.iter().find(|edge| edge.loc_id == 2605).unwrap();
    assert_eq!(edge.player_delta, None);
    assert_eq!(edge.to, tile(2821, 3432 + CELLAR_SHIFT));
}

#[test]
fn melzars_maze_ladder_uses_the_content_coordinate_as_a_player_delta() {
    let fx = Fixture::new();
    write_stage_engine(&fx);
    fx.write("pack/loc.pack", "2605=funladdertop\n");
    fx.write(
        "scripts/quests/quest_dragon/scripts/melzars_maze.rs2",
        "[oploc1,funladdertop]\n~climb_ladder(movecoord(coord, 0, 0, 6400), true);\n",
    );
    write_open_square(&fx, "0 5 40: 2605 10 0\n");
    let defs = loc_defs(&[(2605, 1, 1)]);
    let collision = bake_collision(&fx, &defs, &HashSet::new());
    let graph = derive_transports(fx.path(), &defs, &collision);
    let edge = graph.edges.iter().find(|edge| edge.loc_id == 2605).unwrap();
    assert_eq!(
        edge.player_delta,
        Some(WorldTile {
            x: 0,
            z: 6400,
            level: 0,
        })
    );
    assert_eq!(edge.to, tile(2821, 3432 + CELLAR_SHIFT));
}

fn write_forced_fixture(fx: &Fixture, handler: &str) {
    write_stage_engine(fx);
    fx.write(
        "scripts/skill_agility/scripts/agility.rs2",
        include_str!("engine_forced_procs.rs2"),
    );
    fx.write("pack/loc.pack", "6000=obstacle\n6001=cleared_obstacle\n");
    fx.write(
        "scripts/obstacle.loc",
        "[obstacle]\nop2=Mine\nparam=level,5\n",
    );
    fx.write("scripts/obstacle.rs2", handler);
    // A two-wide obstacle straddles the solid column: neither side can walk
    // around it, but both sides can operate its actual footprint.
    write_barrier_square(fx, &[], "0 3 46: 6000 10 0\n");
}

fn derive_forced(fx: &Fixture) -> (TransportGraph, WorldCollision) {
    let defs = loc_defs(&[(6000, 2, 2), (6001, 2, 2)]);
    let collision = bake_collision(fx, &defs, &HashSet::new());
    (derive_transports(fx.path(), &defs, &collision), collision)
}

fn write_tool_selector(fx: &Fixture) {
    fx.write("pack/obj.pack", "1001=bronze_tool\n1002=high_tool\n");
    fx.write("scripts/tools.constant", "^wearpos_rhand = 3\n");
    fx.write(
        "scripts/tools.obj",
        "\
[bronze_tool]
wearpos=righthand
param=levelrequire,0
[high_tool]
wearpos=righthand
param=levelrequire,61
",
    );
    fx.write("scripts/tool_checker.rs2", "\
[proc,choose_tool]()(obj)
def_int $level = stat(mining);
def_obj $worn = inv_getobj(worn, ^wearpos_rhand);
if ($level >= oc_param(high_tool, levelrequire) & ($worn = high_tool | inv_total(inv, high_tool) > 0)) {
    return(high_tool);
}
if ($level >= oc_param(bronze_tool, levelrequire) & ($worn = bronze_tool | inv_total(inv, bronze_tool) > 0)) {
    return(bronze_tool);
}
return(null);
");
}

const TOOL_OBSTACLE: &str = "\
[oploc2,obstacle] ~clear_obstacle;
[proc,clear_obstacle]
def_obj $tool = ~choose_tool;
if ($tool = null) { mes(\"Need a usable tool.\"); return; }
if (stat(mining) < 50) { mes(\"Need Mining 50.\"); return; }
anim(oc_param($tool, mining_animation), 0);
loc_change(cleared_obstacle, 3);
p_delay(1);
p_delay(1);
if (coordx(coord) > coordx(loc_coord)) {
    ~forcemove(movecoord(coord, -2, 0, 0));
    ~forcemove(movecoord(coord, -1, 0, 1));
} else {
    ~forcemove(movecoord(coord, 2, 0, 0));
    ~forcemove(movecoord(coord, 1, 0, -1));
}
";

#[test]
fn content_tool_gated_forcemoves_cross_the_whole_sequence_only_when_usable() {
    let fx = Fixture::new();
    write_forced_fixture(&fx, TOOL_OBSTACLE);
    write_tool_selector(&fx);
    let (graph, collision) = derive_forced(&fx);
    let from = tile(2818, 3439);
    let to = tile(2821, 3438);
    let edges: Vec<_> = graph
        .edges
        .iter()
        .filter(|edge| edge.loc_id == 6000)
        .collect();
    assert!(!edges.is_empty());
    for edge in &edges {
        assert_eq!(edge.kind, TransportKind::AgilityShortcut);
        assert_eq!(edge.option, 2);
        assert_eq!(edge.at, tile(2819, 3438));
        assert!(edge.takeoff.is_some());
        assert_eq!(edge.player_delta, None);
        assert_eq!(
            edge.open_loc_id, None,
            "a temporary replacement is not an open-door token"
        );
        assert!(
            edge.worn_req.is_empty(),
            "strict tool gates never auto-equip"
        );
        assert_eq!(
            edge.ticks, 8,
            "both delays and both force-walk steps are priced"
        );
        assert!(edge
            .skill_req
            .iter()
            .any(|&(skill, level)| skill == 14 && level >= 50));
        let start = edge.takeoff.unwrap();
        assert_eq!(
            edge.to.x - start.x,
            if start.x > edge.at.x { -3 } else { 3 }
        );
        assert_eq!(
            edge.to.z - start.z,
            if start.x > edge.at.x { 1 } else { -1 }
        );
    }
    let mut legal = WorldState::empty();
    legal.stats.insert(14, 50);
    legal.inv.insert(1001, 1);
    assert!(crosses(&graph, &collision, from, to, &legal, 6000).unwrap());
    let mut worn = legal.clone();
    worn.inv.clear();
    worn.worn.insert(1001);
    assert!(crosses(&graph, &collision, from, to, &worn, 6000).unwrap());
    for (label, mut state) in [
        ("missing tool", WorldState::empty()),
        ("unknown Mining", {
            let mut state = legal.clone();
            state.stats.clear();
            state
        }),
        ("Mining 49", {
            let mut state = legal.clone();
            state.stats.insert(14, 49);
            state
        }),
        ("unusable high tool", {
            let mut state = legal.clone();
            state.inv.clear();
            state.inv.insert(1002, 1);
            state
        }),
    ] {
        if label == "missing tool" {
            state.stats.insert(14, 50);
        }
        assert!(!edges.iter().any(|edge| state.allows(edge)), "{label}");
        assert!(
            matches!(
                crosses(&graph, &collision, from, to, &state, 6000),
                Err(RouteError::NoPath)
            ),
            "{label}"
        );
    }
    let mut high = WorldState::empty();
    high.stats.insert(14, 61);
    high.inv.insert(1002, 1);
    assert!(crosses(&graph, &collision, from, to, &high, 6000).unwrap());
}

#[test]
fn forced_move_conditions_use_the_updated_player_coord_and_checked_arithmetic() {
    let fx = Fixture::new();
    write_forced_fixture(
        &fx,
        "\
[oploc2,obstacle]
if(stat(agility) < 5) return;
if(coordx(coord) ! 2818) return;
~forcemove(movecoord(coord, calc(1 + 2 * 1), 0, 0));
if(coordx(coord) = 2821) {
    ~forcemove(movecoord(coord, calc(8 / 2 - 3), 0, -1));
} else {
    ~forcemove(movecoord(coord, -8, 0, 0));
}
",
    );
    let (graph, _) = derive_forced(&fx);
    let edge = graph
        .edges
        .iter()
        .find(|edge| edge.loc_id == 6000 && edge.takeoff == Some(tile(2818, 3439)))
        .unwrap();
    assert_eq!(
        edge.to,
        tile(2822, 3438),
        "the second branch reads the first move's endpoint"
    );
    assert_eq!(edge.ticks, 5);
}

#[test]
fn category_exactmoves_read_content_params_and_retain_actual_op_and_takeoff() {
    let fx = Fixture::new();
    write_forced_fixture(
        &fx,
        "\
[oploc5,_crossing]
if(stat(agility) < loc_param(level)) return;
if(coordx(coord) ! 2818) return;
~agility_exactmove(jump_anim, 0, 2, coord, movecoord(coord, calc(7 - 4), 0, 0), 0, 76, 1, false);
",
    );
    fx.write(
        "scripts/obstacle.loc",
        "[obstacle]\ncategory=crossing\nop5=Cross\nparam=level,5\n",
    );
    let (graph, _) = derive_forced(&fx);
    let edges: Vec<_> = graph
        .edges
        .iter()
        .filter(|edge| edge.loc_id == 6000)
        .collect();
    assert!(!edges.is_empty());
    for edge in edges {
        assert_eq!(edge.option, 5);
        assert_eq!(edge.skill_req, [(16, 5)]);
        assert_eq!(edge.takeoff.unwrap().x, 2818);
        assert_eq!(edge.to.x, 2821);
        assert_eq!(edge.ticks, 4);
    }
}

#[test]
fn forced_move_unknown_gates_writes_randomness_and_bad_motion_fail_closed() {
    for (label, body) in [
        ("no requirement", "~forcemove(movecoord(coord, 3, 0, 0));"),
        ("hidden quest window", "if(%plain_flag = 2) ~forcemove(movecoord(coord, 3, 0, 0));"),
        ("unknown gate", "if(stat(agility) < 5) return; if(%missing_flag < 1) return; ~forcemove(movecoord(coord, 3, 0, 0));"),
        ("random fail", "if(stat(agility) < 5) return; if(stat_random(agility, 90, 250) = false) return; ~forcemove(movecoord(coord, 3, 0, 0));"),
        ("random destination", "if(stat(agility) < 5) return; ~forcemove(movecoord(coord, random(4), 0, 0));"),
        ("overflow", "if(stat(agility) < 5) return; ~forcemove(movecoord(coord, calc(2147483647 + 1), 0, 0));"),
        ("divide zero", "if(stat(agility) < 5) return; ~forcemove(movecoord(coord, calc(3 / 0), 0, 0));"),
        ("pre-write", "if(stat(agility) < 5) return; %heroquest = 15; ~forcemove(movecoord(coord, 3, 0, 0));"),
        ("post-write", "if(stat(agility) < 5) return; ~forcemove(movecoord(coord, 3, 0, 0)); %heroquest = 15;"),
        ("loop", "if(stat(agility) < 5) return; while(true) { ~forcemove(movecoord(coord, 3, 0, 0)); }"),
        ("unknown proc", "if(stat(agility) < 5) return; ~unknown_proc; ~forcemove(movecoord(coord, 3, 0, 0));"),
        ("returned move", "if(stat(agility) < 5) return; ~forcemove(movecoord(coord, 3, 0, 0)); return(~unmodelled_move);"),
        ("wrong exact start", "if(stat(agility) < 5) return; p_exactmove(movecoord(coord, 1, 0, 0), movecoord(coord, 3, 0, 0), 0, 76, 1);"),
        ("exact plane change", "if(stat(agility) < 5) return; p_exactmove(coord, movecoord(coord, 3, 1, 0), 0, 76, 1);"),
        ("negative delay", "if(stat(agility) < 5) return; p_delay(-1); ~forcemove(movecoord(coord, 3, 0, 0));"),
    ] {
        let fx = Fixture::new();
        write_forced_fixture(&fx, &format!("[oploc2,obstacle]\n{body}\n"));
        let (graph, _) = derive_forced(&fx);
        assert!(!graph.edges.iter().any(|edge| edge.loc_id == 6000), "{label}");
    }
}

#[test]
fn tool_priority_identity_and_movement_proc_drift_are_not_guessed() {
    for label in [
        "tool identity",
        "primitive drift",
        "conflicting tool level",
        "unused selector",
    ] {
        let fx = Fixture::new();
        write_forced_fixture(&fx, TOOL_OBSTACLE);
        write_tool_selector(&fx);
        match label {
            "unused selector" => fx.write("scripts/obstacle.rs2", "[oploc2,obstacle]\ndef_obj $tool = ~choose_tool;\n~forcemove(movecoord(coord, 3, 0, 0));\n"),
            "tool identity" => fx.write(
                "scripts/obstacle.rs2",
                "\
[oploc2,obstacle]
def_obj $tool = ~choose_tool;
if(stat(mining) < 50) return;
if($tool = bronze_tool) { ~forcemove(movecoord(coord, 3, 0, 0)); }
else { ~forcemove(movecoord(coord, -3, 0, 0)); }
",
            ),
            "primitive drift" => fx.write(
                "scripts/skill_agility/scripts/agility.rs2",
                &include_str!("engine_forced_procs.rs2").replace(
                    "~agility_walk($change_x, $change_z, false);",
                    "p_teleport($dest_coord);",
                ),
            ),
            _ => fx.write(
                "scripts/tool_conflict.obj",
                "[bronze_tool]\nparam=levelrequire,99\n[high_tool]\nparam=levelrequire,99\n",
            ),
        }
        let (graph, _) = derive_forced(&fx);
        assert!(
            !graph.edges.iter().any(|edge| edge.loc_id == 6000),
            "{label}"
        );
    }
}

#[test]
fn directional_membership_paths_are_source_proved_and_bound_to_the_final_edge() {
    let fx = Fixture::new();
    write_stage_engine(&fx);
    fx.write("pack/loc.pack", "900=sanctum_door\n1535=loc_1535\n");
    fx.write("pack/obj.pack", "1001=gown\n1002=robe\n");
    fx.write(
        "scripts/sanctum.rs2",
        "\
[oploc1,sanctum_door]
def_boolean $entering = ~check_axis(coord, loc_coord, loc_angle);
if($entering = false) {
    ~open_and_close_door(loc_1535, $entering, false);
    return;
}
if(map_members = ^false) { mes(\"Members only.\"); return; }
if(inv_total(worn, gown) > 0 & inv_total(worn, robe) > 0) {
    ~open_and_close_door(loc_1535, $entering, false);
    return;
}
",
    );
    write_barrier_square(&fx, &[46], "0 4 46: 900 0 2\n");
    let defs = loc_defs(&[(900, 1, 1)]);
    let collision = bake_collision(&fx, &defs, &HashSet::from([900]));
    // Exercise the actual source producer, not unrelated static ship/NPC
    // fixtures that this deliberately small content tree does not declare.
    let mut graph = TransportGraph::default();
    let mut audit = VarpGateAudit::default();
    stage_door_edges(
        fx.path(),
        &loc_ids_by_name(fx.path()),
        &loc_positions(fx.path()),
        &defs,
        &mut graph,
        &collision,
        &mut HashMap::new(),
        &ObservableGates::from_content(fx.path()),
        &mut audit,
    );
    assert_eq!(
        graph.edges.len(),
        2,
        "both source-derived directional paths"
    );
    require_members_guards(fx.path(), &graph, &audit)
        .expect("renamed source arms need no hand table");
    let ingress = graph
        .edges
        .iter()
        .position(|edge| edge.loc_id == 900 && edge.dir == Some(DoorDir::E))
        .unwrap();
    assert!(graph.edges[ingress].members_req);
    assert_eq!(graph.edges[ingress].worn_all_req, [1001, 1002]);
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| edge.loc_id == 900 && !edge.members_req && edge.worn_all_req.is_empty()),
        "the other direction remains free even on F2P"
    );
    let original = graph.edges[ingress].clone();
    graph.edges[ingress].members_req = false;
    assert!(
        require_members_guards(fx.path(), &graph, &audit).is_err(),
        "missing membership cannot reuse a witness"
    );
    graph.edges[ingress] = original.clone();
    graph.edges[ingress].worn_all_req.clear();
    assert!(
        require_members_guards(fx.path(), &graph, &audit).is_err(),
        "changed disguise cannot reuse a witness"
    );
    graph.edges[ingress] = original;
    graph.edges[ingress].ticks += 1;
    assert!(
        require_members_guards(fx.path(), &graph, &audit).is_err(),
        "witnesses bind every edge field"
    );
}
