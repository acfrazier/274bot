//! Complete local world-primitive surface through the production Load bridge.

use serde_json::{json, Value};
use {script_js::LoadIsolate, script_js::LoadShape};

mod common;

fn probe(body: &str) -> Value {
    let source = format!(
        r#"
import Tile, {{ tileFromPosted }} from '../../geometry/Tile.js';
import {{ Area }} from '../../geometry/Area.js';
import {{ Tile as PublicTile, Area as PublicArea }} from '@rs2b0t/api';
export default class WorldPrimitiveProbe extends LoopingBot {{
    loop() {{
        if (globalThis.__worldRan) return;
        globalThis.__worldRan = true;
        {body}
    }}
}}
"#
    );
    let isolate = LoadIsolate::spawn(source, LoadShape::CompatClass, vec![]).unwrap();
    common::post_snapshot_input(&isolate, &common::ingame_snapshot());
    isolate.on_game_tick(1);
    let value = isolate.probe("globalThis.__world").unwrap();
    assert!(
        isolate.drain_interacts().is_empty(),
        "geometry sends no action"
    );
    isolate.join();
    value
}

#[test]
fn all_tile_members_preserve_class_identity_defaults_and_numeric_behavior() {
    assert_eq!(
        probe(
            r#"
const tile = new Tile(10, 20, 2);
const copy = Tile.from(tile);
const translated = tile.translate(0.5, -3);
const empty = new Tile();
const missing = Tile.from({ x: 4, z: 5 });
const nullLevel = Tile.from({ x: 4, z: 5, level: null });
class Derived extends Tile {}
const derived = Derived.from(tile);
globalThis.__world = {
    sameClass: Tile === PublicTile,
    sameArea: Area === PublicArea,
    fields: Object.keys(tile),
    copied: copy !== tile && copy instanceof Tile && copy.equals(tile),
    translated: [translated.x, translated.z, translated.level, translated instanceof Tile],
    original: [tile.x, tile.z, tile.level],
    constructorDefaults: [empty.x === undefined, empty.z === undefined, empty.level],
    missingLevel: missing.level,
    nullLevel: nullLevel.level,
    staticFromUsesBaseClass: derived instanceof Tile && !(derived instanceof Derived),
    samePlane: tile.distanceTo(new Tile(13, 16, 2)),
    otherPlane: tile.distanceTo(new Tile(13, 16, 1)),
    implicitDistancePlane: new Tile(4, 5).distanceTo({ x: 4, z: 8 }),
    nanDistance: Number.isNaN(new Tile(NaN, 2).distanceTo(new Tile(1, 2))),
    infinityDistance: new Tile(Infinity, 2).distanceTo(new Tile(1, 2)) === Infinity,
    text: tile.toString(),
};
"#
        ),
        json!({
            "sameClass": true,
            "sameArea": true,
            "fields": ["x", "z", "level"],
            "copied": true,
            "translated": [10.5, 17, 2, true],
            "original": [10, 20, 2],
            "constructorDefaults": [true, true, 0],
            "missingLevel": 0,
            "nullLevel": 0,
            "staticFromUsesBaseClass": true,
            "samePlane": 4,
            "otherPlane": 1_000_004,
            "implicitDistancePlane": 3,
            "nanDistance": true,
            "infinityDistance": true,
            "text": "(10, 20, 2)",
        })
    );
}

#[test]
fn tile_copy_translation_and_equality_keep_observable_order_and_thrown_values() {
    assert_eq!(
        probe(
            r#"
const trace = [];
const source = {
    get x() { trace.push('x'); return 10; },
    get z() { trace.push('z'); return 'twenty'; },
    get level() { trace.push('level'); return 2; },
};
const delta = name => ({ [Symbol.toPrimitive](hint) { trace.push(name + ':' + hint); return 3; } });
const moved = Tile.prototype.translate.call(source, delta('dx'), delta('dz'));
const translateTrace = trace.splice(0);
const copied = Tile.from(source);
const copyTrace = trace.splice(0);
const sentinel = { marker: 'same thrown object' };
const other = { get x() { trace.push('other.x'); return 99; }, get z() { throw sentinel; } };
const shortCircuit = Tile.prototype.equals.call(source, other);
const equalityTrace = trace.splice(0);
let thrownCopy = false;
let thrownTranslate = false;
let thrownEquals = false;
try { Tile.from({ get x() { throw sentinel; } }); } catch (error) { thrownCopy = error === sentinel; }
try { new Tile(1, 2).translate({ valueOf() { throw sentinel; } }, 0); } catch (error) { thrownTranslate = error === sentinel; }
try { new Tile(1, 2).equals({ get x() { throw sentinel; } }); } catch (error) { thrownEquals = error === sentinel; }
let nullError = false;
try { new Tile(1, 2).equals(null); } catch (error) { nullError = error instanceof TypeError; }
globalThis.__world = {
    moved: [moved.x, moved.z, moved.level],
    copied: [copied.x, copied.z, copied.level],
    translateTrace, copyTrace, equalityTrace, shortCircuit,
    thrownCopy, thrownTranslate, thrownEquals, nullError,
    strictTypes: !new Tile(1, 2).equals({ x: '1', z: 2, level: 0 }),
    missingPlane: !new Tile(1, 2).equals({ x: 1, z: 2 }),
    signedZero: new Tile(-0, 2).equals(new Tile(0, 2)),
    nanUnequal: !new Tile(NaN, 2).equals(new Tile(NaN, 2)),
};
"#
        ),
        json!({
            "moved": [13, "twenty3", 2],
            "copied": [10, "twenty", 2],
            "translateTrace": ["x", "dx:default", "z", "dz:default", "level"],
            "copyTrace": ["x", "z", "level"],
            "equalityTrace": ["x", "other.x"],
            "shortCircuit": false,
            "thrownCopy": true,
            "thrownTranslate": true,
            "thrownEquals": true,
            "nullError": true,
            "strictTypes": true,
            "missingPlane": true,
            "signedZero": true,
            "nanUnequal": true,
        })
    );
}

#[test]
fn tile_text_uses_exact_js_string_conversion_and_preserves_utf16() {
    assert_eq!(
        probe(
            r#"
const trace = [];
const source = {};
for (const key of ['x', 'z', 'level']) {
    Object.defineProperty(source, key, { get() {
        trace.push('get:' + key);
        return { [Symbol.toPrimitive](hint) { trace.push(key + ':' + hint); return key; } };
    } });
}
const text = Tile.prototype.toString.call(source);
const sentinel = {};
let thrown = false;
try { new Tile({ toString() { throw sentinel; } }, 0).toString(); } catch (error) { thrown = error === sentinel; }
let symbolError = false;
try { new Tile(Symbol('x'), 0).toString(); } catch (error) { symbolError = error instanceof TypeError; }
const surrogate = new Tile('\ud800', 0).toString();
globalThis.__world = {
    text, trace, thrown, symbolError,
    numericText: new Tile(-0, Infinity, NaN).toString(),
    surrogateCodeUnit: surrogate.charCodeAt(1),
};
"#
        ),
        json!({
            "text": "(x, z, level)",
            "trace": ["get:x", "x:string", "get:z", "z:string", "get:level", "level:string"],
            "thrown": true,
            "symbolError": true,
            "numericText": "(0, Infinity, NaN)",
            "surrogateCodeUnit": 55_296,
        })
    );
}

#[test]
fn posted_factory_validates_all_bounds_and_preserves_existing_tile_identity() {
    assert_eq!(
        probe(
            r#"
const tile = new Tile(0, 16_383, 3);
class Derived extends Tile {}
const derived = new Derived(1, 2, 1);
const plain = tileFromPosted({ x: 0, z: 16_383 });
const array = tileFromPosted(Object.assign([], { x: 1, z: 2, level: 0 }));
const invalid = [null, undefined, false, 0, 'tile', Object.assign(() => {}, { x: 1, z: 2 }), {},
    { x: -1, z: 2 }, { x: 16_384, z: 2 }, { x: 1, z: -1 }, { x: 1, z: 16_384 },
    { x: 1.5, z: 2 }, { x: NaN, z: 2 }, { x: Infinity, z: 2 }, { x: '1', z: 2 },
    { x: new Number(1), z: 2 }, { x: 1, z: 2, level: -1 }, { x: 1, z: 2, level: 4 },
    { x: 1, z: 2, level: 1.5 }, { x: 1, z: 2, level: null }, { x: 1, z: 2, level: '0' },
    { x: Number.MAX_SAFE_INTEGER + 1, z: 2 }];
const negativeZero = tileFromPosted({ x: -0, z: -0, level: -0 });
globalThis.__world = {
    sameTile: tileFromPosted(tile) === tile,
    sameDerived: tileFromPosted(derived) === derived,
    defaultPlane: plain instanceof Tile && plain.equals(new Tile(0, 16_383, 0)),
    arrayObject: array instanceof Tile && array.equals(new Tile(1, 2, 0)),
    invalidCount: invalid.filter(value => tileFromPosted(value) === null).length,
    expectedInvalid: invalid.length,
    negativeZero: Object.is(negativeZero.x, -0) && Object.is(negativeZero.z, -0) && Object.is(negativeZero.level, -0),
    sameHostFactory: globalThis.__rs2b0t_tileFromPosted === tileFromPosted,
};
"#
        ),
        json!({
            "sameTile": true,
            "sameDerived": true,
            "defaultPlane": true,
            "arrayObject": true,
            "invalidCount": 22,
            "expectedInvalid": 22,
            "negativeZero": true,
            "sameHostFactory": true,
        })
    );
}

#[test]
fn posted_factory_preserves_validation_getter_order_and_exception_identity() {
    assert_eq!(
        probe(
            r#"
const trace = [];
const source = {};
for (const [key, value] of [['x', 1], ['z', 2], ['level', 3]]) {
    Object.defineProperty(source, key, { get() { trace.push(key); return value; } });
}
const tile = tileFromPosted(source);
const sentinel = {};
let thrown = false;
try { tileFromPosted({ get x() { throw sentinel; } }); } catch (error) { thrown = error === sentinel; }
let unsafeReads = 0;
const unsafe = tileFromPosted({ get x() { unsafeReads++; return unsafeReads === 1 ? Number.MAX_SAFE_INTEGER + 1 : 0; } });
globalThis.__world = { trace, thrown, unsafeReads, unsafe: unsafe === null, tile: [tile.x, tile.z, tile.level] };
"#
        ),
        json!({
            "trace": ["x", "x", "x", "z", "z", "z", "level", "level", "level", "level", "x", "z", "level"],
            "thrown": true,
            "unsafeReads": 1,
            "unsafe": true,
            "tile": [1, 2, 3],
        })
    );
}

#[test]
fn complete_area_surface_keeps_edges_mutable_centers_random_tiles_and_overrides() {
    assert_eq!(
        probe(
            r#"
const rectangle = Area.rectangular(new Tile(12, 20, 1), new Tile(10, 18, 3));
const point = Area.rectangular(new Tile(7, 8, 2), new Tile(7, 8, 0));
const center = new Tile(40, 50, 2);
const circle = Area.circular(center, 5);
const edge = circle.contains(new Tile(43, 54, 2));
const outside = !circle.contains(new Tile(43, 55, 2));
const wrongPlane = !circle.contains(new Tile(40, 50, 1));
center.x = 41; center.z = 51; center.level = 3;
const movedCenter = circle.contains(new Tile(41, 51, 3)) && !circle.contains(new Tile(40, 50, 2));
const zero = Area.circular(center, 0).getRandomTile();
let rectangleSamples = true;
let circleSamples = true;
for (let i = 0; i < 128; i++) {
    const r = rectangle.getRandomTile();
    const c = circle.getRandomTile();
    rectangleSamples &&= r instanceof Tile && rectangle.contains(r) && Number.isInteger(r.x) && Number.isInteger(r.z);
    circleSamples &&= c instanceof Tile && circle.contains(c) && Number.isInteger(c.x) && Number.isInteger(c.z);
}
class CustomArea extends Area { contains(tile) { return tile.x === 7; } getRandomTile() { return new Tile(8, 9, 1); } }
const custom = new CustomArea();
globalThis.__world = {
    areaIdentity: rectangle instanceof Area && circle instanceof Area,
    edges: rectangle.contains(new Tile(10, 18, 1)) && rectangle.contains(new Tile(12, 20, 1)),
    outsideRectangle: !rectangle.contains(new Tile(13, 20, 1)),
    firstCornerPlane: !rectangle.contains(new Tile(10, 18, 3)),
    missingPlane: !rectangle.contains({ x: 10, z: 18 }),
    nonNumericPlane: !rectangle.contains({ x: 10, z: 18, level: '1' }),
    degenerate: point.getRandomTile().equals(new Tile(7, 8, 2)),
    edge, outside, wrongPlane, movedCenter, rectangleSamples, circleSamples,
    zeroRadius: zero instanceof Tile && zero.equals(center),
    custom: custom instanceof Area && custom.contains({ x: 7 }) && custom.getRandomTile().equals(new Tile(8, 9, 1)),
};
"#
        ),
        json!({
            "areaIdentity": true,
            "edges": true,
            "outsideRectangle": true,
            "firstCornerPlane": true,
            "missingPlane": true,
            "nonNumericPlane": true,
            "degenerate": true,
            "edge": true,
            "outside": true,
            "wrongPlane": true,
            "movedCenter": true,
            "rectangleSamples": true,
            "circleSamples": true,
            "zeroRadius": true,
            "custom": true,
        })
    );
}

#[test]
fn invalid_and_empty_areas_keep_explicit_failures_and_bounded_fallbacks() {
    assert_eq!(
        probe(
            r#"
const capture = fn => { try { fn(); return null; } catch (error) { return error.message; } };
const invalidRectangle = Area.rectangular(new Tile(NaN, 0), new Tile(1, 1));
const invalidCircle = Area.circular(new Tile(4, 5, 2), NaN);
const fallback = invalidCircle.getRandomTile();
globalThis.__world = {
    uninitializedContains: capture(() => new Area().contains(new Tile(1, 2))),
    uninitializedRandom: capture(() => new Area().getRandomTile()),
    missingCorner: capture(() => Area.rectangular(null, new Tile(1, 2))),
    missingRadius: capture(() => Area.circular(new Tile(1, 2))),
    invalidRectangleEmpty: !invalidRectangle.contains(new Tile(0, 0)),
    invalidRectangleRandom: Number.isNaN(invalidRectangle.getRandomTile().x),
    invalidCircleEmpty: !invalidCircle.contains(new Tile(4, 5, 2)),
    invalidCircleFallback: fallback instanceof Tile && fallback.equals(new Tile(4, 5, 2)),
};
"#
        ),
        json!({
            "uninitializedContains": "invalid area: expected an Area value",
            "uninitializedRandom": "invalid area: expected an Area value",
            "missingCorner": "invalid area: a must be a Tile-like object",
            "missingRadius": "invalid area: radius must be a number",
            "invalidRectangleEmpty": true,
            "invalidRectangleRandom": true,
            "invalidCircleEmpty": true,
            "invalidCircleFallback": true,
        })
    );
}

#[test]
fn every_geometry_method_makes_one_native_call_without_extra_tile_copy_dispatch() {
    assert_eq!(
        probe(
            r#"
const calls = [];
for (const [name, label] of [['__rs2b0t_tile', 'tile'], ['__rs2b0t_area', 'area'], ['__rs2b0t_distance', 'distance']]) {
    const native = globalThis[name];
    globalThis[name] = (...args) => { calls.push([label, label === 'distance' ? null : args[0]]); return native(...args); };
}
const tile = new Tile(1, 2);
Tile.from(tile);
tile.translate(1, 1);
tile.equals(tile);
tile.toString();
tileFromPosted(tile);
tile.distanceTo(tile);
const rectangle = Area.rectangular(tile, tile);
const circle = Area.circular(tile, 0);
rectangle.contains(tile);
rectangle.getRandomTile();
circle.contains(tile);
circle.getRandomTile();
globalThis.__world = calls;
"#
        ),
        json!([
            ["tile", 0],
            ["tile", 1],
            ["tile", 2],
            ["tile", 3],
            ["tile", 4],
            ["distance", null],
            ["area", 0],
            ["area", 1],
            ["area", 2],
            ["area", 3],
            ["area", 2],
            ["area", 3],
        ])
    );
}
