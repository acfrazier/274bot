import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { extractCampTables, extractCookPlans, extractFishingFacts, gatherCatalogFiles, gatherCatalogJson } from './extractors/gather-catalog.ts';
import { LiteralMapped, LiteralSet, pinnedLiterals, requireEnvPath } from './extractors/rs2b0t-literals.ts';

/** The reader loads TypeScript from an engine checkout; the fixtures below need no rs2b0t source. */
const engine = requireEnvPath('GAME_DATA_289_ENGINE', 'the 289 engine root');
const repoRoot = path.resolve(import.meta.dirname, '../..');
const read = (source: string) => pinnedLiterals(engine, 'fixture.ts', source, 'fixture');

// Helpers with default parameters, shared constants, property reads, spreads and `undefined`.
{
    const literal = await read(`
const BANK = { a: new Tile(1, 2, 0), b: new Tile(3, 4, 1) } as const;
function camp(name: string, spot: Tile, bankStand: Tile, resources: readonly string[], notes?: string, verified = true) {
    return { name, spot, bankStand, verified, resources, notes };
}
const bait = (name: string, restock = 100) => ({ name, min: 1, restock });
export const CAMPS = [
    camp('A', new Tile(5, 6, 0), BANK.a, ['x']),
    camp('B', new Tile(7, 8, 0), BANK.a, ['y', 'z'], 'note', false),
];
export const GEAR = [bait('Bait'), bait('Feather', 7)];
export const ALL = [...CAMPS, ...GEAR];
export const SET = new Set([1, 2, 3]);
export const NEGATIVE = [-1, true, false, null];
`);
    const camps = literal('CAMPS') as { name: string; verified: boolean; notes?: string; bankStand: object }[];
    assert.deepEqual(camps[0], { name: 'A', spot: { x: 5, z: 6, level: 0 }, bankStand: { x: 1, z: 2, level: 0 }, verified: true, resources: ['x'], notes: undefined });
    assert.deepEqual(Object.keys(camps[0]), ['name', 'spot', 'bankStand', 'verified', 'resources', 'notes'], 'an undefined value keeps its key, in source order');
    assert.equal(camps[1].verified, false);
    assert.equal(camps[1].notes, 'note');
    assert.equal(camps[0].bankStand, camps[1].bankStand, 'a shared constant tile is one object');
    assert.deepEqual(literal('GEAR'), [{ name: 'Bait', min: 1, restock: 100 }, { name: 'Feather', min: 1, restock: 7 }]);
    assert.equal((literal('ALL') as unknown[]).length, 4);
    assert.ok(literal('SET') instanceof LiteralSet);
    assert.deepEqual((literal('SET') as LiteralSet).values, [1, 2, 3]);
    assert.deepEqual(literal('NEGATIVE'), [-1, true, false, null]);
}

// The two recognised library calls, by exact source text only.
{
    const literal = await read(`
function row(name: string) { return { name }; }
export const SORTED = [row('b'), row('Ab'), row('a'), row('B'), row('(x)'), row('Southeast'), row('South-east')].sort((a, b) => a.name.localeCompare(b.name));
export const MAPPED = ([row('a')] as Row[]).map(derive);
function derive(r: Row) { return r; }
export const OTHER_SORT = [row('b'), row('a')].sort((a, b) => b.name.localeCompare(a.name));
export const OTHER_MAP = [row('a')].map(row);
`);
    const names = (literal('SORTED') as { name: string }[]).map((entry) => entry.name);
    assert.deepEqual(names, ['(x)', 'a', 'Ab', 'b', 'B', 'South-east', 'Southeast'].sort((a, b) => a.localeCompare(b)), 'the sort is the en-US localeCompare order');
    assert.deepEqual(names, [...names].sort((a, b) => a.localeCompare(b)));
    const mapped = literal('MAPPED');
    assert.ok(mapped instanceof LiteralMapped);
    assert.equal(mapped.helper, 'derive');
    assert.deepEqual(mapped.rows, [{ name: 'a' }]);
    assert.throws(() => literal('OTHER_SORT'), /unsupported call/, 'a different comparator is not recognised');
    // `row` is a local helper, so this `.map(row)` is a recognised shape; an unknown helper is not.
    assert.ok(literal('OTHER_MAP') instanceof LiteralMapped);
    const unknown = await read('export const X = [1].map(mystery);\nexport const Y = mystery(1);\nexport const Z = fetch;\nexport const W = [1].filter((v) => v);\n');
    assert.throws(() => unknown('X'), /unsupported call/);
    assert.throws(() => unknown('Y'), /unknown function mystery/);
    assert.throws(() => unknown('Z'), /unknown identifier fetch/);
    assert.throws(() => unknown('W'), /unsupported call/);
    assert.throws(() => unknown('MISSING'), /no constant MISSING/);
}

// The junk helper is read by its exact text: a changed body refuses.
{
    const body = (code: string) => `function junk(quest: string, items: readonly (readonly [number, string])[]) {\n    ${code}\n}\nexport const J = [...junk('Q', [[1, 'a'], [2, 'b']]), ...junk('R', [[3, 'c']])];\n`;
    assert.deepEqual((await read(body('return items.map(([id, name]) => ({ id, name, quest }));')))('J'), [
        { id: 1, name: 'a', quest: 'Q' }, { id: 2, name: 'b', quest: 'Q' }, { id: 3, name: 'c', quest: 'R' },
    ]);
    await assert.rejects(async () => (await read(body('return items.map(([id, name]) => ({ id: id + 1, name, quest }));')))('J'), /helper junk changed/);
}

// Camp tables: key order, undefined-valued keys, duplicate names and the withCampCook map.
{
    const woodcutting = `const BANK = { a: new Tile(1, 2, 0) };
function camp(name: string, spot: Tile, bankStand: Tile, resources: readonly string[], notes?: string, verified = true) {
    return { name, spot, bankStand, boothName: 'Bank booth', boothOp: 'Use-quickly', verified, resources, notes };
}
export const WOODCUTTING_LOCATIONS = [camp('One', new Tile(5, 6, 0), BANK.a, ['logs'], 'n'), camp('Two', new Tile(7, 8, 0), BANK.a, ['oak'])];
`;
    const mining = `const BANK = { a: new Tile(1, 2, 0) };
function mine(name: string, spot: Tile, bankStand: Tile, resources: readonly string[], notes?: string, verified = true, recommendedCombat?: number) {
    return { name, spot, bankStand, boothName: 'Bank booth', boothOp: 'Use-quickly', verified, resources, notes, recommendedCombat };
}
export const MINING_LOCATIONS = [mine('B Mine', new Tile(5, 6, 0), BANK.a, ['tin'], undefined, true, 65), mine('A Mine', new Tile(7, 8, 0), BANK.a, ['iron'])].sort((a, b) => a.name.localeCompare(b.name));
`;
    const fishing = `export const SHILO_WATER_VENDOR = { keeper: 'Obli', stand: new Tile(1, 2, 0), item: 'Vial of water' } as const;
function withCampCook(loc: Loc) { return loc; }
export const FISHING_LOCATIONS = ([
    { name: 'Pier', spot: new Tile(1, 2, 0), bankStand: new Tile(3, 4, 0), verified: true, resources: ['shrimp'] },
    { name: 'Far', spot: new Tile(5, 6, 0), bankStand: new Tile(3, 4, 0), campRadius: 80, chaseRadius: 28, sweep: [new Tile(5, 6, 0)], baitVendor: { keeper: 'K', stand: new Tile(7, 8, 0), price: 2, item: 'Feather' }, obstacles: ['door'], verified: false },
] as Loc[]).map(withCampCook);
`;
    const tables = await extractCampTables(engine, { woodcutting, mining, fishing });
    assert.deepEqual(tables.woodcutting.map((row) => row.name), ['One', 'Two']);
    assert.deepEqual(tables.woodcutting[1].keys, ['name', 'spot', 'bankStand', 'boothName', 'boothOp', 'verified', 'resources', 'notes']);
    assert.equal(tables.woodcutting[1].notes, undefined);
    assert.deepEqual(tables.mining.map((row) => row.name), ['A Mine', 'B Mine'], 'the post-construction name sort is applied');
    assert.equal(tables.mining[1].recommendedCombat, 65);
    assert.ok(tables.mining[0].keys.includes('recommendedCombat') && tables.mining[0].recommendedCombat === undefined);
    assert.deepEqual(tables.fishing[0].keys, ['name', 'spot', 'bankStand', 'verified', 'resources']);
    assert.deepEqual(tables.fishing[1].keys, ['name', 'spot', 'bankStand', 'campRadius', 'chaseRadius', 'sweep', 'baitVendor', 'obstacles', 'verified']);
    assert.equal(tables.fishing[0].boothName, undefined);
    assert.deepEqual(tables.shiloWaterVendor, { keeper: 'Obli', stand: { x: 1, z: 2, level: 0 }, item: 'Vial of water' });
    const json = JSON.parse(gatherCatalogJson(tables, { methods: [], whirlpoolIds: [], shopCosts: [], gerrantOnly: [], gerrant: { keeper: 'g', stand: { x: 0, z: 0, level: 0 }, bankStand: { x: 0, z: 0, level: 0 } }, harry: { keeper: 'h', stand: { x: 0, z: 0, level: 0 }, bankStand: { x: 0, z: 0, level: 0 } } }, { surfaces: [], plans: [] }, []));
    assert.equal(json.schema, 1);
    assert.equal(json.facts.mining_camps[0].name, 'A Mine');
    assert.deepEqual(json.facts.mining_camps[0].spot, { x: 7, z: 8, level: 0 });
    assert.equal(json.facts.mining_camps[1].recommended_combat, 65);
    assert.equal(json.facts.mining_camps[0].recommended_combat, null);
    assert.deepEqual(json.facts.mining_camps[0].keys, ['Name', 'Spot', 'BankStand', 'BoothName', 'BoothOp', 'Verified', 'Resources', 'Notes', 'RecommendedCombat']);
    await assert.rejects(extractCampTables(engine, { woodcutting: woodcutting.replace("camp('Two'", "camp('One'"), mining, fishing }), /empty or duplicate names/);
    await assert.rejects(extractCampTables(engine, { woodcutting, mining, fishing: fishing.replace('.map(withCampCook)', '') }), /expected the withCampCook map/);
    await assert.rejects(extractCampTables(engine, { woodcutting, mining, fishing: fishing.replace("verified: true,", "verified: true, surprise: 1,") }), /unexpected key surprise/);
}

// Fishing methods, shop facts and the fish-camp cook plans (shared surfaces keep one index).
{
    const methods = `export const WHIRLPOOL_IDS: Set<number> = new Set([403, 404]);
const tool = (name: string) => ({ name, min: 1, restock: 1 });
const bait = (name: string, restock = 100) => ({ name, min: 1, restock });
export const FISHING_METHODS = [{ name: 'Net', op: 'Net', pair: 'Bait', gear: [tool('Small fishing net')] }, { name: 'Rod', op: 'Bait', pair: '', gear: [tool('Fishing rod'), bait('Fishing bait')] }];
`;
    const acquire = `export const FISHING_SHOP_COSTS: Readonly<Record<string, number>> = { 'Small fishing net': 5, Harpoon: 5 };
export const GERRANT_ONLY_FISHING: ReadonlySet<string> = new Set(['feather']);
export const GERRANT_VENDOR = { keeper: 'Gerrant', stand: new Tile(1, 2, 0), bankStand: new Tile(3, 4, 0) };
export const HARRY_VENDOR = { keeper: 'Harry', stand: new Tile(5, 6, 0), bankStand: new Tile(7, 8, 0) };
`;
    const facts = await extractFishingFacts(engine, { fishingMethods: methods, toolAcquire: acquire });
    assert.deepEqual(facts.whirlpoolIds, [403, 404]);
    assert.deepEqual(facts.methods[1].gear, [{ name: 'Fishing rod', min: 1, restock: 1 }, { name: 'Fishing bait', min: 1, restock: 100 }]);
    assert.deepEqual(facts.shopCosts, [['Small fishing net', 5], ['Harpoon', 5]]);
    assert.deepEqual(facts.gerrantOnly, ['feather']);
    assert.equal(facts.harry.keeper, 'Harry');
    await assert.rejects(extractFishingFacts(engine, { fishingMethods: methods.replace("new Set([403, 404])", '[403]'), toolAcquire: acquire }), /expected a Set/);

    const ranges = `const SHARED = { stand: new Tile(1, 2, 0), loc: new Tile(1, 3, 0), locName: 'Range', kind: 'range', label: 'L', notes: 'n' };
export const FISH_CAMP_COOK_PLANS = {
    Catherby: { pier: SHARED, bank: SHARED },
    Seers: { pier: { approach: new Tile(9, 9, 0), stand: new Tile(4, 5, 0), loc: new Tile(4, 6, 0), locName: 'Fire', kind: 'fire' } },
};
`;
    const plans = await extractCookPlans(engine, ranges);
    assert.deepEqual(plans.plans, [{ camp: 'Catherby', pier: 0, bank: 0 }, { camp: 'Seers', pier: 1 }]);
    assert.equal(plans.surfaces.length, 2, 'the shared range is one surface');
    assert.equal(plans.surfaces[1].approachFirst, true);
    assert.equal(plans.surfaces[1].label, undefined);
    await assert.rejects(extractCookPlans(engine, ranges.replace("kind: 'fire'", "kind: 'oven'")), /unknown kind oven/);
    await assert.rejects(extractCookPlans(engine, ranges.replace('{ approach: new Tile(9, 9, 0), stand: new Tile(4, 5, 0)', '{ stand: new Tile(4, 5, 0), approach: new Tile(9, 9, 0)')), /approach must be the first key/);
}

// The rs2b0t export: the generated catalog and the table shapes the Rust code relies on.
{
    const root = requireEnvPath('RS2B0T', 'the rs2b0t checkout root');
    const files = await gatherCatalogFiles(engine, root);
    for (const [file, bytes] of Object.entries(files)) {
        assert.equal(fs.readFileSync(path.join(repoRoot, 'crates/api/data/game-data', file), 'utf8'), bytes, `${file} matches the rs2b0t export (run tools/game-data/generate-gather-catalogs.ts)`);
    }
    const read = (relative: string) => fs.readFileSync(path.join(root, relative), 'utf8');
    const tables = await extractCampTables(engine, {
        woodcutting: read('src/bot/data/woodcuttingLocations.ts'), mining: read('src/bot/data/miningLocations.ts'), fishing: read('src/bot/data/fishingLocations.ts'),
    });
    assert.deepEqual([tables.woodcutting.length, tables.mining.length, tables.fishing.length], [14, 24, 9]);
    const miningNames = tables.mining.map((row) => row.name);
    assert.deepEqual(miningNames, [...miningNames].sort((a, b) => a.localeCompare(b)), 'mining camps are in localeCompare name order');
    assert.equal(tables.mining.find((row) => row.name === 'Dwarven Mine')?.recommendedCombat, 65);
    assert.deepEqual(tables.fishing.find((row) => row.name === 'Shilo Village')?.keys.includes('boothName'), false);
    const fishing = await extractFishingFacts(engine, { fishingMethods: read('src/bot/data/fishingMethods.ts'), toolAcquire: read('src/bot/api/acquisition/ToolAcquire.ts') });
    assert.equal(fishing.methods.length, 9);
    assert.deepEqual(fishing.whirlpoolIds, [403, 404, 405, 406]);
    assert.equal(fishing.shopCosts.length, 8);
    const plans = await extractCookPlans(engine, read('src/bot/data/cookingRanges.ts'));
    assert.deepEqual(plans.plans.map((plan) => plan.camp), ['Catherby', 'Seers (fly fishing)', 'Fishing Guild', 'Barbarian Village', 'Draynor Village']);
    assert.equal(plans.plans[0].pier, plans.plans[0].bank, 'Catherby pier and bank are the one CATHERBY_RANGE');
    assert.notEqual(plans.plans[4].pier, plans.plans[4].bank, 'Draynor pier and bank are two objects with equal values');
}
