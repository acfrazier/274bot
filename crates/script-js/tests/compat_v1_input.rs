//! Compat v1 removal tests for the eight uncalled Input verbs.

mod common;

use script_js::{LoadIsolate, LoadShape};
use serde_json::json;

const INDEX_DTS: &str = include_str!("../compat-js/index.d.ts");
const ABI_CONTRACT: &str = include_str!("../compat-js/abi-contract.json");

fn input_declaration() -> &'static str {
    INDEX_DTS
        .split("export const Input: {")
        .nth(1)
        .expect("Input declaration exists")
        .split("};")
        .next()
        .expect("Input declaration closes")
}

fn assert_removed(member: &str) {
    assert!(
        !input_declaration().contains(member),
        "removed declaration remains in Input: {member}"
    );
    assert!(
        !ABI_CONTRACT.contains(&format!("Input.{member}")),
        "removed ABI contract remains: Input.{member}"
    );

    let source = r#"
import { Input } from '../../input/Input.js';
const key = '__MEMBER__';
const read = (target, name) => {
    try { return { ok: true, value: target[name] }; }
    catch (error) { return { ok: false, message: error.message }; }
};
export default class T extends LoopingBot {
    loop() {
        const plain = {};
        const removed = read(Input, key);
        const unknown = read(Input, '__compat_census_unknown');
        globalThis.__probe = {
            own: Object.hasOwn(Input, key),
            descriptor: Object.getOwnPropertyDescriptor(Input, key) === undefined,
            key: Object.keys(Input).includes(key),
            plain: plain[key] === undefined,
            removed,
            unknown,
            walk: typeof Input.walk === 'function' && Input.walk(NaN, 0) === false,
            invButton: typeof Input.invButton === 'function' && Input.invButton(NaN, 0, 0, 1) === false,
        };
    }
}
"#;
    let source = source.replace("__MEMBER__", member);
    let isolate = LoadIsolate::spawn(source, LoadShape::CompatClass, vec![])
        .expect("Input removal probe links");
    let snapshot = common::ingame_snapshot();
    common::post_snapshot_input(&isolate, &snapshot);
    isolate.on_game_tick(1);
    let probe = isolate.probe("globalThis.__probe");
    let logs = isolate.join();
    let probe =
        probe.unwrap_or_else(|error| panic!("Input probe failed: {error}; logs: {logs:#?}"));
    assert_eq!(
        probe,
        json!({
            "own": false,
            "descriptor": true,
            "key": false,
            "plain": true,
            "removed": {"ok": false, "message": format!("not impl: Input.{member}")},
            "unknown": {"ok": false, "message": "not impl: Input.__compat_census_unknown"},
            "walk": true,
            "invButton": true,
        })
    );
}

#[test]
#[expect(non_snake_case, reason = "the census names this removal test")]
fn compat_v1_input__input_useitemonloc__removed() {
    assert_removed("useItemOnLoc");
}

#[test]
#[expect(non_snake_case, reason = "the census names this removal test")]
fn compat_v1_input__input_useitemonnpc__removed() {
    assert_removed("useItemOnNpc");
}

#[test]
#[expect(non_snake_case, reason = "the census names this removal test")]
fn compat_v1_input__input_useitemonobj__removed() {
    assert_removed("useItemOnObj");
}

#[test]
#[expect(non_snake_case, reason = "the census names this removal test")]
fn compat_v1_input__input_useitemonitem__removed() {
    assert_removed("useItemOnItem");
}

#[test]
#[expect(non_snake_case, reason = "the census names this removal test")]
fn compat_v1_input__input_castonnpc__removed() {
    assert_removed("castOnNpc");
}

#[test]
#[expect(non_snake_case, reason = "the census names this removal test")]
fn compat_v1_input__input_castonloc__removed() {
    assert_removed("castOnLoc");
}

#[test]
#[expect(non_snake_case, reason = "the census names this removal test")]
fn compat_v1_input__input_castonitem__removed() {
    assert_removed("castOnItem");
}

#[test]
#[expect(non_snake_case, reason = "the census names this removal test")]
fn compat_v1_input__input_continuedialog__removed() {
    assert_removed("continueDialog");
}
