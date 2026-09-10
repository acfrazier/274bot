import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { extractFacts, parseRows } from './generate.ts';

const rows = parseRows(`
// repeated aliases and typed tuples
[food]
data=consumable,bread
data=consumable,anchovies
data=stat_heal,hitpoints,4,0
data=stat_heal,energy,2,0
data=stat_change,attack,1,5
`);
assert.equal(rows.length, 1);
assert.deepEqual(rows[0].values.consumable, [['bread'], ['anchovies']]);
assert.deepEqual(rows[0].values.stat_heal[1], ['energy', '2', '0']);

const content = fs.mkdtempSync(path.join(os.tmpdir(), 'game-data-fixture-'));
const consumption = path.join(content, 'scripts/player/configs/consumption');
const thieving = path.join(content, 'scripts/skill_thieving/configs/pickpocking');
fs.mkdirSync(consumption, { recursive: true });
fs.mkdirSync(thieving, { recursive: true });
fs.writeFileSync(path.join(consumption, 'consume_normal.dbrow'), `[food]\ndata=consumable,bread\ndata=consumable,anchovies\ndata=stat_heal,hitpoints,4,0\n[zero]\ndata=consumable,zero_food\ndata=stat_heal,hitpoints,0,0\n`);
fs.writeFileSync(path.join(consumption, 'consume_effects.dbrow'), `[potion]\ndata=consumable,potion\ndata=stat_change,attack,1,5\ndata=stat_heal,hitpoints,5,10\ndata=healenergy,3\n`);
fs.writeFileSync(path.join(thieving, 'pickpocket.dbrow'), `[guard]\ndata=npc,guard1\ndata=npc,guard2\ndata=level,40\ndata=experience,30\ndata=stun_ticks,8\ndata=stun_damage,3\ndata=success_chance,1,2\ndata=pocket,coins\ndata=loot,coin,1,5,10\ndata=loot,coin,2,3,20\n`);
const facts = extractFacts(content,
    [
        { id: 1, debugname: 'bread', name: 'Bread', cost: 0, stackable: false, members: false, certlink: -1, certtemplate: -1, wearpos: -1, wearpos2: -1, wearpos3: -1 },
        { id: 2, debugname: 'anchovies', name: 'Anchovies', cost: 0, stackable: false, members: false, certlink: -1, certtemplate: -1, wearpos: -1, wearpos2: -1, wearpos3: -1 },
        { id: 3, debugname: 'zero_food', name: 'Zero food', cost: 0, stackable: false, members: false, certlink: -1, certtemplate: -1, wearpos: -1, wearpos2: -1, wearpos3: -1 },
        { id: 4, debugname: 'potion', name: 'Potion', cost: 0, stackable: false, members: false, certlink: -1, certtemplate: -1, wearpos: -1, wearpos2: -1, wearpos3: -1 },
        { id: 5, debugname: 'coin', name: 'Coins', cost: 0, stackable: true, members: false, certlink: -1, certtemplate: -1, wearpos: -1, wearpos2: -1, wearpos3: -1 },
    ],
    [
        { id: 10, debugname: 'guard1', name: 'Guard' },
        { id: 11, debugname: 'guard2', name: 'Guard' },
    ],
);
assert.equal(facts.consumption.length, 4);
assert.equal(facts.consumption.filter((fact) => fact.qualification === 'fixed_hp_heal').length, 2);
assert.equal(facts.consumption.find((fact) => fact.item.alias === 'zero_food')?.qualification, 'not_fixed_hp_heal');
assert.equal(facts.consumption.find((fact) => fact.item.alias === 'potion')?.stat_change[0].percent, 5);
assert.equal(facts.pickpocket[0].npcs.length, 2);
assert.equal(facts.pickpocket[0].loot.length, 2);
assert.deepEqual(facts.pickpocket[0].success_chance, { numerator: 1, denominator: 2 });
console.log('generate fixture passed');
