'use strict';

const assert = require('node:assert/strict');
const { execFileSync } = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const test = require('node:test');
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
}) : null;

test('the generated ABI contract and family graph are complete', () => {
    validateArtifacts(contract, predecessors, navFamilies);
    const nav = new Set(predecessors.families.filter(row => row.nav_consuming).map(row => row.family_id));
    for (const family of ['bank', 'shop', 'dialogue', 'thieving', 'trade', 'acquisition', 'runtime', 'cooking']) {
        assert.ok(nav.has(family), `${family} reaches a walker or route service`);
    }
    assert.ok(nav.has('hunt'), 'hunt retains its own nav classification');
    assert.ok(!nav.has('combat'), 'combat has no nav use outside the separately classified hunt family');
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
    assert.equal(contract.not_impl_debt.length, 212);
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
