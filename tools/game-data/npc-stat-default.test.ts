import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { extractNpcNamesFacts } from './extractors/npc-names.ts';
import { parseCombatScripts } from './extractors/combat.ts';

const engineNpcTypeSource = `export default class NpcType {
    stats = [1, 1, 1, 1, 1, 1];
}`;
const missingHitpointsEngineNpcTypeSource = `export default class NpcType {
    stats = [1, 1, 1];
}`;

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
[max_dealt]
type=int
default=1000
`;

const npcConfig = `[plain_attacker]
name=Plain attacker
op1=Attack
hitpoints=2
param=stabdefence,-1
param=max_dealt,3
[explicit_zero]
name=Explicit zero
op1=Attack
attack=0
defence=0
strength=0
hitpoints=0
ranged=0
magic=0
param=max_dealt,0
[explicit_value]
name=Explicit value
op1=Attack
attack=20
defence=18
strength=24
hitpoints=33
ranged=3
magic=4
param=damagetype,^stab_style
[bespoke_attacker]
name=Bespoke attacker
op1=Attack
hitpoints=5
[ordinary_melee]
name=Ordinary melee
op1=Attack
strength=35
[default_melee_mode]
name=Default melee mode
op1=Attack
strength=2
param=strengthbonus,7
[poisoned_melee]
name=Poisoned melee
op1=Attack
param=poison_severity,10
[extra_hit]
name=Extra hit
op1=Attack
[multi_melee]
name=Multi melee
op1=Attack
param=npc_forcemulti,1
`;
const bespokeScript = `[ai_applayer2,bespoke_attacker]
npc_changetype(plain_attacker, 1000);
npc_setmode(opplayer2);
[ai_applayer2,plain_attacker]
~npc_default_attack();
[ai_opplayer2,ordinary_melee]
@ordinary_melee_attack;
[label,ordinary_melee_attack]
if (%death = ^true) {
    npc_setmode(null);
    return;
}
anim(%com_defendanim, 0);
~npc_meleeattack;
[ai_applayer2,default_melee_mode]
npc_say("Stand and deliver!");
npc_setmode(opplayer2);
[ai_opplayer2,_] gosub(npc_default_attack);
[proc,npc_default_attack]
if (%death = ^true) {
    npc_setmode(null);
    return;
}
anim(%com_defendanim, 0);
~npc_meleeattack;
[ai_opplayer2,poisoned_melee] @poisoned_melee_attack;
[label,poisoned_melee_attack]
anim(%com_defendanim, 0);
~npc_meleeattack;
[ai_opplayer2,extra_hit]
~npc_meleeattack;
~playerhit_n_melee(true, 99, npc_param(attackrate));
[ai_opplayer2,multi_melee] @multi_melee_attack;
[label,multi_melee_attack]
anim(%com_defendanim, 0);
~npc_meleeattack;
[ai_applayer2,conditional_attacker]
if (random(1) = 0) {
    npc_changetype(plain_attacker, 1000);
    npc_setmode(opplayer2);
} else {
    npc_changetype(second_form, 1000);
    npc_setmode(opplayer2);
}
[ai_applayer2,multiple_handler_attacker] npc_changetype(plain_attacker, 1000); npc_setmode(opplayer2);
[ai_applayer2,multiple_handler_attacker] npc_changetype(plain_attacker, 500); npc_setmode(opplayer2);
[ai_applayer2,multiple_target_attacker]
npc_changetype(plain_attacker, 1000);
npc_changetype(second_form, 1000);
npc_setmode(opplayer2);
[ai_applayer2,dynamic_attacker]
npc_changetype(npc_param(next_npc_type), 1000);
npc_setmode(opplayer2);
[ai_applayer2,missing_target_attacker]
npc_changetype(not_in_pack, 1000);
npc_setmode(opplayer2);
[ai_applayer2,unsafe_helper_attacker] @unsafe_transform;
[label,unsafe_transform]
npc_changetype(plain_attacker, 1000);
npc_setmode(opplayer2);
[ai_applayer2,cycle_a] npc_changetype(cycle_b, 1000); npc_setmode(opplayer2);
[ai_applayer2,cycle_b] npc_changetype(cycle_a, 1000); npc_setmode(opplayer2);
[ai_applayer2,transitive_start] npc_changetype(transitive_middle, 1000); npc_setmode(opplayer2);
[ai_applayer2,transitive_middle] npc_changetype(plain_attacker, 1000); npc_setmode(opplayer2);
[ai_applayer2,wolf_dormant] npc_changetype(wolf_awake, 1000); npc_setmode(opplayer2);
`;

function fixture(npcSource = npcConfig, paramSource = combatParams): string {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'npc-stat-default-'));
    fs.mkdirSync(path.join(root, 'pack'), { recursive: true });
    fs.mkdirSync(path.join(root, 'scripts/skill_combat/configs'), { recursive: true });
    fs.mkdirSync(path.join(root, 'scripts/npc/configs'), { recursive: true });
    fs.mkdirSync(path.join(root, 'scripts/skill_combat/scripts/npc'), { recursive: true });
    fs.mkdirSync(path.join(root, 'scripts/npc/scripts'), { recursive: true });
    fs.writeFileSync(path.join(root, 'pack/npc.pack'), [
        '1=plain_attacker',
        '2=explicit_zero',
        '3=explicit_value',
        '4=bespoke_attacker',
        '5=ordinary_melee',
        '6=default_melee_mode',
        '7=poisoned_melee',
        '8=extra_hit',
        '9=multi_melee',
        '10=conditional_attacker',
        '11=multiple_handler_attacker',
        '12=multiple_target_attacker',
        '13=second_form',
        '14=dynamic_attacker',
        '15=missing_target_attacker',
        '16=cycle_a',
        '17=cycle_b',
        '18=transitive_start',
        '19=transitive_middle',
        '20=wolf_dormant',
        '21=wolf_awake',
        '22=unsafe_helper_attacker',
        '477=khazard_warlord',
        '',
    ].join('\n'));
    fs.writeFileSync(path.join(root, 'scripts/skill_combat/configs/combat.param'), paramSource);
    fs.writeFileSync(path.join(root, 'scripts/npc/configs/stat-default.npc'), npcSource);
    fs.writeFileSync(path.join(root, 'scripts/skill_combat/configs/npc_combat.param'), '[damagetype]\ntype=int\ndefault=2\n');
    fs.writeFileSync(path.join(root, 'scripts/skill_combat/configs/combat_damagetypes.constant'), '^stab_style=0\n^slash_style=1\n^crush_style=2\n');
    fs.writeFileSync(path.join(root, 'scripts/skill_combat/scripts/npc/melee.rs2'), `[proc,npc_melee_maxhit]()(int)
def_int $strengthbonus = npc_param(strengthbonus);
def_int $effective_strength = ~combat_effective_stat(npc_stat(strength), 100);
$effective_strength = add($effective_strength, 9);
def_int $melee_strength = ~combat_stat($effective_strength, $strengthbonus);
return(~combat_maxhit($melee_strength));
[proc,npc_meleeattack]
def_int $attack_roll = ~npc_melee_attack_roll;
def_int $defence_roll = ~player_defence_roll_specific(npc_param(damagetype));
def_int $maxhit = ~npc_melee_maxhit;
if (randominc($attack_roll) > randominc($defence_roll)) {
    ~playerhit_n_melee(true, randominc($maxhit), npc_param(attackrate));
    ~npc_poison_player;
    return;
}
~playerhit_n_melee(true, 0, npc_param(attackrate));
`);
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
    assert.equal(facts.combat_facts_version, 2, `${revision}: emitted combat-fact version`);
    const row = (config: string) => {
        const found = facts.rows.find((entry) => entry.config === config);
        assert.ok(found, `${revision}: missing ${config}`);
        return found;
    };
    assert.equal(row('plain_attacker').combat_transform, null, `${revision}: ordinary NPC has no attack-approach transform`);
    assert.deepEqual(row('bespoke_attacker').combat_transform, { kind: 'known', npc_id: 1 }, `${revision}: direct static target`);
    assert.deepEqual(row('wolf_dormant').combat_transform, { kind: 'known', npc_id: 21 }, `${revision}: non-crab uses the same source form`);
    assert.deepEqual(row('multiple_handler_attacker').combat_transform, { kind: 'known', npc_id: 1 }, `${revision}: repeated handlers with the same target`);
    assert.deepEqual(row('transitive_start').combat_transform, { kind: 'known', npc_id: 1 }, `${revision}: resolve a source-derived target chain`);
    for (const config of ['conditional_attacker', 'multiple_target_attacker', 'dynamic_attacker', 'missing_target_attacker', 'unsafe_helper_attacker', 'cycle_a', 'cycle_b']) {
        assert.deepEqual(row(config).combat_transform, { kind: 'unknown' }, `${revision}: ${config} fails closed`);
    }

    const plain = row('plain_attacker');
    assert.deepEqual(
        [plain.attack, plain.defence, plain.strength, plain.hitpoints, plain.ranged, plain.magic],
        [1, 1, 1, 2, 1, 1],
        `${revision}: omitted levels use the NpcType source default; hitpoints honors its explicit override`,
    );
    // Old: omitted damage type was null. New: read the declared crush default.
    assert.equal(plain.damagetype, '^crush_style', `${revision}: declared NPC damage type default`);
    assert.equal(plain.attackbonus, 0, `${revision}: omitted attackbonus uses combat.param default 0`);
    assert.deepEqual(
        [
            plain.strengthbonus,
            plain.rangebonus,
            plain.attackrate,
            plain.poison_severity,
            plain.forcemulti,
            plain.max_dealt,
        ],
        [0, 0, 4, 0, false, 3],
        `${revision}: omitted values use combat.param defaults and max_dealt keeps its explicit cap`,
    );
    assert.equal(plain.rangeattack, 0);
    assert.equal(plain.magicattack, 0);
    assert.equal(plain.stabdefence, -1, `${revision}: signed NPC defence bonuses are preserved`);
    assert.equal(plain.bespoke, false, `${revision}: a shared attack stays non-bespoke`);
    assert.equal(plain.ops.includes('Attack'), true);

    const ordinaryMelee = row('ordinary_melee');
    assert.equal(ordinaryMelee.bespoke, false, `${revision}: a proved ordinary melee wrapper is not bespoke`);
    assert.equal(ordinaryMelee.attack_kind, null);
    assert.equal(ordinaryMelee.forced_max_hit, null);

    const defaultMelee = row('default_melee_mode');
    assert.equal(defaultMelee.bespoke, false, `${revision}: an AP attack routed to the shared default melee handler is not bespoke`);

    const poisonedMelee = row('poisoned_melee');
    assert.equal(poisonedMelee.bespoke, true, `${revision}: poison keeps the attack bespoke`);

    const extraHit = row('extra_hit');
    assert.equal(extraHit.bespoke, true, `${revision}: an additional direct hit stays bespoke`);
    const multiMelee = row('multi_melee');
    assert.equal(multiMelee.bespoke, true, `${revision}: multi-hit keeps the attack bespoke`);



    const zero = row('explicit_zero');
    assert.deepEqual(
        [zero.attack, zero.defence, zero.strength, zero.hitpoints, zero.ranged, zero.magic],
        [0, 0, 0, 0, 0, 0],
        `${revision}: explicit zero stats override the engine defaults`,
    );
    assert.equal(zero.max_dealt, 0, `${revision}: explicit zero max_dealt is preserved`);

    const valued = row('explicit_value');
    assert.deepEqual(
        [valued.attack, valued.defence, valued.strength, valued.hitpoints, valued.ranged, valued.magic],
        [20, 18, 24, 33, 3, 4],
        `${revision}: explicit combat levels override the engine defaults`,
    );
    assert.equal(valued.max_dealt, 1000, `${revision}: omitted max_dealt uses its declared default`);
    assert.equal(valued.damagetype, '^stab_style', `${revision}: explicit damage type overrides its default`);

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
    assert.deepEqual(
        [
            absent.strengthbonus,
            absent.rangebonus,
            absent.attackrate,
            absent.poison_severity,
            absent.forcemulti,
            absent.max_dealt,
        ],
        [0, 0, 4, 0, false, 1000],
        `${revision}: a pack row without a content block uses declared combat defaults`,
    );
}

function facts_damage_type_requires_declared_default() {
    // Old: extraction succeeded without this default. New: it throws.
    const root = fixture();
    try {
        fs.writeFileSync(path.join(root, 'scripts/skill_combat/configs/npc_combat.param'), '[damagetype]\ntype=int\n');
        assert.throws(
            () => extractNpcNamesFacts(root, 289, engineNpcTypeSource, scripts, maxHits),
            /damagetype npc_combat\.param default is missing/,
        );
    } finally {
        fs.rmSync(root, { recursive: true, force: true });
    }
}

facts_damage_type_requires_declared_default();

function facts_missing_fact_fails_generation() {
    const facts = extractNpcNamesFacts(content, 274, engineNpcTypeSource, scripts, maxHits);
    const plain = facts.rows.find((row) => row.config === 'plain_attacker');
    assert.ok(plain, 'plain attacker row exists');
    assert.equal(plain.attackbonus, 0, 'omitted attackbonus uses its declared combat.param default');

    const requiredDefaults = [
        ['attackbonus', '0'],
        ['rangeattack', '0'],
        ['magicattack', '0'],
        ['stabdefence', '0'],
        ['slashdefence', '0'],
        ['crushdefence', '0'],
        ['magicdefence', '0'],
        ['rangedefence', '0'],
        ['strengthbonus', '0'],
        ['rangebonus', '0'],
        ['attackrate', '4'],
        ['poison_severity', '0'],
        ['npc_forcemulti', '0'],
        ['max_dealt', '1000'],
    ] as const;
    for (const [name, value] of requiredDefaults) {
        const expected = `[${name}]\ntype=int\ndefault=${value}`;
        const missing = combatParams.replace(expected, `[${name}]\ntype=int`);
        assert.notEqual(missing, combatParams, `${name} fixture default exists`);
        const root = fixture(npcConfig, missing);
        try {
            assert.throws(
                () => extractNpcNamesFacts(root, 274, engineNpcTypeSource, scripts, maxHits),
                new RegExp(`${name} combat\\.param default is missing`),
                `${name}: missing declared combat default fails generation`,
            );
        } finally {
            fs.rmSync(root, { recursive: true, force: true });
        }
    }

    // The same child-mode invocation exits 0 on the previous extractor and fails
    // on this one because the pinned NpcType source omits the hitpoints default.
    const child = spawnSync(process.execPath, [import.meta.filename, '--missing-hitpoints-child'], {
        cwd: path.resolve(import.meta.dirname, '../..'),
        encoding: 'utf8',
    });
    assert.ok(
        child.status !== null && child.status !== 0,
        `missing engine hitpoints default exits nonzero in a subprocess (status ${String(child.status)})`,
    );
    assert.match(
        String(child.stderr),
        /missing the engine default for hitpoints/,
        'the subprocess reports the absent engine hitpoints default',
    );
}

facts_missing_fact_fails_generation();
