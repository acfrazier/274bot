'use strict';

const assert = require('node:assert/strict');
const { execFileSync } = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const test = require('node:test');
const ts = require('typescript');
const {
    callbackFamilyRows,
    classifyModule,
    collectModuleNotImplValueExports,
    deriveNativeFamilyFamilies,
    deriveNavFamilies,
    validateArtifacts,
} = require('./abi-contract.cjs');

const here = __dirname;
const crate = path.resolve(here, '..');
const frozenRoot = process.env.RS2B0T ? fs.realpathSync(process.env.RS2B0T) : null;
const frozenRevision = '2de6bb2397ceb9ffd05f2354f67dde328e324531';
const frozenRepository = 'https://github.com/rs2b2t/rs2b0t.git';
const contractText = fs.readFileSync(path.join(here, 'abi-contract.json'), 'utf8');
const predecessorText = fs.readFileSync(path.join(here, 'family-predecessors.json'), 'utf8');
const contract = JSON.parse(contractText);
const predecessors = JSON.parse(predecessorText);
const currentIndex = fs.readFileSync(path.join(here, 'index.d.ts'), 'utf8');
const registry = JSON.parse(fs.readFileSync(path.join(here, 'modules.json'), 'utf8'));
const shimSources = Object.fromEntries(registry.map(entry => [
    entry.shim,
    fs.readFileSync(path.join(crate, 'src/shim', entry.shim), 'utf8'),
]));
const callbackSnapshot = JSON.parse(fs.readFileSync(path.join(here, 'abi-callbacks.json'), 'utf8'));
assert.equal(callbackSnapshot.schema_version, 2);
const nativeFamilyFamilies = deriveNativeFamilyFamilies(callbackSnapshot, registry, shimSources);
const navFamilies = frozenRoot ? deriveNavFamilies({
    frozenSourceRoot: path.join(frozenRoot, 'src'),
    nativeSourceRoot: path.join(crate, 'src'),
    nativeFamilyFamilies,
    registry,
}) : null;

test('the generated ABI contract and family graph are complete', () => {
    validateArtifacts(contract, predecessors, navFamilies, currentIndex);
    const nav = new Set(predecessors.families.filter(row => row.nav_consuming).map(row => row.family_id));
    assert.deepEqual(
        [...nav].sort(),
        ['acquisition', 'bank', 'clues', 'duel', 'hunt', 'quests', 'runtime', 'shop', 'tasks', 'thieving', 'walking'],
        'nav families are exactly those reached from served or declared ABI sources and called native backends',
    );
});

test('module-only families have explicit non-nav predecessor rows without activation credit', () => {
    assert.equal(predecessors.families.length, 41);
    assert.equal(predecessors.families.filter(row => row.activation_state === 'not-activated').length, 33);
    for (const [familyId, modules] of [
        ['magic', ['api/magic/Autocast.js']],
        ['prayer', ['api/prayer/Prayer.js']],
        ['catalog-data', ['data/dropdb.js', 'data/herbs.js', 'data/itemdb.js', 'data/shopdb.js', 'data/spelldb.js']],
    ]) {
        const family = predecessors.families.find(row => row.family_id === familyId);
        assert.ok(family, familyId);
        assert.equal(family.nav_consuming, false);
        assert.equal(family.activation_state, 'not-activated');
        assert.deepEqual(family.predecessors, ['F0', 'T1.3', 'T2.1']);
        assert.deepEqual(
            contract.typed_modules.filter(row => row.family_id === familyId).map(row => row.module),
            modules,
        );
    }
    assert.deepEqual(
        contract.typed_modules.map(row => row.module).sort(),
        ['@rs2b0t/api', ...registry.map(row => row.module)].sort(),
    );
});

test('the internal gathering adapter has complete gathering family coverage', () => {
    assert.deepEqual(
        contract.typed_modules.find(row => row.module === 'shim/gathering_locations.js'),
        { module: 'shim/gathering_locations.js', family_id: 'gathering', nav_consuming: false },
    );
    const helpers = new Set([
        'DEFAULT_BOOTH_NAME', 'DEFAULT_BOOTH_OP', 'MAP_SQUARE', 'sameMapSquare',
        'locationOptions', 'boothFields', 'resolveGatheringLocation',
    ]);
    assert.ok(!contract.not_impl_debt.some(row => helpers.has(row.selector)));
});

test('every typed module requires a family predecessor row', () => {
    for (const module of contract.typed_modules) {
        const drift = structuredClone(predecessors);
        drift.families = drift.families.filter(row => row.family_id !== module.family_id);
        assert.throws(
            () => validateArtifacts(contract, drift, navFamilies, currentIndex),
            /Missing family predecessor classification for typed module/,
            module.module,
        );
    }
});

test('the typed declaration AST rejects omitted module family rows', () => {
    for (const module of contract.typed_modules) {
        const drift = structuredClone(contract);
        drift.typed_modules = drift.typed_modules.filter(row => row.module !== module.module);
        assert.throws(
            () => validateArtifacts(drift, predecessors, navFamilies, currentIndex),
            /Missing typed module family row/,
            module.module,
        );
    }
});

test('typed module classification rejects duplicate and incorrect family rows', () => {
    const duplicate = structuredClone(contract);
    duplicate.typed_modules.push(duplicate.typed_modules[0]);
    assert.throws(() => validateArtifacts(duplicate, predecessors, navFamilies), /Duplicate typed module family row/);
    const incorrect = structuredClone(contract);
    incorrect.typed_modules.find(row => row.module === 'api/magic/Autocast.js').family_id = 'combat';
    assert.throws(() => validateArtifacts(incorrect, predecessors, navFamilies), /Typed module family classification disagrees/);
});

test('only complete bank, chatbox, cooking, cow, gathering, inventory, runecrafting, skills and world-primitives families are activated', () => {
    assert.equal(contract.activation_state, 'partially-activated');
    assert.deepEqual(contract.active_families, ['bank', 'chatbox', 'cooking', 'cow', 'gathering', 'inventory', 'runecrafting', 'skills', 'world-primitives']);
    for (const family of predecessors.families) {
        assert.equal(
            family.activation_state,
            contract.active_families.includes(family.family_id) ? 'activated' : 'not-activated',
            family.family_id,
        );
    }
    assert.ok(!contract.not_impl_debt.some(row => row.selector === 'nearestUsableBank'));
    assert.ok(!contract.not_impl_debt.some(row => row.selector === 'backpackSnapshots'));
    assert.ok(!contract.not_impl_debt.some(row => row.selector === 'backpackCapacity'));
    assert.deepEqual(
        contract.typed_modules.filter(row => row.family_id === 'inventory').map(row => row.module).sort(),
        ['api/equipment/Equipment.js', 'api/inventory/Inventory.js', 'api/inventory/packRules.js'],
    );
    assert.equal(
        predecessors.families.find(row => row.family_id === 'inventory').counts.not_impl_debt,
        0,
    );
    assert.equal(nativeFamilyFamilies.get('bank_wait'), 'bank');
});
test('inventory root and public module declarations expose exact consumer types', () => {
    const scratch = fs.mkdtempSync(path.join(os.tmpdir(), 't2-inventory-consumer-'));
    try {
        const consumer = path.join(scratch, 'consumer.ts');
        fs.writeFileSync(consumer, `
import {
    Inventory as RootInventory,
    InvItem as RootInvItem,
    Equipment as RootEquipment,
} from '@rs2b0t/api';
import * as RootInventoryModule from '@rs2b0t/api';
import {
    Inventory as ModuleInventory,
    InvItem as ModuleInvItem,
    backpackSnapshots,
    backpackCapacity,
} from '#/bot/api/inventory/Inventory.js';
import { Equipment as ModuleEquipment } from '#/bot/api/equipment/Equipment.js';
import {
    type PackItem,
    matchesAny,
    countMatching,
    slotsMatching,
    shouldBank,
    shouldRestock,
    shouldEat,
    shouldPanic,
} from '#/bot/api/inventory/packRules.js';
import type { InvItemSnapshot } from '#/bot/adapter/ClientAdapter.js';
import type { Loc } from '#/bot/api/model/Loc.js';
import type { Npc } from '#/bot/api/model/Npc.js';
import type { Player } from '#/bot/api/model/Player.js';
import type { GroundItem } from '#/bot/api/grounditems/GroundItems.js';

declare const row: InvItemSnapshot;
declare const loc: Loc;
declare const npc: Npc;
declare const player: Player;
declare const groundItem: GroundItem;

const moduleRows: InvItemSnapshot[] = backpackSnapshots();
const moduleCapacity: number = backpackCapacity();
const moduleItems: ModuleInvItem[] = ModuleInventory.items();
const rootItems: RootInvItem[] = RootInventory.items();
const moduleFirst: ModuleInvItem | null = ModuleInventory.first('Shark');
const rootFirst: RootInvItem | null = RootInventory.first('Shark');
const moduleContains: boolean = ModuleInventory.contains('Shark');
const rootContains: boolean = RootInventory.contains('Shark');
const used: number = ModuleInventory.used();
const count: number = ModuleInventory.count('Shark');
const countById: number = ModuleInventory.countById(385);
const full: boolean = ModuleInventory.isFull();
const free: number = ModuleInventory.free();
const rootUsed: number = RootInventory.used();
const rootCount: number = RootInventory.count('Shark');
const rootCountById: number = RootInventory.countById(385);
const rootFull: boolean = RootInventory.isFull();
const rootFree: number = RootInventory.free();
const moduleEquipmentItems: ModuleInvItem[] = ModuleEquipment.items();
const rootEquipmentItems: RootInvItem[] = RootEquipment.items();
const equipmentContains: boolean = ModuleEquipment.contains('Shark');
const rootEquipmentContains: boolean = RootEquipment.contains('Shark');
const equip: Promise<boolean> = ModuleEquipment.equip('Shark');
const unequip: Promise<boolean> = ModuleEquipment.unequip('Shark');
const rootEquip: Promise<boolean> = RootEquipment.equip('Shark');
const rootUnequip: Promise<boolean> = RootEquipment.unequip('Shark');

const item = new ModuleInvItem(row);
const rootItem: RootInvItem | null = moduleFirst;
const name: string | null = item.name;
const id: number = item.id;
const slot: number = item.slot;
const quantity: number = item.count;
const noted: boolean = item.noted;
const cert: number = item.cert;
const actions: string[] = item.actions();
const interacted: boolean = item.interact('Eat');
const usedOnItem: boolean = item.useOn(item);
const usedOnLoc: boolean = item.useOn(loc);
const usedOnNpc: boolean = item.useOn(npc);
const usedOnPlayer: boolean = item.useOn(player);
const usedOnGroundItem: boolean = item.useOn(groundItem);
const rootConstructedItem = new RootInvItem(row);
const rootName: string | null = rootConstructedItem.name;
const rootId: number = rootConstructedItem.id;
const rootSlot: number = rootConstructedItem.slot;
const rootQuantity: number = rootConstructedItem.count;
const rootNoted: boolean = rootConstructedItem.noted;
const rootCert: number = rootConstructedItem.cert;
const rootActions: string[] = rootConstructedItem.actions();
const rootInteracted: boolean = rootConstructedItem.interact('Eat');
const rootUsedOnItem: boolean = rootConstructedItem.useOn(item);
const rootUsedOnLoc: boolean = rootConstructedItem.useOn(loc);
const rootUsedOnNpc: boolean = rootConstructedItem.useOn(npc);
const rootUsedOnPlayer: boolean = rootConstructedItem.useOn(player);
const rootUsedOnGroundItem: boolean = rootConstructedItem.useOn(groundItem);

const pack: PackItem = { name: 'Shark', count: 2 };
const packRows: readonly PackItem[] = [pack];
const anyMatch: boolean = matchesAny(pack.name, ['shark']);
const matchingCount: number = countMatching(packRows, ['shark']);
const matchingSlots: number = slotsMatching(packRows, ['shark']);
const bank: boolean = shouldBank(8, 8, false);
const restock: boolean = shouldRestock(1, 2);
const eat: boolean = shouldEat(10, 10, 5, 1);
const panic: boolean = shouldPanic(0.2, 0.25, 0);

// @ts-expect-error backpack snapshot helpers take no arguments
backpackSnapshots(1);
// @ts-expect-error backpackSnapshots is a public-module-only helper
RootInventoryModule.backpackSnapshots();
// @ts-expect-error backpackCapacity is a public-module-only helper
RootInventoryModule.backpackCapacity();
// @ts-expect-error module backpack capacity helper takes no arguments
backpackCapacity(28);
// @ts-expect-error count takes an item name, not an object id
ModuleInventory.count(385);
// @ts-expect-error root count takes an item name, not an object id
RootInventory.count(385);
// @ts-expect-error root countById requires a numeric id
RootInventory.countById('385');
// @ts-expect-error root contains takes an item name
RootInventory.contains(385);
// @ts-expect-error root equipment methods take an item name
RootEquipment.equip(385);
// @ts-expect-error the module InvItem constructor requires a posted row
new ModuleInvItem();
// @ts-expect-error the module InvItem constructor takes one posted row
new ModuleInvItem(row, true);
// @ts-expect-error InvItem does not expose its private source row
item.snap;
// @ts-expect-error InvItem fields are readonly
item.name = 'mutated';
// @ts-expect-error cert is numeric
const wrongCert: string = item.cert;
// @ts-expect-error useOn accepts only supported posted target classes
item.useOn('Shark');
// @ts-expect-error PackItem contains only name and count
const extraPackField: PackItem = { name: 'Shark', count: 2, id: 385 };
// @ts-expect-error root InvItem does not expose its private source row
rootConstructedItem.snap;
// @ts-expect-error root InvItem fields are readonly
rootConstructedItem.name = 'mutated';
// @ts-expect-error root useOn accepts only supported posted target classes
rootConstructedItem.useOn('Shark');
// @ts-expect-error PackItem is a type-only export
void PackItem;
// @ts-expect-error matchesAny requires a string array
matchesAny('Shark', 'shark');
// @ts-expect-error countMatching returns a number
const wrongMatchingCount: string = countMatching(packRows, ['shark']);
// @ts-expect-error shouldEat requires numeric HP and item counts
shouldEat('10', 10, 5, 1);

void [
    moduleRows, moduleCapacity, moduleItems, rootItems,
    moduleFirst, rootFirst, moduleContains, rootContains, used, count, countById,
    full, free, moduleEquipmentItems, rootEquipmentItems, equipmentContains,
    rootEquipmentContains, equip, unequip, rootEquip, rootUnequip, rootItem, name,
    id, slot, quantity, noted, cert, actions, interacted, usedOnItem, usedOnLoc,
    usedOnNpc, usedOnPlayer, usedOnGroundItem, anyMatch, matchingCount,
    matchingSlots, bank, restock, eat, panic, wrongCert, extraPackField,
    wrongMatchingCount,
    rootUsed, rootCount, rootCountById, rootFull, rootFree, rootConstructedItem,
    rootName, rootId, rootSlot, rootQuantity, rootNoted, rootCert, rootActions,
    rootInteracted, rootUsedOnItem, rootUsedOnLoc, rootUsedOnNpc, rootUsedOnPlayer,
    rootUsedOnGroundItem,
];
`);
        const program = ts.createProgram(
            [path.join(here, 'index.d.ts'), consumer],
            {
                target: ts.ScriptTarget.ESNext,
                module: ts.ModuleKind.ESNext,
                moduleResolution: ts.ModuleResolutionKind.Bundler,
                strict: true,
                skipLibCheck: false,
                noEmit: true,
            },
        );
        const diagnostics = ts.getPreEmitDiagnostics(program);
        assert.deepEqual(
            diagnostics.map(diagnostic => ts.flattenDiagnosticMessageText(diagnostic.messageText, '\n')),
            [],
            'inventory consumers must type-check against the generated exact declarations',
        );
    } finally {
        fs.rmSync(scratch, { recursive: true, force: true });
    }
});

test('an activated family cannot lose its complete-unit graph state', () => {
    for (const familyId of contract.active_families) {
        const drift = structuredClone(predecessors);
        drift.families.find(row => row.family_id === familyId).activation_state = 'not-activated';
        assert.throws(
            () => validateArtifacts(contract, drift, navFamilies),
            new RegExp(`Unexpected activation state for family ${familyId}`),
        );
    }
});

test('a declared-only runtime member roots its implementation for nav scanning', () => {
    const scratch = fs.mkdtempSync(path.join(os.tmpdir(), 't2-abi-declared-nav-'));
    const sourceRoot = path.join(scratch, 'src');
    const botRoot = path.join(sourceRoot, 'bot');
    const packageRoot = path.join(scratch, 'packages/rs2b0t-api');
    const acquisition = path.join(botRoot, 'api/acquisition/ItemAcquisition.ts');
    const nativeRoot = path.join(scratch, 'native');
    try {
        fs.mkdirSync(path.dirname(acquisition), { recursive: true });
        fs.mkdirSync(path.join(botRoot, 'runtime'), { recursive: true });
        fs.mkdirSync(packageRoot, { recursive: true });
        fs.mkdirSync(nativeRoot, { recursive: true });
        fs.writeFileSync(acquisition, "import { Traversal } from '../walking/Traversal.js';\nexport class AcquireTask { execute() { return Traversal.walkResilient(); } }\n");
        fs.writeFileSync(
            path.join(botRoot, 'runtime/abi.ts'),
            "import { AcquireTask } from '../api/acquisition/ItemAcquisition.js';\nfunction installAbi() { const abi = Object.freeze({ AcquireTask }); }\n",
        );
        fs.writeFileSync(path.join(packageRoot, 'index.d.ts'), 'export class AcquireTask { execute(): void; }\n');
        fs.writeFileSync(path.join(packageRoot, 'index.js'), 'export const { AcquireTask } = abi;\n');

        const derived = deriveNavFamilies({
            frozenSourceRoot: sourceRoot,
            nativeSourceRoot: nativeRoot,
            nativeFamilyFamilies: new Map(),
            registry: [],
        });
        assert.deepEqual([...derived].sort(), ['acquisition']);
    } finally {
        fs.rmSync(scratch, { recursive: true, force: true });
    }
});

test('code-derived nav evidence agrees with the family graph', { skip: !navFamilies }, () => {
    validateArtifacts(contract, predecessors, navFamilies);
});

test('a hand-edited nav flag that contradicts source evidence fails validation', { skip: !navFamilies }, () => {
    const drift = structuredClone(predecessors);
    drift.families.find(row => row.family_id === 'shop').nav_consuming = false;
    assert.throws(
        () => validateArtifacts(contract, drift, navFamilies),
        /Navigation classification disagrees with code evidence for family shop/,
    );
});

test('native callback ownership follows the registered shim callers', () => {
    assert.equal(nativeFamilyFamilies.get('dialog'), 'quests');
    for (const family of ['clue_duel_handshake', 'clue_duel_helper', 'clue_duel_travel']) {
        assert.equal(nativeFamilyFamilies.get(family), 'duel');
    }
    assert.equal(nativeFamilyFamilies.get('hunt-key'), 'hunt');
    assert.equal(nativeFamilyFamilies.get('periodic_bank'), 'tasks');
    assert.equal(nativeFamilyFamilies.get('bank_nearest'), 'bank');
    assert.equal(nativeFamilyFamilies.get('bank_withdraw_to'), 'thieving');
    assert.equal(nativeFamilyFamilies.get('bank_close_confirm'), 'thieving');
});
test('registered callback-free backends participate in navigation classification', () => {
    assert.ok(!callbackSnapshot.rows.some(row => row.families?.includes('light-fire')));
    assert.equal(nativeFamilyFamilies.get('light-fire'), 'firemaking');
});

test('the contract callback rows equal the family-expanded native ABI census', () => {
    assert.deepEqual(
        contract.callbacks,
        callbackFamilyRows(callbackSnapshot, nativeFamilyFamilies, navFamilies),
    );
    for (const callback of contract.callbacks) {
        const bound = callback.path === 'ask' ? 'ASKS_PER_STEP=512' : 'CALLS_PER_TICK=256';
        assert.ok(callback.multiplicity.includes(bound), `${callback.native_family}/${callback.member}`);
    }
});
test('callback source citations resolve and identify each member', { skip: !frozenRoot }, () => {
    const citationPattern = /\$(TS|S)\/([^:;\s]+):([0-9,-]+)/g;
    const sourceCache = new Map();
    let citationCount = 0;
    for (const row of callbackSnapshot.rows) {
        const citations = [...row.evidence.matchAll(citationPattern)];
        assert.ok(citations.length > 0, `${row.member} has source evidence`);
        let citedText = '';
        for (const [, source, relative, ranges] of citations) {
            const base = source === 'TS'
                ? path.join(frozenRoot, 'src', 'bot')
                : path.join(crate, 'src');
            const file = path.join(base, relative);
            assert.ok(fs.existsSync(file), `${row.member} evidence file exists: ${file}`);
            let lines = sourceCache.get(file);
            if (!lines) {
                lines = fs.readFileSync(file, 'utf8').split(/\r?\n/);
                sourceCache.set(file, lines);
            }
            for (const range of ranges.split(',')) {
                const [start, end = start] = range.split('-').map(Number);
                assert.ok(start >= 1 && end >= start && end <= lines.length, `${row.member} evidence range ${relative}:${range}`);
                citedText += lines.slice(start - 1, end).join('\n');
                citationCount++;
            }
        }
        const memberToken = row.member.toLowerCase().replace(/[^a-z0-9]/g, '');
        const normalizedEvidence = citedText.toLowerCase().replace(/[^a-z0-9]/g, '');
        assert.ok(normalizedEvidence.includes(memberToken), `${row.member} appears at a cited source line`);
    }
    assert.ok(citationCount > 0);
});


test('types follow the family their ABI describes', () => {
    for (const [name, familyId] of [
        ['QuestStatus', 'questlog'],
        ['TradeItem', 'trade'],
        ['BankItemSnapshot', 'bank'],
        ['BankObjectAccess', 'bank'],
    ]) {
        const declaration = contract.declarations.find(row => row.name === name);
        assert.ok(declaration, `missing ${name}`);
        assert.equal(declaration.family_id, familyId);
        for (const member of declaration.members) {
            assert.equal(member.family_id, familyId, `${name}.${member.name}`);
            for (const signature of member.signatures) assert.equal(signature.family_id, familyId);
        }
    }
});

test('refusal debt is unique and names each missing notImplValue export', () => {
    assert.equal(contract.not_impl_debt.length, 198);
    const selectors = contract.not_impl_debt.map(row => row.selector);
    assert.equal(new Set(selectors).size, selectors.length);
    for (const selector of [
        'BranchTask.validate',
        'BranchTask.success',
        'BranchTask.failure',
        'LeafTask.execute',
        'resolveWoodcuttingLocation',
    ]) {
        assert.equal(selectors.filter(value => value === selector).length, 1, selector);
    }
    const newRefusals = [
        ['runtime/Settings.js', ['GLOBAL_SETTINGS_CORE', 'NAV_SETTINGS', 'GLOBAL_SETTINGS', 'MAP_PICKER_SETTINGS_NS', 'MAP_PICKER_SETTINGS', 'LOADOUT_SETTINGS_NS', 'LOADOUT_SETTINGS', 'PRICE_BOOK_SETTINGS_NS', 'PRICE_BOOK_SETTINGS']],
        ['api/ai/quests/defs/murder/areas.js', ['ARHEIN', 'GUARD_START', 'GUARD_HANDIN', 'SALESMAN', 'SUSPECTS', 'MURDER_EVIDENCE']],
        ['api/ai/clues/data/toolAcquire.js', ['SPADE_SPAWNS', 'PROFESSOR', 'MURPHY', 'KOJO', 'KOJO_EXIT']],
        ['api/ai/quests/engine/QuestEngine.js', ['COIN_FLOAT', 'PROVISION_BANK', 'QuestEngine']],
        ['paint/paintLogic.js', ['paintState', 'WHEEL_ROWS']],
        ['adapter/ClientAdapter.js', ['WELCOME_SCREEN']],
    ];
    for (const [module, members] of newRefusals) {
        for (const member of members) {
            const row = contract.not_impl_debt.find(debt => debt.module === module && debt.member === member);
            assert.ok(row, `${module} must record ${member}`);
            assert.ok(row.source.includes('crates/script/src/shim/'), `${member} names its source file`);
        }
    }
    assert.equal(newRefusals.reduce((count, [, members]) => count + members.length, 0), 26);
    assert.equal(collectModuleNotImplValueExports(registry, shimSources, navFamilies).length, 28);
});

test('an unclassified source module fails closed', () => {
    assert.throws(
        () => classifyModule('api/new-unclassified/Member.js'),
        /Missing ABI family classification/,
    );
});

test('a missing required predecessor fails graph validation', () => {
    const drift = structuredClone(predecessors);
    drift.families[0].predecessors = drift.families[0].predecessors.filter(value => value !== 'T1.3');
    assert.throws(() => validateArtifacts(contract, drift, navFamilies), /missing predecessor T1.3/);
});

test('the frozen-source freshness check runs offline', { skip: !frozenRoot }, () => {
    execFileSync(process.execPath, [path.join(here, 'generate.cjs'), '--check'], {
        cwd: here,
        env: { ...process.env, RS2B0T: frozenRoot, npm_config_offline: 'true' },
        stdio: 'pipe',
    });
});

test('the real generator rejects a seeded nav classification drift', { skip: !frozenRoot }, () => {
    const scratch = fs.mkdtempSync(path.join(os.tmpdir(), 't2-abi-nav-seed-'));
    const sourceCopy = path.join(scratch, 'frozen');
    try {
        fs.mkdirSync(sourceCopy, { recursive: true });
        const archive = execFileSync('git', [
            '-C',
            frozenRoot,
            'archive',
            frozenRevision,
            'LICENSE',
            'src',
            'packages/rs2b0t-api',
        ], { maxBuffer: 64 * 1024 * 1024 });
        execFileSync('tar', ['-x', '-C', sourceCopy], { input: archive });
        fs.writeFileSync(path.join(scratch, 'REFERENCE.json'), JSON.stringify({
            repository: frozenRepository,
            commit: frozenRevision,
            method: `git archive ${frozenRevision} | tar -x`,
            export: fs.realpathSync(sourceCopy),
        }, null, 2));
        const special = path.join(sourceCopy, 'src/bot/api/combat/Special.ts');
        const source = fs.readFileSync(special, 'utf8');
        assert.doesNotMatch(source, /import\s*\{\s*Traversal\s*\}/);
        fs.writeFileSync(special, `import { Traversal } from '../walking/Traversal.js';\n${source}`);
        assert.throws(
            () => execFileSync(process.execPath, [path.join(here, 'generate.cjs'), '--check'], {
                cwd: here,
                env: { ...process.env, RS2B0T: sourceCopy, npm_config_offline: 'true' },
                stdio: 'pipe',
            }),
            error => {
                const output = `${error.stdout || ''}${error.stderr || ''}`;
                return error.status === 1
                    && /Navigation classification disagrees with code evidence for family combat/.test(output);
            },
        );
    } finally {
        fs.rmSync(scratch, { recursive: true, force: true });
    }
});
test('the real generator follows same-family #/ imports when classifying navigation', { skip: !frozenRoot }, () => {
    const scratch = fs.mkdtempSync(path.join(os.tmpdir(), 't2-abi-nav-hash-alias-seed-'));
    const sourceCopy = path.join(scratch, 'frozen');
    try {
        fs.mkdirSync(sourceCopy, { recursive: true });
        const archive = execFileSync('git', [
            '-C',
            frozenRoot,
            'archive',
            frozenRevision,
            'LICENSE',
            'src',
            'packages/rs2b0t-api',
        ], { maxBuffer: 64 * 1024 * 1024 });
        execFileSync('tar', ['-x', '-C', sourceCopy], { input: archive });
        fs.writeFileSync(path.join(scratch, 'REFERENCE.json'), JSON.stringify({
            repository: frozenRepository,
            commit: frozenRevision,
            method: `git archive ${frozenRevision} | tar -x`,
            export: fs.realpathSync(sourceCopy),
        }, null, 2));
        const partnerTrade = path.join(sourceCopy, 'src/bot/api/trade/PartnerTrade.ts');
        const source = fs.readFileSync(partnerTrade, 'utf8');
        assert.doesNotMatch(source, /import\s*['"]#\/bot\/api\/trade\/PartnerHandoff\.js['"]/);
        fs.writeFileSync(partnerTrade, `import '#/bot/api/trade/PartnerHandoff.js';\n${source}`);
        assert.throws(
            () => execFileSync(process.execPath, [path.join(here, 'generate.cjs'), '--check'], {
                cwd: here,
                env: { ...process.env, RS2B0T: sourceCopy, npm_config_offline: 'true' },
                stdio: 'pipe',
            }),
            error => {
                const output = `${error.stdout || ''}${error.stderr || ''}`;
                return error.status === 1
                    && /Navigation classification disagrees with code evidence for family trade/.test(output);
            },
        );
    } finally {
        fs.rmSync(scratch, { recursive: true, force: true });
    }
});

test('the real generator rejects a walking backend without callback rows', { skip: !frozenRoot }, () => {
    const scratch = fs.mkdtempSync(path.join(os.tmpdir(), 't2-abi-no-callback-nav-seed-'));
    const crateCopy = path.join(scratch, 'script');
    try {
        fs.cpSync(crate, crateCopy, {
            recursive: true,
            filter: sourcePath => {
                const relative = path.relative(crate, sourcePath);
                return relative === '' ||
                    !relative.split(path.sep).some(part => part === 'node_modules' || part === 'target');
            },
        });
        fs.symlinkSync(path.join(here, 'node_modules'), path.join(crateCopy, 'compat-js/node_modules'), 'dir');
        const fire = path.join(crateCopy, 'src/fire.rs');
        const source = fs.readFileSync(fire, 'utf8');
        assert.doesNotMatch(source, /use crate::walk::Resilient;/);
        const seeded = source.replace(
            'use crate::machine::{Begin, Cx, Family, Step};',
            'use crate::walk::Resilient;\nuse crate::machine::{Begin, Cx, Family, Step};',
        );
        assert.notEqual(seeded, source);
        fs.writeFileSync(fire, seeded);
        const nodePath = [path.join(here, 'node_modules'), process.env.NODE_PATH]
            .filter(Boolean)
            .join(path.delimiter);
        assert.throws(
            () => execFileSync(process.execPath, [path.join(crateCopy, 'compat-js/generate.cjs'), '--check'], {
                cwd: path.join(crateCopy, 'compat-js'),
                env: {
                    ...process.env,
                    NODE_PATH: nodePath,
                    RS2B0T: frozenRoot,
                    npm_config_offline: 'true',
                },
                stdio: 'pipe',
            }),
            error => {
                const output = `${error.stdout || ''}${error.stderr || ''}`;
                return error.status === 1
                    && /Navigation classification disagrees with code evidence for family firemaking/.test(output);
            },
        );
    } finally {
        fs.rmSync(scratch, { recursive: true, force: true });
    }
});
