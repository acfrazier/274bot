import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { plugin } from 'bun';
import { requireEnvPath } from './extractors/rs2b0t-literals.ts';

/**
 * The expected results of the gather contract tests, taken from the real upstream code.
 *
 * It imports the pinned rs2b0t data and `ToolAcquire` modules (with the browser-bound modules they import
 * stubbed out, since no case uses them), evaluates every case below against them, and writes the source of each
 * case with its encoded result to `crates/script-js/tests/fixtures/compat_v1_gather_oracle.json`. The Rust
 * contract tests (`crates/script-js/tests/compat_v1_gather.rs`) run the same case source against the native
 * implementation, through the module route and the package-root barrel, and compare the encoded result. The
 * expected values therefore come from executing upstream, never from the generated tables under test.
 *
 *   RS2B0T=<rs2b0t ae56ccc95 checkout> bun tools/game-data/gather-oracle.ts
 *
 * Cases that call upstream's own error paths compare the error class only: this runs on JavaScriptCore, whose
 * messages differ from V8's.
 */
const root = path.resolve(import.meta.dirname, '../..');
const fixture = path.join(root, 'crates/script-js/tests/fixtures/compat_v1_gather_oracle.json');

/** `encode(value, Tile)`: key order, own keys with `undefined` values, class tags, Set versus array, -0. */
export const ENCODE = `const encode = (value, Tile) => {
    const enc = (v) => {
        if (v === undefined) return 'undefined';
        if (v === null) return 'null';
        const t = typeof v;
        if (t === 'string') return JSON.stringify(v);
        if (t === 'number') return Object.is(v, -0) ? '-0' : String(v);
        if (t === 'boolean' || t === 'bigint') return String(v);
        if (t === 'function') return 'function';
        if (v instanceof Set) return 'Set(' + [...v].map(enc).join(',') + ')';
        if (Array.isArray(v)) return '[' + Array.from(v, enc).join(',') + ']';
        const proto = Object.getPrototypeOf(v);
        const tag = v instanceof Tile ? 'Tile' : proto === Object.prototype ? 'Object' : proto === null ? 'Null' : 'Other';
        return tag + '{' + Reflect.ownKeys(v).map((k) => String(k) + ':' + enc(v[k])).join(',') + '}';
    };
    return enc(value);
};
const E = (value) => encode(value, Tile);
const errname = (fn) => { try { fn(); return 'no throw'; } catch (e) { return 'THROW ' + e.constructor.name; } };
`;

type Case = {
    /** The census selector this case belongs to. */
    selector: string;
    /** Names the body reads from the module or barrel. */
    uses: string[];
    /** The census module route of the selector; every name in `uses` must then be served by a module. */
    modules: string[];
    /** The body of `() => string`; `E`, `errname` and `Tile` are in scope. */
    body: string;
};
const cases: Record<string, Case> = {};
function add(id: string, selector: string, uses: string[], modules: string[], body: string) {
    if (cases[id]) throw new Error(`duplicate case ${id}`);
    cases[id] = { selector, uses, modules, body: body.trim() };
}

const LOC_DATA = 'data/gatheringLocations.js';
const MINING = 'data/miningLocations.js';
const WOOD = 'data/woodcuttingLocations.js';
const FISH_LOCS = 'data/fishingLocations.js';
const FISH_METHODS = 'data/fishingMethods.js';
const COOKING = 'data/cookingRanges.js';

// ---------------------------------------------------------------- gathering
for (const name of ['USE_CLOSEST', 'USE_START_POSITION', 'USE_CUSTOM_POSITION', 'AUTO_LEGACY', 'NONE_LEGACY', 'DEFAULT_CAMP_RADIUS', 'DEFAULT_CHASE_RADIUS']) {
    add(`${name}`, name, [name], [LOC_DATA], `return E([${name}, typeof ${name}]);`);
}
add('USE_CUSTOM_POSITION.resolves-no-preset', 'USE_CUSTOM_POSITION', ['USE_CUSTOM_POSITION', 'resolveGatheringLocation', 'MINING_LOCATIONS'], [], `
    return E([resolveGatheringLocation(USE_CUSTOM_POSITION, {x: 3000, z: 3300, level: 0}, MINING_LOCATIONS), resolveGatheringLocation(' use custom position ', {x: 3000, z: 3300, level: 0}, MINING_LOCATIONS)]);`);
add('USE_START_POSITION.resolves-no-preset', 'USE_START_POSITION', ['USE_START_POSITION', 'resolveGatheringLocation', 'MINING_LOCATIONS'], [], `
    return E(resolveGatheringLocation(USE_START_POSITION, {x: 3000, z: 3300, level: 0}, MINING_LOCATIONS));`);
add('NONE_LEGACY.does-not-select-a-camp', 'NONE_LEGACY', ['NONE_LEGACY', 'resolveGatheringLocation', 'MINING_LOCATIONS', 'locationOptions'], [], `
    return E([resolveGatheringLocation(NONE_LEGACY, {x: 3000, z: 3300, level: 0}, MINING_LOCATIONS), locationOptions(MINING_LOCATIONS).slice(0, 6)]);`);
add('AUTO_LEGACY.same-map-square-only', 'AUTO_LEGACY', ['AUTO_LEGACY', 'resolveGatheringLocation', 'MINING_LOCATIONS', 'MAP_SQUARE'], [], `
    const near = resolveGatheringLocation(AUTO_LEGACY, {x: 3290, z: 3370, level: 0}, MINING_LOCATIONS);
    const remote = resolveGatheringLocation(AUTO_LEGACY, {x: 100, z: 100, level: 0}, MINING_LOCATIONS);
    const closest = resolveGatheringLocation('Use Closest', {x: 100, z: 100, level: 0}, MINING_LOCATIONS);
    return E([near && near.name, remote, closest && closest.name, MAP_SQUARE]);`);
add('USE_CLOSEST.searches-every-camp', 'USE_CLOSEST', ['USE_CLOSEST', 'resolveGatheringLocation', 'MINING_LOCATIONS'], [], `
    const far = resolveGatheringLocation(USE_CLOSEST, {x: 100, z: 100, level: 0}, MINING_LOCATIONS);
    const here = resolveGatheringLocation(USE_CLOSEST, {x: 3290, z: 3370, level: 0}, MINING_LOCATIONS);
    return E([far && far.name, here && here.name]);`);
const radiusInputs = [
    'undefined', 'null', 'NaN', 'Infinity', '-Infinity', '0', '1.9', '2', '2.5', '80.7', '-5', "'80'", 'true', '[]', 'new Number(7)',
];
for (const [fn, label] of [['resolveCampRadius', 'camp'], ['resolveChaseRadius', 'chase']] as const) {
    add(`${fn}.input`, fn, [fn], [LOC_DATA], `return E([${radiusInputs.map((value) => `${fn}(${value})`).join(', ')}]);`);
    add(`${fn}.fallback`, fn, [fn], [LOC_DATA], `return E([${['undefined', '10.6', 'null', '3', 'NaN', '7.2', '1', "'9'", '-4', 'Infinity', "'x'", '[]', 'true'].map((value) => `${fn}(undefined, ${value})`).join(', ')}, ${fn}(5, 'x'), ${fn}(NaN, 12.9), ${fn}(null, null), ${fn}(80, undefined), ${fn}.length]);`);
    add(`${fn}.bigint`, fn, [fn], [LOC_DATA], `return E([errname(() => ${fn}(undefined, 5n)), ${fn}(7, 5n)]);`);
    void label;
}

// ------------------------------------------------------------------- tables
const tableCases: [string, string, string[], string][] = [
    ['WOODCUTTING_LOCATIONS', WOOD, [], 'WOODCUTTING_LOCATIONS'],
    ['WOODCUTTING_LOCATION_OPTIONS', WOOD, [], 'WOODCUTTING_LOCATION_OPTIONS'],
    ['MINING_LOCATIONS', MINING, [], 'MINING_LOCATIONS'],
    ['MINING_LOCATION_OPTIONS', MINING, [], 'MINING_LOCATION_OPTIONS'],
    ['MINING_LOCATION_OPTION_LABELS', MINING, [], 'MINING_LOCATION_OPTION_LABELS'],
    ['FISHING_LOCATIONS', FISH_LOCS, [], 'FISHING_LOCATIONS'],
    ['FISHING_LOCATION_OPTIONS', FISH_LOCS, [], 'FISHING_LOCATION_OPTIONS'],
    ['FISHING_METHOD_OPTIONS', FISH_METHODS, [], 'FISHING_METHOD_OPTIONS'],
    ['FISHING_METHODS', FISH_METHODS, [], 'FISHING_METHODS'],
    ['ALL_FISHING_GEAR_NAMES', FISH_METHODS, [], 'ALL_FISHING_GEAR_NAMES'],
    ['WHIRLPOOL_IDS', FISH_METHODS, [], 'WHIRLPOOL_IDS'],
    ['SHILO_WATER_VENDOR', FISH_LOCS, [], 'SHILO_WATER_VENDOR'],
    ['FISHING_SHOP_COSTS', '', [], 'FISHING_SHOP_COSTS'],
    ['GERRANT_ONLY_FISHING', '', [], 'GERRANT_ONLY_FISHING'],
];
for (const [name, module, , expression] of tableCases) {
    add(`${name}.table`, name, [name], module ? [module] : [], `return E(${expression});`);
}
add('MINING_LOCATIONS.shared-bank-tiles', 'MINING_LOCATIONS', ['MINING_LOCATIONS'], [MINING], `
    const row = (name) => MINING_LOCATIONS.find((l) => l.name === name);
    return E([row('Rimmington Mine').bankStand === row('Dwarven Mine').bankStand, row('Al Kharid Mine').bankStand === row('Barbarian Village').bankStand,
        row('Al Kharid Mine').spot === row('Al Kharid Mine').bankStand, Array.isArray(MINING_LOCATIONS), MINING_LOCATIONS.every((l) => l.spot instanceof Tile && l.bankStand instanceof Tile)]);`);
add('MINING_LOCATIONS.order', 'MINING_LOCATIONS', ['MINING_LOCATIONS'], [MINING], `
    const names = MINING_LOCATIONS.map((l) => l.name);
    return E([names, [...names].sort((a, b) => a.localeCompare(b)).join() === names.join(), MINING_LOCATIONS.map((l) => l.recommendedCombat)]);`);
add('WOODCUTTING_LOCATIONS.shared-bank-tiles', 'WOODCUTTING_LOCATIONS', ['WOODCUTTING_LOCATIONS'], [WOOD], `
    const row = (name) => WOODCUTTING_LOCATIONS.find((l) => l.name === name);
    return E([row('Draynor (trees)').bankStand === row('Draynor Oaks').bankStand, row('Seers Oaks').bankStand === row('Draynor Oaks').bankStand, Array.isArray(WOODCUTTING_LOCATIONS)]);`);
add('FISHING_LOCATIONS.shared-objects', 'FISHING_LOCATIONS', ['FISHING_LOCATIONS', 'cookSurfaceForFishCamp', 'SHILO_WATER_VENDOR'], [], `
    const row = (name) => FISHING_LOCATIONS.find((l) => l.name === name);
    const catherby = row('Catherby');
    const draynor = row('Draynor Village');
    return E([catherby.rangeStand === cookSurfaceForFishCamp('Catherby', 'pier').stand, draynor.rangeStand === cookSurfaceForFishCamp('Draynor Village', 'pier').stand,
        draynor.rangeStand === cookSurfaceForFishCamp('Draynor Village', 'bank').stand, row('Draynor Village').bankStand === row('Karamja (Musa Point)').bankStand,
        row('Taverley Dungeon (lava eels)').rangeStand, SHILO_WATER_VENDOR.stand instanceof Tile]);`);
add('FISHING_LOCATIONS.keys', 'FISHING_LOCATIONS', ['FISHING_LOCATIONS'], [FISH_LOCS], `
    return E(FISHING_LOCATIONS.map((l) => [l.name, Reflect.ownKeys(l), 'rangeName' in l, l.obstacles, l.notes]));`);

// ----------------------------------------------------------- location labels
add('miningLocationLabel.rows', 'miningLocationLabel', ['miningLocationLabel', 'MINING_LOCATIONS'], [MINING], `
    return E(MINING_LOCATIONS.map((l) => miningLocationLabel(l)));`);
add('miningLocationLabel.custom', 'miningLocationLabel', ['miningLocationLabel'], [MINING], `
    const reads = [];
    const tracked = { get name() { reads.push('name'); return 'Tracked'; }, get recommendedCombat() { reads.push('rec'); return 12; } };
    const label = miningLocationLabel(tracked);
    return E([label, reads.length, miningLocationLabel({ name: 'A', recommendedCombat: 0 }), miningLocationLabel({ name: 'B', recommendedCombat: '7' }),
        miningLocationLabel({ name: 'C' }), miningLocationLabel({ name: 'D', recommendedCombat: null }), miningLocationLabel({ name: 5 }),
        miningLocationLabel({ name: 5, recommendedCombat: 1.5 }), miningLocationLabel({ name: 'E', recommendedCombat: NaN }),
        errname(() => miningLocationLabel(undefined)), errname(() => miningLocationLabel(null)), miningLocationLabel({ name: undefined, recommendedCombat: 3 })]);`);
add('miningLocationLabel.labels-table-agrees', 'miningLocationLabel', ['miningLocationLabel', 'MINING_LOCATIONS', 'MINING_LOCATION_OPTION_LABELS'], [MINING], `
    return E(MINING_LOCATIONS.filter((l) => l.recommendedCombat != null).every((l) => MINING_LOCATION_OPTION_LABELS[l.name] === miningLocationLabel(l)));`);

// -------------------------------------------------------- location resolvers
const starts = [
    "{x: 3290, z: 3370, level: 0}", "{x: 3181, z: 3371, level: 0}", "{x: 100, z: 100, level: 0}", "{x: 3290, z: 3370, level: 1}",
    "{x: 2703, z: 3330.5, level: 0}", "{x: 3021, z: 9800, level: 0}", "{x: 2850, z: 3430, level: 0}", "{x: 3093, z: 3243, level: 0}",
];
const settings = [
    "'Use Closest'", "'use closest'", "' USE CLOSEST '", "'Auto'", "'auto'", "'\\t Auto \\n'", "'Use Start Position'", "'use start position'", "'Use Custom Position'",
    "'None'", "'none'", "''", "'nowhere'", "'\\u00a0Auto\\u00a0'", "'AUTO\\u2003'",
];
function resolverCases(selector: string, fn: string, table: string, module: string, names: string[]) {
    add(`${selector}.settings-by-start`, selector, [fn, table], [module], `
        const out = [];
        for (const setting of [${settings.join(', ')}]) {
            for (const start of [${starts.join(', ')}]) {
                const found = ${fn}(setting, start);
                out.push(found === null ? null : ${table}.indexOf(found));
            }
        }
        return E(out);`);
    add(`${selector}.named`, selector, [fn, table], [module], `
        const out = [];
        for (const name of ${JSON.stringify(names)}) {
            for (const variant of [name, name.toUpperCase(), name.toLowerCase(), ' ' + name + ' ', name + 'x']) {
                const found = ${fn}(variant, {x: 100, z: 100, level: 0});
                out.push(found === null ? null : ${table}.indexOf(found));
            }
        }
        return E(out);`);
    add(`${selector}.returns-table-rows-by-identity`, selector, [fn, table], [module], `
        const found = ${fn}('Use Closest', {x: 3290, z: 3370, level: 0});
        return E([found === ${table}[${table}.indexOf(found)], ${table}.indexOf(found) >= 0, found instanceof Object]);`);
    add(`${selector}.errors`, selector, [fn], [module], `
        return E([errname(() => ${fn}(undefined, {x: 0, z: 0, level: 0})), errname(() => ${fn}(5, {x: 0, z: 0, level: 0})), errname(() => ${fn}(null, {x: 0, z: 0, level: 0})),
            errname(() => ${fn}('Use Closest', undefined)), errname(() => ${fn}('Use Closest', null)), errname(() => ${fn}('Auto', undefined)),
            errname(() => ${fn}('Use Start Position', undefined)), errname(() => ${fn}('nowhere', undefined)), errname(() => ${fn}({ trim() { return { toLowerCase() { return 'use custom position'; } }; } }, {x: 0, z: 0, level: 0}))]);`);
    add(`${selector}.start-coercion`, selector, [fn, table], [module], `
        const probe = (start) => { const found = ${fn}('Use Closest', start); return found === null ? null : ${table}.indexOf(found); };
        return E([probe({x: '3290', z: '3370', level: 0}), probe({x: 3290.5, z: 3370.25}), probe({x: NaN, z: 3370}), probe({x: 3290, z: 3370, level: undefined}), probe({x: 3290, z: 3370, level: 'a'}),
            ${fn}('Auto', {x: 3290, z: 3370, level: 0}) === ${fn}('auto', {x: 3290, z: 3370, level: 0}), probe({ get x() { return 3000; }, get z() { return 3300; } })]);`);
}
resolverCases('resolveMiningLocation', 'resolveMiningLocation', 'MINING_LOCATIONS', MINING, ['Dwarven Mine', 'Al Kharid Mine', 'Legends Guild Iron (east)', 'Mining Guild', 'Southeast Varrock Mine']);
resolverCases('resolveWoodcuttingLocation', 'resolveWoodcuttingLocation', 'WOODCUTTING_LOCATIONS', WOOD, ['Draynor (trees)', 'Seers Oaks', "Sorcerer's Tower", 'Gnome Stronghold']);
resolverCases('resolveFishingLocation', 'resolveFishingLocation', 'FISHING_LOCATIONS', FISH_LOCS, ['Catherby', 'Fishing Guild', 'Seers (fly fishing)', 'Shilo Village', 'Taverley Dungeon (lava eels)']);
add('resolveMiningLocation.table-order-ties', 'resolveMiningLocation', ['resolveMiningLocation'], [MINING], `
    return E([resolveMiningLocation('Use Closest', {x: 2703, z: 3330.5, level: 0}).name, resolveMiningLocation('Use Closest', {x: 2703, z: 3330.5, level: 1}).name]);`);
add('resolveWoodcuttingLocation.auto-local-and-ties', 'resolveWoodcuttingLocation', ['resolveWoodcuttingLocation'], [WOOD], `
    return E([resolveWoodcuttingLocation('Auto', {x: 3098, z: 3242, level: 0}).name, resolveWoodcuttingLocation('Use Closest', {x: 3099, z: 3243, level: 0}).name,
        resolveWoodcuttingLocation('Auto', {x: 3098, z: 3242, level: 1}), resolveWoodcuttingLocation('Auto', {x: -5, z: -5, level: 0})]);`);

// ---------------------------------------------------------------- GAS_ROCK_IDS
add('GAS_ROCK_IDS.set', 'GAS_ROCK_IDS', ['GAS_ROCK_IDS'], ['data/miningRocks.js'], `
    return E([GAS_ROCK_IDS, GAS_ROCK_IDS.size, GAS_ROCK_IDS instanceof Set, GAS_ROCK_IDS.has(2125), GAS_ROCK_IDS.has(2096), [...GAS_ROCK_IDS].every((id) => Number.isInteger(id))]);`);

// ------------------------------------------------------------- fishing methods
const METHOD = (index: number) => `FISHING_METHODS[${index}]`;
add('resolveFishMethod.names', 'resolveFishMethod', ['resolveFishMethod', 'FISHING_METHODS', 'FISHING_METHOD_OPTIONS'], [FISH_METHODS], `
    return E([...FISHING_METHOD_OPTIONS.map((name) => FISHING_METHODS.indexOf(resolveFishMethod(name))), FISHING_METHODS.indexOf(resolveFishMethod('nowhere')),
        FISHING_METHODS.indexOf(resolveFishMethod('small net — shrimp/anchovy')), FISHING_METHODS.indexOf(resolveFishMethod(undefined)), FISHING_METHODS.indexOf(resolveFishMethod(null)),
        FISHING_METHODS.indexOf(resolveFishMethod(5)), FISHING_METHODS.indexOf(resolveFishMethod({})), FISHING_METHODS.indexOf(resolveFishMethod(new String(FISHING_METHOD_OPTIONS[3]))),
        FISHING_METHODS.indexOf(resolveFishMethod(' ' + FISHING_METHOD_OPTIONS[2]))]);`);
add('resolveFishMethod.identity', 'resolveFishMethod', ['resolveFishMethod', 'FISHING_METHODS'], [FISH_METHODS], `
    return E([resolveFishMethod('Harpoon — sharks') === FISHING_METHODS[7], resolveFishMethod('x') === FISHING_METHODS[0], resolveFishMethod.length]);`);
add('gearKeepNames', 'gearKeepNames', ['gearKeepNames', 'FISHING_METHODS'], [FISH_METHODS], `
    return E([...FISHING_METHODS.map((m) => gearKeepNames(m)), gearKeepNames({ gear: [] }), gearKeepNames({ gear: [{ name: 5 }, { name: 'a' }, { name: undefined }] }),
        errname(() => gearKeepNames({})), errname(() => gearKeepNames(undefined)), errname(() => gearKeepNames({ gear: 'x' })), errname(() => gearKeepNames({ gear: { length: 0 } }))]);`);
add('gearKeepNames.fresh-array', 'gearKeepNames', ['gearKeepNames', 'FISHING_METHODS'], [FISH_METHODS], `
    const a = gearKeepNames(FISHING_METHODS[1]);
    const b = gearKeepNames(FISHING_METHODS[1]);
    return E([a === b, Array.isArray(a), a.length]);`);
add('gearLabel', 'gearLabel', ['gearLabel', 'FISHING_METHODS'], [FISH_METHODS], `
    return E([...FISHING_METHODS.map((m) => gearLabel(m)), gearLabel({ gear: [] }), gearLabel({ gear: [{ name: 'a' }, { name: null }, { name: undefined }, { name: 3 }] }),
        gearLabel({ gear: [{ name: 'only' }] }), errname(() => gearLabel({})), errname(() => gearLabel(null))]);`);
add('hasFishingGear', 'hasFishingGear', ['hasFishingGear', 'FISHING_METHODS'], [FISH_METHODS], `
    const run = (method, held) => { const log = []; const result = hasFishingGear(method, (n) => (log.push(n), held[n] ?? 0)); return [result, log]; };
    return E([run(${METHOD(2)}, {}), run(${METHOD(2)}, {'Fly fishing rod': 1}), run(${METHOD(2)}, {'Fly fishing rod': 1, Feather: 5}), run(${METHOD(0)}, {'Small fishing net': 1}),
        run({ gear: [] }, {}), run({ gear: [{ name: 'x', min: 2, restock: 1 }] }, {x: 1}), run({ gear: [{ name: 'x', min: NaN, restock: 1 }] }, {x: 1}),
        run({ gear: [{ name: 'x', min: 1 }, { name: 'y', min: 1 }] }, {x: 0, y: 1}), run({ gear: [{ name: 'x', min: 1 }, { name: 'y', min: 1 }] }, {x: 1, y: 1})]);`);
add('hasFishingGear.callback-values-and-errors', 'hasFishingGear', ['hasFishingGear'], [FISH_METHODS], `
    const piece = { name: 'x', min: 1, restock: 1 };
    const receivers = [];
    hasFishingGear({ gear: [piece] }, function () { receivers.push(this === undefined || this === globalThis ? 'plain' : 'other'); return 1; });
    return E([hasFishingGear({ gear: [piece] }, () => '2'), hasFishingGear({ gear: [piece] }, () => true), hasFishingGear({ gear: [piece] }, () => undefined), receivers,
        errname(() => hasFishingGear({}, () => 0)), errname(() => hasFishingGear(undefined, () => 0)), errname(() => hasFishingGear({ gear: [piece] }, undefined)),
        errname(() => hasFishingGear({ gear: [piece] }, () => { throw new RangeError('x'); })), hasFishingGear({ gear: [] }, undefined)]);`);
add('missingFishingGear', 'missingFishingGear', ['missingFishingGear', 'FISHING_METHODS'], [FISH_METHODS], `
    const run = (method, held) => { const log = []; const result = missingFishingGear(method, (n) => (log.push(n), held[n] ?? 0)); return [result.map((g) => g.name), log, result.every((g, i) => g === method.gear[method.gear.indexOf(g)])]; };
    return E([run(${METHOD(2)}, {}), run(${METHOD(2)}, {'Fly fishing rod': 1}), run(${METHOD(2)}, {'Fly fishing rod': 1, Feather: 1}), run(${METHOD(8)}, {'Fishing bait': 5}),
        run({ gear: [] }, {}), run({ gear: [{ name: 'x', min: 3, restock: 1 }, { name: 'y', min: 1, restock: 1 }] }, {x: 2, y: 0}), errname(() => missingFishingGear({}, () => 0)),
        errname(() => missingFishingGear({ gear: [{ name: 'x', min: 1 }] }, 5))]);`);
add('fishingRestockPlan', 'fishingRestockPlan', ['fishingRestockPlan', 'FISHING_METHODS'], [FISH_METHODS], `
    const run = (method, inv, bank) => { const log = []; const plan = fishingRestockPlan(method, (n) => (log.push('inv:' + n), inv[n] ?? 0), (n) => (log.push('bank:' + n), bank[n] ?? 0)); return [plan, log]; };
    return E([run(${METHOD(2)}, {}, {}), run(${METHOD(2)}, {Feather: 30}, {'Fly fishing rod': 1, Feather: 50}), run(${METHOD(2)}, {Feather: 30}, {'Fly fishing rod': 1, Feather: 500}),
        run(${METHOD(2)}, {'Fly fishing rod': 1, Feather: 100}, {Feather: 500}), run(${METHOD(1)}, {'Fishing rod': 0, 'Fishing bait': 99}, {'Fishing rod': 1, 'Fishing bait': 7}),
        run({ gear: [{ name: 'x', min: 1, restock: NaN }] }, {}, {x: 5}), run({ gear: [{ name: 'x', min: 1, restock: 4 }] }, {x: 1}, {x: 2.5}), run({ gear: [] }, {}, {}),
        run({ gear: [{ name: 'x', min: 1, restock: 4 }] }, {x: 1}, {x: -1}), errname(() => fishingRestockPlan({}, () => 0, () => 0)), errname(() => fishingRestockPlan({ gear: [{ name: 'x', restock: 2 }] }, undefined, () => 0)),
        errname(() => fishingRestockPlan({ gear: [{ name: 'x', restock: 2 }] }, () => 0, undefined))]);`);
add('spotMatchesMethod', 'spotMatchesMethod', ['spotMatchesMethod', 'FISHING_METHODS'], [FISH_METHODS], `
    const m = (op, pair) => ({ op, pair });
    return E([spotMatchesMethod(['Net', 'Bait'], m('Net', 'Bait')), spotMatchesMethod(['NET', 'bait'], m('net', 'BAIT')), spotMatchesMethod(['Net'], m('Net', 'Bait')), spotMatchesMethod(['Bait'], m('Net', 'Bait')),
        spotMatchesMethod(['Net'], m('Net', '')), spotMatchesMethod(['Net'], m('Net', '   ')), spotMatchesMethod(['Net'], m('Net', 'net')), spotMatchesMethod(['Net'], m('Net', ' NET ')), spotMatchesMethod(['Net', 'Lure'], m('Net', ' lure ')),
        spotMatchesMethod([], m('Net', '')), spotMatchesMethod(['Cage', 'Harpoon'], m('Harpoon', 'Cage')), spotMatchesMethod(['Harpoon'], m('Harpoon', 'Cage')), spotMatchesMethod(['Cage'], m('Harpoon', 'Cage')),
        ...FISHING_METHODS.map((method) => spotMatchesMethod([method.op, method.pair || method.op], method)), errname(() => spotMatchesMethod([5], m('a', ''))), errname(() => spotMatchesMethod(undefined, m('a', ''))),
        errname(() => spotMatchesMethod(['a'], m(undefined, ''))), errname(() => spotMatchesMethod(['a'], m('a', undefined))), errname(() => spotMatchesMethod(['a'], undefined)),
        spotMatchesMethod(['a'], m('b', undefined)), spotMatchesMethod(['A', 'İ'], m('a', 'i̇'))]);`);

// ------------------------------------------------------------- tool acquire (fishing)
const METHOD_GEAR = 'FISHING_METHODS';
add('fishingShopCost', 'fishingShopCost', ['fishingShopCost', 'FISHING_SHOP_COSTS'], [], `
    return E([...Object.keys(FISHING_SHOP_COSTS).flatMap((name) => [fishingShopCost(name), fishingShopCost(name.toUpperCase()), fishingShopCost(name.toLowerCase())]),
        fishingShopCost('Oily fishing rod'), fishingShopCost(''), fishingShopCost('Harpoon '), errname(() => fishingShopCost(undefined)), errname(() => fishingShopCost(5)),
        errname(() => fishingShopCost(null)), fishingShopCost({ toLowerCase() { return 'harpoon'; } })]);`);
add('fishingVendorFor', 'fishingVendorFor', ['fishingVendorFor'], [], `
    const GERRANT_VENDOR = fishingVendorFor('feather');
    const HARRY_VENDOR = fishingVendorFor('Fishing bait');
    const keeper = (v) => (v === GERRANT_VENDOR ? 'G' : v === HARRY_VENDOR ? 'H' : '?');
    const names = ['Feather', 'feather', 'FLY FISHING ROD', 'Fly fishing rod', 'Fishing bait', 'Big fishing net', 'Harpoon', 'Lobster pot', 'Small fishing net', 'Fishing rod', 'Oily fishing rod', 'anything'];
    const nears = [undefined, null, 0, false, '', {x: 2833, z: 3443}, {x: 3013, z: 3224}, {x: 2923, z: 3333.5}, {x: 2923, z: 3333}, {x: 2833, z: 3224}, {x: '2833', z: '3443'}, {x: NaN, z: 0}, {}, {x: 2833, z: 3443, level: 1}, 5, true];
    return E(nears.map((near) => names.map((name) => keeper(fishingVendorFor(name, near)))));`);
add('fishingVendorFor.shape', 'fishingVendorFor', ['fishingVendorFor'], [], `
    const GERRANT_VENDOR = fishingVendorFor('feather');
    const HARRY_VENDOR = fishingVendorFor('Fishing bait');
    return E([fishingVendorFor('feather') === GERRANT_VENDOR, fishingVendorFor('Fishing bait') === HARRY_VENDOR, fishingVendorFor('Harpoon') === GERRANT_VENDOR, GERRANT_VENDOR, HARRY_VENDOR,
        GERRANT_VENDOR.stand instanceof Tile, GERRANT_VENDOR.stand === GERRANT_VENDOR.bankStand, errname(() => fishingVendorFor(undefined)), errname(() => fishingVendorFor(5, {x: 1, z: 1})),
        fishingVendorFor('Lobster pot', { x: 2833, get z() { return 3443; } }) === HARRY_VENDOR, fishingVendorFor.length]);`);
add('isFishingBaitPiece', 'isFishingBaitPiece', ['isFishingBaitPiece', 'FISHING_METHODS'], [], `
    const p = (name, restock) => ({ name, restock });
    return E([...FISHING_METHODS.flatMap((m) => m.gear.map((g) => isFishingBaitPiece(g))), isFishingBaitPiece(p('Fishing bait', 2)), isFishingBaitPiece(p('FEATHER', 100)), isFishingBaitPiece(p('Feather', 1)),
        isFishingBaitPiece(p('Feather', 0)), isFishingBaitPiece(p('Feather', -1)), isFishingBaitPiece(p('Feather', NaN)), isFishingBaitPiece(p('Feather', undefined)), isFishingBaitPiece(p('Feather', '5')),
        isFishingBaitPiece(p('Harpoon', 100)), isFishingBaitPiece(p('Fishing bait ', 100)), errname(() => isFishingBaitPiece(p(undefined, 100))), isFishingBaitPiece(p(undefined, 1)),
        errname(() => isFishingBaitPiece(p(5, 100))), errname(() => isFishingBaitPiece(undefined))]);`);
add('withBaitTarget', 'withBaitTarget', ['withBaitTarget', 'FISHING_METHODS'], [], `
    const method = FISHING_METHODS[2];
    const results = [250.7, 1000, 1, 0.5, 0, -3, NaN, Infinity, undefined, null, '40', 'x', true].map((qty) => withBaitTarget(method, qty));
    const copy = withBaitTarget({ gear: [{ name: 'Feather', min: 1, restock: 100, extra: 'x' }, { name: 'Harpoon', min: 1, restock: 100 }] }, 50);
    const shared = { name: 'Fishing rod', min: 1, restock: 1 };
    const kept = withBaitTarget({ gear: [shared, { name: 'Feather', min: 1, restock: 100 }] }, 9);
    return E([results, copy, kept.gear[0] === shared, kept.gear[1] !== undefined, method.gear[1].restock, results[0].gear[0] === method.gear[0], results[0].gear[1] === method.gear[1],
        Reflect.ownKeys(results[0]), errname(() => withBaitTarget({}, 5)), errname(() => withBaitTarget(undefined, 5)), errname(() => withBaitTarget({ gear: 'x' }, 5)), errname(() => withBaitTarget(method, 5n)),
        withBaitTarget({ gear: [] }, 5)]);`);
const WORLD = `const world = (held, bank, inv) => { const log = []; const w = {
        heldCount(n) { log.push('held:' + n); return held[n] ?? 0; }, invCount(n) { log.push('inv:' + n); return (inv ?? held)[n] ?? 0; }, bankCount(n) { log.push('bank:' + n); return bank[n] ?? 0; } }; return [w, log]; };`;
const planUses = (fn: string) => [fn, 'FISHING_METHODS'];
const planScenarios = `
    const run = (method, held, bank, opts, inv) => { const [w, log] = world(held, bank, inv); const plans = FN(method, w, opts); return [plans, log]; };
    const fly = FISHING_METHODS[2], rod = FISHING_METHODS[1], net = FISHING_METHODS[0], lobster = FISHING_METHODS[5], harpoon = FISHING_METHODS[6], oily = FISHING_METHODS[8];
    const rich = { Coins: 10000 };
    const scenarios = [
        run(fly, rich, {}, undefined), run(fly, rich, {}, {}), run(fly, rich, {}, { baitQty: 40 }), run(fly, { Coins: 100 }, {}, undefined), run(fly, {}, { Coins: 100 }, undefined), run(fly, { Coins: 4 }, {}, undefined),
        run(fly, { Coins: 5 }, {}, undefined), run(fly, { Coins: 5, 'Fly fishing rod': 1 }, {}, undefined), run(fly, { Coins: 100, 'Fly fishing rod': 1, Feather: 4 }, { Feather: 10 }, { baitQty: 20 }),
        run(fly, { 'Fly fishing rod': 1 }, { Feather: 5000 }, undefined), run(rod, rich, {}, { baitQty: 0 }), run(rod, rich, {}, { baitQty: 1 }), run(rod, rich, {}, { baitQty: 2 }), run(rod, rich, {}, { baitQty: -7 }),
        run(rod, rich, {}, { baitQty: NaN }), run(rod, rich, {}, { baitQty: '25' }), run(rod, rich, {}, { baitQty: null }), run(rod, rich, {}, { baitQty: 7.9 }), run(net, rich, {}, undefined), run(net, { Coins: 5 }, {}, undefined),
        run(net, { Coins: 4 }, {}, undefined), run(lobster, rich, {}, { near: { x: 2833, z: 3443 } }), run(lobster, rich, {}, { near: { x: 3013, z: 3224 } }), run(lobster, rich, {}, { near: null }),
        run(rod, rich, {}, { near: { x: 2833, z: 3443 }, baitQty: 30 }), run(rod, rich, {}, { near: { x: 3013, z: 3224 }, baitQty: 30 }), run(harpoon, rich, {}, undefined), run(oily, rich, {}, { baitQty: 12 }),
        run(oily, { Coins: 3 }, {}, { baitQty: 12 }), run({ gear: [] }, rich, {}, undefined), run({ gear: [{ name: 'Harpoon', min: 2, restock: 1 }] }, rich, {}, undefined),
        run({ gear: [{ name: 'Harpoon', min: 1, restock: 1 }, { name: 'Harpoon', min: 1, restock: 1 }] }, { Coins: 6 }, {}, undefined), run({ gear: [{ name: 'Feather', min: 5, restock: 100 }] }, rich, {}, { baitQty: 3 }),
        run({ gear: [{ name: 'Feather', min: 1, restock: 100 }] }, { Coins: 3 }, {}, { baitQty: 1000 }), run({ gear: [{ name: 'Feather', min: 1, restock: 100 }] }, { Coins: 1 }, {}, { baitQty: 1000 }),
        run({ gear: [{ name: 'Fishing bait', min: 1, restock: 1 }] }, rich, {}, { baitQty: 60 }), run({ gear: [{ name: 'Fishing bait', min: 1, restock: 100 }, { name: 'Harpoon', min: 1, restock: 1 }] }, rich, {}, { baitQty: 1 }),
        run(fly, { Coins: 10 }, { Coins: 5 }, undefined, { Coins: 3 }), run(fly, { 'Fly fishing rod': 0 }, { Coins: 100 }, undefined, { Coins: 100 }),
    ];`;
function planFn(fn: string) {
    return planScenarios.replace(/FN\(/g, `${fn}(`);
}
add('planFishingGearBuys.scenarios', 'planFishingGearBuys', planUses('planFishingGearBuys'), [], `
    ${WORLD}
    ${planFn('planFishingGearBuys')}
    return E(scenarios);`);
add('planFishingGearBuys.shape-and-identity', 'planFishingGearBuys', [...planUses('planFishingGearBuys'), 'fishingVendorFor'], [], `
    ${WORLD}
    const GERRANT_VENDOR = fishingVendorFor('feather');
    const [w] = world({ Coins: 100 }, {});
    const plans = planFishingGearBuys(FISHING_METHODS[2], w);
    return E([plans.length, plans[0].vendor === GERRANT_VENDOR, plans[1].vendor === GERRANT_VENDOR, Reflect.ownKeys(plans[0]), plans[0].equip, plans[0].kind, plans[0].reason, plans[1].reason,
        planFishingGearBuys.length, errname(() => planFishingGearBuys({}, w)), errname(() => planFishingGearBuys(FISHING_METHODS[2], undefined)), errname(() => planFishingGearBuys(FISHING_METHODS[2], w, null)),
        errname(() => planFishingGearBuys(FISHING_METHODS[2], { heldCount: () => 0, invCount: () => 0 })), errname(() => planFishingGearBuys({ gear: [] }, {})),
        errname(() => planFishingGearBuys({ gear: [{ name: undefined, min: 1, restock: 1 }] }, { heldCount: () => 0, invCount: () => 100, bankCount: () => 0 })),
        errname(() => planFishingGearBuys(FISHING_METHODS[2], { heldCount: () => { throw new RangeError('x'); }, invCount: () => 1, bankCount: () => 1 }))]);`);
add('planFishingGearBuys.near-reads', 'planFishingGearBuys', planUses('planFishingGearBuys'), [], `
    ${WORLD}
    const reads = [];
    const near = { get x() { reads.push('x'); return 2833; }, get z() { reads.push('z'); return 3443; } };
    const [w] = world({ Coins: 1000 }, {});
    const plans = planFishingGearBuys(FISHING_METHODS[5], w, { near });
    return E([plans.length, reads.join('')]);`);
add('planFishingGearAcquire.scenarios', 'planFishingGearAcquire', planUses('planFishingGearAcquire'), [], `
    ${WORLD}
    ${planFn('planFishingGearAcquire')}
    return E([scenarios, planFishingGearAcquire.length]);`);
add('planFishingGearAcquire.null-and-first', 'planFishingGearAcquire', planUses('planFishingGearAcquire'), [], `
    ${WORLD}
    const [w] = world({ Coins: 100, 'Fly fishing rod': 1 }, { Feather: 5000 });
    const [broke] = world({}, {});
    const [rich] = world({ Coins: 1000 }, {});
    return E([planFishingGearAcquire(FISHING_METHODS[2], w), planFishingGearAcquire(FISHING_METHODS[2], broke), planFishingGearAcquire(FISHING_METHODS[2], rich).name, planFishingGearAcquire({ gear: [] }, rich)]);`);
add('fishingGearShopCart.scenarios', 'fishingGearShopCart', ['fishingGearShopCart', 'FISHING_METHODS'], [], `
    ${WORLD}
    ${planFn('fishingGearShopCart')}
    const mixed = { gear: [{ name: 'Lobster pot', min: 1, restock: 1 }, { name: 'Feather', min: 1, restock: 100 }, { name: 'Big fishing net', min: 1, restock: 1 }, { name: 'Harpoon', min: 1, restock: 1 }] };
    const [w] = world({ Coins: 100000 }, {});
    return E([scenarios, ['H', 'G'].map((c) => fishingGearShopCart(mixed, w, { near: c === 'H' ? { x: 2833, z: 3443 } : { x: 3013, z: 3224 } }).map((p) => p.name)),
        fishingGearShopCart(mixed, w).map((p) => p.name), fishingGearShopCart({ gear: [] }, w), fishingGearShopCart.length]);`);
add('buyPlansCost', 'buyPlansCost', ['buyPlansCost'], [], `
    const sparse = [{ cost: 1 }, , { cost: 2 }];
    return E([buyPlansCost([]), buyPlansCost([{ cost: 5 }, { cost: 20 }, { cost: 3 }]), buyPlansCost([{ cost: '1' }, { cost: 2 }]), buyPlansCost([{ cost: 1.5 }, { cost: 2.25 }]), buyPlansCost([{ cost: NaN }, { cost: 1 }]),
        buyPlansCost([{}]), buyPlansCost(sparse), buyPlansCost([{ cost: 1n }].slice(1)), errname(() => buyPlansCost([{ cost: 1n }])), errname(() => buyPlansCost(undefined)), errname(() => buyPlansCost([undefined])),
        errname(() => buyPlansCost([null])), errname(() => buyPlansCost({ length: 2, 0: { cost: 4 }, 1: { cost: 6 } })), buyPlansCost([{ get cost() { return 7; } }]), buyPlansCost.length]);`);
add('shopableMissingFishingGear', 'shopableMissingFishingGear', ['shopableMissingFishingGear', 'FISHING_METHODS'], [], `
    const run = (gear, held) => { const log = []; const names = shopableMissingFishingGear(gear, (n) => (log.push(n), held[n] ?? 0)); return [names, log]; };
    return E([run(FISHING_METHODS[2].gear, {}), run(FISHING_METHODS[2].gear, {'Fly fishing rod': 1}), run(FISHING_METHODS[8].gear, {}), run(FISHING_METHODS[8].gear, {'Oily fishing rod': 1}),
        run([], {}), run([{ name: 'Harpoon', min: 2, restock: 1 }], {Harpoon: 1}), run([{ name: 'Harpoon', min: 1, restock: 1 }], {Harpoon: 1}), run([{ name: 'HARPOON', min: 1, restock: 1 }], {}),
        errname(() => shopableMissingFishingGear([{ name: undefined, min: 1 }], () => 0)), shopableMissingFishingGear([{ name: undefined, min: 1 }], () => 5), errname(() => shopableMissingFishingGear(undefined, () => 0)),
        errname(() => shopableMissingFishingGear([{ name: 'Harpoon', min: 1 }], undefined)), errname(() => shopableMissingFishingGear({}, () => 0)), shopableMissingFishingGear.length]);`);

// ------------------------------------------------------------------- cooking
const camps = ['Catherby', 'Seers (fly fishing)', 'Fishing Guild', 'Barbarian Village', 'Draynor Village', 'Nowhere', '', 'constructor', '__proto__', 'toString', 'hasOwnProperty'];
add('cookSurfaceForFishCamp.plans', 'cookSurfaceForFishCamp', ['cookSurfaceForFishCamp'], [COOKING], `
    return E([${JSON.stringify(camps)}.map((camp) => [cookSurfaceForFishCamp(camp), cookSurfaceForFishCamp(camp, 'pier'), cookSurfaceForFishCamp(camp, 'bank'), cookSurfaceForFishCamp(camp, 'other'), cookSurfaceForFishCamp(camp, undefined), cookSurfaceForFishCamp(camp, null)])]);`);
add('cookSurfaceForFishCamp.identity', 'cookSurfaceForFishCamp', ['cookSurfaceForFishCamp'], [COOKING], `
    const same = (a, b) => cookSurfaceForFishCamp(a[0], a[1]) === cookSurfaceForFishCamp(b[0], b[1]);
    return E([same(['Catherby', 'pier'], ['Catherby', 'bank']), same(['Catherby', 'pier'], ['Catherby']), same(['Draynor Village', 'pier'], ['Draynor Village', 'bank']), same(['Seers (fly fishing)', 'pier'], ['Seers (fly fishing)', 'bank']),
        same(['Fishing Guild', 'pier'], ['Fishing Guild', 'bank']), same(['Barbarian Village', 'pier'], ['Barbarian Village', 'bank']), cookSurfaceForFishCamp('Catherby') === cookSurfaceForFishCamp('Catherby'), cookSurfaceForFishCamp('Nowhere', 'bank'),
        cookSurfaceForFishCamp.length, errname(() => cookSurfaceForFishCamp(Symbol('x'))), cookSurfaceForFishCamp(undefined), cookSurfaceForFishCamp(5), cookSurfaceForFishCamp(['Catherby'])]);`);
add('cookSurfaceForFishCamp.tiles', 'cookSurfaceForFishCamp', ['cookSurfaceForFishCamp'], [COOKING], `
    const s = cookSurfaceForFishCamp('Seers (fly fishing)', 'pier');
    return E([s.stand instanceof Tile, s.approach instanceof Tile, s.loc instanceof Tile, Reflect.ownKeys(s), Object.getPrototypeOf(s) === Object.prototype]);`);
const spots = ['{x: 3086, z: 3231}', '{x: 3086, z: 3231, level: 0}', '{x: 3086, z: 3231, level: 1}', '{x: 100, z: 100, level: 0}', '{x: 2809, z: 3441, level: 0}', '{x: 3185, z: 3440, level: 0}', '{x: 2845, z: 3431, level: 0}', '{x: 3200, z: 3200, level: 0}'];
add('resolveFishCampCookSurface.curated', 'resolveFishCampCookSurface', ['resolveFishCampCookSurface', 'cookSurfaceForFishCamp'], [COOKING], `
    const out = [];
    for (const camp of ${JSON.stringify(camps.concat([''])).replace(']', ', null, undefined, 0, false]')}) {
        for (const role of ['pier', 'bank', undefined, 'other']) {
            const r = resolveFishCampCookSurface(camp, {x: 100, z: 100, level: 0}, 0, role);
            out.push([camp, role, r === cookSurfaceForFishCamp(camp, role), r === null ? null : r.label]);
        }
    }
    return E(out);`);
add('resolveFishCampCookSurface.nearest', 'resolveFishCampCookSurface', ['resolveFishCampCookSurface'], [COOKING], `
    const out = [];
    for (const camp of [null, undefined, '', 'Nowhere']) {
        for (const spot of [${spots.join(', ')}]) {
            for (const max of [undefined, 0, 1, 5, 20, 64, 200, NaN, Infinity, -1, '30', null]) {
                out.push(resolveFishCampCookSurface(camp, spot, max));
            }
        }
    }
    return E(out);`);
add('resolveFishCampCookSurface.fresh-and-defaults', 'resolveFishCampCookSurface', ['resolveFishCampCookSurface'], [COOKING], `
    const a = resolveFishCampCookSurface(null, {x: 3019, z: 3240, level: 0});
    const b = resolveFishCampCookSurface(null, {x: 3019, z: 3240, level: 0});
    const c = resolveFishCampCookSurface('Catherby', {x: 3019, z: 3240, level: 0});
    const d = resolveFishCampCookSurface('Catherby', {x: 3019, z: 3240, level: 0}, 64, 'bank');
    return E([a === b, a && a.stand instanceof Tile, a && a.loc instanceof Tile, a && Reflect.ownKeys(a), c === d, c && c.label, resolveFishCampCookSurface.length,
        errname(() => resolveFishCampCookSurface(null, undefined)), errname(() => resolveFishCampCookSurface(null, null)), errname(() => resolveFishCampCookSurface('Catherby', undefined)),
        resolveFishCampCookSurface('Nowhere', {x: 3019, z: 3240, level: 0}, 64, 'bank') === null]);`);
add('resolveFishCampCookSurface.origin-coercion', 'resolveFishCampCookSurface', ['resolveFishCampCookSurface'], [COOKING], `
    const near = (spot, max) => { const r = resolveFishCampCookSurface(null, spot, max); return r && [r.loc.x, r.loc.z, r.notes]; };
    return E([near({x: '3019', z: '3240', level: 0}), near({x: 3019.5, z: 3240.5, level: 0}), near({x: 3019, z: 3240, level: undefined}), near({x: 3019, z: 3240, level: null}), near({x: 3019, z: 3240, level: '0'}),
        near({x: NaN, z: 3240, level: 0}), near({x: 3019, z: 3240, level: 0}, '64'), near({ get x() { return 3019; }, get z() { return 3240; }, get level() { return 0; } })]);`);

/** The module each upstream name lives in. A name absent here is served by the package-root barrel only. */
const MODULE_OF: Record<string, string> = {
    ...Object.fromEntries(['DEFAULT_BOOTH_NAME', 'DEFAULT_BOOTH_OP', 'MAP_SQUARE', 'sameMapSquare', 'locationOptions', 'boothFields', 'resolveGatheringLocation', 'USE_CLOSEST', 'USE_START_POSITION',
        'USE_CUSTOM_POSITION', 'AUTO_LEGACY', 'NONE_LEGACY', 'DEFAULT_CAMP_RADIUS', 'DEFAULT_CHASE_RADIUS', 'resolveCampRadius', 'resolveChaseRadius'].map((name) => [name, LOC_DATA])),
    ...Object.fromEntries(['MINING_LOCATIONS', 'MINING_LOCATION_OPTIONS', 'MINING_LOCATION_OPTION_LABELS', 'miningLocationLabel', 'resolveMiningLocation'].map((name) => [name, MINING])),
    ...Object.fromEntries(['WOODCUTTING_LOCATIONS', 'WOODCUTTING_LOCATION_OPTIONS', 'resolveWoodcuttingLocation'].map((name) => [name, WOOD])),
    ...Object.fromEntries(['SHILO_WATER_VENDOR', 'FISHING_LOCATIONS', 'FISHING_LOCATION_OPTIONS', 'resolveFishingLocation'].map((name) => [name, FISH_LOCS])),
    ...Object.fromEntries(['WHIRLPOOL_IDS', 'FISHING_METHODS', 'FISHING_METHOD_OPTIONS', 'ALL_FISHING_GEAR_NAMES', 'resolveFishMethod', 'gearKeepNames', 'hasFishingGear', 'missingFishingGear', 'gearLabel',
        'fishingRestockPlan', 'spotMatchesMethod'].map((name) => [name, FISH_METHODS])),
    ...Object.fromEntries(['cookSurfaceForFishCamp', 'resolveFishCampCookSurface'].map((name) => [name, COOKING])),
    GAS_ROCK_IDS: 'data/miningRocks.js',
};

// ------------------------------------------------------------------ runner
async function main() {
    const rs2b0t = requireEnvPath('RS2B0T', 'the rs2b0t checkout the cases run against');
    plugin({
        name: 'stub-browser-modules',
        setup(build) {
            build.onResolve({ filter: /\/(Execution|Game|Traversal|Locs|ClientAdapter)\.(js|ts)$/ }, (args) => ({ path: args.path, namespace: 'stub' }));
            build.onLoad({ filter: /.*/, namespace: 'stub' }, () => ({ contents: 'export const Execution = {}, Game = {}, Traversal = {}, Locs = {}, reader = {}; export default {};', loader: 'js' }));
        },
    });
    const load = (relative: string) => import(pathToFileURL(path.join(rs2b0t, 'src/bot', relative)).href);
    const modules = await Promise.all([
        load('geometry/Tile.ts'), load('data/gatheringLocations.ts'), load('data/miningLocations.ts'), load('data/woodcuttingLocations.ts'), load('data/fishingLocations.ts'),
        load('data/fishingMethods.ts'), load('data/cookingRanges.ts'), load('api/acquisition/ToolAcquire.ts'), load('data/miningRocks.ts'),
    ]);
    const Tile = modules[0].default;
    const scope: Record<string, unknown> = {};
    for (const module of modules.slice(1)) Object.assign(scope, module);
    const names = Object.keys(scope);
    const out: Record<string, { selector: string; uses: string[]; body: string; expected: string }> = {};
    for (const [id, test] of Object.entries(cases)) {
        for (const use of test.uses) if (!(use in scope)) throw new Error(`${id}: upstream has no export ${use}`);
        if (test.modules.length) for (const use of test.uses) if (!MODULE_OF[use]) throw new Error(`${id}: ${use} has no module route but the selector names ${test.modules[0]}`);
        const factory = new Function(...test.uses, 'Tile', `${ENCODE}\nreturn () => { ${test.body} };`);
        let expected: string;
        try {
            expected = factory(...test.uses.map((use) => scope[use]), Tile)();
        } catch (error) {
            expected = `THROW ${(error as Error).constructor.name}`;
        }
        if (typeof expected !== 'string') throw new Error(`${id}: a case body must return a string`);
        out[id] = { selector: test.selector, uses: test.uses, body: test.body, expected };
    }
    void names;
    const text = `${JSON.stringify({ upstream: 'rs2b0t ae56ccc95ba5681b12f1f66ec2adea14427fe497', encode: ENCODE, moduleOf: MODULE_OF, cases: out }, null, 1)}\n`;
    fs.mkdirSync(path.dirname(fixture), { recursive: true });
    fs.writeFileSync(fixture, text);
    console.log(JSON.stringify({ fixture: path.relative(root, fixture), cases: Object.keys(out).length }));
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main().catch((error) => { console.error(error); process.exitCode = 1; });
