use rs2vm::machine::{Entry, Limits, Machine, View};
use rs2vm::profile::EngineProfile;
use rs2vm::program::{Instruction, Operand, Program, Script};
use rs2vm::world::{Alloc, LocInfo, PlacedId, PlacedLoc, World, ZoneKey};
use rs2vm::{Coord, LocTypeId, ScriptId, Site};

const TILE: Coord = Coord::new(3200, 0, 3200);

#[derive(Default)]
struct ZoneWorld(Vec<PlacedLoc>);

impl ZoneWorld {
    fn populated() -> Self {
        Self(
            (0..1024)
                .map(|id| PlacedLoc {
                    id: PlacedId(id),
                    tile: TILE,
                    info: LocInfo {
                        ty: LocTypeId(1),
                        shape: 0,
                        angle: 0,
                    },
                })
                .collect(),
        )
    }
}

impl World for ZoneWorld {
    fn zone_locs(&self, zone: ZoneKey) -> &[PlacedLoc] {
        if zone == ZoneKey::of(TILE) {
            &self.0
        } else {
            &[]
        }
    }
    fn placed_loc(&self, id: PlacedId) -> Option<PlacedLoc> {
        self.0.get(id.0 as usize).copied()
    }
    fn zone_alloc(&self, _: Coord) -> Alloc {
        Alloc::default()
    }
}

fn int(opcode: u16, value: i32) -> Instruction {
    Instruction {
        opcode,
        operand: Operand::Int(value),
    }
}

fn program(code: Vec<Instruction>) -> Program {
    let mut program = Program::default();
    for (number, name) in [
        (1, "PUSH_CONSTANT_INT"),
        (2, "loc_findallzone"),
        (3, "loc_add"),
        (4, "RETURN"),
        (5, "PUSH_CONSTANT_STRING"),
    ] {
        program.opcodes.insert(number, name.into());
    }
    program.scripts.insert(
        ScriptId(0),
        Script {
            code,
            ..Script::default()
        },
    );
    program
}

fn assert_zone_bound(program: &Program, pc: u32) {
    let empty = ZoneWorld::default();
    let full = ZoneWorld::populated();
    let baseline = Machine::new(program, EngineProfile::R289, &empty)
        .run(Entry::new(ScriptId(0)), View::NAV, Limits::default())
        .expect("empty zone stays inside the default bound");
    let limit = baseline.stats.bytes + 1024;
    let error = Machine::new(program, EngineProfile::R289, &full)
        .run(
            Entry::new(ScriptId(0)),
            View::NAV,
            Limits {
                bytes: limit,
                ..Limits::default()
            },
        )
        .expect_err("the copied world list must use the entry budget");
    assert_eq!(error.bound, "bytes");
    assert_eq!(
        error.at,
        Site {
            script: ScriptId(0),
            pc
        }
    );
    assert_eq!(error.limit, limit);
    assert!(error.count > error.limit);
}

#[test]
fn loc_iterator_list_is_bounded_before_the_copy() {
    assert_zone_bound(&program(vec![int(1, TILE.0), int(2, 0), int(4, 0)]), 1);
}

#[test]
fn loc_mutation_list_is_bounded_before_the_copy() {
    assert_zone_bound(
        &program(vec![
            int(1, TILE.0),
            int(1, 1),
            int(1, 0),
            int(1, 0),
            int(1, 3),
            int(3, 0),
            int(4, 0),
        ]),
        5,
    );
}

#[test]
fn literal_payload_is_bounded_at_its_instruction() {
    let program = program(vec![
        Instruction {
            opcode: 5,
            operand: Operand::Text("x".repeat(8192).into_boxed_str()),
        },
        int(4, 0),
    ]);
    let world = ZoneWorld::default();
    let error = Machine::new(&program, EngineProfile::R289, &world)
        .run(
            Entry::new(ScriptId(0)),
            View::NAV,
            Limits {
                bytes: 1024,
                ..Limits::default()
            },
        )
        .expect_err("a literal payload cannot bypass the entry budget");
    assert_eq!(error.bound, "bytes");
    assert_eq!(
        error.at,
        Site {
            script: ScriptId(0),
            pc: 0
        }
    );
    assert!(error.count > error.limit);
}

#[test]
fn an_entry_argument_index_cannot_alias_an_earlier_atom() {
    let mut program = program(vec![int(4, 0)]);
    let script = program.scripts.get_mut(&ScriptId(0)).expect("entry exists");
    script.int_args = 257;
    script.int_locals = 257;
    let world = ZoneWorld::default();
    let run = Machine::new(&program, EngineProfile::R289, &world)
        .run(Entry::new(ScriptId(0)), View::NAV, Limits::default())
        .expect("an invalid header is a located unsupported path");
    assert_eq!(run.paths.len(), 1);
    assert!(
        matches!(&run.paths[0].end, rs2vm::machine::End::Unsupported(f)
        if f.at.script == ScriptId(0) && f.at.pc == 0
        && f.reason.as_ref() == "entry argument index exceeds the bytecode header")
    );
    assert_eq!(run.transfers.unsupported_prefixes, 1);
    assert!(run.paths[0].effects.is_empty());
}
