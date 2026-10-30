import fs from 'node:fs';
import path from 'node:path';
import { extractQuestJunk, questJunkRust } from './quest-junk.ts';
import { bool, header, int, list, LiteralMapped, LiteralSet, onlyKeys, pinnedLiterals, record, str, text, type Literal } from './rs2b0t-literals.ts';

/**
 * Generated gather catalogs: the curated camp tables (woodcutting, mining and
 * fishing camps), the fishing methods and the fishing facts of
 * `ToolAcquire.ts`, the fish-camp cook plans, and the bank quest-junk table.
 * Every table is read from the pinned rs2b0t AST (`pinnedLiterals`, which
 * evaluates nothing) and emitted as an immutable Rust table in
 * `crates/api/data/game-data/`. Fields upstream computes with code
 * (`withCampCook`, `rangeStandFromLoc`) are computed in Rust from these inputs.
 */

type Tile = { x: number; z: number; level: number };

export const CAMP_KEYS = [
    'name', 'spot', 'bankStand', 'boothName', 'boothOp', 'verified', 'obstacles', 'campRadius', 'chaseRadius',
    'resources', 'avoidSpots', 'sweep', 'baitVendor', 'notes', 'recommendedCombat',
] as const;
type CampKey = (typeof CAMP_KEYS)[number];
type BaitVendor = { keeper: string; stand: Tile; price: number; item: string };
export type Camp = {
    name: string; spot: Tile; bankStand: Tile; boothName?: string; boothOp?: string; verified: boolean;
    obstacles?: string[]; campRadius?: number; chaseRadius?: number; resources?: string[];
    avoidSpots?: Tile[]; sweep?: Tile[]; baitVendor?: BaitVendor; notes?: string; recommendedCombat?: number;
    /** Own-property order of the source object, including keys whose value is `undefined`. */
    keys: CampKey[];
};
export type GearPiece = { name: string; min: number; restock: number };
export type FishingMethod = { name: string; op: string; pair: string; gear: GearPiece[] };
type Vendor = { keeper: string; stand: Tile; bankStand: Tile };
export type FishingFacts = {
    methods: FishingMethod[]; whirlpoolIds: number[]; shopCosts: [string, number][]; gerrantOnly: string[];
    gerrant: Vendor; harry: Vendor;
};
export type CookKind = 'range' | 'fire' | 'fireplace';
export type CookSurface = {
    approachFirst: boolean; approach?: Tile; stand: Tile; loc?: Tile; locName: string; kind: CookKind; label?: string; notes?: string;
};
export type CookPlans = { surfaces: CookSurface[]; plans: { camp: string; pier?: number; bank?: number }[] };

function tile(input: Literal, label: string): Tile {
    const row = record(input, label);
    return { x: int(row.x, label), z: int(row.z, label), level: int(row.level, label) };
}
function strings(input: Literal, label: string) {
    return list(input, label).map((value) => str(value, label));
}
/** One camp row. `undefined` values keep their key (`notes: undefined` is an own property upstream). */
function camp(input: Literal, label: string): Camp {
    const row = record(input, label);
    onlyKeys(row, CAMP_KEYS, label);
    const out: Camp = {
        name: str(row.name, `${label} name`), spot: tile(row.spot, `${label} spot`), bankStand: tile(row.bankStand, `${label} bankStand`),
        verified: bool(row.verified, `${label} verified`), keys: Object.keys(row) as CampKey[],
    };
    if (row.boothName !== undefined) out.boothName = str(row.boothName, label);
    if (row.boothOp !== undefined) out.boothOp = str(row.boothOp, label);
    if (row.obstacles !== undefined) out.obstacles = strings(row.obstacles, label);
    if (row.campRadius !== undefined) out.campRadius = int(row.campRadius, label);
    if (row.chaseRadius !== undefined) out.chaseRadius = int(row.chaseRadius, label);
    if (row.resources !== undefined) out.resources = strings(row.resources, label);
    if (row.avoidSpots !== undefined) out.avoidSpots = list(row.avoidSpots, label).map((value) => tile(value, label));
    if (row.sweep !== undefined) out.sweep = list(row.sweep, label).map((value) => tile(value, label));
    if (row.baitVendor !== undefined) {
        const vendor = record(row.baitVendor, label);
        onlyKeys(vendor, ['keeper', 'stand', 'price', 'item'], label);
        out.baitVendor = { keeper: str(vendor.keeper, label), stand: tile(vendor.stand, label), price: int(vendor.price, label), item: str(vendor.item, label) };
    }
    if (row.notes !== undefined) out.notes = str(row.notes, label);
    if (row.recommendedCombat !== undefined) out.recommendedCombat = int(row.recommendedCombat, label);
    if (new Set(out.keys).size !== out.keys.length) throw new Error(`${label}: duplicate keys`);
    return out;
}

function campTable(input: Literal, label: string, helper?: string): Camp[] {
    let rows = input;
    if (rows instanceof LiteralMapped) {
        if (rows.helper !== helper) throw new Error(`${label}: unexpected helper ${rows.helper}`);
        rows = rows.rows;
    } else if (helper !== undefined) {
        throw new Error(`${label}: expected the ${helper} map`);
    }
    const camps = list(rows, label).map((row, index) => camp(row, `${label}[${index}]`));
    if (!camps.length || new Set(camps.map((row) => row.name)).size !== camps.length) throw new Error(`${label}: empty or duplicate names`);
    return camps;
}

export type CampTables = { woodcutting: Camp[]; mining: Camp[]; fishing: Camp[]; shiloWaterVendor: { keeper: string; stand: Tile; item: string } };

export async function extractCampTables(engine: string, sources: { woodcutting: string; mining: string; fishing: string }): Promise<CampTables> {
    const woodcutting = await pinnedLiterals(engine, 'woodcuttingLocations.ts', sources.woodcutting, 'woodcutting camps');
    const mining = await pinnedLiterals(engine, 'miningLocations.ts', sources.mining, 'mining camps');
    const fishing = await pinnedLiterals(engine, 'fishingLocations.ts', sources.fishing, 'fishing camps');
    const vendor = record(fishing('SHILO_WATER_VENDOR'), 'Shilo water vendor');
    onlyKeys(vendor, ['keeper', 'stand', 'item'], 'Shilo water vendor');
    return {
        woodcutting: campTable(woodcutting('WOODCUTTING_LOCATIONS'), 'WOODCUTTING_LOCATIONS'),
        mining: campTable(mining('MINING_LOCATIONS'), 'MINING_LOCATIONS'),
        fishing: campTable(fishing('FISHING_LOCATIONS'), 'FISHING_LOCATIONS', 'withCampCook'),
        shiloWaterVendor: { keeper: str(vendor.keeper, 'vendor'), stand: tile(vendor.stand, 'vendor'), item: str(vendor.item, 'vendor') },
    };
}

function vendorOf(input: Literal, label: string): Vendor {
    const row = record(input, label);
    onlyKeys(row, ['keeper', 'stand', 'bankStand'], label);
    return { keeper: str(row.keeper, label), stand: tile(row.stand, label), bankStand: tile(row.bankStand, label) };
}

export async function extractFishingFacts(engine: string, sources: { fishingMethods: string; toolAcquire: string }): Promise<FishingFacts> {
    const methods = await pinnedLiterals(engine, 'fishingMethods.ts', sources.fishingMethods, 'fishing methods');
    const acquire = await pinnedLiterals(engine, 'ToolAcquire.ts', sources.toolAcquire, 'tool acquire');
    const rows = list(methods('FISHING_METHODS'), 'FISHING_METHODS').map((input, index): FishingMethod => {
        const row = record(input, `method ${index}`);
        onlyKeys(row, ['name', 'op', 'pair', 'gear'], `method ${index}`);
        return {
            name: str(row.name, 'method'), op: str(row.op, 'method'), pair: str(row.pair, 'method'),
            gear: list(row.gear, 'gear').map((piece): GearPiece => {
                const gear = record(piece, 'gear piece');
                onlyKeys(gear, ['name', 'min', 'restock'], 'gear piece');
                return { name: str(gear.name, 'gear'), min: int(gear.min, 'gear'), restock: int(gear.restock, 'gear') };
            }),
        };
    });
    if (!rows.length || new Set(rows.map((row) => row.name)).size !== rows.length) throw new Error('fishing methods: empty or duplicate names');
    const whirlpool = methods('WHIRLPOOL_IDS');
    if (!(whirlpool instanceof LiteralSet)) throw new Error('WHIRLPOOL_IDS: expected a Set');
    const costs = record(acquire('FISHING_SHOP_COSTS'), 'FISHING_SHOP_COSTS');
    const gerrantOnly = acquire('GERRANT_ONLY_FISHING');
    if (!(gerrantOnly instanceof LiteralSet)) throw new Error('GERRANT_ONLY_FISHING: expected a Set');
    return {
        methods: rows,
        whirlpoolIds: whirlpool.values.map((value) => int(value, 'whirlpool')),
        shopCosts: Object.entries(costs).map(([name, cost]) => [name, int(cost, 'cost')]),
        gerrantOnly: gerrantOnly.values.map((value) => str(value, 'gerrant only')),
        gerrant: vendorOf(acquire('GERRANT_VENDOR'), 'GERRANT_VENDOR'),
        harry: vendorOf(acquire('HARRY_VENDOR'), 'HARRY_VENDOR'),
    };
}

const COOK_KINDS = ['range', 'fire', 'fireplace'];
const COOK_SURFACE_KEYS = ['approach', 'stand', 'loc', 'locName', 'kind', 'label', 'notes'];

export async function extractCookPlans(engine: string, source: string): Promise<CookPlans> {
    const ranges = await pinnedLiterals(engine, 'cookingRanges.ts', source, 'fish camp cook plans');
    const plans = record(ranges('FISH_CAMP_COOK_PLANS'), 'FISH_CAMP_COOK_PLANS');
    const surfaces: CookSurface[] = [];
    const seen = new Map<object, number>();
    const index = (input: Literal, label: string) => {
        const row = record(input, label);
        const known = seen.get(row);
        if (known !== undefined) return known;
        onlyKeys(row, COOK_SURFACE_KEYS, label);
        const keys = Object.keys(row);
        if (keys.includes('approach') && keys[0] !== 'approach') throw new Error(`${label}: approach must be the first key`);
        const kind = str(row.kind, label);
        if (!COOK_KINDS.includes(kind)) throw new Error(`${label}: unknown kind ${kind}`);
        const surface: CookSurface = {
            approachFirst: keys[0] === 'approach', stand: tile(row.stand, label), locName: str(row.locName, label), kind: kind as CookKind,
        };
        if (row.approach !== undefined) surface.approach = tile(row.approach, label);
        if (row.loc !== undefined) surface.loc = tile(row.loc, label);
        if (row.label !== undefined) surface.label = str(row.label, label);
        if (row.notes !== undefined) surface.notes = str(row.notes, label);
        surfaces.push(surface);
        seen.set(row, surfaces.length - 1);
        return surfaces.length - 1;
    };
    const rows = Object.entries(plans).map(([name, input]) => {
        const plan = record(input, name);
        onlyKeys(plan, ['pier', 'bank'], name);
        const out: { camp: string; pier?: number; bank?: number } = { camp: name };
        if (plan.pier !== undefined) out.pier = index(plan.pier, `${name} pier`);
        if (plan.bank !== undefined) out.bank = index(plan.bank, `${name} bank`);
        return out;
    });
    return { surfaces, plans: rows };
}

const worldTile = (value: Tile) => `WorldTile { x: ${value.x}, z: ${value.z}, level: ${value.level} }`;
const option = <T>(value: T | undefined, emit: (value: T) => string) => (value === undefined ? 'None' : `Some(${emit(value)})`);
const slice = <T>(values: readonly T[], emit: (value: T) => string) => `&[${values.map(emit).join(', ')}]`;
const pascal = (key: string) => key.charAt(0).toUpperCase() + key.slice(1);

function campRust(row: Camp) {
    return `    GatherCamp { name: ${text(row.name)}, spot: ${worldTile(row.spot)}, bank_stand: ${worldTile(row.bankStand)}, `
        + `booth_name: ${option(row.boothName, text)}, booth_op: ${option(row.boothOp, text)}, verified: ${row.verified}, `
        + `obstacles: ${option(row.obstacles, (value) => slice(value, text))}, camp_radius: ${option(row.campRadius, String)}, chase_radius: ${option(row.chaseRadius, String)}, `
        + `resources: ${option(row.resources, (value) => slice(value, text))}, avoid_spots: ${option(row.avoidSpots, (value) => slice(value, worldTile))}, `
        + `sweep: ${option(row.sweep, (value) => slice(value, worldTile))}, `
        + `bait_vendor: ${option(row.baitVendor, (value) => `BaitVendor { keeper: ${text(value.keeper)}, stand: ${worldTile(value.stand)}, price: ${value.price}, item: ${text(value.item)} }`)}, `
        + `notes: ${option(row.notes, text)}, recommended_combat: ${option(row.recommendedCombat, String)}, `
        + `keys: ${slice(row.keys, (key) => `CampKey::${pascal(key)}`)} },\n`;
}

export function campsRust(constName: string, source: string, rows: Camp[], extra = '') {
    return header(source) + extra + `pub const ${constName}: &[GatherCamp] = &[\n${rows.map(campRust).join('')}];\n`;
}

export function shiloVendorRust(vendor: CampTables['shiloWaterVendor']) {
    return `pub const SHILO_WATER_VENDOR: WaterVendor = WaterVendor { keeper: ${text(vendor.keeper)}, stand: ${worldTile(vendor.stand)}, item: ${text(vendor.item)} };\n`;
}

export function fishingFactsRust(facts: FishingFacts) {
    const vendor = (value: Vendor) => `FishingVendor { keeper: ${text(value.keeper)}, stand: ${worldTile(value.stand)}, bank_stand: ${worldTile(value.bankStand)} }`;
    return header('data/fishingMethods.ts and api/acquisition/ToolAcquire.ts')
        + `pub const WHIRLPOOL_IDS: &[i32] = &[${facts.whirlpoolIds.join(', ')}];\n`
        + 'pub const FISHING_METHODS: &[FishingMethod] = &[\n'
        + facts.methods.map((method) => `    FishingMethod { name: ${text(method.name)}, op: ${text(method.op)}, pair: ${text(method.pair)}, gear: ${slice(method.gear, (piece) => `GearPiece { name: ${text(piece.name)}, min: ${piece.min}, restock: ${piece.restock} }`)} },\n`).join('')
        + '];\n'
        + `pub const FISHING_SHOP_COSTS: &[(&str, i32)] = ${slice(facts.shopCosts, ([name, cost]) => `(${text(name)}, ${cost})`)};\n`
        + `pub const GERRANT_ONLY_FISHING: &[&str] = ${slice(facts.gerrantOnly, text)};\n`
        + `pub const GERRANT_VENDOR: FishingVendor = ${vendor(facts.gerrant)};\n`
        + `pub const HARRY_VENDOR: FishingVendor = ${vendor(facts.harry)};\n`;
}

export function cookPlansRust(plans: CookPlans) {
    const kind = (value: CookKind) => `FishCookKind::${pascal(value)}`;
    return header('data/cookingRanges.ts')
        + 'pub const FISH_CAMP_COOK_SURFACES: &[FishCookSurface] = &[\n'
        + plans.surfaces.map((surface) => `    FishCookSurface { approach_first: ${surface.approachFirst}, approach: ${option(surface.approach, worldTile)}, stand: ${worldTile(surface.stand)}, `
            + `loc: ${option(surface.loc, worldTile)}, loc_name: ${text(surface.locName)}, kind: ${kind(surface.kind)}, label: ${option(surface.label, text)}, notes: ${option(surface.notes, text)} },\n`).join('')
        + '];\n'
        + 'pub const FISH_CAMP_COOK_PLANS: &[FishCampCookPlan] = &[\n'
        + plans.plans.map((plan) => `    FishCampCookPlan { camp: ${text(plan.camp)}, pier: ${option(plan.pier, String)}, bank: ${option(plan.bank, String)} },\n`).join('')
        + '];\n';
}

/** Every generated gather file and its bytes, keyed by file name under `crates/api/data/game-data/`. */
export async function gatherCatalogFiles(engine: string, rs2b0tRoot: string): Promise<Record<string, string>> {
    const read = (relative: string) => fs.readFileSync(path.join(rs2b0tRoot, relative), 'utf8');
    const camps = await extractCampTables(engine, {
        woodcutting: read('src/bot/data/woodcuttingLocations.ts'), mining: read('src/bot/data/miningLocations.ts'), fishing: read('src/bot/data/fishingLocations.ts'),
    });
    const fishing = await extractFishingFacts(engine, { fishingMethods: read('src/bot/data/fishingMethods.ts'), toolAcquire: read('src/bot/api/acquisition/ToolAcquire.ts') });
    const cook = await extractCookPlans(engine, read('src/bot/data/cookingRanges.ts'));
    const junk = await extractQuestJunk(engine, read('src/bot/api/bank/bankQuestJunk.ts'));
    return {
        'woodcutting-camps.rs': campsRust('WOODCUTTING_CAMPS', 'data/woodcuttingLocations.ts', camps.woodcutting),
        'mining-camps.rs': campsRust('MINING_CAMPS', 'data/miningLocations.ts (en-US localeCompare name order)', camps.mining),
        'fishing-camps.rs': campsRust('FISHING_CAMPS', 'data/fishingLocations.ts (before withCampCook)', camps.fishing, shiloVendorRust(camps.shiloWaterVendor)),
        'fishing-methods.rs': fishingFactsRust(fishing),
        'fish-camp-cook-plans.rs': cookPlansRust(cook),
        'quest-junk-catalog.rs': questJunkRust(junk),
    };
}
