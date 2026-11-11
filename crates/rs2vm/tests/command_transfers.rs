use std::collections::BTreeMap;

use rs2vm::machine::{
    AtomKind, CondClass, Effect, End, Modal, PathResult, PlayerPos, Pred, Run, Stored, Text, Value,
    View,
};
use rs2vm::profile::EngineProfile;
use rs2vm::program::{
    ConfigValue, Enum, Instruction, LocConfig, Operand, Param, Program, Script, Varp,
};
use rs2vm::world::{Alloc, LocInfo, LocObj, PlacedId, PlacedLoc, Tri, World, ZoneKey};
use rs2vm::{Coord, LocTypeId, ScriptId, VarpId};

// These numbers are fixture data. The interpreter sees only the symbolic names below.
const OPCODES: &[(&str, u16)] = &[
    ("PUSH_CONSTANT_INT", 1),
    ("PUSH_CONSTANT_STRING", 2),
    ("POP_STRING_DISCARD", 3),
    ("BRANCH_LESS_THAN_OR_EQUALS", 4),
    ("BRANCH_GREATER_THAN_OR_EQUALS", 5),
    ("RETURN", 6),
    ("PUSH_VARP", 7),
    ("POP_VARP", 8),
    ("enum", 9),
    ("loc_param", 10),
    ("add", 11),
    ("sub", 12),
    ("multiply", 13),
    ("divide", 14),
    ("modulo", 15),
    ("split_init", 16),
    ("split_pagecount", 17),
    ("split_linecount", 18),
    ("p_pausebutton", 19),
    ("inv_total", 20),
    ("p_arrivedelay", 21),
    ("p_delay", 22),
    ("p_teleport", 23),
    ("p_exactmove", 24),
    ("if_openchat", 25),
    ("if_close", 26),
    ("mes", 27),
    ("p_finduid", 28),
    ("uid", 29),
    ("loc_add", 30),
    ("loc_del", 31),
    ("loc_type", 32),
    ("loc_coord", 33),
    ("loc_findallzone", 34),
    ("loc_findnext", 35),
    ("npc_add", 36),
    ("npc_findall", 37),
    ("npc_findnext", 38),
    ("loc_change", 39),
];

fn opcode(name: &str) -> u16 {
    OPCODES
        .iter()
        .find_map(|(candidate, number)| (*candidate == name).then_some(*number))
        .expect("test opcode has a manual number")
}

fn int(name: &str, value: i32) -> Instruction {
    Instruction {
        opcode: opcode(name),
        operand: Operand::Int(value),
    }
}

fn text(name: &str, value: &str) -> Instruction {
    Instruction {
        opcode: opcode(name),
        operand: Operand::Text(value.into()),
    }
}

fn script(id: u32, code: Vec<Instruction>) -> Script {
    Script {
        id: ScriptId(id),
        code,
        ..Script::default()
    }
}

fn program(scripts: Vec<Script>) -> Program {
    let mut program = Program::default();
    for &(name, number) in OPCODES {
        program.opcodes.insert(number, name.into());
    }
    program.triggers.insert("IF_CLOSE".into(), 1);
    program
        .scripts
        .extend(scripts.into_iter().map(|script| (script.id, script)));
    program
}

fn entry(script: u32) -> rs2vm::machine::Entry {
    rs2vm::machine::Entry {
        script: ScriptId(script),
        ints: Vec::new(),
        strings: Vec::new(),
        active_loc: None,
        stand: None,
        protected: false,
    }
}

fn run(program: &Program, world: &dyn World, entry: rs2vm::machine::Entry) -> Run {
    rs2vm::machine::Machine::new(program, EngineProfile::R289, world)
        .run(entry, View::ALL, Default::default())
        .expect("synthetic interpreter entry stays within default limits")
}

#[derive(Default)]
struct TestWorld {
    zones: BTreeMap<ZoneKey, Vec<PlacedLoc>>,
    allocations: BTreeMap<ZoneKey, Alloc>,
    default_allocation: Alloc,
    spare_npc: Tri,
}

impl TestWorld {
    fn allocated() -> Self {
        Self {
            default_allocation: Alloc {
                on_members: Tri::Yes,
                on_free: Tri::Yes,
            },
            ..Self::default()
        }
    }

    fn with_locs(locs: impl IntoIterator<Item = PlacedLoc>) -> Self {
        let mut world = Self::allocated();
        for loc in locs {
            world
                .zones
                .entry(ZoneKey::of(loc.tile))
                .or_default()
                .push(loc);
        }
        world
    }

    fn allocation(&mut self, tile: Coord, allocation: Alloc) {
        self.allocations.insert(ZoneKey::of(tile), allocation);
    }
}

impl World for TestWorld {
    fn zone_locs(&self, zone: ZoneKey) -> &[PlacedLoc] {
        self.zones.get(&zone).map_or(&[], Vec::as_slice)
    }

    fn placed_loc(&self, id: PlacedId) -> Option<PlacedLoc> {
        self.zones
            .values()
            .flat_map(|locs| locs.iter())
            .find(|loc| loc.id == id)
            .copied()
    }

    fn zone_alloc(&self, tile: Coord) -> Alloc {
        self.allocations
            .get(&ZoneKey::of(tile))
            .copied()
            .unwrap_or(self.default_allocation)
    }

    fn npc_spare_capacity(&self) -> Tri {
        self.spare_npc
    }
}

fn tile(x: i32, z: i32) -> Coord {
    Coord::new(x, 0, z)
}

fn placed(id: u32, tile: Coord, ty: i32, shape: u8) -> PlacedLoc {
    PlacedLoc {
        id: PlacedId(id),
        tile,
        info: LocInfo {
            ty: LocTypeId(ty),
            shape,
            angle: 0,
        },
    }
}

fn assert_engine_at(path: &PathResult, pc: u32) {
    match &path.end {
        End::EngineError(fault) => assert_eq!(fault.at.pc, pc, "{fault:?}"),
        other => panic!("expected engine error at pc {pc}, got {other:?}"),
    }
}

fn assert_unsupported_at(path: &PathResult, pc: u32) {
    match &path.end {
        End::Unsupported(fault) => assert_eq!(fault.at.pc, pc, "{fault:?}"),
        other => panic!("expected unsupported path at pc {pc}, got {other:?}"),
    }
}

fn has_move(path: &PathResult, destination: Coord) -> bool {
    path.effects.iter().any(|record| {
        matches!(
            &record.effect,
            Effect::Move {
                to: PlayerPos::Tile(tile),
                ..
            } if *tile == destination
        )
    })
}

#[test]
fn row_06_string_enum_absence_pushes_int_default() {
    let present = script(
        1,
        vec![
            int("PUSH_CONSTANT_INT", i32::from(b'i')),
            int("PUSH_CONSTANT_INT", i32::from(b's')),
            int("PUSH_CONSTANT_INT", 8),
            int("PUSH_CONSTANT_INT", 1),
            int("enum", 0),
            int("RETURN", 0),
        ],
    );
    let absent = script(
        2,
        vec![
            int("PUSH_CONSTANT_INT", i32::from(b'i')),
            int("PUSH_CONSTANT_INT", i32::from(b's')),
            int("PUSH_CONSTANT_INT", 8),
            int("PUSH_CONSTANT_INT", 2),
            int("enum", 0),
            int("POP_STRING_DISCARD", 0),
            int("RETURN", 0),
        ],
    );
    let mut program = program(vec![present, absent]);
    program.packs.enums.insert(
        8,
        Enum {
            input: b'i',
            output: b's',
            int_default: -91,
            string_default: "fallback".into(),
            values: BTreeMap::from([(1, ConfigValue::Text("present".into()))]),
        },
    );
    let world = TestWorld::allocated();

    let present_run = run(&program, &world, entry(1));
    assert_eq!(present_run.paths.len(), 1);
    assert_eq!(
        present_run.paths[0].strings,
        vec![Text::Lit("present".into())]
    );
    assert!(present_run.paths[0].ints.is_empty());

    let absent_run = run(&program, &world, entry(2));
    assert_eq!(absent_run.paths.len(), 1);
    assert_unsupported_at(&absent_run.paths[0], 5);
    assert_eq!(absent_run.paths[0].ints, vec![Value::Known(-91)]);
    assert!(absent_run.paths[0].strings.is_empty());
}

#[test]
fn row_07_string_params_and_varps_use_the_string_stack() {
    let loc = tile(10, 20);
    let world = TestWorld::with_locs([placed(4, loc, 70, 0)]);
    let mut loc_program = program(vec![script(
        1,
        vec![
            int("PUSH_CONSTANT_INT", 9),
            int("loc_param", 0),
            int("RETURN", 0),
        ],
    )]);
    loc_program.packs.params.insert(
        9,
        Param {
            string: true,
            int_default: -1,
            string_default: "missing".into(),
        },
    );
    loc_program.packs.locs.insert(
        LocTypeId(70),
        LocConfig {
            params: BTreeMap::from([(9, ConfigValue::Text("loc text".into()))]),
            ..LocConfig::default()
        },
    );
    let mut loc_entry = entry(1);
    loc_entry.active_loc = Some(LocObj::Placed(PlacedId(4)));
    let loc_run = run(&loc_program, &world, loc_entry);
    assert_eq!(loc_run.paths.len(), 1);
    assert!(loc_run.paths[0].ints.is_empty());
    assert_eq!(loc_run.paths[0].strings, vec![Text::Lit("loc text".into())]);

    let mut varp_program = program(vec![script(
        2,
        vec![
            text("PUSH_CONSTANT_STRING", "saved text"),
            int("POP_VARP", 6),
            int("PUSH_VARP", 6),
            int("RETURN", 0),
        ],
    )]);
    varp_program.packs.varps.insert(
        VarpId(6),
        Varp {
            string: true,
            protect: false,
            transmit: false,
            name: "text_varp".into(),
        },
    );
    let varp_run = run(&varp_program, &world, entry(2));
    assert_eq!(varp_run.paths.len(), 1);
    assert!(varp_run.paths[0].ints.is_empty());
    assert_eq!(
        varp_run.paths[0].strings,
        vec![Text::Lit("saved text".into())]
    );
    assert!(varp_run.paths[0].effects.iter().any(|record| {
        matches!(
            &record.effect,
            Effect::VarWrite {
                varp: VarpId(6),
                value: Stored::Str(Text::Lit(value)),
                ..
            } if value.as_ref() == "saved text"
        )
    }));
}

#[test]
fn row_08_arithmetic_matches_ecmascript_number_cases() {
    let cases = [
        ("multiply", i32::MAX, i32::MAX, 0),
        ("multiply", 65_536, 65_536, 0),
        ("divide", 7, 0, 0),
        ("divide", -7, 2, -3),
        ("modulo", -7, 3, -1),
        ("modulo", 7, 0, 0),
        ("add", i32::MAX, 1, i32::MIN),
    ];
    let mut code = Vec::new();
    for &(name, lhs, rhs, _) in &cases {
        code.push(int("PUSH_CONSTANT_INT", lhs));
        code.push(int("PUSH_CONSTANT_INT", rhs));
        code.push(int(name, 0));
    }
    code.push(int("RETURN", 0));
    let program = program(vec![script(1, code)]);
    let world = TestWorld::allocated();
    let run = run(&program, &world, entry(1));

    assert_eq!(run.paths.len(), 1);
    assert_eq!(
        run.paths[0].ints,
        cases
            .iter()
            .map(|&(_, _, _, expected)| Value::Known(expected))
            .collect::<Vec<_>>()
    );
}

#[test]
fn row_15_and_a4_layout_counts_start_positive_and_bad_inputs_fail_closed() {
    let loop_script = script(
        1,
        vec![
            text("PUSH_CONSTANT_STRING", "page text"),
            int("PUSH_CONSTANT_INT", 100),
            int("PUSH_CONSTANT_INT", 4),
            int("PUSH_CONSTANT_INT", 1),
            int("split_init", 0),
            int("PUSH_CONSTANT_INT", 0),
            int("split_pagecount", 0),
            // Target pc 9: the loop exit is after the pause body at pc 8.
            int("BRANCH_GREATER_THAN_OR_EQUALS", 1),
            int("p_pausebutton", 0),
            int("RETURN", 0),
        ],
    );
    let empty_pagecount = script(2, vec![int("split_pagecount", 0), int("RETURN", 0)]);
    let empty_linecount = script(
        3,
        vec![
            int("PUSH_CONSTANT_INT", 0),
            int("split_linecount", 0),
            int("RETURN", 0),
        ],
    );
    let invalid_lines = script(
        4,
        vec![
            text("PUSH_CONSTANT_STRING", "page text"),
            int("PUSH_CONSTANT_INT", 100),
            int("PUSH_CONSTANT_INT", 0),
            int("PUSH_CONSTANT_INT", 1),
            int("split_init", 0),
            int("RETURN", 0),
        ],
    );
    let negative_page = script(
        5,
        vec![
            text("PUSH_CONSTANT_STRING", "page text"),
            int("PUSH_CONSTANT_INT", 100),
            int("PUSH_CONSTANT_INT", 4),
            int("PUSH_CONSTANT_INT", 1),
            int("split_init", 0),
            int("PUSH_CONSTANT_INT", -1),
            int("split_linecount", 0),
            int("RETURN", 0),
        ],
    );
    let program = program(vec![
        loop_script,
        empty_pagecount,
        empty_linecount,
        invalid_lines,
        negative_page,
    ]);
    let world = TestWorld::allocated();

    let loop_run = run(&program, &world, entry(1));
    assert_eq!(loop_run.paths.len(), 1);
    assert_unsupported_at(&loop_run.paths[0], 8);
    assert!(loop_run
        .paths
        .iter()
        .all(|path| !matches!(&path.end, End::Finished)));

    let initial_count = run(&program, &world, entry(2));
    assert_eq!(initial_count.paths.len(), 1);
    assert_eq!(initial_count.paths[0].ints, vec![Value::Known(0)]);

    let uninitialized_read = run(&program, &world, entry(3));
    assert_eq!(uninitialized_read.paths.len(), 1);
    assert_engine_at(&uninitialized_read.paths[0], 1);

    let invalid_lpp = run(&program, &world, entry(4));
    assert_eq!(invalid_lpp.paths.len(), 1);
    assert_unsupported_at(&invalid_lpp.paths[0], 4);

    let invalid_page = run(&program, &world, entry(5));
    assert_eq!(invalid_page.paths.len(), 1);
    assert_engine_at(&invalid_page.paths[0], 6);
}

fn inventory_after_suspension(id: u32, suspension: &str) -> Script {
    let mut code = vec![
        int("PUSH_CONSTANT_INT", 1),
        int("PUSH_CONSTANT_INT", 42),
        int("inv_total", 0),
        int("PUSH_CONSTANT_INT", 0),
        int("BRANCH_LESS_THAN_OR_EQUALS", 0),
    ];
    if suspension == "p_delay" {
        code.push(int("PUSH_CONSTANT_INT", 0));
    }
    code.push(int(suspension, 0));
    code.extend([
        int("PUSH_CONSTANT_INT", 1),
        int("PUSH_CONSTANT_INT", 42),
        int("inv_total", 0),
        int("PUSH_CONSTANT_INT", 0),
        int("BRANCH_LESS_THAN_OR_EQUALS", 0),
        int("PUSH_CONSTANT_INT", tile(40, 40).0),
        int("p_teleport", 0),
        int("RETURN", 0),
    ]);
    let return_pc = code.len() - 1;
    let first_branch = 4;
    let second_branch = code
        .iter()
        .enumerate()
        .filter(|(_, instruction)| instruction.opcode == opcode("BRANCH_LESS_THAN_OR_EQUALS"))
        .nth(1)
        .map(|(pc, _)| pc)
        .expect("second inventory condition has a branch");
    code[first_branch].operand = Operand::Int(return_pc as i32 - first_branch as i32 - 1);
    code[second_branch].operand = Operand::Int(return_pc as i32 - second_branch as i32 - 1);
    script(id, code)
}

#[test]
fn row_24_inventory_reads_after_suspension_are_live_rivals() {
    let destination = tile(40, 40);
    let stand = tile(10, 10);
    let world = TestWorld::allocated();
    let mut program = program(vec![
        inventory_after_suspension(1, "p_arrivedelay"),
        inventory_after_suspension(2, "p_delay"),
    ]);
    program.packs.invs.insert(1, "inventory".into());

    for script_id in [1, 2] {
        let mut input = entry(script_id);
        input.stand = Some(stand);
        input.protected = true;
        let run = run(&program, &world, input);
        let (moved_index, moved) = run
            .paths
            .iter()
            .enumerate()
            .find(|(_, path)| has_move(path, destination))
            .expect("positive second inventory read can teleport");
        assert!(moved.conds.iter().any(|condition| {
            condition.class == CondClass::Live
                && matches!(
                    &condition.pred,
                    Pred::Cmp { lhs, .. }
                        if run.atom_of(*lhs).is_some_and(|atom| matches!(
                            &atom.kind,
                            AtomKind::InvTotal { inv: 1, obj: 42 }
                        ))
                )
        }));
        assert!(moved
            .conds
            .iter()
            .any(|condition| condition.class == CondClass::Requirement));

        let generalized = run.generalise(
            moved_index,
            |candidate| {
                matches!(&candidate.end, End::Finished)
                    && candidate.player == PlayerPos::Tile(destination)
            },
            |_| 0,
        );
        let rivals = run.rivals(&generalized);
        assert!(rivals.iter().any(|path| {
            matches!(&path.end, End::Finished) && path.player == PlayerPos::Tile(stand)
        }));
        assert!(rivals.iter().any(|path| !has_move(path, destination)));
    }
}

#[test]
fn row_25_protected_varps_survive_suspension_but_unprotected_reads_refresh() {
    let read_twice = |id| {
        script(
            id,
            vec![
                int("PUSH_VARP", 9),
                int("PUSH_CONSTANT_INT", 0),
                int("p_delay", 0),
                int("PUSH_VARP", 9),
                int("RETURN", 0),
            ],
        )
    };
    let mut program = program(vec![read_twice(1), read_twice(2)]);
    program.packs.varps.insert(
        VarpId(9),
        Varp {
            string: false,
            protect: true,
            transmit: false,
            name: "protected".into(),
        },
    );
    program.packs.varps.insert(
        VarpId(10),
        Varp {
            string: false,
            protect: false,
            transmit: false,
            name: "unprotected".into(),
        },
    );
    // The second script reads the unprotected variable.
    program
        .scripts
        .get_mut(&ScriptId(2))
        .expect("script exists")
        .code[0] = int("PUSH_VARP", 10);
    program
        .scripts
        .get_mut(&ScriptId(2))
        .expect("script exists")
        .code[3] = int("PUSH_VARP", 10);
    let world = TestWorld::allocated();

    let mut protected_entry = entry(1);
    protected_entry.protected = true;
    let protected = run(&program, &world, protected_entry);
    assert_eq!(protected.paths.len(), 1);
    assert_eq!(protected.paths[0].ints.len(), 2);
    assert_eq!(protected.paths[0].ints[0], protected.paths[0].ints[1]);
    assert_eq!(
        protected
            .atom_of(protected.paths[0].ints[0])
            .expect("protected varp observation")
            .class,
        CondClass::Hidden
    );

    let mut unprotected_entry = entry(2);
    unprotected_entry.protected = true;
    let unprotected = run(&program, &world, unprotected_entry);
    assert_eq!(unprotected.paths.len(), 1);
    assert_eq!(unprotected.paths[0].ints.len(), 2);
    assert_ne!(unprotected.paths[0].ints[0], unprotected.paths[0].ints[1]);
    assert_eq!(
        unprotected
            .atom_of(unprotected.paths[0].ints[1])
            .expect("post-suspension varp observation")
            .class,
        CondClass::Live
    );
}

fn loc_iterator_script(id: u32, suspension: &str) -> Script {
    let mut code = vec![
        int("PUSH_CONSTANT_INT", tile(10, 20).0),
        int("loc_findallzone", 0),
    ];
    if suspension == "p_delay" {
        code.push(int("PUSH_CONSTANT_INT", 0));
    }
    code.push(int(suspension, 0));
    code.extend([int("loc_findnext", 0), int("RETURN", 0)]);
    script(id, code)
}

fn npc_iterator_script(id: u32) -> Script {
    script(
        id,
        vec![
            int("PUSH_CONSTANT_INT", tile(10, 20).0),
            int("PUSH_CONSTANT_INT", 1),
            int("PUSH_CONSTANT_INT", 0),
            int("PUSH_CONSTANT_INT", 0),
            int("npc_findall", 0),
            int("PUSH_CONSTANT_INT", 0),
            int("p_delay", 0),
            int("npc_findnext", 0),
            int("RETURN", 0),
        ],
    )
}

fn assert_has_finished_int(run: &Run, value: i32) {
    assert!(run
        .paths
        .iter()
        .any(|path| { matches!(&path.end, End::Finished) && path.ints == [Value::Known(value)] }));
}

#[test]
fn row_26_suspension_ages_location_and_npc_iterators() {
    let loc_tile = tile(10, 20);
    let nonempty_world = TestWorld::with_locs([placed(1, loc_tile, 3, 0)]);
    let empty_world = TestWorld::allocated();

    let stale_loc_program = program(vec![loc_iterator_script(1, "p_delay")]);
    let mut input = entry(1);
    input.protected = true;
    let stale_loc = run(&stale_loc_program, &nonempty_world, input);
    assert_eq!(stale_loc.paths.len(), 1);
    assert_engine_at(&stale_loc.paths[0], 4);

    let empty_loc_program = program(vec![loc_iterator_script(2, "p_delay")]);
    let mut input = entry(2);
    input.protected = true;
    let empty_loc = run(&empty_loc_program, &empty_world, input);
    assert_eq!(empty_loc.paths.len(), 1);
    assert_has_finished_int(&empty_loc, 0);

    let maybe_stale_loc_program = program(vec![loc_iterator_script(3, "p_arrivedelay")]);
    let mut input = entry(3);
    input.protected = true;
    let maybe_stale_loc = run(&maybe_stale_loc_program, &nonempty_world, input);
    assert_eq!(maybe_stale_loc.paths.len(), 2);
    assert!(maybe_stale_loc
        .paths
        .iter()
        .any(|path| matches!(&path.end, End::EngineError(_))));
    assert_has_finished_int(&maybe_stale_loc, 1);

    let npc_program = program(vec![npc_iterator_script(4)]);
    let mut input = entry(4);
    input.protected = true;
    let npc = run(&npc_program, &empty_world, input);
    assert_eq!(npc.paths.len(), 2);
    assert!(npc
        .paths
        .iter()
        .any(|path| matches!(&path.end, End::EngineError(_))));
    assert_has_finished_int(&npc, 0);
}

#[test]
fn row_18_delete_add_delay_restores_the_placed_location() {
    let loc_tile = tile(10, 20);
    let world = TestWorld::with_locs([placed(500, loc_tile, 10, 0)]);
    let program = program(vec![script(
        1,
        vec![
            int("PUSH_CONSTANT_INT", 500),
            int("loc_del", 0),
            int("PUSH_CONSTANT_INT", loc_tile.0),
            int("PUSH_CONSTANT_INT", 20),
            int("PUSH_CONSTANT_INT", 0),
            int("PUSH_CONSTANT_INT", 0),
            int("PUSH_CONSTANT_INT", 3),
            int("loc_add", 0),
            int("PUSH_CONSTANT_INT", 2),
            int("p_delay", 0),
            int("loc_type", 0),
            int("RETURN", 0),
        ],
    )]);
    let mut input = entry(1);
    input.active_loc = Some(LocObj::Placed(PlacedId(500)));
    input.protected = true;
    let run = run(&program, &world, input);

    assert_eq!(run.paths.len(), 1);
    assert!(matches!(&run.paths[0].end, End::Finished));
    assert_eq!(run.paths[0].ints, vec![Value::Known(10)]);
    assert_eq!(run.paths[0].ticks.min, 3);
    assert_eq!(run.paths[0].ticks.max, 3);
    assert!(run.paths[0].effects.iter().any(|record| {
        matches!(
            &record.effect,
            Effect::Loc {
                kind: rs2vm::machine::LocKind::Del,
                obj: LocObj::Placed(PlacedId(500)),
                ..
            }
        )
    }));
    assert!(run.paths[0].effects.iter().any(|record| {
        matches!(
            &record.effect,
            Effect::Loc {
                kind: rs2vm::machine::LocKind::Add,
                obj: LocObj::Placed(PlacedId(500)),
                ty: LocTypeId(20),
                ..
            }
        )
    }));
}

#[test]
fn location_effects_resolve_placed_and_created_geometry() {
    let mut source = placed(500, tile(10, 20), 10, 10);
    source.info.angle = 3;
    let world = TestWorld::with_locs([source]);

    for created in [false, true] {
        let (obj, geometry, shape, angle) = if created {
            (LocObj::Created(0), tile(11, 20), 4, 2)
        } else {
            (LocObj::Placed(PlacedId(500)), source.tile, 10, 3)
        };
        let mut code = Vec::new();
        let mut expected = Vec::new();
        if created {
            for value in [geometry.0, 30, i32::from(angle), i32::from(shape), 5] {
                code.push(int("PUSH_CONSTANT_INT", value));
            }
            code.push(int("loc_add", 0));
            expected.push(Effect::Loc {
                kind: rs2vm::machine::LocKind::Add,
                obj,
                ty: LocTypeId(30),
                tile: geometry,
                shape,
                angle,
                duration: 5,
            });
        }
        code.extend([
            int("PUSH_CONSTANT_INT", 20),
            int("PUSH_CONSTANT_INT", 5),
            int("loc_change", 0),
            int("PUSH_CONSTANT_INT", 3),
            int("loc_del", 0),
            int("RETURN", 0),
        ]);
        expected.extend([
            Effect::Loc {
                kind: rs2vm::machine::LocKind::Change,
                obj,
                ty: LocTypeId(20),
                tile: geometry,
                shape,
                angle,
                duration: 5,
            },
            Effect::Loc {
                kind: rs2vm::machine::LocKind::Del,
                obj,
                ty: LocTypeId(20),
                tile: geometry,
                shape,
                angle,
                duration: 3,
            },
        ]);
        let program = program(vec![script(1, code)]);
        let mut input = entry(1);
        input.active_loc = Some(LocObj::Placed(PlacedId(500)));
        let result = run(&program, &world, input);

        assert_eq!(result.paths.len(), 1);
        assert_eq!(result.paths[0].end, End::Finished);
        let actual: Vec<_> = result.paths[0]
            .effects
            .iter()
            .filter(|record| matches!(record.effect, Effect::Loc { .. }))
            .map(|record| record.effect.clone())
            .collect();
        assert_eq!(actual, expected, "created={created}");
    }
}

#[test]
fn row_23_zone_mutation_invalidates_an_active_location_iterator() {
    let loc_tile = tile(10, 20);
    let world = TestWorld::with_locs([placed(13, loc_tile, 10, 0)]);
    let program = program(vec![script(
        1,
        vec![
            int("PUSH_CONSTANT_INT", loc_tile.0),
            int("loc_findallzone", 0),
            int("PUSH_CONSTANT_INT", 3),
            int("loc_del", 0),
            int("loc_findnext", 0),
            int("RETURN", 0),
        ],
    )]);
    let mut input = entry(1);
    input.active_loc = Some(LocObj::Placed(PlacedId(13)));
    let run = run(&program, &world, input);

    assert_eq!(run.paths.len(), 1);
    assert_unsupported_at(&run.paths[0], 4);
    assert!(matches!(
        &run.paths[0].end,
        End::Unsupported(fault) if fault.reason.contains("loc list changed during iteration")
    ));
}

#[test]
fn row_28_input_pause_cancels_tail_and_closes_modal() {
    let first = tile(20, 30);
    let second = tile(40, 50);
    let program = program(vec![script(
        1,
        vec![
            int("PUSH_CONSTANT_INT", first.0),
            int("p_teleport", 0),
            int("PUSH_CONSTANT_INT", 61),
            int("if_openchat", 0),
            int("p_pausebutton", 0),
            int("PUSH_CONSTANT_INT", second.0),
            int("p_teleport", 0),
            int("RETURN", 0),
        ],
    )]);
    let world = TestWorld::allocated();
    let run = run(&program, &world, entry(1));

    assert_eq!(run.paths.len(), 1);
    let path = &run.paths[0];
    assert!(matches!(&path.end, End::AwaitingInput));
    assert_eq!(path.player, PlayerPos::Tile(first));
    assert_eq!(path.modal, Modal::default());
    assert_eq!(
        path.effects
            .iter()
            .filter(|record| matches!(&record.effect, Effect::Move { .. }))
            .count(),
        1
    );
    assert!(path
        .effects
        .iter()
        .any(|record| matches!(&record.effect, Effect::WeakQueueCleared)));
    assert!(!has_move(path, second));
}

fn close_lookup_key(trigger: u8, kind: i32, subject: i32) -> i32 {
    i32::from(trigger) | (kind << 8) | subject.wrapping_shl(10)
}

#[test]
fn row_29_close_callback_blocks_a_cancelled_pause_candidate() {
    let destination = tile(20, 30);
    let interface = 61;
    let mut program = program(vec![script(
        1,
        vec![
            int("PUSH_CONSTANT_INT", destination.0),
            int("p_teleport", 0),
            int("PUSH_CONSTANT_INT", interface),
            int("if_openchat", 0),
            int("p_pausebutton", 0),
            text("PUSH_CONSTANT_STRING", "cancelled tail"),
            int("mes", 0),
            int("RETURN", 0),
        ],
    )]);
    let mut callback = script(
        99,
        vec![
            int("PUSH_CONSTANT_INT", 123),
            int("POP_VARP", 7),
            int("RETURN", 0),
        ],
    );
    callback.lookup = close_lookup_key(1, 2, interface);
    program.scripts.insert(callback.id, callback);
    program.packs.varps.insert(
        VarpId(7),
        Varp {
            string: false,
            protect: true,
            transmit: false,
            name: "callback_write".into(),
        },
    );
    let world = TestWorld::allocated();
    let mut input = entry(1);
    input.protected = true;
    let run = run(&program, &world, input);

    assert_eq!(run.paths.len(), 1);
    assert_unsupported_at(&run.paths[0], 4);
    assert!(matches!(
        &run.paths[0].end,
        End::Unsupported(fault) if fault.reason.contains("close callback")
    ));
    assert_eq!(run.paths[0].modal.chat, Some(interface));
    assert!(!run.paths[0]
        .effects
        .iter()
        .any(|record| matches!(&record.effect, Effect::Message { .. })));
    assert!(!run.paths[0].effects.iter().any(|record| {
        matches!(
            &record.effect,
            Effect::VarWrite {
                varp: VarpId(7),
                ..
            }
        )
    }));
    assert!(!run
        .paths
        .iter()
        .any(|path| { matches!(&path.end, End::Finished | End::AwaitingInput) }));
}

#[test]
fn row_30_input_pause_without_modal_fails_closed() {
    let destination = tile(20, 30);
    let program = program(vec![script(
        1,
        vec![
            int("PUSH_CONSTANT_INT", destination.0),
            int("p_teleport", 0),
            int("p_pausebutton", 0),
            int("RETURN", 0),
        ],
    )]);
    let world = TestWorld::allocated();
    let run = run(&program, &world, entry(1));

    assert_eq!(run.paths.len(), 1);
    assert_unsupported_at(&run.paths[0], 2);
    assert!(!matches!(&run.paths[0].end, End::AwaitingInput));
}

#[test]
fn row_31_automatic_close_differs_from_explicit_close() {
    let automatic = script(
        1,
        vec![
            int("PUSH_CONSTANT_INT", 61),
            int("if_openchat", 0),
            int("PUSH_CONSTANT_INT", 0),
            int("p_delay", 0),
            int("RETURN", 0),
        ],
    );
    let explicit = script(
        2,
        vec![
            int("PUSH_CONSTANT_INT", 61),
            int("if_openchat", 0),
            int("if_close", 0),
            int("RETURN", 0),
        ],
    );
    let possible = script(
        3,
        vec![
            int("PUSH_CONSTANT_INT", 61),
            int("if_openchat", 0),
            int("p_arrivedelay", 0),
            int("RETURN", 0),
        ],
    );
    let program = program(vec![automatic, explicit, possible]);
    let world = TestWorld::allocated();

    let mut input = entry(1);
    input.protected = true;
    let automatic = run(&program, &world, input);
    assert_eq!(automatic.paths.len(), 1);
    assert!(matches!(&automatic.paths[0].end, End::Finished));
    assert_eq!(automatic.paths[0].modal, Modal::default());
    assert!(!automatic.paths[0]
        .effects
        .iter()
        .any(|record| matches!(&record.effect, Effect::WeakQueueCleared)));

    let explicit = run(&program, &world, entry(2));
    assert_eq!(explicit.paths.len(), 1);
    assert!(matches!(&explicit.paths[0].end, End::Finished));
    assert_eq!(explicit.paths[0].modal, Modal::default());
    assert!(explicit.paths[0]
        .effects
        .iter()
        .any(|record| matches!(&record.effect, Effect::WeakQueueCleared)));

    let mut input = entry(3);
    input.protected = true;
    let possible = run(&program, &world, input);
    assert_eq!(possible.paths.len(), 1);
    assert_unsupported_at(&possible.paths[0], 3);
}

#[test]
fn row_32_p_finduid_keeps_the_one_players_protected_state() {
    let matching = script(
        1,
        vec![
            int("PUSH_CONSTANT_INT", 77),
            int("POP_VARP", 12),
            int("uid", 0),
            int("p_finduid", 0),
            int("PUSH_VARP", 12),
            int("RETURN", 0),
        ],
    );
    let wrong_uid = script(
        2,
        vec![
            int("PUSH_CONSTANT_INT", 77),
            int("POP_VARP", 12),
            int("PUSH_VARP", 12),
            int("p_finduid", 0),
            int("PUSH_VARP", 12),
            int("RETURN", 0),
        ],
    );
    let unprotected = script(
        3,
        vec![int("uid", 0), int("p_finduid", 0), int("RETURN", 0)],
    );
    let mut program = program(vec![matching, wrong_uid, unprotected]);
    program.packs.varps.insert(
        VarpId(12),
        Varp {
            string: false,
            protect: true,
            transmit: false,
            name: "protected".into(),
        },
    );
    let world = TestWorld::allocated();

    let mut input = entry(1);
    input.protected = true;
    let matching = run(&program, &world, input);
    assert_eq!(matching.paths.len(), 1);
    assert!(matches!(&matching.paths[0].end, End::Finished));
    assert_eq!(
        matching.paths[0].ints,
        vec![Value::Known(1), Value::Known(77)]
    );

    let mut input = entry(2);
    input.protected = true;
    let wrong_uid = run(&program, &world, input);
    assert_eq!(wrong_uid.paths.len(), 1);
    assert_unsupported_at(&wrong_uid.paths[0], 3);
    assert!(wrong_uid.paths[0].ints.is_empty());

    let unprotected = run(&program, &world, entry(3));
    assert_eq!(unprotected.paths.len(), 1);
    assert_unsupported_at(&unprotected.paths[0], 1);
}

#[test]
fn row_44_multiple_moves_keep_the_last_landing_and_delay_ticks() {
    let start = tile(10, 10);
    let first = tile(20, 20);
    let second = tile(30, 30);
    let exact_end = tile(40, 40);
    let multi = script(
        1,
        vec![
            int("PUSH_CONSTANT_INT", first.0),
            int("p_teleport", 0),
            int("PUSH_CONSTANT_INT", 1),
            int("p_delay", 0),
            int("PUSH_CONSTANT_INT", second.0),
            int("p_teleport", 0),
            int("RETURN", 0),
        ],
    );
    let exact_then_teleport = script(
        2,
        vec![
            int("PUSH_CONSTANT_INT", start.0),
            int("PUSH_CONSTANT_INT", exact_end.0),
            int("PUSH_CONSTANT_INT", 0),
            int("PUSH_CONSTANT_INT", 1),
            int("PUSH_CONSTANT_INT", 0),
            int("p_exactmove", 0),
            int("PUSH_CONSTANT_INT", 1),
            int("p_delay", 0),
            int("PUSH_CONSTANT_INT", exact_end.0),
            int("p_teleport", 0),
            int("RETURN", 0),
        ],
    );
    let program = program(vec![multi, exact_then_teleport]);
    let world = TestWorld::allocated();

    let mut input = entry(1);
    input.protected = true;
    let multi = run(&program, &world, input);
    assert_eq!(multi.paths.len(), 1);
    assert_eq!(multi.paths[0].player, PlayerPos::Tile(second));
    assert_eq!(multi.paths[0].ticks.min, 2);
    assert_eq!(multi.paths[0].ticks.max, 2);
    assert!(has_move(&multi.paths[0], first));
    assert!(has_move(&multi.paths[0], second));

    let mut input = entry(2);
    input.protected = true;
    input.stand = Some(start);
    let exact = run(&program, &world, input);
    assert_eq!(exact.paths.len(), 1);
    assert_eq!(exact.paths[0].player, PlayerPos::Tile(exact_end));
    assert_eq!(exact.paths[0].ticks.min, 2);
    assert_eq!(exact.paths[0].ticks.max, 2);
    assert!(exact.paths[0].effects.iter().any(|record| {
        matches!(
            &record.effect,
            Effect::Move {
                kind: rs2vm::machine::MoveKind::Exact { .. },
                to: PlayerPos::Tile(tile),
            } if *tile == exact_end
        )
    }));
}

#[test]
fn row_45_unallocated_teleport_records_failure_and_keeps_running() {
    let stand = tile(10, 10);
    let destination = tile(100, 100);
    let mut world = TestWorld::allocated();
    world.allocation(
        destination,
        Alloc {
            on_members: Tri::No,
            on_free: Tri::No,
        },
    );
    let program = program(vec![script(
        1,
        vec![
            int("PUSH_CONSTANT_INT", destination.0),
            int("p_teleport", 0),
            int("RETURN", 0),
        ],
    )]);
    let mut input = entry(1);
    input.stand = Some(stand);
    let run = run(&program, &world, input);

    assert_eq!(run.paths.len(), 1);
    assert!(matches!(&run.paths[0].end, End::Finished));
    assert_eq!(run.paths[0].player, PlayerPos::Tile(stand));
    assert!(run.paths[0]
        .effects
        .iter()
        .any(|record| { matches!(&record.effect, Effect::NoMove { to } if *to == destination) }));
    assert!(run.paths[0].effects.iter().any(|record| {
        matches!(
            &record.effect,
            Effect::Message {
                text: Text::Lit(message)
            } if message.as_ref() == "Invalid teleport!"
        )
    }));
}

#[test]
fn row_46_membership_allocation_uses_the_stand_world_kind() {
    let free_stand = tile(10, 10);
    let members_stand = tile(30, 30);
    let destination = tile(100, 100);
    let mut world = TestWorld::allocated();
    world.allocation(
        free_stand,
        Alloc {
            on_members: Tri::Yes,
            on_free: Tri::Yes,
        },
    );
    world.allocation(
        members_stand,
        Alloc {
            on_members: Tri::Yes,
            on_free: Tri::No,
        },
    );
    world.allocation(
        destination,
        Alloc {
            on_members: Tri::Yes,
            on_free: Tri::No,
        },
    );
    let program = program(vec![script(
        1,
        vec![
            int("PUSH_CONSTANT_INT", destination.0),
            int("p_teleport", 0),
            int("RETURN", 0),
        ],
    )]);

    let mut free_input = entry(1);
    free_input.stand = Some(free_stand);
    let free = run(&program, &world, free_input);
    assert_eq!(free.paths.len(), 2);
    let moved = free
        .paths
        .iter()
        .find(|path| has_move(path, destination))
        .expect("members path reaches the allocated zone");
    assert!(moved.conds.iter().any(|condition| {
        condition.class == CondClass::Requirement
            && matches!(
                &condition.pred,
                Pred::Cmp { lhs, .. }
                    if free.atom_of(*lhs).is_some_and(|atom| {
                        matches!(&atom.kind, AtomKind::Members)
                    })
            )
    }));
    assert!(free.paths.iter().any(|path| {
        path.player == PlayerPos::Tile(free_stand)
            && path
                .effects
                .iter()
                .any(|record| matches!(&record.effect, Effect::NoMove { .. }))
    }));

    let mut members_input = entry(1);
    members_input.stand = Some(members_stand);
    let members = run(&program, &world, members_input);
    assert_eq!(members.paths.len(), 1);
    assert!(has_move(&members.paths[0], destination));
    assert!(members.paths[0].conds.is_empty());
}

#[test]
fn row_22_secondary_loc_iteration_preserves_primary_and_reads_its_coordinate() {
    let first_tile = tile(10, 20);
    let second_tile = tile(11, 20);
    let primary_tile = tile(40, 40);
    let world = TestWorld::with_locs([
        placed(10, first_tile, 11, 0),
        placed(11, second_tile, 22, 0),
        placed(12, primary_tile, 33, 0),
    ]);
    let program = program(vec![script(
        1,
        vec![
            int("PUSH_CONSTANT_INT", first_tile.0),
            int("loc_findallzone", 0),
            int("loc_findnext", 1),
            int("loc_type", 1),
            int("loc_findnext", 1),
            int("loc_coord", 1),
            int("loc_findnext", 1),
            int("loc_type", 0),
            int("RETURN", 0),
        ],
    )]);
    let mut input = entry(1);
    input.active_loc = Some(LocObj::Placed(PlacedId(12)));
    let run = run(&program, &world, input);

    assert_eq!(run.paths.len(), 1);
    assert_eq!(
        run.paths[0].ints,
        vec![
            Value::Known(1),
            Value::Known(22),
            Value::Known(1),
            Value::Known(first_tile.0),
            Value::Known(0),
            Value::Known(33),
        ]
    );
}

#[test]
fn decision_a3_npc_add_full_errors_before_teleport_and_unknown_keeps_rival() {
    let destination = tile(50, 60);
    let code = vec![
        int("PUSH_CONSTANT_INT", tile(10, 20).0),
        int("PUSH_CONSTANT_INT", 7),
        int("PUSH_CONSTANT_INT", 3),
        int("npc_add", 0),
        int("PUSH_CONSTANT_INT", destination.0),
        int("p_teleport", 0),
        int("RETURN", 0),
    ];
    let program = program(vec![script(1, code)]);
    let mut full_world = TestWorld::allocated();
    full_world.spare_npc = Tri::No;
    let full = run(&program, &full_world, entry(1));
    assert_eq!(full.paths.len(), 1);
    assert_engine_at(&full.paths[0], 3);
    assert!(!full.paths[0]
        .effects
        .iter()
        .any(|record| matches!(&record.effect, Effect::Move { .. })));

    let mut spare_world = TestWorld::allocated();
    spare_world.spare_npc = Tri::Yes;
    let spare = run(&program, &spare_world, entry(1));
    assert_eq!(spare.paths.len(), 1);
    assert!(matches!(&spare.paths[0].end, End::Finished));
    assert!(has_move(&spare.paths[0], destination));
    assert!(spare.paths[0].effects.iter().any(|record| {
        matches!(
            &record.effect,
            Effect::Npc { command } if *command == "npc_add"
        )
    }));

    let unknown_world = TestWorld::allocated();
    let unknown = run(&program, &unknown_world, entry(1));
    assert_eq!(unknown.paths.len(), 2);
    let moved_index = unknown
        .paths
        .iter()
        .position(|path| has_move(path, destination))
        .expect("spare-capacity arm reaches the teleport");
    assert!(unknown.paths.iter().any(|path| {
        matches!(&path.end, End::EngineError(_))
            && path.at.pc == 3
            && path.conds.iter().any(|condition| {
                condition.class == CondClass::Live
                    && matches!(
                        &condition.pred,
                        Pred::Cmp { lhs, .. }
                            if unknown.atom_of(*lhs).is_some_and(|atom| {
                                matches!(
                                    &atom.kind,
                                    AtomKind::Live(name) if *name == "npc_capacity"
                                )
                            })
                    )
            })
    }));
    let generalized = unknown.generalise(
        moved_index,
        |path| matches!(&path.end, End::Finished) && path.player == PlayerPos::Tile(destination),
        |_| 0,
    );
    assert!(generalized.is_empty());
    let rivals = unknown.rivals(&generalized);
    assert_eq!(rivals.len(), 2);
    assert!(rivals
        .iter()
        .any(|path| matches!(&path.end, End::EngineError(_))));
    assert!(rivals.iter().any(|path| has_move(path, destination)));
}

#[test]
fn one_suspension_before_a_fork_proves_each_automatic_close() {
    let mut program = program(vec![script(
        1,
        vec![
            int("PUSH_CONSTANT_INT", 0),
            int("p_delay", 0),
            int("PUSH_VARP", 7),
            int("PUSH_CONSTANT_INT", 0),
            int("BRANCH_GREATER_THAN_OR_EQUALS", 3),
            text("PUSH_CONSTANT_STRING", "left"),
            int("mes", 0),
            int("RETURN", 0),
            text("PUSH_CONSTANT_STRING", "right"),
            int("mes", 0),
            int("RETURN", 0),
        ],
    )]);
    program.packs.varps.insert(
        VarpId(7),
        Varp {
            string: false,
            protect: false,
            transmit: true,
            name: "fork".into(),
        },
    );
    let world = TestWorld::allocated();
    let mut input = entry(1);
    input.protected = true;
    let result = run(&program, &world, input);
    assert_eq!(result.paths.len(), 2);
    assert_eq!(result.transfers.suspensions, 1);
    assert_eq!(result.transfers.automatic_closes, 2);
    for path in &result.paths {
        assert_eq!(path.end, End::Finished);
        assert_eq!(path.transfers.suspensions, 1);
        assert_eq!(path.transfers.automatic_closes, 1);
    }

    let mut input = entry(1);
    input.protected = true;
    let joined = rs2vm::machine::Machine::new(&program, EngineProfile::R289, &world)
        .run(input, View::NAV, Default::default())
        .expect("bounded NAV erasure");
    assert_eq!(joined.paths.len(), 1);
    assert_eq!(joined.paths[0].transfers.suspensions, 2);
    assert_eq!(joined.paths[0].transfers.automatic_closes, 2);
}

#[test]
fn terminal_erasure_keeps_both_input_cancellation_proofs() {
    let mut program = program(vec![script(
        1,
        vec![
            int("PUSH_CONSTANT_INT", 61),
            int("if_openchat", 0),
            int("PUSH_VARP", 7),
            int("PUSH_CONSTANT_INT", 0),
            int("BRANCH_GREATER_THAN_OR_EQUALS", 2),
            int("p_pausebutton", 0),
            int("RETURN", 0),
            int("p_pausebutton", 0),
            int("RETURN", 0),
        ],
    )]);
    program.packs.varps.insert(
        VarpId(7),
        Varp {
            string: false,
            protect: false,
            transmit: true,
            name: "fork".into(),
        },
    );
    let result = run(&program, &TestWorld::allocated(), entry(1));
    assert_eq!(result.stats.paths, 2);
    assert_eq!(result.paths.len(), 1);
    assert_eq!(result.paths[0].end, End::AwaitingInput);
    assert_eq!(result.transfers.cancellations, 2);
    assert_eq!(result.paths[0].transfers.cancellations, 2);
    assert_eq!(result.paths[0].modal, Modal::default());
}
