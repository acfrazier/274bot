#![cfg(feature = "load")]

use script::{LoadIsolate, LoadShape};

const API_BOT: &str = r#"
import * as Gathering from '@rs2b0t/api';
import {
    DEFAULT_BOOTH_NAME as namedBoothName,
    DEFAULT_BOOTH_OP as namedBoothOp,
    MAP_SQUARE as namedMapSquare,
    sameMapSquare as namedSameMapSquare,
    locationOptions as namedLocationOptions,
    boothFields as namedBoothFields,
    resolveGatheringLocation as namedResolveGatheringLocation,
} from '@rs2b0t/api';

globalThis.__gathering = Gathering;
globalThis.__gatheringNamed = {
    namedBoothName,
    namedBoothOp,
    namedMapSquare,
    namedSameMapSquare,
    namedLocationOptions,
    namedBoothFields,
    namedResolveGatheringLocation,
};

export default class GatheringProbe extends LoopingBot {
    loop() {}
}
"#;

fn gathering_isolate() -> LoadIsolate {
    let isolate = LoadIsolate::spawn(API_BOT.to_owned(), LoadShape::CompatClass, vec![])
        .expect("spawn gathering API isolate");
    assert_eq!(
        isolate.probe("typeof globalThis.__gathering").unwrap(),
        "object"
    );
    isolate
}

#[test]
fn gathering_helpers_are_root_exports_and_match_generic_policy() {
    let isolate = gathering_isolate();
    let result = isolate
        .probe(
            r#"(() => {
                const api = globalThis.__gathering;
                const namedExports = globalThis.__gatheringNamed;
                const start = { x: 63, z: 10, level: 0 };
                const inside = { name: 'inside', spot: { x: 60, z: 10, level: 0 } };
                const acrossChunk = { name: 'across', spot: { x: 64, z: 10, level: 0 } };
                const duplicateA = { name: 'Birch' };
                const duplicateB = { name: 'Birch' };
                const options = api.locationOptions([duplicateA, duplicateB, { name: '' }]);
                const booths = api.boothFields({ boothName: '', boothOp: 0 });
                const defaults = [
                    api.boothFields(null),
                    api.boothFields(undefined),
                    api.boothFields({ boothName: null, boothOp: undefined }),
                ];
                const stableA = { name: 'first tie', spot: { x: 3, z: 4, level: 0 } };
                const stableB = { name: 'second tie', spot: { x: 4, z: 3, level: 5 } };
                const nearOtherPlane = { name: 'other plane', spot: { x: 1, z: 0, level: 1 } };
                const farther = { name: 'same plane', spot: { x: 10, z: 0, level: 0 } };
                const noneByName = { name: 'None', spot: { x: 0, z: 0, level: 0 } };
                const named = { name: 'Camp A', spot: { x: 0, z: 0, level: 0 } };
                const sameSquare = api.sameMapSquare(
                    { x: -1, z: -65, level: 2 },
                    { x: -63, z: -127, level: 2 },
                );
                const negativeEdge = api.sameMapSquare(
                    { x: -64, z: 0, level: 0 },
                    { x: -65, z: 0, level: 0 },
                );
                const positiveEdge = api.sameMapSquare(
                    { x: 63, z: 0, level: 0 },
                    { x: 64, z: 0, level: 0 },
                );
                const otherLevel = api.sameMapSquare(
                    { x: 1, z: 1, level: 0 },
                    { x: 1, z: 1, level: 1 },
                );
                const nanSquare = api.sameMapSquare(
                    { x: NaN, z: 1, level: 0 },
                    { x: NaN, z: 1, level: 0 },
                );
                return {
                    rootIdentity:
                        api.DEFAULT_BOOTH_NAME === namedExports.namedBoothName
                        && api.DEFAULT_BOOTH_OP === namedExports.namedBoothOp
                        && api.MAP_SQUARE === namedExports.namedMapSquare
                        && api.sameMapSquare === namedExports.namedSameMapSquare
                        && api.locationOptions === namedExports.namedLocationOptions
                        && api.boothFields === namedExports.namedBoothFields
                        && api.resolveGatheringLocation === namedExports.namedResolveGatheringLocation,
                    constants: [api.DEFAULT_BOOTH_NAME, api.DEFAULT_BOOTH_OP, api.MAP_SQUARE],
                    options,
                    booths,
                    defaults,
                    chunks: [sameSquare, negativeEdge, positiveEdge, otherLevel, nanSquare],
                    autoPrefersSameChunk: api.resolveGatheringLocation(' Auto ', start, [acrossChunk, inside]) === inside,
                    closestCanLeaveChunk: api.resolveGatheringLocation(' USE CLOSEST ', start, [inside, acrossChunk]) === acrossChunk,
                    closestIgnoresPlane: api.resolveGatheringLocation('Use Closest', { x: 0, z: 0, level: 0 }, [farther, nearOtherPlane]) === nearOtherPlane,
                    closestKeepsFirstTie: api.resolveGatheringLocation('Use Closest', { x: 0, z: 0, level: 0 }, [stableA, stableB]) === stableA,
                    namedCaseAndWhitespace: api.resolveGatheringLocation('  cAmP a  ', { x: 0, z: 0, level: 0 }, [named]) === named,
                    noneCanBeNamed: api.resolveGatheringLocation('None', { x: 0, z: 0, level: 0 }, [noneByName]) === noneByName,
                    startAlias: api.resolveGatheringLocation(' Use Start Position ', start, [inside]) === null,
                    customAlias: api.resolveGatheringLocation('uSe CuStOm PoSiTiOn', start, [inside]) === null,
                    missingName: api.resolveGatheringLocation('missing', start, [inside]) === null,
                    emptyAuto: api.resolveGatheringLocation('Auto', start, []) === null,
                    emptyClosest: api.resolveGatheringLocation('Use Closest', start, []) === null,
                    nanDistanceKeepsFirst: api.resolveGatheringLocation(
                        'Use Closest',
                        { x: NaN, z: 0, level: 0 },
                        [stableA, stableB],
                    ) === stableA,
                };
            })()"#,
        )
        .unwrap();

    assert_eq!(result["rootIdentity"], true);
    assert_eq!(
        result["constants"],
        serde_json::json!(["Bank booth", "Use-quickly", 64])
    );
    assert_eq!(
        result["options"],
        serde_json::json!([
            "Auto",
            "Use Closest",
            "Use Start Position",
            "Use Custom Position",
            "None",
            "Birch",
            "Birch",
            ""
        ])
    );
    assert_eq!(
        result["booths"],
        serde_json::json!({ "boothName": "", "boothOp": 0 })
    );
    assert_eq!(
        result["defaults"],
        serde_json::json!([
            { "boothName": "Bank booth", "boothOp": "Use-quickly" },
            { "boothName": "Bank booth", "boothOp": "Use-quickly" },
            { "boothName": "Bank booth", "boothOp": "Use-quickly" },
        ])
    );
    assert_eq!(
        result["chunks"],
        serde_json::json!([true, false, false, false, false])
    );
    for key in [
        "autoPrefersSameChunk",
        "closestCanLeaveChunk",
        "closestIgnoresPlane",
        "closestKeepsFirstTie",
        "namedCaseAndWhitespace",
        "noneCanBeNamed",
        "startAlias",
        "customAlias",
        "missingName",
        "emptyAuto",
        "emptyClosest",
        "nanDistanceKeepsFirst",
    ] {
        assert_eq!(result[key], true, "{key}: {result}");
    }
}

#[test]
fn gathering_helpers_preserve_observation_order_and_thrown_identity() {
    let isolate = gathering_isolate();
    let result = isolate
        .probe(
            r#"(() => {
                const api = globalThis.__gathering;
                const events = [];
                const a = {
                    get level() { events.push('a.level'); return 0; },
                    get x() { events.push('a.x'); return { valueOf() { events.push('a.x.number'); return 1; } }; },
                    get z() { events.push('a.z'); return 3; },
                };
                const b = {
                    get level() { events.push('b.level'); return 0; },
                    get x() { events.push('b.x'); return { valueOf() { events.push('b.x.number'); return 2; } }; },
                    get z() { events.push('b.z'); return 4; },
                };
                const same = api.sameMapSquare(a, b);

                const shortCircuit = [];
                const notSame = api.sameMapSquare(
                    {
                        get level() { shortCircuit.push('a.level'); return 0; },
                        get x() { shortCircuit.push('a.x'); throw new Error('unread'); },
                    },
                    {
                        get level() { shortCircuit.push('b.level'); return 1; },
                        get x() { shortCircuit.push('b.x'); throw new Error('unread'); },
                    },
                );

                const conversionOrder = [];
                const coord = (label, number) => ({
                    valueOf() { conversionOrder.push(label + '.number'); return number; },
                });
                const start = {
                    get x() { conversionOrder.push('start.x'); return coord('start.x', 0); },
                    get z() { conversionOrder.push('start.z'); return coord('start.z', 0); },
                };
                const row = {
                    spot: {
                        get x() { conversionOrder.push('bank.x'); return coord('bank.x', 3); },
                        get z() { conversionOrder.push('bank.z'); return coord('bank.z', 4); },
                    },
                };
                const closest = api.resolveGatheringLocation('Use Closest', start, [row]);

                const settingOrder = [];
                const setting = {
                    get trim() {
                        settingOrder.push('trim.get');
                        return function() {
                            settingOrder.push('trim.call');
                            return {
                                get toLowerCase() {
                                    settingOrder.push('lower.get');
                                    return function() {
                                        settingOrder.push('lower.call');
                                        return 'use start position';
                                    };
                                },
                            };
                        };
                    },
                };
                const freeform = api.resolveGatheringLocation(setting, null, null);

                const sentinel = { tag: 'thrown-value' };
                let mapThrowIdentity = false;
                try {
                    api.locationOptions([{ get name() { throw sentinel; } }]);
                } catch (error) {
                    mapThrowIdentity = error === sentinel;
                }
                let levelThrowIdentity = false;
                try {
                    api.sameMapSquare({ get level() { throw sentinel; } }, { level: 0 });
                } catch (error) {
                    levelThrowIdentity = error === sentinel;
                }
                let numberThrowIdentity = false;
                try {
                    api.sameMapSquare(
                        { level: 0, x: { valueOf() { throw sentinel; } } },
                        { level: 0, x: 0 },
                    );
                } catch (error) {
                    numberThrowIdentity = error === sentinel;
                }
                return {
                    same,
                    events,
                    shortCircuit: [notSame, shortCircuit],
                    closestIdentity: closest === row,
                    conversionOrder,
                    freeform: freeform === null,
                    settingOrder,
                    mapThrowIdentity,
                    levelThrowIdentity,
                    numberThrowIdentity,
                };
            })()"#,
        )
        .unwrap();

    assert_eq!(result["same"], true);
    assert_eq!(
        result["events"],
        serde_json::json!([
            "a.level",
            "b.level",
            "a.x",
            "a.x.number",
            "b.x",
            "b.x.number",
            "a.z",
            "b.z"
        ])
    );
    assert_eq!(
        result["shortCircuit"],
        serde_json::json!([false, ["a.level", "b.level"]])
    );
    assert_eq!(result["closestIdentity"], true);
    assert_eq!(
        result["conversionOrder"],
        serde_json::json!([
            "bank.x",
            "start.x",
            "bank.x.number",
            "start.x.number",
            "bank.z",
            "start.z",
            "bank.z.number",
            "start.z.number"
        ])
    );
    assert_eq!(result["freeform"], true);
    assert_eq!(
        result["settingOrder"],
        serde_json::json!(["trim.get", "trim.call", "lower.get", "lower.call"])
    );
    for key in [
        "mapThrowIdentity",
        "levelThrowIdentity",
        "numberThrowIdentity",
    ] {
        assert_eq!(result[key], true, "{key}: {result}");
    }
}

#[test]
fn gathering_helpers_reject_malformed_calls_with_js_errors() {
    let isolate = gathering_isolate();
    let result = isolate
        .probe(
            r#"(() => {
                const api = globalThis.__gathering;
                const throwsTypeError = (call) => {
                    try { call(); return false; }
                    catch (error) { return error instanceof TypeError; }
                };
                return {
                    nullTable: throwsTypeError(() => api.locationOptions(null)),
                    badSetting: throwsTypeError(() => api.resolveGatheringLocation(4, {}, [])),
                    badName: throwsTypeError(() => api.resolveGatheringLocation('missing', {}, [null])),
                    badTileCoordinate: throwsTypeError(() => api.sameMapSquare({ level: 0, x: Symbol(), z: 0 }, { level: 0, x: 0, z: 0 })),
                };
            })()"#,
        )
        .unwrap();

    for key in ["nullTable", "badSetting", "badName", "badTileCoordinate"] {
        assert_eq!(result[key], true, "{key}: {result}");
    }
}

#[test]
fn gathering_alias_classification_keeps_full_utf16_and_non_string_identity() {
    let isolate = gathering_isolate();
    let result = isolate
        .probe(
            r#"(() => {
                const api = globalThis.__gathering;
                const start = { x: 1, z: 1, level: 0 };
                const names = [
                    'use custom position with a longer name',
                    'use start position with a longer name',
                    'use custom positio\u016f',
                    'a named camp with a lone surrogate \ud800',
                ];
                const rows = names.map(name => ({ name, spot: start }));
                const token = {};
                let coerced = false;
                token.toString = () => { coerced = true; return 'use custom position'; };
                const setting = { trim() { return { toLowerCase() { return token; } }; } };
                const identityRow = { name: { toLowerCase() { return token; } } };
                return {
                    namedRows: rows.map(row =>
                        api.resolveGatheringLocation(row.name, start, rows) === row),
                    nonStringIdentity:
                        api.resolveGatheringLocation(setting, null, [identityRow]) === identityRow,
                    noCoercion: !coerced,
                };
            })()"#,
        )
        .unwrap();
    assert_eq!(
        result["namedRows"],
        serde_json::json!([true, true, true, true])
    );
    assert_eq!(result["nonStringIdentity"], true);
    assert_eq!(result["noCoercion"], true);
}

#[test]
fn shared_resolver_driver_preserves_foreign_pool_rank_and_find_calls() {
    let isolate = gathering_isolate();
    let result = isolate
        .probe(
            r#"(() => {
                const resolve = globalThis.__gathering.resolveGatheringLocation;
                const events = [];
                const receivers = [];
                const setting = (value) => {
                    const lowered = {
                        get toLowerCase() {
                            events.push('lower.get');
                            return function() {
                                events.push('lower.call');
                                receivers.push(this === lowered);
                                return value;
                            };
                        },
                    };
                    const original = {
                        get trim() {
                            events.push('trim.get');
                            return function() {
                                events.push('trim.call');
                                receivers.push(this === original);
                                return lowered;
                            };
                        },
                    };
                    return original;
                };
                const start = {
                    get x() { events.push('start.x'); return 0; },
                    get z() { events.push('start.z'); return 0; },
                };
                const row = (label, distance) => ({
                    get spot() {
                        events.push(label + '.spot');
                        return {
                            get x() { events.push(label + '.x'); return distance; },
                            get z() { events.push(label + '.z'); return 0; },
                        };
                    },
                });
                const first = row('first', 5);
                const second = row('second', 3);
                const pool = {
                    get length() { events.push('length'); return 2; },
                    get 0() { events.push('row.0'); return first; },
                    get 1() { events.push('row.1'); return second; },
                    get filter() { throw new Error('closest must not filter'); },
                    get find() { throw new Error('rank must not find'); },
                };
                const table = {
                    get filter() {
                        events.push('filter.get');
                        return function(predicate) {
                            events.push('filter.call');
                            receivers.push(this === table && typeof predicate === 'function');
                            return pool;
                        };
                    },
                };
                const auto = resolve(setting('auto'), start, table) === second;
                const autoTrace = events.splice(0);
                const closest = resolve(setting('use closest'), start, pool) === second;
                const closestTrace = events.splice(0);
                const name = {
                    get toLowerCase() {
                        events.push('name.lower.get');
                        return function() {
                            events.push('name.lower.call');
                            receivers.push(this === name);
                            return 'camp';
                        };
                    },
                };
                const named = {
                    get name() { events.push('name.get'); return name; },
                    get spot() { throw new Error('named must not rank'); },
                };
                const namedTable = {
                    get find() {
                        events.push('find.get');
                        return function(predicate) {
                            events.push('find.call');
                            receivers.push(this === namedTable);
                            return predicate(named) ? named : undefined;
                        };
                    },
                    get filter() { throw new Error('named must not filter'); },
                };
                const found = resolve(setting('camp'), null, namedTable) === named;
                const findTrace = events.splice(0);
                const sentinel = {};
                const throwsSame = (call) => {
                    try { call(); return false; }
                    catch (error) { return error === sentinel; }
                };
                const errors = [
                    throwsSame(() => resolve('Auto', null, {
                        get filter() { throw sentinel; },
                    })),
                    throwsSame(() => resolve('Auto', null, {
                        filter() { throw sentinel; },
                    })),
                    throwsSame(() => resolve('Use Closest', start, {
                        get length() { throw sentinel; },
                    })),
                    throwsSame(() => resolve('Use Closest', start, [
                        first, { get spot() { throw sentinel; } },
                    ])),
                    throwsSame(() => resolve('camp', null, {
                        get find() { throw sentinel; },
                    })),
                    throwsSame(() => resolve('camp', null, {
                        find() { throw sentinel; },
                    })),
                    throwsSame(() => resolve('camp', null, [
                        { get name() { throw sentinel; } },
                    ])),
                ];
                return {
                    auto, closest, found, autoTrace, closestTrace, findTrace,
                    receivers: receivers.every(Boolean), errors,
                };
            })()"#,
        )
        .unwrap();
    let normalization = ["trim.get", "trim.call", "lower.get", "lower.call"];
    let ranking = [
        "length",
        "row.0",
        "first.spot",
        "first.x",
        "start.x",
        "first.z",
        "start.z",
        "length",
        "row.1",
        "second.spot",
        "second.x",
        "start.x",
        "second.z",
        "start.z",
        "length",
    ];
    let auto_trace: Vec<_> = normalization
        .into_iter()
        .chain(["filter.get", "filter.call"])
        .chain(ranking)
        .collect();
    let closest_trace: Vec<_> = normalization.into_iter().chain(ranking).collect();
    let find_trace: Vec<_> = normalization
        .into_iter()
        .chain([
            "find.get",
            "find.call",
            "name.get",
            "name.lower.get",
            "name.lower.call",
        ])
        .collect();
    assert_eq!(result["autoTrace"], serde_json::json!(auto_trace));
    assert_eq!(result["closestTrace"], serde_json::json!(closest_trace));
    assert_eq!(result["findTrace"], serde_json::json!(find_trace));
    for key in ["auto", "closest", "found", "receivers"] {
        assert_eq!(result[key], true, "{key}: {result}");
    }
    assert_eq!(
        result["errors"],
        serde_json::json!([true, true, true, true, true, true, true])
    );
}
