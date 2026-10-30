import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { extractNpcNamesFacts } from './extractors/npc-names.ts';
import { parseCombatScripts } from './extractors/combat.ts';

const engineNpcTypeSource = 'export default class NpcType { stats = [1, 1, 1, 1, 1, 1]; }';
const missingHitpointsEngineNpcTypeSource = 'export default class NpcType { stats = [1, 1, 1]; }';

const combatParams = `[strengthbonus]
type=int
default=0
[rangebonus]
type=int
default=0
[attackbonus]
type=int
default=0
[rangeattack]
type=int
default=0
[magicattack]
type=int
default=0
[stabdefence]
type=int
default=0
[slashdefence]
type=int
default=0
[crushdefence]
type=int
default=0
[magicdefence]
type=int
default=0
[rangedefence]
type=int
default=0
[attackrate]
type=int
default=4
[poison_severity]
type=int
default=0
[npc_forcemulti]
type=int
default=0
`;

const npcConfig = `[plain_attacker]
name=Plain attacker
op1=Attack
hitpoints=2
param=stabdefence,-1
[explicit_zero]
name=Explicit zero
op1=Attack
attack=0
defence=0
strength=0
hitpoints=0
ranged=0
magic=0
[explicit_value]
name=Explicit value
op1=Attack
attack=20
defence=18
strength=24
hitpoints=33
ranged=3
magic=4
[bespoke_attacker]
name=Bespoke attacker
op1=Attack
hitpoints=5
`;

const bespokeScript = `[ai_applayer2,bespoke_attacker]
~npc_changetype(plain_attacker, 1000);
~npc_setmode(opplayer2);
[ai_applayer2,plain_attacker]
~npc_default_attack();
`;

function fixture(npcSource = npcConfig, paramSource = combatParams): string {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'npc-stat-default-'));
    fs.mkdirSync(path.join(root, 'pack'), { recursive: true });
    fs.mkdirSync(path.join(root, 'scripts/skill_combat/configs'), { recursive: true });
    fs.mkdirSync(path.join(root, 'scripts/npc/configs'), { recursive: true });
    fs.mkdirSync(path.join(root, 'scripts/npc/scripts'), { recursive: true });
    fs.writeFileSync(path.join(root, 'pack/npc.pack'), [
        '1=plain_attacker',
        '2=explicit_zero',
        '3=explicit_value',
        '4=bespoke_attacker',
        '477=khazard_warlord',
        '',
    ].join('\n'));
    fs.writeFileSync(path.join(root, 'scripts/skill_combat/configs/combat.param'), paramSource);
    fs.writeFileSync(path.join(root, 'scripts/npc/configs/stat-default.npc'), npcSource);
    fs.writeFileSync(path.join(root, 'scripts/npc/scripts/stat-default.rs2'), bespokeScript);
    return root;
}
if (process.argv[2] === '--missing-hitpoints-child') {
    const childContent = fixture(npcConfig.replace('hitpoints=2\n', ''));
    try {
        const childScripts = parseCombatScripts(childContent);
        const extract = extractNpcNamesFacts as unknown as (...args: unknown[]) => unknown;
        try {
            // The baseline call shape accepts the fixture without an engine stats source.
            extract(childContent, 274, childScripts, new Map());
        } catch (error) {
            if (!(error instanceof TypeError)) throw error;
            // The new call shape requires the source and fails on its missing hitpoints slot.
            extract(childContent, 274, missingHitpointsEngineNpcTypeSource, childScripts, new Map());
        }
    } finally {
        fs.rmSync(childContent, { recursive: true, force: true });
    }
    process.exit(0);
}

const content = fixture();
const scripts = parseCombatScripts(content);
const maxHits = new Map<string, number>();
for (const revision of [274, 289] as const) {
    const facts = extractNpcNamesFacts(content, revision, engineNpcTypeSource, scripts, maxHits);
    assert.equal(facts.combat_facts_version, 1, `${revision}: emitted combat-fact version`);
    const row = (config: string) => {
        const found = facts.rows.find((entry) => entry.config === config);
        assert.ok(found, `${revision}: missing ${config}`);
        return found;
    };

    const plain = row('plain_attacker');
    assert.deepEqual(
        [plain.attack, plain.defence, plain.strength, plain.hitpoints, plain.ranged, plain.magic],
        [1, 1, 1, 2, 1, 1],
        `${revision}: omitted levels use the NpcType source default; hitpoints honors its explicit override`,
    );
    assert.equal(plain.attackbonus, 0, `${revision}: omitted attackbonus uses combat.param default 0`);
    assert.equal(plain.rangeattack, 0);
    assert.equal(plain.magicattack, 0);
    assert.equal(plain.stabdefence, -1, `${revision}: signed NPC defence bonuses are preserved`);
    assert.equal(plain.bespoke, false, `${revision}: a shared attack stays non-bespoke`);
    assert.equal(plain.ops.includes('Attack'), true);

    const zero = row('explicit_zero');
    assert.deepEqual(
        [zero.attack, zero.defence, zero.strength, zero.hitpoints, zero.ranged, zero.magic],
        [0, 0, 0, 0, 0, 0],
        `${revision}: explicit zero stats override the engine defaults`,
    );

    const valued = row('explicit_value');
    assert.deepEqual(
        [valued.attack, valued.defence, valued.strength, valued.hitpoints, valued.ranged, valued.magic],
        [20, 18, 24, 33, 3, 4],
        `${revision}: explicit combat levels override the engine defaults`,
    );

    const bespoke = row('bespoke_attacker');
    assert.equal(bespoke.bespoke, true, `${revision}: a type-change attack stays bespoke`);
    assert.equal(bespoke.attack_kind, null, `${revision}: the bespoke row's attack kind is unchanged`);
    assert.equal(bespoke.forced_max_hit, null, `${revision}: the bespoke row's forced hit is unchanged`);
    assert.deepEqual([bespoke.attack, bespoke.defence, bespoke.magic], [1, 1, 1]);

    const absent = row('khazard_warlord');
    assert.deepEqual(
        [absent.attack, absent.defence, absent.strength, absent.hitpoints, absent.ranged, absent.magic],
        [1, 1, 1, 1, 1, 1],
        `${revision}: a pack row without a content block still uses engine stat defaults`,
    );
}

const missingAttackbonusDefault = fixture(
    npcConfig,
    combatParams.replace('[attackbonus]\ntype=int\ndefault=0', '[attackbonus]\ntype=int'),
);
try {
    assert.throws(
        () => extractNpcNamesFacts(missingAttackbonusDefault, 274, engineNpcTypeSource, scripts, maxHits),
        /attackbonus combat\.param default is missing/,
        'the generator rejects a missing required attackbonus content default',
    );
} finally {
    fs.rmSync(missingAttackbonusDefault, { recursive: true, force: true });
}
// The same child-mode invocation exits 0 on the previous extractor and fails
// on this one because the pinned NpcType source omits the hitpoints default.
assert.throws(
    () => execFileSync(process.execPath, [import.meta.filename, '--missing-hitpoints-child'], {
        cwd: path.resolve(import.meta.dirname, '../..'),
        encoding: 'utf8',
    }),
    (error: unknown) => {
        const failure = error as { status?: number; stderr?: string | Buffer };
        return failure.status !== undefined
            && failure.status !== 0
            && /missing the engine default for hitpoints/.test(String(failure.stderr));
    },
    'missing engine hitpoints default fails in a subprocess even though the NPC row omits hitpoints',
);
