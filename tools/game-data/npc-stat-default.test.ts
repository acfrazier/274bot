import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { extractNpcNamesFacts } from './extractors/npc-names.ts';

/**
 * Engine stat default (`NpcType.ts:106` on 289, `:107` on 274).
 * An absent `strength=` / `ranged=` line becomes 1. An explicit value, including 0, is kept.
 * A pack row with no `.npc` block stays absent. A bespoke attack stays bespoke.
 */

const combatParams = `[strengthbonus]
type=int
default=0
[rangebonus]
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
[explicit_zero]
name=Explicit zero
op1=Attack
strength=0
ranged=0
[explicit_value]
name=Explicit value
op1=Attack
strength=24
ranged=3
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

function fixture(): string {
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
    fs.writeFileSync(path.join(root, 'scripts/skill_combat/configs/combat.param'), combatParams);
    fs.writeFileSync(path.join(root, 'scripts/npc/configs/stat-default.npc'), npcConfig);
    fs.writeFileSync(path.join(root, 'scripts/npc/scripts/stat-default.rs2'), bespokeScript);
    return root;
}

const content = fixture();
for (const revision of [274, 289] as const) {
    const facts = extractNpcNamesFacts(content, revision);
    const row = (config: string) => {
        const found = facts.rows.find((entry) => entry.config === config);
        assert.ok(found, `${revision}: missing ${config}`);
        return found;
    };

    const plain = row('plain_attacker');
    assert.equal(plain.strength, 1, `${revision}: an absent strength= line takes the engine default`);
    assert.equal(plain.ranged, 1, `${revision}: an absent ranged= line takes the engine default`);
    assert.equal(plain.bespoke, false, `${revision}: a shared attack stays non-bespoke`);
    assert.equal(plain.ops.includes('Attack'), true);

    const zero = row('explicit_zero');
    assert.equal(zero.strength, 0, `${revision}: an explicit strength=0 is kept`);
    assert.equal(zero.ranged, 0, `${revision}: an explicit ranged=0 is kept`);

    const valued = row('explicit_value');
    assert.equal(valued.strength, 24, `${revision}: an explicit strength is kept`);
    assert.equal(valued.ranged, 3, `${revision}: an explicit ranged is kept`);

    const bespoke = row('bespoke_attacker');
    assert.equal(bespoke.bespoke, true, `${revision}: a type-change attack stays bespoke`);
    assert.equal(bespoke.attack_kind, null, `${revision}: the bespoke row's attack kind is unchanged`);
    assert.equal(bespoke.forced_max_hit, null, `${revision}: the bespoke row's forced hit is unchanged`);
    assert.equal(bespoke.strength, 1, `${revision}: the bespoke row still takes the engine strength default`);
    assert.equal(bespoke.ranged, 1, `${revision}: the bespoke row still takes the engine ranged default`);

    const absent = row('khazard_warlord');
    assert.equal(absent.strength, null, `${revision}: a pack row with no config block stays absent`);
    assert.equal(absent.ranged, null, `${revision}: a pack row with no config block stays absent`);
}
