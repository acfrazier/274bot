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
~npc_changetype(plain_attacker, 1000);
~npc_setmode(opplayer2);
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
`;
function fixture(): string {
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
        '477=khazard_warlord',
        '',
    ].join('\n'));
    fs.writeFileSync(path.join(root, 'scripts/skill_combat/configs/combat.param'), combatParams);
    fs.writeFileSync(path.join(root, 'scripts/npc/configs/stat-default.npc'), npcConfig);
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
