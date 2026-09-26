//! Quest-stage/guild doors and guarded ladders ([`super::super::stage_doors`]):
//! synthetic content shaped like the 289 openers each test cites.

use super::*;
use crate::router::{find_with, FindOptions, Leg, RouteError};
use crate::world_state::WorldState;

/// `scripts/doors/scripts/door_procs.rs2:74-101` (check_axis and
/// check_axis_locactive, verbatim).
const DOOR_PROCS: &str = "\
[proc,check_axis](coord $coord, coord $loc_coord, int $angle)(boolean)
switch_int($angle) {
    case ^loc_north, ^loc_south :
        if (coordz($coord) = coordz($loc_coord)) {
            return(true);
        }
    case ^loc_west, ^loc_east :
        if (coordx($coord) = coordx($loc_coord)) {
            return(true);
        }
}
return(false);

[proc,check_axis_locactive](coord $coord)(boolean)
switch_int(loc_angle) {
    case ^loc_north, ^loc_south :
        if (coordz($coord) = coordz(loc_coord)) {
            return(true);
        }
    case ^loc_west, ^loc_east :
        if (coordx($coord) = coordx(loc_coord)) {
            return(true);
        }
}
return(false);
";

/// `scripts/doors/scripts/open_and_close_doors.rs2:9-62` and
/// `open_and_close_double_doors.rs2:63-104`: the teleport sequence of each
/// proc (the leaf/sound tails trimmed).
const OPEN_PROCS_RS2: &str = "\
[proc,open_and_close_door](loc $replacement, boolean $entering, boolean $play_locked_synth)
def_coord $loc_coord = loc_coord;
def_int $angle = loc_angle;
$x, $z = ~door_open($angle, loc_shape);
def_coord $dest = $loc_coord;
if ($entering = true) {
    if (coord ! $loc_coord) {
        p_teleport($loc_coord);
        p_delay(1);
    }
    $dest = movecoord($loc_coord, $x, 0, $z);
}
p_teleport($dest);

[proc,open_and_close_door2](loc $replacement, boolean $entering, synth $sound)
def_int $angle = loc_angle;
def_coord $loc_coord = loc_coord;
$x, $z = ~door_open($angle, loc_shape);
def_coord $dest = $loc_coord;
if ($entering = true) {
    if (coord ! $loc_coord) {
        p_teleport($loc_coord);
        p_delay(1);
    }
    $dest = movecoord(loc_coord, $x, 0, $z);
}
p_teleport($dest);

[proc,open_and_close_double_door2](boolean $entering, int $side, synth $sound)
def_coord $loc_coord = loc_coord;
def_int $angle = loc_angle;
$x, $z = ~door_open($angle, loc_shape);
def_coord $dest = $loc_coord;
if ($entering = true) {
    if (coord ! $loc_coord & coord ! $opposite_coord) {
        p_teleport($loc_coord);
        p_delay(1);
    }
    $dest = movecoord($loc_coord, $x, 0, $z);
}
p_teleport($dest);
";

/// `ladders+stairs/scripts/ladders.rs2:154-161`.
const CLIMB_LADDER_RS2: &str = "\
[proc,climb_ladder](coord $coord, boolean $up)
if ($up = true) {
    anim(human_reachforladder, 0);
} else {
    anim(human_pickupfloor, 0);
}
p_delay(0);
p_telejump($coord);
";

/// The shared engine facts every stage-door fixture needs: the door and
/// ladder procs, `^left`/`^right`, `^true`, the journal colour rule and
/// one quest row (`%heroquest`, "Hero's Quest", complete at 15), a
/// transmitted `%tbwt_main`, an untransmitted `%plain_flag` with no
/// journal row, and the `ladder_cellar` block the climb price follows.
fn write_stage_engine(fx: &Fixture) {
    fx.write("scripts/doors/scripts/door_procs.rs2", DOOR_PROCS);
    fx.write(
        "scripts/doors/scripts/open_and_close_doors.rs2",
        OPEN_PROCS_RS2,
    );
    fx.write(
        "scripts/ladders+stairs/scripts/ladders.rs2",
        &format!(
            "[oploc1,ladder_cellar]\np_arrivedelay;\n~climb_ladder(movecoord(coord(), 0, 0, 6400), false);\n\n{CLIMB_LADDER_RS2}"
        ),
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
