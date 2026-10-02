//! `GAS_ROCK_IDS` is an ordinary `Set` per isolate, built full at module evaluation from the selected core: every
//! operation, reflection and intrinsic included, sees the same 21 ids from the start; freezing, sealing, clearing
//! and redefining members behave exactly as on a hand-built `Set`, and mutations stay in that isolate.
use client::io::ClientRevision;
use script::{LoadIsolate, LoadShape};
use serde_json::{json, Value};
use std::sync::Arc;

type Data = Arc<api::game_data::SelectedGameData>;

const REVISIONS: [ClientRevision; 2] = [ClientRevision::R274, ClientRevision::R289];

fn spawn(src: &str, data: &Data) -> LoadIsolate {
    let iso =
        LoadIsolate::spawn_with_game_data(src.into(), LoadShape::CompatClass, vec![], data.clone())
            .unwrap();
    assert_eq!(iso.probe("1").unwrap(), json!(1));
    iso
}

fn tick(iso: &LoadIsolate, n: u64) -> Value {
    iso.on_game_tick(n);
    let raw = iso.probe("globalThis.__probe").unwrap();
    serde_json::from_str(raw.as_str().unwrap()).unwrap()
}

/// One tick of a script whose `loop` stores `JSON.stringify(<expr>)` as the probe.
fn first_tick(expr: &str, data: &Data) -> Value {
    let src = format!(
        r#"
import {{ GAS_ROCK_IDS }} from '../../data/miningRocks.js';
export default class T extends LoopingBot {{
    loop() {{
        globalThis.__probe = JSON.stringify({expr});
    }}
}}
"#
    );
    let iso = spawn(&src, data);
    let value = tick(&iso, 1);
    iso.join();
    value
}

fn gas_ids() -> Vec<i32> {
    (2119..=2139).collect()
}

/// Whatever a script does first, including intrinsics and reflection that bypass property lookup, it sees the full
/// ordered set, and the exports of the relative shim module and of `@rs2b0t/api` are that one object.
#[test]
fn every_first_operation_sees_the_full_ordered_set() {
    let firsts = [
        ("GAS_ROCK_IDS.size", json!(21)),
        ("GAS_ROCK_IDS.has(2125)", json!(true)),
        ("[...GAS_ROCK_IDS]", json!(gas_ids())),
        ("Array.from(GAS_ROCK_IDS)", json!(gas_ids())),
        ("new Set(GAS_ROCK_IDS).size", json!(21)),
        ("[...GAS_ROCK_IDS.keys()].length", json!(21)),
        ("[...GAS_ROCK_IDS.entries()].length", json!(21)),
        (
            "(() => { let n = 0; GAS_ROCK_IDS.forEach(() => n++); return n; })()",
            json!(21),
        ),
        ("GAS_ROCK_IDS[Symbol.iterator]().next().value", json!(2119)),
        ("GAS_ROCK_IDS.isSubsetOf(new Set([2119]))", json!(false)),
        ("Set.prototype.has.call(GAS_ROCK_IDS, 2125)", json!(true)),
        (
            "Reflect.apply(Object.getOwnPropertyDescriptor(Set.prototype, 'size').get, GAS_ROCK_IDS, [])",
            json!(21),
        ),
        ("Reflect.ownKeys(GAS_ROCK_IDS).length", json!(0)),
    ];
    for revision in REVISIONS {
        let data = api::game_data::for_revision(revision).unwrap();
        for (first, expected) in &firsts {
            let src = format!(
                r#"
import {{ GAS_ROCK_IDS }} from '../../data/miningRocks.js';
import {{ GAS_ROCK_IDS as apiGas }} from '@rs2b0t/api';
export default class T extends LoopingBot {{
    loop() {{
        const first = ({first});
        globalThis.__probe = JSON.stringify({{
            first,
            isSet: GAS_ROCK_IDS instanceof Set && Object.getPrototypeOf(GAS_ROCK_IDS) === Set.prototype,
            same: GAS_ROCK_IDS === apiGas,
            order: [...GAS_ROCK_IDS],
        }});
    }}
}}
"#
            );
            let iso = spawn(&src, &data);
            let value = tick(&iso, 1);
            iso.join();
            assert_eq!(
                value,
                json!({ "first": expected, "isSet": true, "same": true, "order": gas_ids() }),
                "{revision:?} {first}"
            );
        }
    }
}

/// Freezing or sealing the set before any other use leaves it readable like any frozen `Set`: the freeze only
/// concerns object properties, never the set's entries.
#[test]
fn freezing_or_sealing_first_leaves_the_full_set_readable() {
    for revision in REVISIONS {
        let data = api::game_data::for_revision(revision).unwrap();
        for lock in ["freeze", "seal", "preventExtensions"] {
            let value = first_tick(
                &format!(
                    "(() => {{ Object.{lock}(GAS_ROCK_IDS); return {{ size: GAS_ROCK_IDS.size, has: GAS_ROCK_IDS.has(2125), ids: [...GAS_ROCK_IDS] }}; }})()"
                ),
                &data,
            );
            assert_eq!(
                value,
                json!({ "size": 21, "has": true, "ids": gas_ids() }),
                "{revision:?} {lock}"
            );
        }
    }
}

/// Clearing the set and then defining, redefining or deleting same-named own members never brings ids back.
#[test]
fn a_cleared_set_stays_cleared_whatever_members_are_redefined() {
    let src = r#"
import { GAS_ROCK_IDS } from '../../data/miningRocks.js';
let step = 0;
export default class T extends LoopingBot {
    loop() {
        step++;
        if (step === 1) {
            GAS_ROCK_IDS.has(2125);
            GAS_ROCK_IDS.clear();
            Object.defineProperty(GAS_ROCK_IDS, 'has', { value: Set.prototype.has, configurable: true });
            const redefined = GAS_ROCK_IDS.has(2125);
            delete GAS_ROCK_IDS.has;
            Object.defineProperty(GAS_ROCK_IDS, 'size', { get: () => -1, configurable: true });
            const shadowed = GAS_ROCK_IDS.size;
            delete GAS_ROCK_IDS.size;
            globalThis.__probe = JSON.stringify({ redefined, shadowed, size: GAS_ROCK_IDS.size, has: GAS_ROCK_IDS.has(2125), ids: [...GAS_ROCK_IDS] });
        } else {
            globalThis.__probe = JSON.stringify({ size: GAS_ROCK_IDS.size, has: GAS_ROCK_IDS.has(2125), own: Reflect.ownKeys(GAS_ROCK_IDS) });
        }
    }
}
"#;
    for revision in REVISIONS {
        let data = api::game_data::for_revision(revision).unwrap();
        let iso = spawn(src, &data);
        assert_eq!(
            tick(&iso, 1),
            json!({ "redefined": false, "shadowed": -1, "size": 0, "has": false, "ids": [] }),
            "{revision:?}"
        );
        assert_eq!(
            tick(&iso, 2),
            json!({ "size": 0, "has": false, "own": [] }),
            "{revision:?}: still cleared on a later tick"
        );
        iso.join();
    }
}

/// Mutations behave exactly like a hand-built `Set`, persist on later ticks and never reach the selected data or
/// a second isolate.
#[test]
fn mutations_persist_within_the_isolate_and_stay_local_to_it() {
    let src = r#"
import { GAS_ROCK_IDS } from '../../data/miningRocks.js';
let step = 0;
export default class T extends LoopingBot {
    loop() {
        step++;
        if (step === 1) {
            const added = GAS_ROCK_IDS.add(999999);
            const deleted = GAS_ROCK_IDS.delete(2125);
            const missing = GAS_ROCK_IDS.delete(2125);
            globalThis.__probe = JSON.stringify({
                addReturnsSet: added === GAS_ROCK_IDS,
                deleted,
                missing,
                hasAdded: GAS_ROCK_IDS.has(999999),
                hasDeleted: GAS_ROCK_IDS.has(2125),
                size: GAS_ROCK_IDS.size,
            });
        } else if (step === 2) {
            const token = { thisArg: true };
            const receivers = [];
            GAS_ROCK_IDS.forEach(function (value, key, collection) {
                receivers.push(collection === GAS_ROCK_IDS && key === value && this === token);
            }, token);
            globalThis.__probe = JSON.stringify({
                size: GAS_ROCK_IDS.size,
                hasAdded: GAS_ROCK_IDS.has(999999),
                hasDeleted: GAS_ROCK_IDS.has(2125),
                appended: [...GAS_ROCK_IDS].at(-1),
                receivers: receivers.length > 0 && receivers.every(Boolean),
                count: receivers.length,
            });
        } else {
            const cleared = GAS_ROCK_IDS.clear();
            GAS_ROCK_IDS.add(7);
            globalThis.__probe = JSON.stringify({
                clearReturnsUndefined: cleared === undefined,
                after: [...GAS_ROCK_IDS],
                size: GAS_ROCK_IDS.size,
            });
        }
    }
}
"#;
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let iso = spawn(src, &data);

    let first = tick(&iso, 1);
    assert_eq!(
        first,
        json!({
            "addReturnsSet": true, "deleted": true, "missing": false,
            "hasAdded": true, "hasDeleted": false, "size": 21,
        })
    );

    // forEach hands the callback the exported set itself as its third argument.
    let second = tick(&iso, 2);
    assert_eq!(second["size"], 21, "{second}");
    assert_eq!(second["hasAdded"], true, "{second}");
    assert_eq!(second["hasDeleted"], false, "{second}");
    assert_eq!(second["appended"], 999999, "{second}");
    assert_eq!(second["count"], 21, "{second}");
    assert_eq!(
        second["receivers"], true,
        "forEach passes the exported set itself: {second}"
    );

    // A second isolate of the same selected data starts from the untouched ids.
    let other = spawn(
        r#"
import { GAS_ROCK_IDS } from '../../data/miningRocks.js';
export default class T extends LoopingBot {
    loop() {
        globalThis.__probe = JSON.stringify({ ids: [...GAS_ROCK_IDS], size: GAS_ROCK_IDS.size });
    }
}
"#,
        &data,
    );
    let fresh = tick(&other, 1);
    assert_eq!(fresh, json!({ "ids": gas_ids(), "size": 21 }));

    let third = tick(&iso, 3);
    assert_eq!(
        third,
        json!({ "clearReturnsUndefined": true, "after": [7], "size": 1 })
    );
    let untouched = tick(&other, 2);
    assert_eq!(
        untouched,
        json!({ "ids": gas_ids(), "size": 21 }),
        "another isolate's clear() is invisible here"
    );
    other.join();
    iso.join();

    assert_eq!(
        api::gather_methods::gas_rock_ids(data.as_ref(), data.mining_hazards()).unwrap(),
        gas_ids(),
        "script mutations never reach the selected data"
    );
}

/// Selected data without the generated gas slice is the fail-closed miss on every member, never an empty set.
#[test]
fn selected_data_without_the_gas_slice_fails_closed_on_every_member() {
    let raw = r#"{
        "schema_version": 4,
        "revision": 274,
        "provenance": {
            "cache_identity": {"cache_id": "x"},
            "inputs": [],
            "content_inputs": [],
            "decoder_sources": []
        },
        "items": [],
        "consumption": [],
        "pickpocket": []
    }"#;
    let data: Data = Arc::new(serde_json::from_str(raw).expect("selected data without the slice"));
    let value = first_tick(
        r#"(() => {
            const miss = (read) => { try { read(); return 'no error'; } catch (e) { return e.message; } };
            return {
                isSet: GAS_ROCK_IDS instanceof Set,
                has: miss(() => GAS_ROCK_IDS.has(2125)),
                size: miss(() => GAS_ROCK_IDS.size),
                again: miss(() => GAS_ROCK_IDS.has(2125)),
                iterate: miss(() => [...GAS_ROCK_IDS]),
            };
        })()"#,
        &data,
    );
    assert_eq!(
        value,
        json!({
            "isSet": false,
            "has": "not impl: GAS_ROCK_IDS.has",
            "size": "not impl: GAS_ROCK_IDS.size",
            "again": "not impl: GAS_ROCK_IDS.has",
            "iterate": "not impl: GAS_ROCK_IDS",
        })
    );
}
