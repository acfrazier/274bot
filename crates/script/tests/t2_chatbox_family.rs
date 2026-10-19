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
fn all_readers_share_original_history_rows_in_recording_order() {
    let iso = spawn();
    let rows = [row(12, "host newer"), row(11, "host older")];
    let input = lines(&rows);
    tick(&iso, &input);

    assert_eq!(
        value(
            &iso,
            r#"(() => {
                const posted = globalThis.__rs2b0t_host.snapshot.chat_lines;
                const mark = __gameMessages.mark();
                const rows = __gameMessages.since(0);
                const again = __gameMessages.since(0);
                const recent = __gameMessages.recent(64);
                const first = __gameMessages.firstSince(0, /host/);
                __gameMessages.record('script local');
                const local = __gameMessages.since(mark);
                const localFirst = __gameMessages.firstSince(mark, /script local/);
                const localRecent = __gameMessages.recent(1);
                const localMark = __gameMessages.mark();
                const localIsVisible = local.length === 1 && __gameMessages.sawSince(mark, /script local/);
                const original = rows[0];
                original.text = 'caller-edited history';
                const originalIsMutable = __gameMessages.since(0)[0] === original &&
                    __gameMessages.sawSince(0, /caller-edited/) &&
                    __gameMessages.firstSince(0, /caller-edited/) === original;
                __gameMessages.reset();
                const resetMark = __gameMessages.mark();
                const emptyAfterReset = __gameMessages.recent().length === 0 &&
                    __gameMessages.since(0).length === 0 &&
                    !__gameMessages.sawSince(0, /host/) &&
                    __gameMessages.firstSince(0, /host/) === undefined;
                __gameMessages.record('after reset');
                return {
                    mark,
                    order: rows.map(row => row.seq),
                    originalRows: rows[0] === recent[1] && rows[1] === recent[0] &&
                        rows.every((row, index) => row === again[index]),
                    freshResultArray: rows !== again,
                    hostTextOrder: recent.map(row => row.text),
                    notPostedDtos: rows.every(row => !posted.includes(row)),
                    firstIsOriginal: first === rows[0],
                    localMark,
                    localOrder: local.map(row => [row.seq, row.text]),
                    localIsOriginal: local[0] === localFirst && localFirst === localRecent[0],
                    localIsVisible,
                    originalIsMutable,
                    cantReach: __cantReach.test("I can't reach that!"),
                    wrongSide: __wrongSide.test('You cannot do that from here.'),
                    resetMark,
                    emptyAfterReset,
                    resetSequence: __gameMessages.mark(),
                    postedPageUnchanged: posted[0].seq === 12 && posted[1].seq === 11,
                };
            })()"#,
        ),
        json!({
            "mark": 2,
            "order": [1, 2],
            "originalRows": true,
            "freshResultArray": true,
            "hostTextOrder": ["host newer", "caller-edited history"],
            "notPostedDtos": true,
            "firstIsOriginal": true,
            "localMark": 3,
            "localOrder": [[3, "script local"]],
            "localIsOriginal": true,
            "localIsVisible": true,
            "originalIsMutable": true,
            "cantReach": true,
            "wrongSide": true,
            "resetMark": 0,
            "emptyAfterReset": true,
            "resetSequence": 1,
            "postedPageUnchanged": true,
        })
    );
    iso.join();
}

#[test]
fn only_type_zero_host_lines_enter_history_and_ignored_rows_advance_the_cursor() {
    let iso = spawn();
    let mixed = [
        row(14, "game after"),
        ChatLineInput {
            type_: 2,
            username: Some("player"),
            ..row(13, "someone says hello")
        },
        row(12, "game before"),
    ];
    let mut input = lines(&mixed);
    tick(&iso, &input);
    assert_eq!(
        value(
            &iso,
            r#"({
                mark: __gameMessages.mark(),
                rows: __gameMessages.since(0).map(row => [row.seq, row.text]),
                afterFirst: __gameMessages.since(1).map(row => row.text),
                sawPublic: __gameMessages.sawSince(0, /someone says hello/),
                firstPublicAbsent: __gameMessages.firstSince(0, /someone says hello/) === undefined,
                firstGame: __gameMessages.firstSince(0, /game/).text,
                recent: __gameMessages.recent().map(row => row.text),
            })"#,
        ),
        json!({
            "mark": 2,
            "rows": [[1, "game before"], [2, "game after"]],
            "afterFirst": ["game after"],
            "sawPublic": false,
            "firstPublicAbsent": true,
            "firstGame": "game before",
            "recent": ["game after", "game before"],
        })
    );

    let ignored = [ChatLineInput {
        type_: 2,
        ..row(15, "ignored newest")
    }];
    input.chat_lines = &ignored;
    input.tick = 2;
    tick(&iso, &input);
    assert_eq!(value(&iso, "__gameMessages.mark()"), json!(2));
    assert_eq!(value(&iso, "__gameMessages.since(2)"), json!([]));
    let same_sequence = [row(15, "changed type must not replay")];
    input.chat_lines = &same_sequence;
    input.tick = 3;
    tick(&iso, &input);
    assert_eq!(value(&iso, "__gameMessages.mark()"), json!(2));
    assert_eq!(value(&iso, "__gameMessages.since(2)"), json!([]));

    iso.probe("__gameMessages.reset(), true").unwrap();
    input.chat_lines = &mixed;
    input.tick = 4;
    tick(&iso, &input);
    assert_eq!(value(&iso, "__gameMessages.mark()"), json!(0));
    assert_eq!(value(&iso, "__gameMessages.since(0)"), json!([]));
    iso.reset_session_work(None);
    let _ = iso.probe("true").unwrap();
    input.tick = 5;
    tick(&iso, &input);
    assert_eq!(
        value(
            &iso,
            "__gameMessages.since(0).map(row => [row.seq, row.text])"
        ),
        json!([[1, "game before"], [2, "game after"]])
    );
    iso.join();
}

#[test]
fn cavern_slice_last_two_keeps_the_two_game_lines() {
    // Preserve review-r2/cavern-slice-probe.rs and frozen upass/cavern.ts:187.
    let iso = spawn();
    let rows = [
        ChatLineInput {
            type_: 2,
            username: Some("player"),
            ..row(14, "someone says hello")
        },
        row(13, "the witch is talking to her cat"),
        row(12, "you knock at the door"),
    ];
    tick(&iso, &lines(&rows));
    let said = value(
        &iso,
        r#"__gameMessages.since(0).map(m => m.text)
            .filter(t => !t.startsWith('get ')).slice(-2).join(' ')"#,
    );
    assert_eq!(
        said,
        json!("you knock at the door the witch is talking to her cat"),
        "frozen slice(-2) keeps the two game lines and drops the public chat line"
    );
    iso.join();
}

#[test]
fn foreign_history_methods_getters_coercion_and_thrown_values_are_preserved() {
    let iso = spawn();
    let initial = [row(1, "initial")];
    let input = lines(&initial);
    tick(&iso, &input);

    assert_eq!(
        value(
            &iso,
            r#"(() => {
                let postedPageReads = 0;
                Object.defineProperty(globalThis.__rs2b0t_host.snapshot, 'chat_lines', {
                    configurable: true,
                    get() { postedPageReads++; throw new Error('history reader used posted page'); },
                });
                const markBefore = __gameMessages.mark();
                const events = [];
                const mark = { valueOf() { events.push('mark'); return 6; } };
                const row = {
                    get seq() { events.push('seq'); return '7'; },
                    get text() { events.push('text'); return 'kept'; },
                };
                const filterDescriptor = Object.getOwnPropertyDescriptor(Array.prototype, 'filter');
                let filterReceiver = false;
                let filterArgs;
                let filterCallbackLength;
                let includesGetterRecord = false;
                let stringSequenceAccepted = false;
                Object.defineProperty(Array.prototype, 'filter', {
                    configurable: true,
                    get() {
                        events.push('filter-get');
                        const receiver = this;
                        __gameMessages.record('recorded during filter getter');
                        return function (predicate) {
                            filterReceiver = this === receiver;
                            filterArgs = arguments.length;
                            filterCallbackLength = predicate.length;
                            includesGetterRecord = this.length === 2 &&
                                this[1].text === 'recorded during filter getter';
                            stringSequenceAccepted = predicate({ seq: '100' });
                            return predicate(row) ? [row] : [];
                        };
                    },
                });
                let result;
                try { result = __gameMessages.since(mark); }
                finally { Object.defineProperty(Array.prototype, 'filter', filterDescriptor); }
                const sinceEvents = events.slice();
                events.length = 0;

                const pattern = {
                    lastIndex: 9,
                    test(text) {
                        globalThis.__testReceiver = this === pattern;
                        return text === 'kept';
                    },
                };
                const someDescriptor = Object.getOwnPropertyDescriptor(Array.prototype, 'some');
                let someReceiver = false;
                let someArgs;
                let someCallbackLength;
                Object.defineProperty(Array.prototype, 'some', {
                    configurable: true,
                    get() {
                        events.push('some-get');
                        const receiver = this;
                        return function (predicate) {
                            someReceiver = this === receiver;
                            someArgs = arguments.length;
                            someCallbackLength = predicate.length;
                            return predicate(row);
                        };
                    },
                });
                let saw;
                try { saw = __gameMessages.sawSince(mark, pattern); }
                finally { Object.defineProperty(Array.prototype, 'some', someDescriptor); }
                const sawEvents = events.slice();

                const getterThrow = {};
                let getterIdentity = false;
                Object.defineProperty(Array.prototype, 'filter', {
                    configurable: true,
                    get() { throw getterThrow; },
                });
                try { __gameMessages.since(0); }
                catch (error) { getterIdentity = error === getterThrow; }
                finally { Object.defineProperty(Array.prototype, 'filter', filterDescriptor); }
                const coercionThrow = {};
                const coercionMark = { valueOf() { throw coercionThrow; } };
                const coercionIdentity = [
                    () => __gameMessages.since(coercionMark),
                    () => __gameMessages.sawSince(coercionMark, /./),
                    () => __gameMessages.firstSince(coercionMark, /./),
                ].every(invoke => {
                    try { invoke(); } catch (error) { return error === coercionThrow; }
                    return false;
                });
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
                __gameMessages.record('repeat');
                const regex = /repeat/g;
                regex.lastIndex = 99;
                const repeated = [__gameMessages.sawSince(2, regex), __gameMessages.sawSince(2, regex)];
                let badOpcodeRejected = false;
                let unknownOpcodeRejected = false;
                try { __rs2b0t_game_messages(1.5); } catch (error) { badOpcodeRejected = error instanceof TypeError; }
                try { __rs2b0t_game_messages(99); } catch (error) { unknownOpcodeRejected = error instanceof TypeError; }
                return {
                    markBefore, markAfter: __gameMessages.mark(), postedPageReads,
                    originalForeignRow: result[0] === row,
                    sinceEvents, saw, sawEvents, lastIndex: pattern.lastIndex,
                    filterReceiver, filterArgs, filterCallbackLength, includesGetterRecord,
                    stringSequenceAccepted, someReceiver, someArgs, someCallbackLength,
                    testReceiver: __testReceiver,
                    getterIdentity, coercionIdentity, setterIdentity, testIdentity,
                    lastIndexWasReset: __lastIndexWasReset,
                    callIdentity, callReceiver: __callReceiver, callSawReset: __callSawReset,
                    repeated, repeatedLastIndex: regex.lastIndex,
                    badOpcodeRejected, unknownOpcodeRejected,
                };
            })()"#,
        ),
        json!({
            "markBefore": 1, "markAfter": 3, "postedPageReads": 0,
            "originalForeignRow": true,
            "sinceEvents": ["filter-get", "mark", "seq", "mark"],
            "saw": true, "sawEvents": ["some-get", "seq", "mark", "text"], "lastIndex": 0,
            "filterReceiver": true, "filterArgs": 1, "filterCallbackLength": 1,
            "includesGetterRecord": true, "stringSequenceAccepted": true,
            "someReceiver": true, "someArgs": 1, "someCallbackLength": 1, "testReceiver": true,
            "getterIdentity": true, "coercionIdentity": true, "setterIdentity": true,
            "testIdentity": true, "lastIndexWasReset": true,
            "callIdentity": true, "callReceiver": true, "callSawReset": true,
            "repeated": [true, true], "repeatedLastIndex": 6,
            "badOpcodeRejected": true, "unknownOpcodeRejected": true,
        })
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
                const since = __gameMessages.since(0);
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
                    mark: __gameMessages.mark(),
                    sinceOrder: [since.length, since[0].seq, since[63].seq],
                    sinceOriginals: since.every((row, index) => row === all[63 - index]),
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
            "mark": 70,
            "sinceOrder": [64, 7, 70],
            "sinceOriginals": true,
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
