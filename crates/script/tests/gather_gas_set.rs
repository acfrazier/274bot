//! `GAS_ROCK_IDS` is an ordinary `Set` per isolate: the first touch fills it once from the selected gathering
//! catalog and every later operation, mutation included, stays on that same object. The catalog itself is never
//! mutated, and another isolate starts from its own untouched ids.
use client::io::ClientRevision;
use script::{LoadIsolate, LoadShape};
use serde_json::{json, Value};
use std::sync::Arc;

type Data = Arc<api::game_data::SelectedGameData>;

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

fn gas_ids() -> Vec<i32> {
    (2119..=2139).collect()
}

/// Whatever member a script reaches first fills the set completely, in catalog order, and the exports of the
/// relative shim module and of `@rs2b0t/api` are that one object.
#[test]
fn any_first_touch_materializes_the_full_ordered_set() {
    let firsts = [
        "GAS_ROCK_IDS.size",
        "GAS_ROCK_IDS.has(2125)",
        "[...GAS_ROCK_IDS]",
        "Array.from(GAS_ROCK_IDS)",
        "new Set(GAS_ROCK_IDS).size",
        "[...GAS_ROCK_IDS.keys()]",
        "[...GAS_ROCK_IDS.values()]",
        "[...GAS_ROCK_IDS.entries()].length",
        "(() => { let n = 0; GAS_ROCK_IDS.forEach(() => n++); return n; })()",
        "(() => { const seen = []; for (const id of GAS_ROCK_IDS) seen.push(id); return seen; })()",
        "GAS_ROCK_IDS[Symbol.iterator]().next().value",
        "GAS_ROCK_IDS.isSubsetOf(new Set([2119]))",
    ];
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    for first in firsts {
        let src = format!(
            r#"
import {{ GAS_ROCK_IDS }} from '../../data/miningRocks.js';
import {{ GAS_ROCK_IDS as apiGas }} from '@rs2b0t/api';
export default class T extends LoopingBot {{
    loop() {{
        const isSet = GAS_ROCK_IDS instanceof Set && Object.getPrototypeOf(GAS_ROCK_IDS) === Set.prototype;
        const first = ({first});
        globalThis.__probe = JSON.stringify({{
            first: first === undefined ? null : first,
            isSet,
            same: GAS_ROCK_IDS === apiGas,
            size: GAS_ROCK_IDS.size,
            order: [...GAS_ROCK_IDS],
            plain: Object.getOwnPropertyNames(GAS_ROCK_IDS),
        }});
    }}
}}
"#
        );
        let iso = spawn(&src, &data);
        let value = tick(&iso, 1);
        iso.join();
        assert_eq!(value["isSet"], true, "{first}: {value}");
        assert_eq!(value["same"], true, "{first}: {value}");
        assert_eq!(value["size"], 21, "{first}: {value}");
        assert_eq!(value["order"], json!(gas_ids()), "{first}: {value}");
        assert_eq!(
            value["plain"],
            json!([]),
            "{first}: materialized set is an ordinary Set with no leftover own members"
        );
    }
}

/// Mutations behave exactly like a hand-built `Set`, persist on later ticks and never reach the catalog or a
/// second isolate.
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

    let catalog = api::selected::FamilyPreparation::run({
        let data = data.clone();
        move |worker| data.prepare_gathering(worker)
    })
    .unwrap()
    .join()
    .unwrap()
    .expect("gathering family prepares");
    assert_eq!(
        api::gather_methods::gas_rock_ids(Some(&catalog)).unwrap(),
        gas_ids(),
        "script mutations never reach the shared catalog"
    );
}

/// Selected data without a gathering family keeps the fail-closed miss on every touch, member by member.
#[test]
fn touching_the_set_without_a_family_fails_closed_on_every_member() {
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
    let data: Data = Arc::new(serde_json::from_str(raw).expect("selected data without a pin"));
    let src = r#"
import { GAS_ROCK_IDS } from '../../data/miningRocks.js';
const miss = (read) => { try { read(); return 'no error'; } catch (e) { return e.message; } };
export default class T extends LoopingBot {
    loop() {
        globalThis.__probe = JSON.stringify({
            isSet: GAS_ROCK_IDS instanceof Set,
            has: miss(() => GAS_ROCK_IDS.has(2125)),
            size: miss(() => GAS_ROCK_IDS.size),
            again: miss(() => GAS_ROCK_IDS.has(2125)),
            iterate: miss(() => [...GAS_ROCK_IDS]),
        });
    }
}
"#;
    let iso = spawn(src, &data);
    let value = tick(&iso, 1);
    iso.join();
    assert_eq!(
        value,
        json!({
            "isSet": true,
            "has": "not impl: GAS_ROCK_IDS.has",
            "size": "not impl: GAS_ROCK_IDS.size",
            "again": "not impl: GAS_ROCK_IDS.has",
            "iterate": "not impl: GAS_ROCK_IDS",
        })
    );
}
