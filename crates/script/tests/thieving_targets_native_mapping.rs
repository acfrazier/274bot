//! `chooseTarget`: one native call (`__rs2b0t_choose_target`) walks the
//! caller's own iterable and calls its `reachable` callback.
//!
//! Parity here means the export produces what the frozen
//! `bot/api/thieving/targets.ts` `for...of` body produces: the test drives an
//! inline copy of that body (the reference) and the native export over the same
//! inputs and compares target/blocked identity plus the callback sequence.

use script::isolate_fb::{
    encode_snapshot, encode_snapshot_delta, ItemRowInput, SceneEntityInput, SnapshotFingerprint,
    StatInput,
};
use script::shim::InteractReq;
use script::{LoadIsolate, LoadShape};

mod common;

const SRC: &str = r#"
import { chooseTarget } from '../../api/thieving/targets.js';

const E = { a: { n: 'a' }, b: { n: 'b' }, c: { n: 'c' }, d: { n: 'd' } };
const NAMES = new Map([[E.a, 'a'], [E.b, 'b'], [E.c, 'c'], [E.d, 'd']]);

function label(v) {
    if (v === undefined) return 'undefined';
    if (v === null) return 'null';
    if (typeof v === 'object') return NAMES.get(v) || 'object';
    return String(v);
}

function outcome(result) {
    return { target: label(result.target), blocked: label(result.blocked) };
}

// The previous body, kept verbatim as the parity oracle. The parameter keeps
// the moved function's name, so an engine error that quotes the iterable reads
// identically on both sides.
function reference(candidatesNearestFirst, reachable) {
    for (const c of candidatesNearestFirst) {
        if (reachable(c)) return { target: c, blocked: null };
    }
    return { target: null, blocked: candidatesNearestFirst[0] ?? null };
}

function parity(make, probe) {
    const nativeCalls = [];
    const refCalls = [];
    const nativeArr = make();
    const refArr = make();
    const native = chooseTarget(nativeArr, (c) => { nativeCalls.push(label(c)); return probe(c); });
    const ref = reference(refArr, (c) => { refCalls.push(label(c)); return probe(c); });
    return {
        targetSame: native.target === ref.target,
        blockedSame: native.blocked === ref.blocked,
        callsSame: JSON.stringify(nativeCalls) === JSON.stringify(refCalls),
    };
}

globalThis.__parity = [
    { name: 'empty', make: () => [], probe: () => true },
    { name: 'first-hit', make: () => [E.a, E.b, E.c], probe: (c) => c === E.b },
    { name: 'hit-at-last', make: () => [E.a, E.b, E.c], probe: (c) => c === E.c },
    { name: 'no-hit', make: () => [E.a, E.b, E.c], probe: () => false },
    { name: 'hole-first', make: () => [E.a, , E.c], probe: (c) => c === undefined },
    { name: 'shared-first', make: () => [E.a, E.b], probe: (c) => c === E.d },
].map(({ name, make, probe }) => ({ name, ...parity(make, probe) }));

// First hit: the hit element itself is returned and the scan stops there.
const firstCalls = [];
const first = chooseTarget([E.a, E.b, E.c, E.d], (c) => {
    firstCalls.push(label(c));
    return c === E.c;
});
globalThis.__firstHit = {
    ...outcome(first),
    calls: firstCalls,
    identity: first.target === E.c,
    blockedIsNull: first.blocked === null,
};

// No hit: every candidate is visited and `candidates[0]` is the blocked result.
const noHitCalls = [];
const noHit = chooseTarget([E.a, E.b], (c) => {
    noHitCalls.push(label(c));
    return false;
});
globalThis.__noHit = {
    ...outcome(noHit),
    calls: noHitCalls,
    firstIdentity: noHit.blocked === E.a,
    targetIsNull: noHit.target === null,
};

const empty = chooseTarget([], () => { throw new Error('never probed'); });
globalThis.__empty = {
    ...outcome(empty),
    targetIsNull: empty.target === null,
    blockedIsNull: empty.blocked === null,
};

// Holes read as `undefined`; a nullish first slot is the fallback, a falsy
// non-nullish one is not.
const holeCalls = [];
const holes = [, E.a];
const holeResult = chooseTarget(holes, (c) => {
    holeCalls.push(typeof c);
    return false;
});
globalThis.__holes = {
    ...outcome(holeResult),
    calls: holeCalls,
    blockedIsNull: holeResult.blocked === null,
};
const zero = chooseTarget([0], () => false);
globalThis.__zero = { blockedIsZero: zero.blocked === 0, targetIsNull: zero.target === null };

// Truthiness, never `=== true`.
globalThis.__truthy = [1, 'x', {}, [], -1, Infinity].map(
    (v) => chooseTarget([E.a], () => v).target === E.a,
);
globalThis.__falsy = [0, '', null, undefined, NaN, false].map(
    (v) => chooseTarget([E.a], () => v).target === null,
);

// The live iterator contract: a pushed candidate is visited, a replaced one is
// re-read, and the fallback reads index 0 after exhaustion.
const grown = [E.a];
const grownResult = chooseTarget(grown, (c) => {
    if (grown.length < 2) {
        grown.push(E.b);
        return false;
    }
    return c === E.b;
});
globalThis.__liveLength = { ...outcome(grownResult), identity: grownResult.target === E.b };

const swapped = [E.a, E.b];
const swappedResult = chooseTarget(swapped, (c) => {
    if (c === E.a) {
        swapped[1] = E.c;
        return false;
    }
    return c === E.c;
});
globalThis.__liveElement = { ...outcome(swappedResult), identity: swappedResult.target === E.c };

const edited = [E.a, E.b];
const editedResult = chooseTarget(edited, () => {
    edited[0] = 'replaced';
    return false;
});
globalThis.__mutatedFallback = {
    blocked: String(editedResult.blocked),
    isReplaced: editedResult.blocked === 'replaced',
};

// A throwing callback propagates as-is and stops the scan.
const boomCalls = [];
let boom = null;
try {
    chooseTarget([E.a, E.b], (c) => {
        boomCalls.push(label(c));
        throw new Error('cb boom');
    });
} catch (e) {
    boom = String(e && e.message ? e.message : e);
}
globalThis.__throwing = { error: boom, calls: boomCalls };

// Receiver, argument count and order of the callback.
const receivers = [];
const argCount = [];
function cb(c) {
    receivers.push(this === undefined);
    argCount.push(arguments.length);
    return false;
}
chooseTarget([E.a], cb);
globalThis.__callback = { receivers, argCount };

// Reentrancy: a nested decision does not disturb the one in flight.
let nested = null;
const outer = chooseTarget([E.a, E.b], (c) => {
    if (c === E.a) {
        nested = chooseTarget([E.c], () => true);
        return false;
    }
    return true;
});
globalThis.__reentrant = {
    ...outcome(outer),
    identity: outer.target === E.b,
    nested: outcome(nested),
    nestedIdentity: nested.target === E.c,
};

// The caller's iterator is what is traversed, not an array snapshot.
const fromSet = chooseTarget(new Set([E.a, E.b]), (c) => c === E.b);
globalThis.__set = { ...outcome(fromSet), identity: fromSet.target === E.b };

// `for...of` closes an iterator it leaves early, on a hit and on a throw.
let closed = 0;
function* generate(values) {
    try {
        for (const v of values) yield v;
    } finally {
        closed += 1;
    }
}
const closedHit = chooseTarget(generate([E.a, E.b]), (c) => c === E.a);
const afterHit = closed;
try {
    chooseTarget(generate([E.a, E.b]), () => { throw new Error('close boom'); });
} catch (_) {
    // the callback error is not this case's subject
}
globalThis.__close = {
    afterHit,
    afterThrow: closed,
    identity: closedHit.target === E.a,
};

// The engine's own iterator protocol is what the moved scan has to keep: the
// caller's `next` is read once when its iterator is acquired and called once
// per step, and a primitive `next()` result, a missing or non-callable
// `Symbol.iterator`, a non-callable `return` and a throwing close are the
// engine's errors, not shim ones.
function tracedIterator(values, trace) {
    let at = 0;
    const iterator = {
        get next() {
            trace.push('read:' + at);
            return function () {
                trace.push('call:' + at);
                if (at >= values.length) return { done: true, value: undefined };
                const value = values[at];
                at += 1;
                return { done: false, value };
            };
        },
        [Symbol.iterator]() { return iterator; },
    };
    return iterator;
}

function swappedNextIterator(values, trace) {
    let reads = 0;
    const iterator = {
        get next() {
            reads += 1;
            trace.push('read:' + reads);
            if (reads > 1) {
                return function () {
                    trace.push('call:swapped');
                    return { done: true, value: undefined };
                };
            }
            let at = 0;
            return function () {
                trace.push('call:' + at);
                if (at >= values.length) return { done: true, value: undefined };
                const value = values[at];
                at += 1;
                return { done: false, value };
            };
        },
        [Symbol.iterator]() { return iterator; },
    };
    return iterator;
}

// Runs the same iterable shape through the moved export and the oracle, with
// each side building its own iterator, and reports both runs including the
// callback sequence and any thrown error name/message.
function protocol(make, probe) {
    const runs = {};
    for (const [name, impl] of [['native', chooseTarget], ['reference', reference]]) {
        const trace = [];
        const calls = [];
        try {
            const result = impl(make(trace), (c) => {
                calls.push(label(c));
                return probe(c);
            });
            runs[name] = { ...outcome(result), trace, calls };
        } catch (e) {
            runs[name] = {
                error: e && e.constructor ? e.constructor.name : typeof e,
                message: String(e && e.message ? e.message : e),
                trace,
                calls,
            };
        }
    }
    return runs;
}

globalThis.__protocol = {
    getterNext: protocol((trace) => tracedIterator([E.a, E.b, E.c], trace), (c) => c === E.b),
    swappedNext: protocol((trace) => swappedNextIterator([E.a, E.b, E.c], trace), (c) => c === E.b),
    primitiveNext: protocol(
        () => ({ next: () => 7, [Symbol.iterator]() { return this; } }),
        () => true,
    ),
    missingIterator: protocol(() => ({ 0: E.a, length: 1 }), () => true),
    primitiveIterator: protocol(() => ({ [Symbol.iterator]: () => 7 }), () => true),
    nonCallableReturn: protocol(
        () => ({
            next: () => ({ done: false, value: E.a }),
            return: 5,
            [Symbol.iterator]() { return this; },
        }),
        () => true,
    ),
    throwingReturn: protocol(
        () => ({
            next: () => ({ done: false, value: E.a }),
            return() { throw new Error('close boom'); },
            [Symbol.iterator]() { return this; },
        }),
        () => true,
    ),
};

export default class T extends LoopingBot {
    loop() {}
}
"#;

fn spawn() -> LoadIsolate {
    LoadIsolate::spawn(SRC.to_string(), LoadShape::CompatClass, vec![]).unwrap()
}

fn all_bool(value: &serde_json::Value) -> bool {
    value
        .as_array()
        .expect("array")
        .iter()
        .all(|row| row == &serde_json::Value::Bool(true))
}

fn all_true(value: &serde_json::Value, key: &str) -> bool {
    value
        .as_array()
        .expect("array")
        .iter()
        .all(|row| row[key] == serde_json::Value::Bool(true))
}

#[test]
fn first_hit_and_exhaustion_fallback_are_native_decisions() {
    let iso = spawn();
    assert_eq!(
        iso.probe("__firstHit").unwrap(),
        serde_json::json!({
            "target": "c",
            "blocked": "null",
            "calls": ["a", "b", "c"],
            "identity": true,
            "blockedIsNull": true,
        }),
        "the first accepted candidate is returned and nothing after it is probed"
    );
    assert_eq!(
        iso.probe("__noHit").unwrap(),
        serde_json::json!({
            "target": "null",
            "blocked": "a",
            "calls": ["a", "b"],
            "firstIdentity": true,
            "targetIsNull": true,
        }),
        "exhaustion returns the first candidate as blocked"
    );
    assert_eq!(
        iso.probe("__empty").unwrap(),
        serde_json::json!({
            "target": "null",
            "blocked": "null",
            "targetIsNull": true,
            "blockedIsNull": true,
        }),
        "an empty iterable has no callback call and a null blocked slot"
    );
    assert_eq!(
        iso.probe("__holes").unwrap(),
        serde_json::json!({
            "target": "null",
            "blocked": "null",
            "calls": ["undefined", "object"],
            "blockedIsNull": true,
        }),
        "a hole is probed as undefined and `?? null` keeps null"
    );
    assert_eq!(
        iso.probe("__zero").unwrap(),
        serde_json::json!({ "blockedIsZero": true, "targetIsNull": true }),
        "a falsy non-nullish first candidate is not collapsed to null"
    );
    iso.join();
}

#[test]
fn truthiness_and_live_iteration_are_preserved() {
    let iso = spawn();
    assert!(
        all_bool(&iso.probe("__truthy").unwrap()),
        "a truthy non-true answer is a hit: {:?}",
        iso.probe("__truthy").unwrap()
    );
    assert!(
        all_bool(&iso.probe("__falsy").unwrap()),
        "a falsy answer never ends the scan: {:?}",
        iso.probe("__falsy").unwrap()
    );
    assert_eq!(
        iso.probe("__liveLength").unwrap(),
        serde_json::json!({ "target": "b", "blocked": "null", "identity": true }),
        "a candidate pushed during the scan is still visited"
    );
    assert_eq!(
        iso.probe("__liveElement").unwrap(),
        serde_json::json!({ "target": "c", "blocked": "null", "identity": true }),
        "an element replaced during the scan is re-read live"
    );
    assert_eq!(
        iso.probe("__mutatedFallback").unwrap(),
        serde_json::json!({ "blocked": "replaced", "isReplaced": true }),
        "the fallback reads index 0 after exhaustion, including mutations"
    );
    assert_eq!(
        iso.probe("__set").unwrap(),
        serde_json::json!({ "target": "b", "blocked": "null", "identity": true }),
        "the caller's iterator is traversed, not an array snapshot"
    );
    iso.join();
}

#[test]
fn callback_errors_receiver_order_reentrancy_and_close_are_preserved() {
    let iso = spawn();
    assert_eq!(
        iso.probe("__throwing").unwrap(),
        serde_json::json!({ "error": "cb boom", "calls": ["a"] }),
        "a throwing callback propagates and stops the scan"
    );
    assert_eq!(
        iso.probe("__callback").unwrap(),
        serde_json::json!({ "receivers": [true], "argCount": [1] }),
        "the callback keeps its undefined receiver and single argument"
    );
    assert_eq!(
        iso.probe("__reentrant").unwrap(),
        serde_json::json!({
            "target": "b",
            "blocked": "null",
            "identity": true,
            "nested": { "target": "c", "blocked": "null" },
            "nestedIdentity": true,
        }),
        "a nested chooseTarget does not disturb the outer scan"
    );
    assert_eq!(
        iso.probe("__close").unwrap(),
        serde_json::json!({ "afterHit": 1, "afterThrow": 2, "identity": true }),
        "the iterator is closed on a hit and on a throwing callback"
    );
    iso.join();
}

#[test]
fn the_callers_iterator_protocol_and_errors_are_exactly_the_previous_body() {
    let iso = spawn();
    let protocol = iso.probe("__protocol").unwrap();
    for case in [
        "getterNext",
        "swappedNext",
        "primitiveNext",
        "missingIterator",
        "primitiveIterator",
        "nonCallableReturn",
        "throwingReturn",
    ] {
        assert_eq!(
            protocol[case]["native"], protocol[case]["reference"],
            "{case} must keep the previous body's iterator protocol: {protocol:?}"
        );
    }
    assert_eq!(
        protocol["getterNext"]["native"]["trace"],
        serde_json::json!(["read:0", "call:0", "call:1"]),
        "`next` is read once at acquisition and called once per step: {protocol:?}"
    );
    assert_eq!(
        protocol["getterNext"]["native"]["calls"],
        serde_json::json!(["a", "b"]),
        "the callback order is unchanged: {protocol:?}"
    );
    assert_eq!(
        protocol["swappedNext"]["native"]["target"], "b",
        "a `next` re-read mid-scan must not replace the acquired one: {protocol:?}"
    );
    assert_eq!(
        protocol["primitiveNext"]["native"]["error"], "TypeError",
        "a primitive `next()` result is the engine's TypeError: {protocol:?}"
    );
    assert_eq!(
        protocol["primitiveNext"]["native"]["calls"],
        serde_json::json!([]),
        "a primitive `next()` result never reaches `reachable`: {protocol:?}"
    );
    assert_eq!(
        protocol["missingIterator"]["native"]["error"], "TypeError",
        "a missing `Symbol.iterator` is the engine's TypeError: {protocol:?}"
    );
    assert_eq!(
        protocol["primitiveIterator"]["native"]["error"], "TypeError",
        "a primitive `Symbol.iterator` result is the engine's TypeError: {protocol:?}"
    );
    assert_eq!(
        protocol["nonCallableReturn"]["native"]["error"], "TypeError",
        "a non-callable `iterator.return` on the hit is the engine's TypeError: {protocol:?}"
    );
    assert_eq!(
        protocol["throwingReturn"]["native"]["message"], "close boom",
        "a throwing close still propagates instead of the hit: {protocol:?}"
    );
    iso.join();
}

#[test]
fn the_export_matches_the_frozen_loop_body() {
    let iso = spawn();
    assert!(
        all_true(&iso.probe("__parity").unwrap(), "targetSame")
            && all_true(&iso.probe("__parity").unwrap(), "blockedSame")
            && all_true(&iso.probe("__parity").unwrap(), "callsSame"),
        "the native export matches the frozen `for...of` body: {:?}",
        iso.probe("__parity").unwrap()
    );
    iso.join();
}

#[derive(Clone, Copy)]
enum SnapshotMode {
    Keyframe { chat_open: bool },
    InventoryMissing,
    ChatUnknown,
}

fn thieving_data() -> std::sync::Arc<api::game_data::SelectedGameData> {
    api::game_data::for_revision(client::io::ClientRevision::R289).expect("selected 289 data")
}

fn npc_row<'a>(id: i32, name: &'a str, actions: &'a [String]) -> SceneEntityInput<'a> {
    SceneEntityInput {
        index: 7,
        id,
        name: Some(name),
        x: 3222,
        z: 3222,
        level: 0,
        distance: 1,
        health: 1,
        max_health: 1,
        in_combat: false,
        animating: false,
        actions,
        reachable: true,
        reachable_adj: true,
        combat_level: 1,
        target_kind: 0,
        target_index: -1,
        size: 1,
        nx: 3222,
        nz: 3222,
        shape: 0,
        angle: 0,
    }
}

fn snapshot_interact(
    data: std::sync::Arc<api::game_data::SelectedGameData>,
    action: &str,
    npc: SceneEntityInput<'_>,
    base: i32,
    effective: i32,
    mode: SnapshotMode,
) -> (bool, Vec<InteractReq>) {
    let src = format!(
        r#"
import {{ Npcs }} from '../../api/npcs/Npcs.js';
export default class T extends LoopingBot {{
    loop() {{
        globalThis.__accepted = Npcs.all()[0].interact({action:?});
    }}
}}
"#
    );
    let iso = LoadIsolate::spawn_with_game_data(src, LoadShape::CompatClass, vec![], data).unwrap();
    let stats = [StatInput {
        index: 17,
        name: "thieving",
        xp: 0,
        base,
        effective,
    }];
    let npcs = [npc];
    let mut snapshot = common::ingame_snapshot();
    snapshot.stats = &stats;
    snapshot.npcs = &npcs;
    let bytes = match mode {
        SnapshotMode::Keyframe { chat_open } => {
            snapshot.chat_open = chat_open;
            snapshot.chat_modal_id = if chat_open { 1 } else { -1 };
            encode_snapshot(&snapshot)
        }
        SnapshotMode::InventoryMissing => {
            // Keep the NPC/stat pages and a known-closed chat bit, but omit
            // the inventory vector so the host observes inventory as unknown.
            snapshot.chat_open = false;
            let last = SnapshotFingerprint {
                chat_open: true,
                ..SnapshotFingerprint::default()
            };
            encode_snapshot_delta(Some(&last), &snapshot, false).0
        }
        SnapshotMode::ChatUnknown => {
            // A changed inventory is posted, while unchanged chat_open is
            // absent from this delta and remains unknown to the host.
            let coins = [ItemRowInput::nc(Some("Coins"), 1)];
            snapshot.inv = &coins;
            encode_snapshot_delta(Some(&SnapshotFingerprint::default()), &snapshot, false).0
        }
    };
    iso.post_snapshot(bytes);
    iso.on_game_tick(1);
    let accepted = iso
        .probe("__accepted")
        .unwrap()
        .as_bool()
        .expect("Npc.interact result");
    let ops = iso.drain_interacts();
    iso.join();
    (accepted, ops)
}

fn workman(data: &api::game_data::SelectedGameData) -> (i32, i32) {
    let id = data
        .npc_by_config("digworkman1")
        .expect("selected Digsite Workman")
        .id;
    let required = data
        .required_thieving_npc(id)
        .expect("selected Workman level");
    assert_eq!(required, 25);
    (id, required)
}

fn expected_npc_op(name: &str, action: &str) -> InteractReq {
    InteractReq::Npc {
        name: name.into(),
        action: action.into(),
        index: Some(7),
    }
}

#[test]
fn npc_pickpocket_clicks_through_open_chat_modal() {
    let data = thieving_data();
    let (id, required) = workman(&data);
    let actions = ["Pickpocket".to_string()];
    let (accepted, ops) = snapshot_interact(
        data,
        "Pickpocket",
        npc_row(id, "Workman", &actions),
        required,
        required,
        SnapshotMode::Keyframe { chat_open: true },
    );
    assert!(
        accepted,
        "an open chat modal does not suppress the NPC click"
    );
    assert_eq!(ops, vec![expected_npc_op("Workman", "Pickpocket")]);
}

#[test]
fn npc_pickpocket_rowless_zealot_defaults_to_level_one() {
    let data = thieving_data();
    assert_eq!(
        data.required_thieving_npc(1528),
        None,
        "Zealot has no selected pickpocket row"
    );
    let actions = ["Pickpocket".to_string()];
    let (accepted, ops) = snapshot_interact(
        data,
        "Pickpocket",
        npc_row(1528, "Zealot", &actions),
        1,
        1,
        SnapshotMode::Keyframe { chat_open: false },
    );
    assert!(
        accepted,
        "an unlisted NPC uses the frozen level-one default"
    );
    assert_eq!(ops, vec![expected_npc_op("Zealot", "Pickpocket")]);
}

#[test]
fn npc_pickpocket_does_not_gate_on_unobserved_inventory() {
    let data = thieving_data();
    let (id, required) = workman(&data);
    let actions = ["Pickpocket".to_string()];
    let (accepted, ops) = snapshot_interact(
        data,
        "Pickpocket",
        npc_row(id, "Workman", &actions),
        required,
        required,
        SnapshotMode::InventoryMissing,
    );
    assert!(
        accepted,
        "missing inventory evidence does not refuse the click"
    );
    assert_eq!(ops, vec![expected_npc_op("Workman", "Pickpocket")]);
}

#[test]
fn npc_steal_from_clicks_through_unknown_chat_state() {
    let data = thieving_data();
    let (id, required) = workman(&data);
    let actions = ["Steal-from".to_string()];
    let (accepted, ops) = snapshot_interact(
        data,
        "Steal-from",
        npc_row(id, "Workman", &actions),
        required,
        required,
        SnapshotMode::ChatUnknown,
    );
    assert!(
        accepted,
        "unknown chat state does not suppress the NPC click"
    );
    assert_eq!(ops, vec![expected_npc_op("Workman", "Steal-from")]);
}

#[test]
fn npc_pickpocket_gate_uses_effective_not_base_thieving() {
    let data = thieving_data();
    let (id, required) = workman(&data);
    let actions = ["Pickpocket".to_string()];

    let (accepted, ops) = snapshot_interact(
        data.clone(),
        "Pickpocket",
        npc_row(id, "Workman", &actions),
        required - 1,
        required,
        SnapshotMode::Keyframe { chat_open: false },
    );
    assert!(
        accepted,
        "effective level meets the content gate despite lower base"
    );
    assert_eq!(ops, vec![expected_npc_op("Workman", "Pickpocket")]);

    let (accepted, ops) = snapshot_interact(
        data,
        "Pickpocket",
        npc_row(id, "Workman", &actions),
        required,
        required - 1,
        SnapshotMode::Keyframe { chat_open: false },
    );
    assert!(
        !accepted,
        "base level cannot override a low effective level"
    );
    assert!(
        ops.is_empty(),
        "an effective-level refusal queues no operation"
    );
}
