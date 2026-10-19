#![cfg(feature = "load")]

//! Served-runtime coverage for the native GameMessages implementation.
use script::isolate_fb::{
    decode_snapshot, ChatLineInput, IsolateBuf, SnapshotFingerprint, SnapshotInput,
};
use script::{LoadIsolate, LoadShape};
use serde_json::{json, Value};

mod common;
use common::{ingame_snapshot, post_snapshot_input};

const SOURCE: &str = r#"
import { CANT_REACH, GameMessages, WRONG_SIDE } from '../../api/chatbox/gameMessages.js';
globalThis.__gameMessages = GameMessages;
globalThis.__cantReach = CANT_REACH;
globalThis.__wrongSide = WRONG_SIDE;
export default class T extends LoopingBot {
    loop() {}
}
"#;

fn spawn() -> LoadIsolate {
    let iso = LoadIsolate::spawn(SOURCE.into(), LoadShape::CompatClass, vec![]).unwrap();
    assert_eq!(
        iso.probe("typeof globalThis.__gameMessages").unwrap(),
        json!("object")
    );
    iso
}

fn lines<'a>(rows: &'a [ChatLineInput<'a>]) -> SnapshotInput<'a> {
    let mut input: SnapshotInput<'a> = ingame_snapshot();
    input.chat_lines = rows;
    input
}

fn tick(iso: &LoadIsolate, input: &SnapshotInput<'_>) {
    post_snapshot_input(iso, input);
    iso.on_game_tick(input.tick);
    let _ = iso.probe("true").unwrap();
}

fn row(seq: i32, text: &'static str) -> ChatLineInput<'static> {
    ChatLineInput {
        seq,
        text,
        type_: 0,
        username: None,
    }
}

fn post_delta(
    encoder: &mut IsolateBuf,
    last: &mut Option<SnapshotFingerprint>,
    input: &SnapshotInput<'_>,
) -> Vec<u8> {
    let (bytes, fingerprint) = encoder.encode_snapshot_delta(last.as_ref(), input, false);
    *last = Some(fingerprint);
    bytes
}

fn tick_delta(iso: &LoadIsolate, tick: u64, bytes: Vec<u8>) {
    iso.post_snapshot(bytes);
    iso.on_game_tick(tick);
    let _ = iso.probe("true").unwrap();
}

fn value(iso: &LoadIsolate, expression: &str) -> Value {
    iso.probe(expression).unwrap()
}

#[test]
fn page_queries_and_history_keep_their_distinct_sources_and_identities() {
    let iso = spawn();
    let rows = [row(12, "host newer"), row(11, "host older")];
    let input = lines(&rows);
    tick(&iso, &input);

    assert_eq!(
        value(
            &iso,
            r#"(() => {
                const page = globalThis.__rs2b0t_host.snapshot.chat_lines;
                const mark = __gameMessages.mark();
                const pageRows = __gameMessages.since(10);
                const history = __gameMessages.recent(64);
                const first = __gameMessages.firstSince(0, /host/);
                const localMark = __gameMessages.mark();
                __gameMessages.record('script local');
                const local = __gameMessages.firstSince(2, /script local/);
                const localRecent = __gameMessages.recent(1);
                const separated = !__gameMessages.sawSince(12, /script local/) &&
                    __gameMessages.since(12).length === 0;
                __gameMessages.reset();
                const resetMark = __gameMessages.mark();
                const emptyAfterReset = __gameMessages.recent().length === 0;
                __gameMessages.record('after reset');
                const resetSequence = __gameMessages.recent(1)[0].seq;
                return {
                    mark,
                    page: pageRows.map(row => [row.seq, row.text]),
                    pageRowsAreCopies: pageRows.every((row, i) => row !== page[i]),
                    history: history.map(row => [row.seq, row.text]),
                    firstText: first.text,
                    firstIsOriginal: first === history[1],
                    localMark,
                    localIsOriginal: local === localRecent[0],
                    separated,
                    cantReach: __cantReach.test("I can't reach that!"),
                    wrongSide: __wrongSide.test('You cannot do that from here.'),
                    resetMark,
                    emptyAfterReset,
                    resetSequence,
                };
            })()"#,
        ),
        json!({
            "mark": 12,
            "page": [[12, "host newer"], [11, "host older"]],
            "pageRowsAreCopies": true,
            "history": [[2, "host newer"], [1, "host older"]],
            "firstText": "host older",
            "firstIsOriginal": true,
            "localMark": 12,
            "localIsOriginal": true,
            "separated": true,
            "cantReach": true,
            "wrongSide": true,
            "resetMark": 12,
            "emptyAfterReset": true,
            "resetSequence": 1,
        })
    );
    iso.join();
}

#[test]
fn foreign_page_methods_getters_coercion_and_thrown_values_are_preserved() {
    let iso = spawn();
    let initial = [row(1, "initial")];
    let input = lines(&initial);
    tick(&iso, &input);

    assert_eq!(
        value(
            &iso,
            r#"(() => {
                const events = [];
                const row = {
                    get seq() { events.push('seq'); return 7; },
                    get text() { events.push('text'); return 'kept'; },
                };
                const page = {
                    length: 1,
                    reduce(callback, initial) {
                        globalThis.__reduceReceiver = this === page;
                        globalThis.__reduceArgs = arguments.length;
                        globalThis.__reduceCallbackLength = callback.length;
                        const partial = callback(initial, { seq: 'not numeric' }, 0, this);
                        let seqGetterReads = 0;
                        const changingSeq = {
                            get seq() { seqGetterReads++; return seqGetterReads === 1 ? 13 : 9; },
                        };
                        const next = callback(partial, changingSeq, 1, this);
                        globalThis.__markSeqGetterReads = seqGetterReads;
                        return callback(next, { seq: 13 }, 2, this);
                    },
                    filter(callback) {
                        globalThis.__filterReceiver = this === page;
                        globalThis.__filterArgs = arguments.length;
                        globalThis.__filterCallbackLength = callback.length;
                        const filtered = {
                            map(mapper) {
                                globalThis.__mapReceiver = this === filtered;
                                globalThis.__mapArgs = arguments.length;
                                globalThis.__mapCallbackLength = mapper.length;
                                return [mapper(row, 0, this)];
                            },
                        };
                        globalThis.__nonNumericSinceAccepted =
                            callback({ seq: '100', text: 'wrong' }, 0, this);
                        return callback(row, 1, this) ? filtered : { map() { return []; } };
                    },
                    some(callback) {
                        globalThis.__someReceiver = this === page;
                        globalThis.__someArgs = arguments.length;
                        globalThis.__someCallbackLength = callback.length;
                        globalThis.__nonNumericSawAccepted =
                            callback({ seq: '100', text: 'wrong' }, 0, this);
                        return callback(row, 1, this);
                    },
                };
                globalThis.__rs2b0t_host.snapshot.chat_lines = page;

                const originalMax = Math.max;
                Math.max = function (a, b) {
                    globalThis.__mathMaxReceiver = this === Math;
                    globalThis.__mathMaxArgs = arguments.length;
                    return originalMax(a, b);
                };
                const numericGuardMark = __gameMessages.mark();
                Math.max = originalMax;
                events.length = 0;
                const mark = {
                    valueOf() { events.push('mark'); return 6; },
                };
                const copied = __gameMessages.since(mark);
                const sinceEvents = events.slice();
                events.length = 0;
                const pattern = {
                    lastIndex: 9,
                    test(text) {
                        globalThis.__testReceiver = this === pattern;
                        return text === 'kept';
                    },
                };
                const saw = __gameMessages.sawSince(mark, pattern);
                const sawEvents = events.slice();

                const getterThrow = {};
                Object.defineProperty(page, 'reduce', {
                    configurable: true,
                    get() { throw getterThrow; },
                });
                let getterIdentity = false;
                try { __gameMessages.mark(); } catch (error) { getterIdentity = error === getterThrow; }
                const coercionThrow = {};
                const coercionMark = { valueOf() { throw coercionThrow; } };
                let coercionIdentity = false;
                try { __gameMessages.since(coercionMark); }
                catch (error) { coercionIdentity = error === coercionThrow; }

                globalThis.__rs2b0t_host.snapshot.chat_lines = [{ seq: 2, text: 'throw' }];
                const setterThrow = {};
                const setterPattern = new Proxy({ test() { return true; } }, {
                    set(target, key, value) {
                        if (key === 'lastIndex') throw setterThrow;
                        return Reflect.set(target, key, value);
                    },
                });
                let setterIdentity = false;
                try { __gameMessages.sawSince(0, setterPattern); }
                catch (error) { setterIdentity = error === setterThrow; }

                const testThrow = {};
                const testPattern = {
                    set lastIndex(value) { globalThis.__lastIndexWasReset = value === 0; },
                    get test() { throw testThrow; },
                };
                let testIdentity = false;
                try { __gameMessages.sawSince(0, testPattern); }
                catch (error) { testIdentity = error === testThrow; }
                const callThrow = {};
                const callPattern = {
                    lastIndex: 12,
                    test() {
                        globalThis.__callReceiver = this === callPattern;
                        globalThis.__callSawReset = this.lastIndex === 0;
                        throw callThrow;
                    },
                };
                let callIdentity = false;
                try { __gameMessages.sawSince(0, callPattern); }
                catch (error) { callIdentity = error === callThrow; }

                let badOpcodeRejected = false;
                let unknownOpcodeRejected = false;
                try { __rs2b0t_game_messages(1.5); } catch (error) { badOpcodeRejected = error instanceof TypeError; }
                try { __rs2b0t_game_messages(99); } catch (error) { unknownOpcodeRejected = error instanceof TypeError; }
                return {
                    numericGuardMark,
                    markSeqGetterReads: __markSeqGetterReads,
                    copied: copied.map(item => [item.seq, item.text]),
                    sinceEvents,
                    saw,
                    sawEvents,
                    lastIndex: pattern.lastIndex,
                    getterIdentity,
                    setterIdentity,
                    coercionIdentity,
                    testIdentity,
                    lastIndexWasReset: __lastIndexWasReset,
                    callIdentity,
                    callReceiver: __callReceiver,
                    callSawReset: __callSawReset,
                    reduceReceiver: __reduceReceiver,
                    reduceArgs: __reduceArgs,
                    reduceCallbackLength: __reduceCallbackLength,
                    filterReceiver: __filterReceiver,
                    filterArgs: __filterArgs,
                    filterCallbackLength: __filterCallbackLength,
                    nonNumericSinceAccepted: __nonNumericSinceAccepted,
                    mapReceiver: __mapReceiver,
                    mapArgs: __mapArgs,
                    mapCallbackLength: __mapCallbackLength,
                    someReceiver: __someReceiver,
                    someArgs: __someArgs,
                    someCallbackLength: __someCallbackLength,
                    nonNumericSawAccepted: __nonNumericSawAccepted,
                    testReceiver: __testReceiver,
                    mathMaxReceiver: __mathMaxReceiver,
                    mathMaxArgs: __mathMaxArgs,
                    badOpcodeRejected,
                    unknownOpcodeRejected,
                };
            })()"#,
        ),
        json!({
            "numericGuardMark": 13,
            "markSeqGetterReads": 2,
            "copied": [[7, "kept"]],
            "sinceEvents": ["seq", "seq", "mark", "seq", "text"],
            "saw": true,
            "sawEvents": ["seq", "seq", "mark", "text"],
            "lastIndex": 0,
            "getterIdentity": true,
            "coercionIdentity": true,
            "setterIdentity": true,
            "testIdentity": true,
            "lastIndexWasReset": true,
            "reduceReceiver": true,
            "reduceArgs": 2,
            "reduceCallbackLength": 2,
            "callIdentity": true,
            "callReceiver": true,
            "callSawReset": true,
            "filterReceiver": true,
            "filterArgs": 1,
            "filterCallbackLength": 1,
            "mapReceiver": true,
            "mapArgs": 1,
            "mapCallbackLength": 1,
            "someReceiver": true,
            "someArgs": 1,
            "someCallbackLength": 1,
            "testReceiver": true,
            "badOpcodeRejected": true,
            "unknownOpcodeRejected": true,
            "nonNumericSinceAccepted": false,
            "nonNumericSawAccepted": false,
            "mathMaxReceiver": true,
            "mathMaxArgs": 2,
        })
    );

    assert_eq!(
        value(
            &iso,
            r#"(() => {
                globalThis.__rs2b0t_host.snapshot.chat_lines = [{ seq: 4, text: 'repeat' }];
                const pattern = /repeat/g;
                pattern.lastIndex = 99;
                const first = __gameMessages.sawSince(0, pattern);
                const firstLastIndex = pattern.lastIndex;
                const second = __gameMessages.sawSince(0, pattern);
                return [first, second, firstLastIndex, pattern.lastIndex];
            })()"#,
        ),
        json!([true, true, 6, 6])
    );
    iso.join();
}

#[test]
fn history_capacity_order_limits_and_foreign_slice_reverse_semantics_match() {
    let iso = spawn();
    assert_eq!(
        value(
            &iso,
            r#"(() => {
                for (let i = 0; i < 70; i++) __gameMessages.record(`line-${i}`);
                const all = __gameMessages.recent(NaN);
                const first = __gameMessages.firstSince(0, /line-/);
                const defaults = __gameMessages.recent();
                const fractional = __gameMessages.recent(1.5);
                const fractionalBelowOne = __gameMessages.recent(0.5);
                const negative = __gameMessages.recent(-0.5);
                const numericString = __gameMessages.recent('2.5');
                const coercible = __gameMessages.recent({ valueOf() { return 1.5; } });
                const bigintThrows = (() => {
                    try { __gameMessages.recent(2n); } catch (error) { return error instanceof TypeError; }
                    return false;
                })();

                const sliceDescriptor = Object.getOwnPropertyDescriptor(Array.prototype, 'slice');
                const reverseDescriptor = Object.getOwnPropertyDescriptor(Array.prototype, 'reverse');
                const events = [];
                let sliceGetReceiver = false;
                let sliceCallReceiver = false;
                let sliceStart;
                let reverseGetReceiver = false;
                let reverseCallReceiver = false;
                Object.defineProperty(Array.prototype, 'slice', {
                    configurable: true,
                    enumerable: sliceDescriptor.enumerable,
                    get() {
                        const receiver = this;
                        events.push('slice-get');
                        sliceGetReceiver = Array.isArray(receiver) && receiver.length === 64 && receiver[0].seq === 7;
                        return function (start) {
                            events.push('slice-call');
                            sliceStart = start;
                            sliceCallReceiver = this === receiver;
                            return sliceDescriptor.value.call(this, start);
                        };
                    },
                });
                Object.defineProperty(Array.prototype, 'reverse', {
                    configurable: true,
                    enumerable: reverseDescriptor.enumerable,
                    get() {
                        const receiver = this;
                        events.push('reverse-get');
                        reverseGetReceiver = Array.isArray(receiver) && receiver.length === 2;
                        return function () {
                            events.push('reverse-call');
                            reverseCallReceiver = this === receiver;
                            return reverseDescriptor.value.call(this);
                        };
                    },
                });
                let foreignResult;
                try {
                    foreignResult = __gameMessages.recent({
                        valueOf() { events.push('coerce'); return 2.5; },
                    });
                } finally {
                    Object.defineProperty(Array.prototype, 'slice', sliceDescriptor);
                    Object.defineProperty(Array.prototype, 'reverse', reverseDescriptor);
                }

                return {
                    count: all.length,
                    oldest: [all[63].seq, all[63].text],
                    newest: [all[0].seq, all[0].text],
                    firstIsOldestRetained: first === all[63],
                    defaultOrder: [defaults.length, defaults[0].seq, defaults[7].seq],
                    fractional: [fractional.length, fractional[0].seq],
                    fractionalBelowOne: fractionalBelowOne.length,
                    negative: negative.length,
                    numericString: numericString.map(row => row.seq),
                    coercible: [coercible.length, coercible[0].seq],
                    bigintThrows,
                    foreign: {
                        events,
                        sliceGetReceiver,
                        sliceCallReceiver,
                        sliceStart,
                        reverseGetReceiver,
                        reverseCallReceiver,
                        resultOrder: foreignResult.map(row => row.seq),
                    },
                };
            })()"#,
        ),
        json!({
            "count": 64,
            "oldest": [7, "line-6"],
            "newest": [70, "line-69"],
            "firstIsOldestRetained": true,
            "defaultOrder": [8, 70, 63],
            "fractional": [1, 70],
            "fractionalBelowOne": 64,
            "negative": 0,
            "numericString": [70, 69],
            "coercible": [1, 70],
            "bigintThrows": true,
            "foreign": {
                "events": ["coerce", "slice-get", "coerce", "slice-call", "reverse-get", "reverse-call"],
                "sliceGetReceiver": true,
                "sliceCallReceiver": true,
                "sliceStart": -2.5,
                "reverseGetReceiver": true,
                "reverseCallReceiver": true,
                "resultOrder": [70, 69],
            },
        })
    );
    iso.join();
}

#[test]
fn host_page_dedup_public_reset_and_session_reset_use_distinct_cursors() {
    let iso = spawn();
    let first_page = [row(12, "host newest"), row(11, "host oldest")];
    let changed_page = [row(13, "host newer again"), row(12, "host newest")];
    let mut input = lines(&first_page);
    let mut encoder = IsolateBuf::new();
    let mut last = None;
    let keyframe = post_delta(&mut encoder, &mut last, &input);
    assert!(decode_snapshot(&keyframe).unwrap().has_chat_lines());
    tick_delta(&iso, input.tick, keyframe);

    assert_eq!(
        value(&iso, "__gameMessages.recent(8).map(row => row.text)"),
        json!(["host newest", "host oldest"])
    );
    iso.probe("__gameMessages.reset(), true").unwrap();

    input.tick = 2;
    let omitted = post_delta(&mut encoder, &mut last, &input);
    assert!(!decode_snapshot(&omitted).unwrap().has_chat_lines());
    tick_delta(&iso, input.tick, omitted);
    assert_eq!(value(&iso, "__gameMessages.recent(8)"), json!([]));

    tick(&iso, &input);
    assert_eq!(value(&iso, "__gameMessages.recent(8)"), json!([]));

    input.chat_lines = &changed_page;
    input.tick = 3;
    tick(&iso, &input);
    assert_eq!(
        value(
            &iso,
            "__gameMessages.recent(8).map(row => [row.seq, row.text])"
        ),
        json!([[1, "host newer again"]])
    );

    iso.reconnect_session_work(None);
    let _ = iso.probe("true").unwrap();
    assert_eq!(value(&iso, "__gameMessages.recent(8)"), json!([]));
    input.tick = 4;
    tick(&iso, &input);
    assert_eq!(
        value(
            &iso,
            "__gameMessages.recent(8).map(row => [row.seq, row.text])"
        ),
        json!([[2, "host newer again"], [1, "host newest"]])
    );

    iso.reset_session_work(None);
    let _ = iso.probe("true").unwrap();
    assert_eq!(value(&iso, "__gameMessages.recent(8)"), json!([]));
    input.tick = 5;
    tick(&iso, &input);
    assert_eq!(
        value(
            &iso,
            "__gameMessages.recent(8).map(row => [row.seq, row.text])"
        ),
        json!([[2, "host newer again"], [1, "host newest"]])
    );
    iso.join();
}

#[test]
fn each_runtime_owns_an_independent_history() {
    let first = spawn();
    let second = spawn();
    first
        .probe("__gameMessages.record('first runtime'), true")
        .unwrap();

    assert_eq!(
        value(
            &first,
            "__gameMessages.recent(1).map(row => [row.seq, row.text])"
        ),
        json!([[1, "first runtime"]])
    );
    assert_eq!(value(&second, "__gameMessages.recent()"), json!([]));
    second
        .probe("__gameMessages.record('second runtime'), true")
        .unwrap();
    assert_eq!(
        value(
            &second,
            "__gameMessages.recent(1).map(row => [row.seq, row.text])"
        ),
        json!([[1, "second runtime"]])
    );
    assert_eq!(
        value(&first, "__gameMessages.recent(1).map(row => row.text)"),
        json!(["first runtime"])
    );
    second
        .probe(
            "globalThis.__messageTextRef = {}; __gameMessages.record(globalThis.__messageTextRef), true",
        )
        .unwrap();
    assert_eq!(
        value(
            &second,
            "__gameMessages.recent(1)[0].text === globalThis.__messageTextRef"
        ),
        json!(true)
    );
    first.join();
    second.join();
}
