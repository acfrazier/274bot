import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { extractFacts, extractMagicFacts, extractAutocastControls, extractDuelControls, extractSpecialControls, parsePack, parseRows } from './generate.ts';

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
fs.mkdirSync(path.join(content, 'pack'), { recursive: true });
fs.writeFileSync(path.join(content, 'pack/interface.pack'), `328=combat_staff_2\n349=combat_staff_2:auto_toggle\n353=combat_staff_2:auto_choose\n1829=staff_spells\n1830=staff_spells:ssb0\n6575=duel_select_type\n6412=duel_confirm\n6733=duel_win\n6674=duel_select_type:accept\n6520=duel_confirm:accept\n6671=duel_select_type:otherplayer\n6684=duel_select_type:status\n6571=duel_confirm:status\n`);
fs.writeFileSync(path.join(content, 'pack/varp.pack'), `108=attackstyle_magic\n`);
const autocast = extractAutocastControls(content);
assert.equal(autocast.staff_tab_root, 328);
assert.equal(autocast.choose_com, 353);
assert.equal(autocast.spell_panel_root, 1829);
assert.equal(autocast.spell_grid_base, 1830);
assert.equal(autocast.toggle_com, 349);
assert.equal(autocast.magic_varp, 108);
const duel = extractDuelControls(content);
assert.equal(duel.select_modal, 6575);
assert.equal(duel.confirm_modal, 6412);
assert.equal(duel.win_modal, 6733);
assert.equal(duel.select_accept, 6674);
assert.equal(duel.confirm_accept, 6520);
assert.equal(duel.select_partner, 6671);
assert.equal(duel.select_status, 6684);
assert.equal(duel.confirm_status, 6571);
assert.equal(parsePack('328=combat_staff_2\n').get('combat_staff_2'), 328);
fs.mkdirSync(path.join(content, 'scripts/skill_combat/configs'), { recursive: true });
fs.appendFileSync(path.join(content, 'pack/interface.pack'), '425=combat_blunt\n7462=combat_blunt:specbar\n2423=combat_hacksword\n7587=combat_hacksword:specbar\n');
fs.appendFileSync(path.join(content, 'pack/varp.pack'), '300=sa_energy\n301=sa_attack\n');
fs.writeFileSync(path.join(content, 'pack/param.pack'), '136=specwep\n137=sa_energy\n');
fs.writeFileSync(path.join(content, 'scripts/skill_combat/configs/combat.constant'), '^sa_max_energy = 1000\n^sa_regen_amount = 100\n');
const daggerParams = new Map([[136, 1], [137, 250]]);
const axeParams = new Map([[136, 1]]);
const scimParams = new Map();
const special = extractSpecialControls(content, [
    { id: 1215, debugname: 'dragon_dagger', name: 'Dragon dagger', cost: 0, stackable: false, members: true, certlink: -1, certtemplate: -1, wearpos: 3, wearpos2: -1, wearpos3: -1, params: daggerParams },
    { id: 1377, debugname: 'dragon_battleaxe', name: 'Dragon battleaxe', cost: 0, stackable: false, members: true, certlink: -1, certtemplate: -1, wearpos: 3, wearpos2: -1, wearpos3: -1, params: axeParams },
    { id: 1333, debugname: 'rune_scimitar', name: 'Rune scimitar', cost: 0, stackable: false, members: false, certlink: -1, certtemplate: -1, wearpos: 3, wearpos2: -1, wearpos3: -1, params: scimParams },
]);
assert.equal(special.energy_varp, 300);
assert.equal(special.armed_varp, 301);
assert.equal(special.max_energy, 1000);
assert.equal(special.bars.length, 2);
assert.equal(special.bars.find((bar) => bar.root === 'combat_blunt')?.bar, 7462);
assert.equal(special.bars.find((bar) => bar.root === 'combat_hacksword')?.root_id, 2423);
assert.equal(special.bars.some((bar) => bar.root === 'combat_staff_2'), false);
assert.equal(special.weapons.length, 1);
assert.equal(special.weapons[0].name, 'Dragon dagger');
assert.equal(special.weapons[0].cost, 250);
assert.equal(special.weapons.some((weapon) => weapon.name === 'Dragon battleaxe'), false);
console.log('generate fixture passed');
