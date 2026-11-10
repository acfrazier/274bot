use std::collections::BTreeMap;

use rs2vm::{
    machine::{
        AtomKind, CondClass, Effect, End, Entry, Expr, Limits, Machine, PathResult, PlayerPos,
        Pred, Run, Value, View,
    },
    profile::EngineProfile,
    program::{Instruction, Operand, Packs, Program, Script, Signature, Varbit, Varp},
    world::{Alloc, PlacedId, PlacedLoc, Tri, World, ZoneKey},
    Coord, OpCount, ScriptId, Site, VarpId,
};

const OPCODES: &[(&str, u16)] = &[
    ("PUSH_CONSTANT_INT", 1),
    ("PUSH_CONSTANT_STRING", 2),
    ("PUSH_INT_LOCAL", 3),
    ("POP_INT_LOCAL", 4),
    ("PUSH_STRING_LOCAL", 5),
    ("POP_STRING_LOCAL", 6),
    ("POP_INT_DISCARD", 7),
    ("POP_STRING_DISCARD", 8),
    ("BRANCH", 9),
    ("BRANCH_NOT", 10),
    ("BRANCH_EQUALS", 11),
    ("BRANCH_LESS_THAN", 12),
    ("BRANCH_GREATER_THAN", 13),
    ("BRANCH_LESS_THAN_OR_EQUALS", 14),
    ("BRANCH_GREATER_THAN_OR_EQUALS", 15),
    ("SWITCH", 16),
    ("RETURN", 17),
    ("GOSUB_WITH_PARAMS", 18),
    ("JUMP_WITH_PARAMS", 19),
    ("GOSUB", 20),
    ("JUMP", 21),
    ("JOIN_STRING", 22),
    ("PUSH_VARP", 23),
    ("POP_VARP", 24),
    ("PUSH_VARBIT", 25),
    ("POP_VARBIT", 26),
    ("PUSH_VARN", 27),
    ("PUSH_VARS", 28),
    ("add", 29),
    ("sub", 30),
    ("multiply", 31),
    ("divide", 32),
    ("modulo", 33),
    ("testbit", 34),
    ("getbit_range", 35),
    ("clearbit", 36),
    ("clearbit_range", 37),
    ("setbit_range_toint", 38),
    ("stat", 39),
    ("map_clock", 40),
    ("npc_find", 41),
    ("p_teleport", 42),
    ("p_delay", 43),
    ("mes", 44),
    ("cam_reset", 45),
    ("outside_slice_command", 46),
];

#[derive(Default)]
struct FixtureWorld;

impl World for FixtureWorld {
    fn zone_locs(&self, _: ZoneKey) -> &[PlacedLoc] {
        &[]
    }

    fn placed_loc(&self, _: PlacedId) -> Option<PlacedLoc> {
        None
    }

    fn zone_alloc(&self, _: Coord) -> Alloc {
        Alloc {
            on_members: Tri::Yes,
            on_free: Tri::Yes,
        }
    }
}

static WORLD: FixtureWorld = FixtureWorld;

fn opcode(name: &str) -> u16 {
    OPCODES
        .iter()
        .find_map(|(candidate, number)| (*candidate == name).then_some(*number))
        .expect("test opcode has an explicit synthetic number")
}

fn int(name: &str, value: i32) -> Instruction {
    Instruction {
        opcode: opcode(name),
        operand: Operand::Int(value),
    }
}

fn command(name: &str) -> Instruction {
    int(name, 0)
}

fn string(value: &str) -> Instruction {
    Instruction {
        opcode: opcode("PUSH_CONSTANT_STRING"),
        operand: Operand::Text(value.into()),
    }
}

fn script(id: u32, code: Vec<Instruction>, int_args: u16, int_locals: u16) -> Script {
    Script {
        id: ScriptId(id),
        code,
        int_args,
        int_locals,
        ..Script::default()
    }
}

fn program(scripts: impl IntoIterator<Item = Script>) -> Program {
    let mut program = Program {
        scripts: scripts
            .into_iter()
            .map(|script| (script.id, script))
            .collect(),
        opcodes: OPCODES
            .iter()
            .map(|(name, number)| (*number, (*name).into()))
            .collect(),
        ..Program::default()
    };
    program
        .signatures
        .insert("cam_reset".into(), Signature::default());
    program
}

fn execute(
    program: &Program,
    entry: Entry,
    view: View,
    limits: Limits,
) -> Result<Run, rs2vm::machine::EntryFailure> {
    Machine::new(program, EngineProfile::R289, &WORLD).run(entry, view, limits)
}

fn run(program: &Program, entry: Entry, view: View) -> Run {
    execute(program, entry, view, Limits::default()).expect("fixture entry completes")
}

fn branch_to(code: &mut [Instruction], branch_pc: usize, target_pc: usize) {
    let offset = target_pc as i64 - branch_pc as i64 - 1;
    code[branch_pc].operand = Operand::Int(i32::try_from(offset).expect("branch fits i32"));
}

fn set_varp(packs: &mut Packs, id: u16, protect: bool, transmit: bool) {
    packs.varps.insert(
        VarpId(id),
        Varp {
            string: false,
            protect,
            transmit,
            name: "".into(),
        },
    );
}

fn same_landing(path: &PathResult, destination: Coord) -> bool {
    matches!(&path.end, End::Finished | End::AwaitingInput)
        && path.player == PlayerPos::Tile(destination)
        && path.effects.iter().any(|record| {
            matches!(
                &record.effect,
                Effect::Move {
                    to: PlayerPos::Tile(tile),
                    ..
                } if *tile == destination
            )
        })
}

fn large_entry_limits() -> Limits {
    Limits {
        bytes: 512 * 1024 * 1024,
        instructions: 2_000_000,
        ..Limits::default()
    }
}

#[test]
fn varbit_reads_and_writes_share_the_backing_varp() {
    let mut program = program([script(
        0,
        vec![
            int("PUSH_CONSTANT_INT", 5),
            int("POP_VARBIT", 1),
            int("PUSH_VARP", 7),
            int("PUSH_CONSTANT_INT", 53),
            int("POP_VARP", 7),
            int("PUSH_VARBIT", 1),
            int("PUSH_CONSTANT_INT", 8),
            int("POP_VARBIT", 1),
            int("PUSH_VARBIT", 1),
            int("PUSH_VARP", 7),
            command("RETURN"),
        ],
        0,
        0,
    )]);
    set_varp(&mut program.packs, 7, false, false);
    program.packs.varbits.insert(
        1,
        Varbit {
            base: VarpId(7),
            lo: 2,
            hi: 4,
            name: "".into(),
        },
    );

    let result = run(&program, Entry::new(ScriptId(0)), View::NAV);

    assert_eq!(result.paths.len(), 1);
    let [Value::Sym(updated), Value::Known(5), Value::Known(0), Value::Known(33)] =
        result.paths[0].ints.as_slice()
    else {
        panic!("partial write retains unknown outside bits; later whole writes are exact");
    };
    let Some(Expr::WithBits {
        of,
        lo: 2,
        hi: 4,
        to: Value::Known(5),
    }) = result.arena.expr(*updated)
    else {
        panic!("first read retains the masked update");
    };
    assert!(result
        .arena
        .atom_of(Value::Sym(*of))
        .is_some_and(|atom| atom.kind == AtomKind::Varp(VarpId(7))));
}

#[test]
fn varbit_write_checks_protection_on_its_backing_varp() {
    let mut program = program([script(
        0,
        vec![
            int("PUSH_CONSTANT_INT", 1),
            int("POP_VARBIT", 1),
            command("RETURN"),
        ],
        0,
        0,
    )]);
    set_varp(&mut program.packs, 7, true, false);
    program.packs.varbits.insert(
        1,
        Varbit {
            base: VarpId(7),
            lo: 2,
            hi: 4,
            name: "".into(),
        },
    );
    let mut entry = Entry::new(ScriptId(0));
    entry.protected = false;

    let result = run(&program, entry, View::NAV);

    assert_eq!(result.paths.len(), 1);
    assert!(matches!(
        &result.paths[0].end,
        End::EngineError(fault)
            if fault.at == Site { script: ScriptId(0), pc: 1 }
                && fault.reason.as_ref() == "protected varbit write without access"
    ));
}

#[test]
fn repeated_two_argument_predicate_reuses_its_relation() {
    // Equal terminal NAV results erase this fork. An unsupported opcode instead
    // exposes its prefix, which is the state this predicate test needs to inspect.
    let code = vec![
        int("PUSH_INT_LOCAL", 0),
        int("PUSH_INT_LOCAL", 1),
        int("BRANCH_GREATER_THAN_OR_EQUALS", 0),
        int("PUSH_INT_LOCAL", 0),
        int("PUSH_INT_LOCAL", 1),
        int("BRANCH_GREATER_THAN_OR_EQUALS", 0),
        command("outside_slice_command"),
    ];
    let program = program([script(0, code, 2, 2)]);

    let result = run(&program, Entry::new(ScriptId(0)), View::NAV);

    assert_eq!(result.stats.paths, 2);
    assert_eq!(result.paths.len(), 2);
    for path in &result.paths {
        assert_eq!(path.conds.len(), 1);
        let Pred::Cmp {
            lhs: Value::Sym(lhs),
            op: rs2vm::machine::CmpOp::Ge,
            rhs: Value::Sym(rhs),
        } = &path.conds[0].pred
        else {
            panic!("branch retains the full two-atom relation");
        };
        let left = result
            .arena
            .atom_of(Value::Sym(*lhs))
            .expect("left argument atom");
        let right = result
            .arena
            .atom_of(Value::Sym(*rhs))
            .expect("right argument atom");
        assert_eq!(left.kind, AtomKind::Arg(0));
        assert_eq!(right.kind, AtomKind::Arg(1));
        assert_ne!(lhs, rhs);
    }
}

#[test]
fn add_constant_does_not_refine_a_full_range_across_integer_wrap() {
    let mut code = vec![
        int("PUSH_INT_LOCAL", 0),
        int("PUSH_CONSTANT_INT", 1),
        command("add"),
        int("PUSH_CONSTANT_INT", 0),
        int("BRANCH_GREATER_THAN", 0),
        command("outside_slice_command"),
        int("PUSH_INT_LOCAL", 0),
        int("PUSH_CONSTANT_INT", -1),
        int("BRANCH_EQUALS", 0),
        command("outside_slice_command"),
    ];
    branch_to(&mut code, 4, 6);
    let program = program([script(0, code, 1, 1)]);

    let result = run(&program, Entry::new(ScriptId(0)), View::NAV);

    assert_eq!(result.stats.paths, 3);
    assert!(result.paths.iter().any(|path| {
        path.conds.len() == 2
            && path.conds[0].truth
            && path.conds[1].truth
            && matches!(
                &path.conds[0].pred,
                Pred::Cmp {
                    lhs: Value::Sym(expr),
                    op: rs2vm::machine::CmpOp::Gt,
                    rhs: Value::Known(0),
                } if matches!(result.arena.expr(*expr), Some(Expr::AddConst { c: 1, .. }))
            )
            && matches!(
                &path.conds[1].pred,
                Pred::Cmp {
                    lhs: Value::Sym(value),
                    op: rs2vm::machine::CmpOp::Eq,
                    rhs: Value::Known(-1),
                } if result.arena.atom_of(Value::Sym(*value)).is_some_and(|atom| atom.kind == AtomKind::Arg(0))
            )
    }));
}

#[test]
fn add_constant_refines_when_the_input_range_proves_no_wrap() {
    let mut code = vec![
        int("PUSH_INT_LOCAL", 0),
        int("PUSH_CONSTANT_INT", 0),
        int("BRANCH_LESS_THAN", 0),
        int("PUSH_INT_LOCAL", 0),
        int("PUSH_CONSTANT_INT", 99),
        int("BRANCH_GREATER_THAN", 0),
        int("PUSH_INT_LOCAL", 0),
        int("PUSH_CONSTANT_INT", 1),
        command("add"),
        int("POP_INT_LOCAL", 1),
        int("PUSH_INT_LOCAL", 1),
        int("PUSH_CONSTANT_INT", 0),
        int("BRANCH_GREATER_THAN", 0),
        command("outside_slice_command"),
        int("PUSH_INT_LOCAL", 1),
        command("outside_slice_command"),
        command("outside_slice_command"),
    ];
    branch_to(&mut code, 2, 16);
    branch_to(&mut code, 5, 16);
    branch_to(&mut code, 12, 14);
    let program = program([script(0, code, 1, 2)]);

    let result = run(&program, Entry::new(ScriptId(0)), View::NAV);

    assert_eq!(result.stats.paths, 3);
    let incremented = result
        .paths
        .iter()
        .find(|path| !path.ints.is_empty())
        .expect("the bounded success path returns x + 1");
    assert!(matches!(
        incremented.ints.as_slice(),
        [Value::Sym(expr)]
            if matches!(result.arena.expr(*expr), Some(Expr::AddConst { c: 1, .. }))
    ));
    assert_eq!(
        result
            .paths
            .iter()
            .filter(|path| path.ints.is_empty())
            .count(),
        2
    );
}

#[test]
fn bit_projections_keep_independent_domains_from_each_other_and_the_whole_varp() {
    let program = {
        let mut program = program([script(
            0,
            vec![
                int("PUSH_VARP", 0),
                int("PUSH_CONSTANT_INT", 0),
                int("PUSH_CONSTANT_INT", 3),
                command("getbit_range"),
                int("PUSH_CONSTANT_INT", 5),
                int("BRANCH_EQUALS", 0),
                int("PUSH_VARP", 0),
                int("PUSH_CONSTANT_INT", 4),
                int("PUSH_CONSTANT_INT", 7),
                command("getbit_range"),
                int("PUSH_CONSTANT_INT", 6),
                int("BRANCH_EQUALS", 0),
                int("PUSH_VARP", 0),
                int("PUSH_CONSTANT_INT", 101),
                int("BRANCH_EQUALS", 0),
                command("outside_slice_command"),
            ],
            0,
            0,
        )]);
        set_varp(&mut program.packs, 0, false, true);
        program
    };

    let result = run(&program, Entry::new(ScriptId(0)), View::NAV);

    assert_eq!(result.stats.paths, 8);
    assert_eq!(result.paths.len(), 8);
    for path in &result.paths {
        assert_eq!(path.conds.len(), 3);
        assert!(matches!(
            &path.conds[0].pred,
            Pred::Cmp {
                lhs: Value::Sym(expr),
                ..
            } if matches!(result.arena.expr(*expr), Some(Expr::Bits { lo: 0, hi: 3, .. }))
        ));
        assert!(matches!(
            &path.conds[1].pred,
            Pred::Cmp {
                lhs: Value::Sym(expr),
                ..
            } if matches!(result.arena.expr(*expr), Some(Expr::Bits { lo: 4, hi: 7, .. }))
        ));
        assert!(matches!(
            &path.conds[2].pred,
            Pred::Cmp {
                lhs: Value::Sym(value),
                op: rs2vm::machine::CmpOp::Eq,
                rhs: Value::Known(101),
            } if result.arena.atom_of(Value::Sym(*value)).is_some_and(|atom| atom.kind == AtomKind::Varp(VarpId(0)))
        ));
    }
}

#[test]
fn domain_normal_form_decides_after_excluding_the_first_three_values() {
    let mut code = vec![
        int("PUSH_INT_LOCAL", 0),
        int("PUSH_CONSTANT_INT", 1),
        int("BRANCH_LESS_THAN", 0),
        int("PUSH_INT_LOCAL", 0),
        int("PUSH_CONSTANT_INT", 4),
        int("BRANCH_GREATER_THAN", 0),
    ];
    let mut equality_branches = Vec::new();
    for value in 1..=3 {
        code.push(int("PUSH_INT_LOCAL", 0));
        code.push(int("PUSH_CONSTANT_INT", value));
        equality_branches.push(code.len());
        code.push(int("BRANCH_EQUALS", 0));
    }
    code.extend([
        int("PUSH_INT_LOCAL", 0),
        int("PUSH_CONSTANT_INT", 4),
        int("BRANCH_EQUALS", 0),
        command("outside_slice_command"),
    ]);
    let final_equality = code.len() - 2;
    let matched = code.len();
    code.extend([
        int("PUSH_CONSTANT_INT", 4),
        command("outside_slice_command"),
    ]);
    let out_of_range = code.len();
    code.push(command("outside_slice_command"));
    branch_to(&mut code, 2, out_of_range);
    branch_to(&mut code, 5, out_of_range);
    for pc in equality_branches {
        branch_to(&mut code, pc, out_of_range);
    }
    branch_to(&mut code, final_equality, matched);
    let program = program([script(0, code, 1, 1)]);

    let result = run(&program, Entry::new(ScriptId(0)), View::NAV);

    assert_eq!(result.stats.paths, 6);
    assert_eq!(
        result
            .paths
            .iter()
            .filter(|path| path.ints == [Value::Known(4)])
            .count(),
        1
    );
}

#[test]
fn switch_over_a_bounded_domain_has_only_its_four_case_successors() {
    let mut code = vec![
        int("PUSH_INT_LOCAL", 0),
        int("PUSH_CONSTANT_INT", 1),
        int("BRANCH_LESS_THAN", 0),
        int("PUSH_INT_LOCAL", 0),
        int("PUSH_CONSTANT_INT", 4),
        int("BRANCH_GREATER_THAN", 0),
        int("PUSH_INT_LOCAL", 0),
        int("SWITCH", 0),
        int("PUSH_CONSTANT_INT", 0),
        command("outside_slice_command"),
    ];
    let mut case_offsets = BTreeMap::new();
    for key in 1..=4 {
        let target = code.len();
        code.push(int("PUSH_CONSTANT_INT", key));
        code.push(command("outside_slice_command"));
        case_offsets.insert(key, i32::try_from(target as i64 - 8).unwrap());
    }
    let out_of_range = code.len();
    code.extend([
        int("PUSH_CONSTANT_INT", -1),
        command("outside_slice_command"),
    ]);
    branch_to(&mut code, 2, out_of_range);
    branch_to(&mut code, 5, out_of_range);
    let mut switch_script = script(0, code, 1, 1);
    switch_script.switches.push(case_offsets);
    let program = program([switch_script]);

    let result = run(&program, Entry::new(ScriptId(0)), View::NAV);

    assert_eq!(result.stats.paths, 6);
    let case_values: Vec<_> = result
        .paths
        .iter()
        .filter_map(|path| match path.ints.as_slice() {
            [Value::Known(value @ 1..=4)] => Some(*value),
            _ => None,
        })
        .collect();
    let mut case_values = case_values;
    case_values.sort_unstable();
    assert_eq!(case_values, [1, 2, 3, 4]);
    assert!(!result
        .paths
        .iter()
        .any(|path| path.ints == [Value::Known(0)]));
}

#[test]
fn live_reads_are_fresh_and_a_branch_on_one_does_not_decide_the_next() {
    let program = program([script(
        0,
        vec![
            command("map_clock"),
            int("POP_INT_LOCAL", 0),
            command("map_clock"),
            int("POP_INT_LOCAL", 1),
            int("PUSH_INT_LOCAL", 0),
            int("PUSH_CONSTANT_INT", 0),
            int("BRANCH_EQUALS", 0),
            int("PUSH_INT_LOCAL", 1),
            int("PUSH_CONSTANT_INT", 0),
            int("BRANCH_EQUALS", 0),
            command("RETURN"),
        ],
        0,
        2,
    )]);

    let result = run(&program, Entry::new(ScriptId(0)), View::NAV);

    assert_eq!(result.stats.paths, 4);
    let live_atoms: Vec<_> = result
        .arena
        .atoms
        .iter()
        .filter(|atom| atom.kind == AtomKind::Live("map_clock"))
        .collect();
    assert_eq!(live_atoms.len(), 2);
    assert_ne!(live_atoms[0].site, live_atoms[1].site);
}

#[test]
fn nav_join_erases_cosmetic_forks_but_var_and_all_view_keep_them() {
    fn join_program(true_arm: Vec<Instruction>, with_varp: bool) -> Program {
        let mut helper = vec![
            int("PUSH_CONSTANT_INT", 0),
            command("stat"),
            int("PUSH_CONSTANT_INT", 0),
            int("BRANCH_EQUALS", 0),
            command("RETURN"),
        ];
        branch_to(&mut helper, 3, 5);
        helper.extend(true_arm);
        helper.push(command("RETURN"));
        let mut program = program([
            script(
                0,
                vec![int("GOSUB_WITH_PARAMS", 1), command("RETURN")],
                0,
                0,
            ),
            script(1, helper, 0, 0),
        ]);
        if with_varp {
            set_varp(&mut program.packs, 0, false, true);
        }
        program
    }

    let cosmetic = join_program(vec![command("cam_reset")], false);
    let nav = run(&cosmetic, Entry::new(ScriptId(0)), View::NAV);
    assert_eq!(nav.stats.paths, 2);
    assert_eq!(nav.paths.len(), 1);
    assert!(nav.paths[0].conds.is_empty());
    assert_eq!(nav.paths[0].ops.max - nav.paths[0].ops.min, 1);

    let all = run(&cosmetic, Entry::new(ScriptId(0)), View::ALL);
    assert_eq!(all.stats.paths, 2);
    assert_eq!(all.paths.len(), 2);
    assert!(all.paths.iter().all(|path| path.conds.len() == 1));
    assert_eq!(
        all.paths
            .iter()
            .filter(|path| path.effects.iter().any(|effect| {
                matches!(
                    &effect.effect,
                    Effect::Cosmetic {
                        command: "cam_reset"
                    }
                )
            }))
            .count(),
        1
    );

    let var_write = join_program(vec![int("PUSH_CONSTANT_INT", 1), int("POP_VARP", 0)], true);
    let nav_with_var = run(&var_write, Entry::new(ScriptId(0)), View::NAV);
    assert_eq!(nav_with_var.stats.paths, 2);
    assert_eq!(nav_with_var.paths.len(), 2);
    assert!(nav_with_var.paths.iter().all(|path| path.conds.len() == 1));
    assert_eq!(
        nav_with_var
            .paths
            .iter()
            .filter(|path| path
                .effects
                .iter()
                .any(|effect| { matches!(&effect.effect, Effect::VarWrite { .. }) }))
            .count(),
        1
    );
}

#[test]
fn switch_erasure_requires_all_three_successors_to_match() {
    let mut script = script(
        0,
        vec![
            int("PUSH_INT_LOCAL", 0),
            int("SWITCH", 0),
            int("PUSH_CONSTANT_INT", 1),
            int("POP_VARP", 0),
            command("RETURN"),
            command("RETURN"),
            command("RETURN"),
        ],
        1,
        1,
    );
    script.switches.push(BTreeMap::from([(1, 3), (2, 4)]));
    let mut program = program([script]);
    set_varp(&mut program.packs, 0, false, true);

    let result = run(&program, Entry::new(ScriptId(0)), View::NAV);

    assert_eq!(result.stats.paths, 3);
    assert_eq!(result.paths.len(), 3);
    assert_eq!(
        result
            .paths
            .iter()
            .filter(|path| path.effects.is_empty())
            .count(),
        2
    );
    assert_eq!(
        result
            .paths
            .iter()
            .filter(|path| path
                .effects
                .iter()
                .any(|effect| { matches!(&effect.effect, Effect::VarWrite { .. }) }))
            .count(),
        1
    );
}

#[test]
fn an_ended_live_descendant_blocks_only_the_requirement_region() {
    let mut helper = vec![
        int("PUSH_CONSTANT_INT", 0),
        command("stat"),
        int("PUSH_CONSTANT_INT", 5),
        int("BRANCH_GREATER_THAN_OR_EQUALS", 0),
        command("RETURN"),
        command("map_clock"),
        int("PUSH_CONSTANT_INT", 0),
        int("BRANCH_EQUALS", 0),
        command("RETURN"),
        command("outside_slice_command"),
    ];
    branch_to(&mut helper, 3, 5);
    branch_to(&mut helper, 7, 9);
    let program = program([
        script(
            0,
            vec![
                int("GOSUB_WITH_PARAMS", 1),
                int("PUSH_CONSTANT_INT", Coord::new(3200, 0, 3200).0),
                command("p_teleport"),
                command("RETURN"),
            ],
            0,
            0,
        ),
        script(1, helper, 0, 0),
    ]);
    let destination = Coord::new(3200, 0, 3200);

    let result = run(&program, Entry::new(ScriptId(0)), View::NAV);

    let unsupported = result
        .paths
        .iter()
        .find(|path| matches!(&path.end, End::Unsupported(_)))
        .expect("the live descendant reaches an unsupported command");
    assert!(matches!(
        &unsupported.end,
        End::Unsupported(fault) if fault.at == Site { script: ScriptId(1), pc: 9 }
    ));
    let candidates: Vec<_> = result
        .paths
        .iter()
        .enumerate()
        .filter(|(_, path)| same_landing(path, destination))
        .collect();
    assert_eq!(candidates.len(), 2);

    let p_candidate = candidates
        .iter()
        .find(|(_, path)| path.conds.iter().any(|cond| cond.truth))
        .expect("one landing candidate is under the requirement");
    let not_p_candidate = candidates
        .iter()
        .find(|(_, path)| path.conds.iter().any(|cond| !cond.truth))
        .expect("one landing candidate is outside the requirement");
    let same = |path: &PathResult| same_landing(path, destination);
    assert!(!result.covered(p_candidate.0, same));
    assert!(result.covered(not_p_candidate.0, same));
    assert!(result
        .generalise(p_candidate.0, same, |_| 0)
        .iter()
        .any(|cond| cond.class == CondClass::Requirement));
    assert!(result
        .generalise(not_p_candidate.0, same, |_| 0)
        .iter()
        .any(|cond| cond.class == CondClass::Requirement));
}

#[test]
fn helper_return_fork_joins_after_the_caller_discards_different_values() {
    let mut helper = vec![
        int("PUSH_CONSTANT_INT", 0),
        command("stat"),
        int("PUSH_CONSTANT_INT", 0),
        int("BRANCH_EQUALS", 0),
        int("PUSH_CONSTANT_INT", 2),
        command("RETURN"),
        int("PUSH_CONSTANT_INT", 1),
        int("BRANCH", 0),
        command("RETURN"),
    ];
    branch_to(&mut helper, 3, 6);
    let program = program([
        script(
            0,
            vec![
                int("GOSUB_WITH_PARAMS", 1),
                command("POP_INT_DISCARD"),
                command("RETURN"),
            ],
            0,
            0,
        ),
        script(1, helper, 0, 0),
    ]);

    let result = run(&program, Entry::new(ScriptId(0)), View::NAV);

    assert_eq!(result.stats.paths, 2);
    assert_eq!(result.paths.len(), 1);
    assert_eq!(result.paths[0].end, End::Finished);
    assert!(result.paths[0].conds.is_empty());
    assert!(result.paths[0].ints.is_empty());
    assert_eq!(result.paths[0].ops.max - result.paths[0].ops.min, 1);
}

fn loop20() -> Vec<Instruction> {
    let mut code = Vec::new();
    let start = code.len();
    code.extend([
        int("PUSH_INT_LOCAL", 0),
        int("PUSH_CONSTANT_INT", 1),
        command("sub"),
        int("POP_INT_LOCAL", 0),
    ]);
    for _ in 0..6 {
        code.push(int("PUSH_CONSTANT_INT", 0));
        code.push(command("POP_INT_DISCARD"));
    }
    code.push(int("BRANCH", 0));
    code.extend([
        int("PUSH_INT_LOCAL", 0),
        int("PUSH_CONSTANT_INT", 0),
        int("BRANCH_GREATER_THAN", 0),
    ]);
    let branch_pc = code.len() - 1;
    branch_to(&mut code, branch_pc, start);
    assert_eq!(code.len(), 20);
    code
}

fn count_entry_iterations(cycles: i32) -> Entry {
    let mut entry = Entry::new(ScriptId(0));
    entry.ints.push(Value::Known(cycles));
    entry
}

fn joined_count_program() -> Program {
    let mut helper = vec![
        int("PUSH_CONSTANT_INT", 0),
        command("stat"),
        command("cam_reset"),
        int("PUSH_CONSTANT_INT", 0),
        int("BRANCH_EQUALS", 0),
        command("RETURN"),
    ];
    branch_to(&mut helper, 4, 6);
    helper.extend((0..10).map(|_| command("cam_reset")));
    helper.push(command("RETURN"));

    let mut caller = vec![int("GOSUB_WITH_PARAMS", 1)];
    for _ in 0..4 {
        caller.push(int("PUSH_CONSTANT_INT", 0));
        caller.push(command("POP_INT_DISCARD"));
    }
    caller.extend(loop20());
    caller.push(command("RETURN"));
    program([script(0, caller, 1, 1), script(1, helper, 0, 0)])
}

#[test]
fn joined_instruction_counts_continue_as_an_interval_at_the_engine_boundary() {
    let program = joined_count_program();
    let limits = large_entry_limits();

    let boundary = execute(&program, count_entry_iterations(24_999), View::NAV, limits)
        .expect("entry limits permit observing the engine boundary");
    assert_eq!(boundary.paths.len(), 1);
    assert_eq!(boundary.stats.paths, 2);
    assert_eq!(
        boundary.paths[0].ops,
        OpCount {
            min: 499_991,
            max: 500_001
        }
    );
    assert!(matches!(
        &boundary.paths[0].end,
        End::Unsupported(fault)
            if fault.reason.as_ref() == "InstructionLimitUndecided"
                && fault.at == Site { script: ScriptId(0), pc: 25 }
    ));
    assert!(boundary.paths[0].conds.is_empty());

    let shorter = execute(&program, count_entry_iterations(24_998), View::NAV, limits)
        .expect("entry limits permit the shorter joined path");
    assert_eq!(shorter.paths.len(), 1);
    assert_eq!(shorter.paths[0].end, End::Finished);
    assert_eq!(
        shorter.paths[0].ops,
        OpCount {
            min: 499_976,
            max: 499_986
        }
    );

    let longer = execute(&program, count_entry_iterations(25_000), View::NAV, limits)
        .expect("entry limits permit the longer joined path");
    assert_eq!(longer.paths.len(), 1);
    assert_eq!(
        longer.paths[0].ops,
        OpCount {
            min: 499_991,
            max: 500_001
        }
    );
    assert!(matches!(
        &longer.paths[0].end,
        End::Unsupported(fault)
            if fault.reason.as_ref() == "InstructionLimitUndecided"
                && fault.at == Site { script: ScriptId(0), pc: 25 }
    ));
    // More requested iterations cannot advance through the first undecided step.
    // Without NAV erasure, both exact executions reach the real engine error.
    let unjoined = execute(&program, count_entry_iterations(25_000), View::ALL, limits)
        .expect("entry limits permit the exact engine boundaries");
    assert_eq!(unjoined.paths.len(), 2);
    assert!(unjoined.paths.iter().all(|path| {
        path.ops
            == OpCount {
                min: 500_001,
                max: 500_001,
            }
            && matches!(&path.end, End::EngineError(fault)
                if fault.reason.as_ref() == "engine instruction limit")
    }));
}

#[test]
fn live_rival_does_not_cover_a_candidate_that_only_moves_on_one_outcome() {
    let mut code = vec![
        int("PUSH_CONSTANT_INT", 0),
        int("PUSH_CONSTANT_INT", 1),
        int("PUSH_CONSTANT_INT", 1),
        int("PUSH_CONSTANT_INT", 0),
        command("npc_find"),
        int("PUSH_CONSTANT_INT", 1),
        int("BRANCH_EQUALS", 0),
        command("RETURN"),
        int("PUSH_CONSTANT_INT", Coord::new(3200, 0, 3200).0),
        command("p_teleport"),
        command("RETURN"),
    ];
    branch_to(&mut code, 6, 8);
    let program = program([script(0, code, 0, 0)]);
    let destination = Coord::new(3200, 0, 3200);

    let result = run(&program, Entry::new(ScriptId(0)), View::NAV);

    let (index, candidate) = result
        .paths
        .iter()
        .enumerate()
        .find(|(_, path)| same_landing(path, destination))
        .expect("the successful NPC lookup moves");
    let same = |path: &PathResult| same_landing(path, destination);
    assert!(!result.covered(index, same));
    assert_eq!(result.rivals(&candidate.conds).len(), 2);
    assert!(result.generalise(index, same, |_| 0).is_empty());
    assert!(result
        .rivals(&candidate.conds)
        .iter()
        .any(|rival| !same_landing(rival, destination)));
}

#[test]
fn a_requirement_separates_an_unsupported_rival_from_a_safe_landing() {
    let mut code = vec![
        int("PUSH_CONSTANT_INT", 0),
        command("stat"),
        int("PUSH_CONSTANT_INT", 5),
        int("BRANCH_LESS_THAN", 0),
        int("PUSH_CONSTANT_INT", Coord::new(3200, 0, 3200).0),
        command("p_teleport"),
        command("RETURN"),
        command("outside_slice_command"),
    ];
    branch_to(&mut code, 3, 7);
    let program = program([script(0, code, 0, 0)]);
    let destination = Coord::new(3200, 0, 3200);

    let result = run(&program, Entry::new(ScriptId(0)), View::NAV);
    let unsupported = result
        .paths
        .iter()
        .find(|path| matches!(&path.end, End::Unsupported(_)))
        .expect("stat below five reaches the unsupported rival");
    assert!(matches!(
        &unsupported.end,
        End::Unsupported(fault) if fault.at == Site { script: ScriptId(0), pc: 7 }
    ));
    assert!(unsupported
        .conds
        .iter()
        .any(|cond| cond.class == CondClass::Requirement && cond.truth));

    let (index, candidate) = result
        .paths
        .iter()
        .enumerate()
        .find(|(_, path)| same_landing(path, destination))
        .expect("the stat requirement permits the teleport");
    let same = |path: &PathResult| same_landing(path, destination);
    assert!(result.covered(index, same));
    assert_eq!(result.rivals(&candidate.conds).len(), 1);
    let generalized = result.generalise(index, same, |_| 0);
    assert_eq!(generalized.len(), 1);
    assert_eq!(generalized[0].class, CondClass::Requirement);
    assert!(!generalized[0].truth);
    assert!(matches!(
        &generalized[0].pred,
        Pred::Cmp {
            op: rs2vm::machine::CmpOp::Lt,
            rhs: Value::Known(5),
            ..
        }
    ));
}

#[test]
fn same_landing_remains_covered_across_other_tick_counts() {
    let mut code = vec![
        int("PUSH_CONSTANT_INT", 0),
        int("PUSH_CONSTANT_INT", 1),
        int("PUSH_CONSTANT_INT", 1),
        int("PUSH_CONSTANT_INT", 0),
        command("npc_find"),
        int("PUSH_CONSTANT_INT", 1),
        int("BRANCH_EQUALS", 0),
        int("PUSH_CONSTANT_INT", Coord::new(3200, 0, 3200).0),
        command("p_teleport"),
        command("RETURN"),
        int("PUSH_CONSTANT_INT", 1),
        command("p_delay"),
        int("PUSH_CONSTANT_INT", Coord::new(3200, 0, 3200).0),
        command("p_teleport"),
        command("RETURN"),
    ];
    branch_to(&mut code, 6, 10);
    let program = program([script(0, code, 0, 0)]);
    let destination = Coord::new(3200, 0, 3200);

    let result = run(&program, Entry::new(ScriptId(0)), View::NAV);

    let (index, candidate) = result
        .paths
        .iter()
        .enumerate()
        .find(|(_, path)| same_landing(path, destination) && path.ticks.max == 2)
        .expect("one NPC outcome delays before moving");
    let same = |path: &PathResult| same_landing(path, destination);
    assert!(result.covered(index, same));
    let rivals = result.rivals(&candidate.conds);
    assert_eq!(rivals.len(), 2);
    assert!(rivals.iter().all(|path| same_landing(path, destination)));
    assert_eq!(
        (
            rivals.iter().map(|path| path.ticks.min).min().unwrap(),
            rivals.iter().map(|path| path.ticks.max).max().unwrap(),
        ),
        (0, 2)
    );
}

fn disjunction_program(first_arm_moves: bool) -> Program {
    let destination = Coord::new(3200, 0, 3200).0;
    let mut code = vec![
        int("PUSH_VARP", 0),
        int("PUSH_CONSTANT_INT", 0),
        command("testbit"),
        int("PUSH_CONSTANT_INT", 1),
        int("BRANCH_EQUALS", 0),
        int("PUSH_VARP", 1),
        int("PUSH_CONSTANT_INT", 0),
        int("BRANCH_GREATER_THAN", 0),
        command("RETURN"),
    ];
    let first_arm = code.len();
    if first_arm_moves {
        code.push(int("PUSH_CONSTANT_INT", destination));
        code.push(command("p_teleport"));
    } else {
        code.push(string("first operand message"));
        code.push(command("mes"));
    }
    code.push(command("RETURN"));
    let second_arm = code.len();
    code.extend([
        int("PUSH_CONSTANT_INT", destination),
        command("p_teleport"),
        command("RETURN"),
    ]);
    branch_to(&mut code, 4, first_arm);
    branch_to(&mut code, 7, second_arm);
    let mut program = program([script(0, code, 0, 0)]);
    set_varp(&mut program.packs, 0, false, false);
    set_varp(&mut program.packs, 1, false, true);
    program
}

#[test]
fn generalisation_removes_a_hidden_or_term_when_both_terms_reach_the_same_tile() {
    let destination = Coord::new(3200, 0, 3200);
    let program = disjunction_program(true);

    let result = run(&program, Entry::new(ScriptId(0)), View::NAV);

    let (index, candidate) = result
        .paths
        .iter()
        .enumerate()
        .find(|(_, path)| {
            same_landing(path, destination)
                && path
                    .conds
                    .iter()
                    .any(|cond| cond.class == CondClass::Requirement && cond.truth)
        })
        .expect("the transmitted-varp operand reaches the tile");
    let same = |path: &PathResult| same_landing(path, destination);
    assert!(result.covered(index, same));
    let generalized = result.generalise(index, same, |cond| {
        if cond.class == CondClass::Hidden {
            0
        } else {
            1
        }
    });
    assert_eq!(generalized.len(), 1);
    assert_eq!(generalized[0].class, CondClass::Requirement);
    assert!(generalized[0].truth);
    assert!(matches!(
        &generalized[0].pred,
        Pred::Cmp {
            lhs: Value::Sym(value),
            op: rs2vm::machine::CmpOp::Gt,
            rhs: Value::Known(0),
        } if result.arena.atom_of(Value::Sym(*value)).is_some_and(|atom| atom.kind == AtomKind::Varp(VarpId(1)))
    ));
    assert!(result
        .rivals(&generalized)
        .iter()
        .all(|rival| same_landing(rival, destination)));
    assert_eq!(candidate.conds.len(), 2);
}

#[test]
fn a_different_first_or_arm_keeps_the_unfielded_bit_condition() {
    let destination = Coord::new(3200, 0, 3200);
    let program = disjunction_program(false);

    let result = run(&program, Entry::new(ScriptId(0)), View::NAV);

    let (index, candidate) = result
        .paths
        .iter()
        .enumerate()
        .find(|(_, path)| {
            same_landing(path, destination)
                && path
                    .conds
                    .iter()
                    .any(|cond| cond.class == CondClass::Requirement && cond.truth)
        })
        .expect("the transmitted-varp operand reaches the tile");
    let same = |path: &PathResult| same_landing(path, destination);
    let generalized = result.generalise(index, same, |cond| {
        if cond.class == CondClass::Hidden {
            0
        } else {
            1
        }
    });
    assert_eq!(generalized.len(), 2);
    assert!(generalized.iter().any(|cond| {
        cond.class == CondClass::Hidden
            && !cond.truth
            && cond.site
                == Site {
                    script: ScriptId(0),
                    pc: 4,
                }
            && matches!(
                &cond.pred,
                Pred::Cmp {
                    op: rs2vm::machine::CmpOp::Eq,
                    rhs: Value::Known(1),
                    ..
                }
            )
    }));
    // The full account condition is covered. Dropping its unfielded hidden term
    // below admits the other OR arm, whose message is not the same landing.
    assert!(result.covered(index, same));
    assert!(generalized
        .iter()
        .any(|cond| cond.class == CondClass::Requirement));

    let requirement_only: Vec<_> = generalized
        .iter()
        .filter(|cond| cond.class != CondClass::Hidden)
        .cloned()
        .collect();
    let rivals = result.rivals(&requirement_only);
    assert!(rivals.iter().any(|path| {
        path.effects
            .iter()
            .any(|effect| matches!(&effect.effect, Effect::Message { .. }))
            && !same_landing(path, destination)
    }));
    assert_eq!(candidate.conds.len(), 2);
}

fn three_way_helper() -> Script {
    let mut helper = script(
        1,
        vec![
            int("PUSH_INT_LOCAL", 0),
            int("SWITCH", 0),
            command("RETURN"),
            command("RETURN"),
            command("RETURN"),
        ],
        1,
        1,
    );
    helper.switches.push(BTreeMap::from([(1, 1), (2, 2)]));
    helper
}

fn three_way_switch_program() -> Program {
    program([
        script(
            0,
            vec![
                int("PUSH_INT_LOCAL", 0),
                int("GOSUB_WITH_PARAMS", 1),
                command("RETURN"),
            ],
            1,
            1,
        ),
        three_way_helper(),
    ])
}

fn three_way_join_program() -> Program {
    program([
        script(
            0,
            vec![
                int("PUSH_INT_LOCAL", 0),
                int("GOSUB_WITH_PARAMS", 1),
                command("map_clock"),
                int("PUSH_CONSTANT_INT", 0),
                int("BRANCH_EQUALS", 1),
                command("RETURN"),
                command("RETURN"),
            ],
            1,
            1,
        ),
        three_way_helper(),
    ])
}

#[test]
fn path_count_survives_a_three_way_join_and_limits_the_next_fork_at_its_site() {
    let switch_program = three_way_switch_program();
    let joined = run(&switch_program, Entry::new(ScriptId(0)), View::NAV);
    assert_eq!(joined.stats.paths, 3);
    assert_eq!(joined.paths.len(), 1);
    assert!(joined.paths[0].conds.is_empty());

    let program = three_way_join_program();
    let completed = run(&program, Entry::new(ScriptId(0)), View::NAV);
    assert_eq!(completed.stats.paths, 4);

    let error = execute(
        &program,
        Entry::new(ScriptId(0)),
        View::NAV,
        Limits {
            paths: 3,
            ..Limits::default()
        },
    )
    .expect_err("the post-join fork exceeds the monotonic path count");
    assert_eq!(error.bound, "paths");
    assert_eq!(error.limit, 3);
    assert_eq!(error.count, 4);
    assert_eq!(
        error.at,
        Site {
            script: ScriptId(0),
            pc: 4
        }
    );
}

#[test]
fn entry_byte_and_instruction_limits_fail_without_returning_partial_results() {
    let mut effects = vec![command("cam_reset"); 100];
    effects.push(command("RETURN"));
    let byte_program = program([script(0, effects, 0, 0)]);
    let byte_error = execute(
        &byte_program,
        Entry::new(ScriptId(0)),
        View::NAV,
        Limits {
            bytes: 8_192,
            ..Limits::default()
        },
    )
    .expect_err("recording effects crosses the entry byte limit");
    assert_eq!(byte_error.bound, "bytes");
    assert!(byte_error.count > byte_error.limit);
    assert_eq!(byte_error.at.script, ScriptId(0));
    assert!(byte_error.at.pc > 0 && byte_error.at.pc < 100);

    let instruction_program = program([script(
        0,
        vec![
            int("BRANCH", 0),
            int("BRANCH", 0),
            int("BRANCH", 0),
            command("RETURN"),
        ],
        0,
        0,
    )]);
    let instruction_error = execute(
        &instruction_program,
        Entry::new(ScriptId(0)),
        View::NAV,
        Limits {
            instructions: 2,
            ..Limits::default()
        },
    )
    .expect_err("the third dispatch crosses the entry instruction limit");
    assert_eq!(instruction_error.bound, "instructions");
    assert_eq!(instruction_error.count, 3);
    assert_eq!(
        instruction_error.at,
        Site {
            script: ScriptId(0),
            pc: 2
        }
    );
}

#[test]
fn engine_instruction_limit_allows_500001_dispatches_and_rejects_the_next() {
    let mut exact = loop20();
    exact.push(command("RETURN"));
    let exact_program = program([script(0, exact, 1, 1)]);
    let exact_result = execute(
        &exact_program,
        count_entry_iterations(25_000),
        View::NAV,
        large_entry_limits(),
    )
    .expect("the engine permits its 500,001st dispatch");
    assert_eq!(exact_result.stats.instructions, 500_001);
    assert_eq!(exact_result.paths.len(), 1);
    assert_eq!(exact_result.paths[0].end, End::Finished);

    let mut over = loop20();
    over.push(int("BRANCH", 0));
    over.push(command("RETURN"));
    let over_program = program([script(0, over, 1, 1)]);
    let over_result = execute(
        &over_program,
        count_entry_iterations(25_000),
        View::NAV,
        large_entry_limits(),
    )
    .expect("the engine limit is a path outcome");
    assert_eq!(over_result.stats.instructions, 500_001);
    assert_eq!(over_result.paths.len(), 1);
    assert!(matches!(
        &over_result.paths[0].end,
        End::EngineError(fault)
            if fault.reason.as_ref() == "engine instruction limit"
                && fault.at == Site { script: ScriptId(0), pc: 21 }
    ));
}

#[test]
fn engine_frame_limit_rejects_the_fifty_first_nested_call() {
    let scripts: Vec<_> = (0..=51)
        .map(|id| {
            let code = if id == 51 {
                vec![command("RETURN")]
            } else {
                vec![
                    int("GOSUB_WITH_PARAMS", i32::try_from(id + 1).unwrap()),
                    command("RETURN"),
                ]
            };
            script(id, code, 0, 0)
        })
        .collect();
    let program = program(scripts);

    let result = run(&program, Entry::new(ScriptId(0)), View::NAV);

    assert_eq!(result.paths.len(), 1);
    assert!(matches!(
        &result.paths[0].end,
        End::EngineError(fault)
            if fault.reason.as_ref() == "engine frame limit"
                && fault.at == Site { script: ScriptId(50), pc: 0 }
    ));
}

fn fork_helper() -> Script {
    let mut code = vec![
        command("map_clock"),
        int("PUSH_CONSTANT_INT", 0),
        int("BRANCH_EQUALS", 0),
        command("RETURN"),
        command("RETURN"),
    ];
    branch_to(&mut code, 2, 4);
    script(1, code, 0, 0)
}

#[test]
fn fork_site_key_includes_each_distinct_call_site() {
    let mut caller = Vec::new();
    caller.extend((0..65).map(|_| int("GOSUB_WITH_PARAMS", 1)));
    caller.push(command("RETURN"));
    let program = program([script(0, caller, 0, 0), fork_helper()]);
    let result = execute(
        &program,
        Entry::new(ScriptId(0)),
        View::NAV,
        Limits {
            forks_per_site: 64,
            ..Limits::default()
        },
    )
    .expect("65 distinct helper call sites do not share one fork-site budget");

    assert_eq!(result.stats.paths, 66);
    assert_eq!(result.paths.len(), 1);
    assert_eq!(result.paths[0].end, End::Finished);
}

#[test]
fn fork_site_key_bounds_repeated_calls_from_one_loop_site() {
    let mut caller = vec![
        int("PUSH_INT_LOCAL", 0),
        int("PUSH_CONSTANT_INT", 1),
        command("sub"),
        int("POP_INT_LOCAL", 0),
        int("GOSUB_WITH_PARAMS", 1),
        int("PUSH_INT_LOCAL", 0),
        int("PUSH_CONSTANT_INT", 0),
        int("BRANCH_GREATER_THAN", 0),
        command("RETURN"),
    ];
    branch_to(&mut caller, 7, 0);
    let program = program([script(0, caller, 1, 1), fork_helper()]);
    let result = run(&program, count_entry_iterations(65), View::NAV);

    assert!(result.paths.iter().any(|path| matches!(
        &path.end,
        End::BoundReached(fault)
            if fault.reason.as_ref() == "fork-site limit"
                && fault.at == Site { script: ScriptId(1), pc: 2 }
    )));
}

#[test]
fn stack_target_calls_run_known_scripts_and_reject_symbolic_targets() {
    let known_program = program([
        script(
            0,
            vec![
                int("PUSH_CONSTANT_INT", 1),
                int("GOSUB", 0),
                command("RETURN"),
            ],
            0,
            0,
        ),
        script(
            1,
            vec![int("PUSH_CONSTANT_INT", 42), command("RETURN")],
            0,
            0,
        ),
    ]);
    let known = run(&known_program, Entry::new(ScriptId(0)), View::NAV);
    assert_eq!(known.paths.len(), 1);
    assert_eq!(known.paths[0].ints, [Value::Known(42)]);
    assert_eq!(known.paths[0].end, End::Finished);

    let unknown_program = program([script(
        0,
        vec![int("PUSH_INT_LOCAL", 0), int("GOSUB", 0), command("RETURN")],
        1,
        1,
    )]);
    let unknown = run(&unknown_program, Entry::new(ScriptId(0)), View::NAV);
    assert_eq!(unknown.paths.len(), 1);
    assert!(matches!(
        &unknown.paths[0].end,
        End::Unsupported(fault)
            if fault.reason.as_ref() == "unknown stack call target"
                && fault.at == Site { script: ScriptId(0), pc: 1 }
    ));
}

#[test]
fn secondary_push_varp_is_unsupported() {
    let program = {
        let mut program = program([script(
            0,
            vec![int("PUSH_VARP", 0x1_0000 | 7), command("RETURN")],
            0,
            0,
        )]);
        set_varp(&mut program.packs, 7, false, true);
        program
    };

    let result = run(&program, Entry::new(ScriptId(0)), View::NAV);

    assert_eq!(result.paths.len(), 1);
    assert!(matches!(
        &result.paths[0].end,
        End::Unsupported(fault)
            if fault.reason.as_ref() == "secondary var opcode"
                && fault.at == Site { script: ScriptId(0), pc: 0 }
    ));
}
