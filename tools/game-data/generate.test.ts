import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { extractFacts, extractMagicFacts, parseRows } from './generate.ts';

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

const combat = path.join(content, 'scripts/skill_combat/configs/magic');
const magic = path.join(content, 'scripts/skill_magic/configs');
fs.mkdirSync(combat, { recursive: true });
fs.mkdirSync(magic, { recursive: true });
fs.writeFileSync(path.join(combat, 'magic_combat_spells.dbrow'), `[magic_spell_wind_strike]
data=name,Wind Strike
data=levelrequired,1
data=runesrequired,mindrune,1,airrune,1,null,null
data=continue_by_autocast,true
[magic_spell_iban]
data=levelrequired,50
data=runesrequired,firerune,1,null,null,null,null
data=continue_by_autocast,true
[magic_spell_fire_wave]
data=name,Fire Wave
data=levelrequired,75
data=runesrequired,bloodrune,1,firerune,7,airrune,5
data=continue_by_autocast,true
`);
fs.writeFileSync(path.join(magic, 'magic_staff.dbrow'), `[magic_staff_fire]
data=staff,staff_of_fire
data=staff,lava_battlestaff
data=rune,firerune
[magic_staff_earth]
data=staff,lava_battlestaff
data=rune,earthrune
[magic_staff_air]
data=staff,staff_of_air
data=rune,airrune
`);
const magicItems = [
    { id: 1, debugname: 'mindrune', name: 'Mind rune', cost: 0, stackable: true, members: false, certlink: -1, certtemplate: -1, wearpos: -1, wearpos2: -1, wearpos3: -1 },
    { id: 2, debugname: 'airrune', name: 'Air rune', cost: 0, stackable: true, members: false, certlink: -1, certtemplate: -1, wearpos: -1, wearpos2: -1, wearpos3: -1 },
    { id: 3, debugname: 'firerune', name: 'Fire rune', cost: 0, stackable: true, members: false, certlink: -1, certtemplate: -1, wearpos: -1, wearpos2: -1, wearpos3: -1 },
    { id: 4, debugname: 'bloodrune', name: 'Blood rune', cost: 0, stackable: true, members: false, certlink: -1, certtemplate: -1, wearpos: -1, wearpos2: -1, wearpos3: -1 },
    { id: 5, debugname: 'earthrune', name: 'Earth rune', cost: 0, stackable: true, members: false, certlink: -1, certtemplate: -1, wearpos: -1, wearpos2: -1, wearpos3: -1 },
    { id: 6, debugname: 'staff_of_fire', name: 'Staff of fire', cost: 0, stackable: false, members: false, certlink: -1, certtemplate: -1, wearpos: 3, wearpos2: -1, wearpos3: -1 },
    { id: 7, debugname: 'lava_battlestaff', name: 'Lava battlestaff', cost: 0, stackable: false, members: false, certlink: -1, certtemplate: -1, wearpos: 3, wearpos2: -1, wearpos3: -1 },
    { id: 8, debugname: 'staff_of_air', name: 'Staff of air', cost: 0, stackable: false, members: false, certlink: -1, certtemplate: -1, wearpos: 3, wearpos2: -1, wearpos3: -1 },
];
const magicFacts = extractMagicFacts(content, magicItems);
assert.equal(magicFacts.spells.length, 2, 'unnamed autocast rows are not SPELL_DB');
assert.equal(magicFacts.spells[0].name, 'Wind Strike');
assert.equal(magicFacts.spells[0].ssb, 0);
assert.deepEqual(magicFacts.spells[0].runes.map((rune) => [rune.name, rune.count]), [['Mind rune', 1], ['Air rune', 1]]);
assert.equal(magicFacts.spells[1].name, 'Fire Wave');
assert.equal(magicFacts.spells[1].ssb, 1);
const lava = magicFacts.staves.find((staff) => staff.name === 'Lava battlestaff');
assert.deepEqual(lava?.runes.map((rune) => rune.name).sort(), ['Earth rune', 'Fire rune']);
assert.equal(magicFacts.staves.find((staff) => staff.name === 'Staff of air')?.runes.length, 1);
console.log('generate fixture passed');
