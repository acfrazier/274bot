import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

import { extractNpcNamesFacts } from './extractors/npc-names.ts';

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
wanderrange=9
[branch_attacker]
name=Branch attacker
[dynamic_spell]
name=Dynamic spell
[partial_attacker]
name=Partial attacker
[unknown_attacker]
name=Unknown attacker
[queue_retaliator]
name=Queue retaliator
`;

const combatScript = `[ai_queue1,queue_retaliator]
gosub(npc_default_retaliate_ap);

[ai_applayer2,plain_attacker]
gosub(npc_default_attack);

[ai_opplayer2,branch_attacker]
@branch_attack_label;
[label,branch_attack_label]
jump(ranged_attack_label);
[label,ranged_attack_label]
~npc_rangeattack(2, ^true);

[ai_opplayer2,dynamic_spell]
~npc_cast_spell($spell, 0, 0);

[ai_applayer2,partial_attacker]
@unknown_attack;
[ai_opplayer2,partial_attacker]
~npc_rangeattack(2, ^true);

[ai_opplayer2,unknown_attacker]
gosub(unknown_attack_body);
[proc,unknown_attack_body]
npc_setmode(null);
`;

function writeFixtureFiles(root: string, files: ReadonlyMap<string, string>) {
    for (const [relative, text] of files) {
        const file = path.join(root, relative);
        fs.mkdirSync(path.dirname(file), { recursive: true });
        fs.writeFileSync(file, text);
    }
}

const content = fs.mkdtempSync(path.join(os.tmpdir(), 'combat-377-'));
writeFixtureFiles(content, new Map([
    ['pack/npc.pack', [
        '1=plain_attacker',
        '2=branch_attacker',
        '3=dynamic_spell',
        '4=partial_attacker',
        '5=unknown_attacker',
        '6=queue_retaliator',
        '477=khazard_warlord',
        '',
    ].join('\n')],
    ['scripts/skill_combat/configs/combat.param', combatParams],
    ['scripts/npc/configs/combat-test.npc', npcConfig],
    ['scripts/npc/scripts/combat-test.rs2', combatScript],
]));

const facts377 = extractNpcNamesFacts(content, 377);
const row377 = (config: string) => {
    const row = facts377.rows.find((candidate) => candidate.config === config);
    assert.ok(row, `377: missing ${config}`);
    return row;
};

const plain377 = row377('plain_attacker');
assert.equal(plain377.maxrange, 11, '377 NpcType postDecode replaces -1 with wander+2');
assert.equal(plain377.strength, 1, '377 NpcType defaults strength to one');
assert.equal(plain377.ranged, 1, '377 NpcType defaults ranged to one');
assert.equal(plain377.attackrate, 4, 'NPC attackrate comes from combat.param');
assert.equal(plain377.bespoke, false, 'a 377 gosub to the shared melee proc is not bespoke');

assert.equal(row377('branch_attacker').attack_kind, 'ranged', 'an @ label alias and jump reach the ranged helper');
assert.equal(row377('dynamic_spell').attack_kind, 'magic');
assert.equal(row377('dynamic_spell').forced_max_hit, null, 'a dynamic spell id has no statically known max hit');
assert.equal(row377('unknown_attacker').attack_kind, null, 'an unclassified 377 attack remains UNKNOWN');
assert.equal(row377('partial_attacker').attack_kind, null, 'an unknown branch prevents a partial style from becoming a complete fact');
assert.equal(row377('queue_retaliator').ap_attack, true, '377 ai_queue1 can invoke the AP retaliation helper');
assert.equal(row377('queue_retaliator').bespoke, false, 'retaliation queue code is not itself an attack handler');
assert.equal(row377('khazard_warlord').maxrange, 7, 'a missing NPC block uses the 377 default wander range and postDecode maxrange');
assert.equal(row377('khazard_warlord').strength, null, 'a missing NPC block is not assigned decoder stat defaults');

const facts289 = extractNpcNamesFacts(content, 289);
const row289 = (config: string) => {
    const row = facts289.rows.find((candidate) => candidate.config === config);
    assert.ok(row, `289: missing ${config}`);
    return row;
};
assert.equal(row289('partial_attacker').attack_kind, 'mixed', 'the 289 unknown-section melee fallback stays unchanged');
assert.equal(row289('queue_retaliator').ap_attack, true, 'legacy queue retaliation detection remains unchanged');
assert.equal(row289('queue_retaliator').bespoke, false, 'legacy shared queue retaliation remains non-bespoke');
assert.equal(row289('plain_attacker').attackrate, 4, 'the content-derived attackrate default preserves 289 output');
