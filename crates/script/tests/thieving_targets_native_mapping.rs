//! `chooseTarget`: the caller's iterator stays in JS; the visit order, the
//! first hit, the short circuit and the blocked fallback are native
//! `__rs2b0t_target_step` decisions.
//!
//! Parity here means the shim still produces what the previous `for...of` body
//! produced: the test drives an inline copy of that body (the reference) and the
//! native-backed export over the same inputs and compares target/blocked
//! identity plus the callback sequence.

use script::{LoadIsolate, LoadShape};

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

// The previous body, kept verbatim as the parity oracle.
function reference(candidates, reachable) {
    for (const c of candidates) {
        if (reachable(c)) return { target: c, blocked: null };
    }
    return { target: null, blocked: candidates[0] ?? null };
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

// The direct wire contract of `__rs2b0t_target_step`.
const step = globalThis.rustyscript.functions.__rs2b0t_target_step;
globalThis.__wire = {
    start: step({ op: 'choose' }),
    value: step({ op: 'choose', done: false }),
    done: step({ op: 'choose', done: true }),
    hit: step({ op: 'choose', probed: true }),
    miss: step({ op: 'choose', probed: false }),
    staleDone: step({ op: 'choose', done: false, probed: true }),
    unknown: step({ op: 'nope' }),
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
fn wire_steps_and_frozen_loop_parity_hold() {
    let iso = spawn();
    assert_eq!(
        iso.probe("__wire").unwrap(),
        serde_json::json!({
            "start": { "kind": "next" },
            "value": { "kind": "probe" },
            "done": { "kind": "exhausted" },
            "hit": { "kind": "hit" },
            "miss": { "kind": "next" },
            "staleDone": { "kind": "hit" },
            "unknown": { "kind": "notImpl", "reason": "unknown op" },
        }),
        "the step protocol answers the shim without holding state"
    );
    assert!(
        all_true(&iso.probe("__parity").unwrap(), "targetSame")
            && all_true(&iso.probe("__parity").unwrap(), "blockedSame")
            && all_true(&iso.probe("__parity").unwrap(), "callsSame"),
        "the native-backed export matches the previous `for...of` body: {:?}",
        iso.probe("__parity").unwrap()
    );
    iso.join();
}
