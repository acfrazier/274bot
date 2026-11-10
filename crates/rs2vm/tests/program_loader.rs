use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use rs2vm::program::{ConfigValue, LoadErrorKind, Operand, Program, Script};
use rs2vm::{LocTypeId, ScriptId, Site, VarpId};

struct Fixture {
    _tree: TestTree,
    engine: PathBuf,
    content: PathBuf,
    client_varbits: Vec<u8>,
}

impl Fixture {
    fn new() -> Self {
        let tree = TestTree::new();
        let engine = tree.path.join("engine");
        let content = tree.path.join("content");
        write_file(
            &engine,
            "src/engine/script/ScriptOpcode.ts",
            b"export const enum ScriptOpcode {\n RETURN = 0, // one-byte operand\n POP_INT_DISCARD = 1,\n POP_STRING_DISCARD = 2,\n PUSH_CONSTANT_STRING = 3,\n GOSUB = 4,\n JUMP = 5,\n PUSH_INT = 6,\n PUSH_BYTE = 101,\n}\n",
        );
        write_file(
            &engine,
            "src/engine/script/ServerTriggerType.ts",
            b"export enum ServerTriggerType {\n OPLOC1 = 66,\n IF_CLOSE = 148,\n}\n",
        );
        write_file(
            &content,
            "scripts/engine.rs2",
            b"[command,probe](string $text, coord $tile, boolean $flag)(string, int)\n[command,close]\n",
        );
        write_file(&engine, "data/pack/server/script.dat", &script_dat());
        write_file(&engine, "data/pack/server/script.idx", &script_idx());
        write_file(&engine, "data/pack/server/param.dat", &param_pack());
        write_file(&engine, "data/pack/server/varp.dat", &varp_pack());
        write_file(
            &engine,
            "data/pack/server/varbit.dat",
            &varbit_server_pack(),
        );
        write_file(&engine, "data/pack/server/varn.dat", &[0, 1, 1, b's', 0]);
        write_file(&engine, "data/pack/server/vars.dat", &[0, 1, 1, b's', 0]);
        write_file(&engine, "data/pack/server/inv.dat", &inv_pack());
        write_file(&engine, "data/pack/server/enum.dat", &enum_pack());
        write_file(&engine, "data/pack/server/loc.dat", &loc_pack());
        Self {
            _tree: tree,
            engine,
            content,
            client_varbits: vec![0, 1, 1, 0, 0, 3, 5, 0],
        }
    }

    fn load(&self) -> Result<Program, rs2vm::program::LoadError> {
        Program::load(&self.engine, &self.content, &self.client_varbits)
    }
}

#[test]
fn load_decodes_bytecode_triggers_signatures_and_all_server_packs() {
    let fixture = Fixture::new();
    let program = fixture.load().expect("synthetic program should load");

    assert_eq!(program.trigger("OPLOC1"), Some(66));
    assert_eq!(program.trigger("IF_CLOSE"), Some(148));
    assert_eq!(program.opcode(6), Some("PUSH_INT"));
    assert_eq!(program.lookup(66, Some(1234), Some(7)), Some(ScriptId(0)));

    let script = program.script(ScriptId(0)).expect("script id zero exists");
    assert_eq!(script.name.as_ref(), "entry");
    assert_eq!(script.source.as_ref(), "scripts/engine.rs2");
    assert_eq!(script.params, [b'i', b's']);
    assert_eq!(script.code.len(), 8);
    assert_eq!(script.code[0].operand, Operand::Int(0x0102_0304));
    assert_eq!(script.code[1].operand, Operand::Int(254));
    assert_eq!(script.code[2].operand, Operand::Text("text".into()));
    assert_eq!(script.code[3].operand, Operand::Int(0));
    assert_eq!(script.switches.len(), 2);
    assert_eq!(script.switches[0].get(&-4), Some(&7));
    assert!(script.switches[1].is_empty());
    assert_eq!((script.int_locals, script.string_locals), (2, 3));
    assert_eq!((script.int_args, script.string_args), (1, 1));
    assert_eq!(
        program.format_site(Site {
            script: ScriptId(0),
            pc: 1
        }),
        "scripts/engine.rs2:81 (script 0 pc 1)"
    );
    assert!(!program
        .format_site(Site {
            script: ScriptId(0),
            pc: 1
        })
        .contains("/build/"));

    assert_eq!(program.signatures["probe"].int_in, 2);
    assert_eq!(program.signatures["probe"].string_in, 1);
    assert_eq!(program.signatures["probe"].int_out, 1);
    assert_eq!(program.signatures["probe"].string_out, 1);
    assert_eq!(program.signatures["close"].int_in, 0);

    assert!(program.packs.params[&0].string);
    assert_eq!(program.packs.params[&0].string_default.as_ref(), "fallback");
    assert_eq!(program.packs.varps[&VarpId(0)].name.as_ref(), "varp");
    assert!(!program.packs.varps[&VarpId(0)].protect);
    assert!(program.packs.varps[&VarpId(0)].transmit);
    assert!(program.packs.varps[&VarpId(0)].string);
    assert_eq!(program.packs.varbits[&0].base, VarpId(0));
    assert_eq!(
        (program.packs.varbits[&0].lo, program.packs.varbits[&0].hi),
        (3, 5)
    );
    assert_eq!(program.packs.varbits[&0].name.as_ref(), "bit");
    assert!(program.packs.varns[&0]);
    assert!(program.packs.varss[&0]);
    assert_eq!(program.packs.invs[&0].as_ref(), "worn");

    let enum_config = &program.packs.enums[&0];
    assert_eq!(enum_config.input, b'i');
    assert_eq!(enum_config.output, b's');
    assert_eq!(enum_config.int_default, -7);
    assert_eq!(enum_config.string_default.as_ref(), "fallback");
    assert_eq!(
        enum_config.values.get(&42),
        Some(&ConfigValue::Text("answer".into()))
    );
    assert_eq!(enum_config.values.get(&43), Some(&ConfigValue::Int(13)));

    let loc = &program.packs.locs[&LocTypeId(0)];
    assert_eq!(loc.category, Some(42));
    assert_eq!(loc.name.as_ref(), "door");
    assert_eq!(loc.params.get(&5), Some(&ConfigValue::Int(-7)));
    assert_eq!(loc.params.get(&6), Some(&ConfigValue::Text("gate".into())));
}

#[test]
fn lookup_prefers_type_then_category_then_global() {
    let trigger = 66;
    let subject: i32 = 1234;
    let category: i32 = 7;
    let type_key = i32::from(trigger) | (2 << 8) | subject.wrapping_shl(10);
    let category_key = i32::from(trigger) | (1 << 8) | category.wrapping_shl(10);
    let mut program = Program::default();
    for (id, lookup) in [(1, type_key), (2, category_key), (3, i32::from(trigger))] {
        program.scripts.insert(
            ScriptId(id),
            Script {
                id: ScriptId(id),
                lookup,
                ..Script::default()
            },
        );
    }

    assert_eq!(
        program.lookup(trigger, Some(subject), Some(category)),
        Some(ScriptId(1))
    );
    program.scripts.remove(&ScriptId(1));
    assert_eq!(
        program.lookup(trigger, Some(subject), Some(category)),
        Some(ScriptId(2))
    );
    program.scripts.remove(&ScriptId(2));
    assert_eq!(
        program.lookup(trigger, Some(subject), Some(category)),
        Some(ScriptId(3))
    );
}

#[test]
fn client_varbit_bytes_are_required_and_ranges_are_not_defaulted() {
    let fixture = Fixture::new();
    let error = Program::load(&fixture.engine, &fixture.content, &[])
        .expect_err("missing client bytes fail");
    assert!(matches!(error.kind(), LoadErrorKind::Truncated { .. }));
    assert_eq!(error.offset(), 0);
}

#[test]
fn loc_server_codes_are_strict() {
    let fixture = Fixture::new();
    write_file(&fixture.engine, "data/pack/server/loc.dat", &[0, 1, 1, 0]);
    let error = fixture
        .load()
        .expect_err("unsupported server loc code fails");
    assert!(matches!(
        error.kind(),
        LoadErrorKind::UnsupportedCode {
            pack: "loc",
            code: 1
        }
    ));
    assert_eq!(
        error.path(),
        fixture.engine.join("data/pack/server/loc.dat")
    );
    assert_eq!(error.offset(), 2);
}

#[test]
fn opcode_enum_comments_are_ignored_and_unrecognized_text_fails() {
    let fixture = Fixture::new();
    write_file(
        &fixture.engine,
        "src/engine/script/ScriptOpcode.ts",
        b"const marker = \"/*\";\nexport const enum ScriptOpcode {\n RETURN = 0, // line comment\n /* block\n    comment */\n POP_INT_DISCARD = 1,\n POP_STRING_DISCARD = 2,\n PUSH_CONSTANT_STRING = 3,\n GOSUB = 4,\n JUMP = 5,\n PUSH_INT = 6,\n PUSH_BYTE = 101,\n}\n",
    );
    assert!(fixture.load().is_ok());

    write_file(
        &fixture.engine,
        "src/engine/script/ScriptOpcode.ts",
        b"export const enum ScriptOpcode {\n RETURN = 0,\n unexpected tokens\n}\n",
    );
    let error = fixture.load().expect_err("non-enum text fails");
    assert!(matches!(error.kind(), LoadErrorKind::InvalidFormat(_)));
}

#[test]
fn malformed_command_arguments_fail_with_source_location() {
    let fixture = Fixture::new();
    write_file(
        &fixture.content,
        "scripts/engine.rs2",
        b"[command,broken](string)\n",
    );
    let error = fixture.load().expect_err("missing argument name fails");
    assert!(matches!(error.kind(), LoadErrorKind::InvalidFormat(_)));
    assert_eq!(error.path(), fixture.content.join("scripts/engine.rs2"));
    assert!(error.offset() > 0);
}

#[test]
fn script_trailer_overrun_fails_at_its_length_field() {
    let fixture = Fixture::new();
    let mut bytes = script_dat();
    let offset = bytes.len() - 2;
    bytes[offset..].copy_from_slice(&u16::MAX.to_be_bytes());
    write_file(&fixture.engine, "data/pack/server/script.dat", &bytes);

    let error = fixture.load().expect_err("oversized trailer fails");
    assert!(matches!(error.kind(), LoadErrorKind::InvalidFormat(_)));
    assert_eq!(
        error.path(),
        fixture.engine.join("data/pack/server/script.dat")
    );
    assert_eq!(error.offset(), offset);
}

fn script_dat() -> Vec<u8> {
    let mut bytes = Vec::new();
    push_i32(&mut bytes, 1);
    push_i32(&mut bytes, 27);
    bytes.extend_from_slice(&script_bytes());
    bytes
}

fn script_idx() -> Vec<u8> {
    let mut bytes = Vec::new();
    push_i32(&mut bytes, 1);
    push_i32(&mut bytes, script_bytes().len() as i32);
    bytes
}

fn script_bytes() -> Vec<u8> {
    let mut code = Vec::new();
    push_u16(&mut code, 6);
    push_i32(&mut code, 0x0102_0304);
    push_u16(&mut code, 101);
    code.push(254);
    push_u16(&mut code, 3);
    code.extend_from_slice(b"text\0");
    for opcode in [0, 1, 2, 4, 5] {
        push_u16(&mut code, opcode);
        code.push(opcode as u8);
    }

    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"entry\0/build/content/scripts/engine.rs2\0");
    push_i32(&mut bytes, 66 | (2 << 8) | (1234 << 10));
    bytes.push(2);
    bytes.extend_from_slice(b"is");
    push_u16(&mut bytes, 2);
    push_i32(&mut bytes, 0);
    push_i32(&mut bytes, 81);
    push_i32(&mut bytes, 3);
    push_i32(&mut bytes, 82);
    bytes.extend_from_slice(&code);

    let mut trailer = Vec::new();
    push_i32(&mut trailer, 8);
    push_u16(&mut trailer, 2);
    push_u16(&mut trailer, 3);
    push_u16(&mut trailer, 1);
    push_u16(&mut trailer, 1);
    trailer.push(2);
    push_u16(&mut trailer, 1);
    push_i32(&mut trailer, -4);
    push_i32(&mut trailer, 7);
    push_u16(&mut trailer, 0);
    let trailer_length = u16::try_from(trailer.len() - 12).expect("small synthetic trailer");
    bytes.extend_from_slice(&trailer);
    push_u16(&mut bytes, trailer_length);
    bytes
}

fn param_pack() -> Vec<u8> {
    let mut bytes = vec![0, 1, 1, b's', 5];
    push_string(&mut bytes, "fallback", 10);
    bytes.push(0);
    bytes
}

fn varp_pack() -> Vec<u8> {
    let mut bytes = vec![0, 1, 2, b's', 4, 6, 250];
    push_string(&mut bytes, "varp", 10);
    bytes.push(0);
    bytes
}

fn varbit_server_pack() -> Vec<u8> {
    let mut bytes = vec![0, 1, 250];
    push_string(&mut bytes, "bit", 10);
    bytes.push(0);
    bytes
}

fn inv_pack() -> Vec<u8> {
    let mut bytes = vec![0, 1, 250];
    push_string(&mut bytes, "worn", 10);
    bytes.push(0);
    bytes
}

fn enum_pack() -> Vec<u8> {
    let mut bytes = vec![0, 1, 1, b'i', 2, b's', 3];
    push_string(&mut bytes, "fallback", 10);
    bytes.push(4);
    push_i32(&mut bytes, -7);
    bytes.push(5);
    push_u16(&mut bytes, 1);
    push_i32(&mut bytes, 42);
    push_string(&mut bytes, "answer", 10);
    bytes.push(6);
    push_u16(&mut bytes, 1);
    push_i32(&mut bytes, 43);
    push_i32(&mut bytes, 13);
    bytes.push(0);
    bytes
}

fn loc_pack() -> Vec<u8> {
    let mut bytes = vec![0, 1, 61, 0, 42, 249, 2];
    bytes.extend_from_slice(&[0, 0, 5, 0]);
    push_i32(&mut bytes, -7);
    bytes.extend_from_slice(&[0, 0, 6, 1]);
    push_string(&mut bytes, "gate", 10);
    bytes.push(250);
    push_string(&mut bytes, "door", 10);
    bytes.push(0);
    bytes
}

fn push_string(bytes: &mut Vec<u8>, value: &str, terminator: u8) {
    bytes.extend_from_slice(value.as_bytes());
    bytes.push(terminator);
}

fn push_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_be_bytes());
}

fn push_i32(bytes: &mut Vec<u8>, value: i32) {
    bytes.extend_from_slice(&value.to_be_bytes());
}

fn write_file(root: &Path, relative: &str, bytes: &[u8]) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().expect("test file has parent directory"))
        .expect("create test parent directory");
    fs::write(path, bytes).expect("write synthetic input");
}

struct TestTree {
    path: PathBuf,
}

impl TestTree {
    fn new() -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("rs2vm-program-test-{}-{id}", std::process::id()));
        fs::create_dir(&path).expect("create test directory");
        Self { path }
    }
}

impl Drop for TestTree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
