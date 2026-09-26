//! Quest-stage/guild doors and guarded ladders ([`super::super::stage_doors`]):
//! synthetic content shaped like the 289 openers each test cites.

use super::*;
use crate::router::{find_with, FindOptions, Leg, RouteError};
use crate::world_state::WorldState;

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
            ((2821, 3438), 'E', (2822, 3438)),
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
        vec![((2821, 3432), 'S', (2821, 3431))]
    );
}

/// Openers the evaluator cannot prove never cross: talk before the door, an
/// NPC lookup, a key taken, a varp written, a `switch`, a movement after
/// the crossing, a wrong `$entering` constant, an in-progress window
/// (`>= a & < complete`: Hazeel Cult's wall, `quest_hazeelcult.rs2:72-78`),
/// and two worn items at once (Guidor's door, `quest_biohazard.rs2:1-15`).
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
        "if (inv_total(worn, priest_gown) > 0 & inv_total(worn, priest_robe) > 0) {\n    ~open_and_close_door(loc_1535, ~check_axis(coord, loc_coord, loc_angle), false);\n}",
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
        vec![((2821, 3432), 'E', (2822, 3432))],
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
[proc,nudge]\np_teleport(movecoord(coord, 0, 0, 5));\n"
        )
    };
    for (call, crosses) in [
        ("~say_goodbye;", true),
        ("~relocate_after_open;", false),
        ("~nudge;", false),
        ("~missing_proc;", false),
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
