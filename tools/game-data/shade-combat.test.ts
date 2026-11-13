import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { extractNpcNamesFacts } from './extractors/npc-names.ts';
import { parseCombatScripts } from './extractors/combat.ts';

// Mort'ton's shadows and shades (content `minigames/game_mortton`): a shadow wakes as its
// `next_npc_type` shade when it finds a player and the shade attacks with a shared melee roll that
// records combat variables only on a hit. The routine bodies below are verbatim content; the
// extractor pins them, so any change must fail closed to bespoke / unknown.

const engineNpcTypeSource = `export default class NpcType {
    stats = [1, 1, 1, 1, 1, 1];
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

const npcConfig = `[shadow]
name=Shadow
category=shade
op2=Attack
attack=45
defence=26
strength=30
hitpoints=38
param=damagetype,^crush_style
param=next_npc_type,shade
[shade]
name=Shade
category=shade
op2=Attack
attack=45
defence=26
strength=30
hitpoints=38
param=damagetype,^crush_style
param=shadeshadow,shadow
[poisoned_shade]
name=Poisoned shade
category=shade
op2=Attack
strength=30
param=poison_severity,10
[altered_shadow]
name=Altered shadow
category=altered
op2=Attack
strength=30
param=next_npc_type,shade
[orphan_shadow]
name=Orphan shadow
category=shade
op2=Attack
strength=30
param=next_npc_type,not_in_pack
`;

const shadeScript = `[ai_opplayer2,_shade]
if(npc_param(next_npc_type) = null) {
    ~shade_attack;
} else {
    //They did not do the shade rise anim when transforming from hunting in old videos
    //https://youtu.be/3d4dxPVz-7k?si=VjWR5sOid5NHU_V-&t=411
    //https://youtu.be/Cgfa0ebrSac?si=mHSBgvg_rRdqOm1J&t=70
    if (npc_stat(hitpoints) = 0) {
        return;
    }

    if(%npc_aggressive_player ! uid & testbit(%morttonmulti, ^shadeattack) = ^false) {
        //guess on distance.
        .npc_findall(npc_coord, npc_type, 10, ^vis_lineofsight);
        while (.npc_findnext = true) {
            if(.npc_getmode = wander) {
                .npc_setmode(opplayer2);
                .%npc_aggressive_player = uid;
            }
        }
    }

    %morttonmulti = setbit(%morttonmulti, ^shadeattack);
    clearqueue(shadeattack_reset);
    longqueue(shadeattack_reset, 100, 0, ^accelerate);
    sound_synth(shade_appear, 1, 0); //This is area sound in osrs. Should it exist in 04?
    npc_changetype(npc_param(next_npc_type), 500);
    %npc_attacking_uid = uid;
    npc_queue(14, 0, 95);
    ~shade_attack;
}

[proc,shade_attack]
if (%npc_action_delay > map_clock) {
    return;
}
if (~npc_check_notcombat = false) {
    npc_changetype(npc_param(shadeshadow), ^max_32bit_int);
    npc_setmode(null);
    %npc_int2 = 0;
    if (npc_type ! shadeshadow_level1) {
        npc_statheal(hitpoints, 0, 100);
    }
    return;
}
if (~npc_check_notcombat_self = false) {
    return;
}
anim(%com_defendanim, 0);
npc_anim(npc_param(attack_anim), 0);
if (npc_param(attack_sound) ! null) {
    sound_synth(npc_param(attack_sound), 1, 0);
}
~shade_meleeattack;

[proc,shade_meleeattack]
def_int $attack_roll = ~npc_melee_attack_roll;
def_int $defence_roll = ~player_defence_roll_specific(npc_param(damagetype));
def_int $maxhit = ~npc_melee_maxhit;

def_int $damage = 0;
def_boolean $hit = false;
if (randominc($attack_roll) > randominc($defence_roll)) {
    $damage = randominc($maxhit);
    $hit = true;
}

~shade_playerhit($damage, npc_param(attackrate), $hit);

[proc,shade_playerhit](int $damage, int $delay, boolean $hit)
if_close;
%npc_action_delay = add(map_clock, $delay);

if (npc_stat(hitpoints) = 0) {
    return;
}

//Special shade logic... They only set combat vars on a successful hit. 
//Other npcs can start combat if they miss multiple times in a row
if ($hit = true) {
    ~npc_set_attack_vars;
    queue*(playerhit_n_retaliate, 0)(npc_uid);
}

if (~check_protect_prayer(^melee_style) = true) { 
    $damage = 0;
}

//OSRS moved this to inside the hit check at some point after ~2021?
//In all old videos it always shows the hitsplat, even if the player does not actively start combat / retaliate because the attack missed.
//https://youtu.be/Cgfa0ebrSac?si=zvUg4j1hTfAje3Bb&t=73
queue(combat_damage_player, 0, $damage);

if ($damage > 0) {
    spotanim_pl(shade_attack, 0, 0);
    
    // 1/20 chance to drain strength by 1 https://x.com/JagexAsh/status/1698259314773733573
    if (random(20) = 0) {
        stat_sub(strength, 1, 0);
        mes("You feel weakened.");
    }
}

[proc,npc_melee_attack_roll]()(int)
def_int $attackbonus = npc_param(attackbonus);
def_int $effective_attack = ~combat_effective_stat(npc_stat(attack), 100); // no prayerbonus
$effective_attack = add($effective_attack, 9); // 'style bonus' of 1
//npcs are essentially always on controlled
return(~combat_stat($effective_attack, $attackbonus));

// perform a basic melee attack from the npc to the player.

[ai_opplayer2,_altered]
if(npc_param(next_npc_type) = null) {
    ~shade_attack;
} else {
    npc_changetype(npc_param(next_npc_type), 500);
    ~shade_attack;
}
[proc,npc_melee_maxhit]()(int)
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
`;

function fixture(script = shadeScript): string {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'shade-combat-'));
    fs.mkdirSync(path.join(root, 'pack'), { recursive: true });
    fs.mkdirSync(path.join(root, 'scripts/skill_combat/configs'), { recursive: true });
    fs.mkdirSync(path.join(root, 'scripts/npc/configs'), { recursive: true });
    fs.mkdirSync(path.join(root, 'scripts/npc/scripts'), { recursive: true });
    fs.writeFileSync(path.join(root, 'pack/npc.pack'), [
        '1=shadow',
        '2=shade',
        '3=poisoned_shade',
        '4=altered_shadow',
        '5=orphan_shadow',
        '477=khazard_warlord',
        '',
    ].join('\n'));
    fs.writeFileSync(path.join(root, 'scripts/skill_combat/configs/combat.param'), combatParams);
    fs.writeFileSync(path.join(root, 'scripts/skill_combat/configs/npc_combat.param'), '[damagetype]\ntype=int\ndefault=2\n');
    fs.writeFileSync(path.join(root, 'scripts/skill_combat/configs/combat_damagetypes.constant'), '^stab_style=0\n^slash_style=1\n^crush_style=2\n');
    fs.writeFileSync(path.join(root, 'scripts/npc/configs/shade.npc'), npcConfig);
    fs.writeFileSync(path.join(root, 'scripts/npc/scripts/shade.rs2'), script);
    return root;
}

function rows(script?: string) {
    const root = fixture(script);
    try {
        const facts = extractNpcNamesFacts(root, 289, engineNpcTypeSource, parseCombatScripts(root), new Map());
        return (config: string) => {
            const found = facts.rows.find((entry) => entry.config === config);
            assert.ok(found, `missing ${config}`);
            return found;
        };
    } finally {
        fs.rmSync(root, { recursive: true, force: true });
    }
}

const row = rows();
// Old: both rows were bespoke with an unknown awakening (MissingFacts at admission).
assert.deepEqual(row('shadow').combat_transform, { kind: 'known', npc_id: 2 }, 'a shadow wakes as its next_npc_type shade');
assert.equal(row('shade').combat_transform, null, 'a shade attacks in place');
for (const config of ['shadow', 'shade']) {
    assert.equal(row(config).bespoke, false, `${config}: the pinned shade attack is a shared melee roll`);
    assert.equal(row(config).combat_vars_on_hit, true, `${config}: combat variables are recorded only on a hit`);
    assert.equal(row(config).attack_kind, null);
    assert.equal(row(config).forced_max_hit, null);
}
assert.deepEqual([row('shade').attackrate, row('shade').strengthbonus, row('shade').attackbonus], [4, 0, 0], 'content defaults apply');
assert.equal(row('khazard_warlord').combat_vars_on_hit, false, 'an ordinary row does not record on hit only');

// Both directions: anything that is not the pinned shade handler stays bespoke and unknown.
assert.equal(row('poisoned_shade').bespoke, true, 'poison keeps the shade attack bespoke');
assert.equal(row('poisoned_shade').combat_vars_on_hit, false);
assert.equal(row('altered_shadow').bespoke, true, 'a different handler is not the pinned shade attack');
assert.deepEqual(row('altered_shadow').combat_transform, { kind: 'unknown' }, 'an unpinned type change fails closed');
assert.deepEqual(row('orphan_shadow').combat_transform, { kind: 'unknown' }, 'a target outside the pack fails closed');

const changed = rows(shadeScript.replace('if (random(20) = 0) {', 'if (random(2) = 0) {'));
for (const config of ['shadow', 'shade']) {
    assert.equal(changed(config).bespoke, true, `${config}: a changed pinned routine is bespoke again`);
    assert.equal(changed(config).combat_vars_on_hit, false);
}
assert.deepEqual(changed('shadow').combat_transform, { kind: 'unknown' }, 'a changed pinned routine leaves the awakening unknown');
