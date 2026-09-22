import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { extractDropFacts, extractFacts, extractEquipmentNamesFacts, extractFlourSixFacts, extractGatherMethodsFacts, extractHerbFacts, extractMagicFacts, extractAutocastControls, extractDuelControls, extractNurmofEssenceFacts, extractPrayerFacts, extractSpecialControls, extractTeleportSpells, herbKeyFromName, identifiedHerbLevelDefault, joinEquipmentName, loadEquipmentNamesCurated, parseFrozenEquipmentNameArrays, parseFrozenEquipmentSingleQuoted, parseIdentifyHerbPairs, parseInvShopStock, parseJm2LocPlacements, parseMapsquarePath, parseObjSections, parsePack, parseParamDefinitions, parsePrayerInterface, parseQuestEnumEntry, parseRows } from './generate.ts';

const repoRoot = path.resolve(import.meta.dirname, '../..');

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
fs.appendFileSync(
    path.join(content, 'pack/interface.pack'),
    '1164=magic:varrock_teleport\n1167=magic:lumbridge_teleport\n1922=magic:highlvl_alchemy\n',
);
fs.writeFileSync(path.join(content, 'scripts/skill_magic/configs/magic_spells.dbrow'), `[magic_spell_teleport_varrock]
data=spell,^varrock_teleport
data=members,false
data=levelrequired,25
data=runesrequired,firerune,1,airrune,3,lawrune,1
data=experience,350
data=tele_coord,0_50_53_13_32
[magic_spell_high_alch]
data=spell,^highlvl_alchemy
data=members,false
data=levelrequired,55
data=runesrequired,naturerune,1,firerune,5,null,null
data=experience,650
[magic_spell_teleport_lumbridge]
data=spell,^lumbridge_teleport
data=members,false
data=levelrequired,31
data=runesrequired,earthrune,1,airrune,3,lawrune,1
data=experience,410
data=tele_coord,0_50_50_21_18
`);
const teleportItems = [
    ...magicItems,
    { id: 9, debugname: 'lawrune', name: 'Law rune', cost: 0, stackable: true, members: false, certlink: -1, certtemplate: -1, wearpos: -1, wearpos2: -1, wearpos3: -1 },
    { id: 10, debugname: 'naturerune', name: 'Nature rune', cost: 0, stackable: true, members: false, certlink: -1, certtemplate: -1, wearpos: -1, wearpos2: -1, wearpos3: -1 },
];
const teleports = extractTeleportSpells(content, teleportItems);
assert.equal(teleports.length, 2, 'only tele_coord rows are teleports');
assert.equal(teleports[0].name, 'Varrock');
assert.equal(teleports[0].component_id, 1164);
assert.equal(teleports[0].level, 25);
assert.equal(teleports[0].x, 3213);
assert.equal(teleports[0].z, 3424);
assert.equal(teleports[0].plane, 0);
assert.deepEqual(teleports[0].runes.map((rune) => [rune.name, rune.count]), [['Fire rune', 1], ['Air rune', 3], ['Law rune', 1]]);
assert.equal(teleports[1].name, 'Lumbridge');
assert.equal(teleports[1].component_id, 1167);
assert.equal(teleports.some((row) => row.spell === 'highlvl_alchemy'), false);

assert.equal(herbKeyFromName('Guam leaf'), 'guam');
assert.equal(herbKeyFromName('Dwarf weed'), 'dwarf weed');
assert.equal(herbKeyFromName('Snake weed'), 'snake weed');
assert.equal(herbKeyFromName('Ranarr weed'), 'ranarr');
const herbObj = parseObjSections(`[guam_leaf]
name=Guam leaf
cost=3
param=identified_herb_exp,25
[marentill]
name=Marrentill
cost=5
param=identified_herb_level,5
`);
assert.equal(herbObj.get('guam_leaf')?.name, 'Guam leaf');
assert.equal(herbObj.get('guam_leaf')?.cost, 3);
assert.deepEqual(parseIdentifyHerbPairs(`[opheld1,unidentified_guam]
~attempt_identify_herb(guam_leaf, last_slot());
`), [{ unidAlias: 'unidentified_guam', idAlias: 'guam_leaf' }]);
const herblore = path.join(content, 'scripts/skill_herblore');
fs.mkdirSync(path.join(herblore, 'configs/identifying'), { recursive: true });
fs.mkdirSync(path.join(herblore, 'scripts/identifying'), { recursive: true });
fs.writeFileSync(path.join(herblore, 'configs/identifying/identify.param'), `[identified_herb_level]
type=int
default=3
`);
fs.writeFileSync(path.join(herblore, 'configs/herbs.obj'), `[guam_leaf]
name=Guam leaf
cost=999
[snake_weed]
name=Snake weed
param=identified_herb_level,3
`);
fs.writeFileSync(path.join(herblore, 'scripts/identifying/identify.rs2'), `[opheld1,unidentified_guam]
~attempt_identify_herb(guam_leaf, last_slot());
[opheld1,unidentified_snake_weed]
~attempt_identify_herb(snake_weed, last_slot());
`);
const herbItems = [
    { id: 199, debugname: 'unidentified_guam', name: 'Herb', cost: 0, stackable: false, members: true, certlink: -1, certtemplate: -1, wearpos: -1, wearpos2: -1, wearpos3: -1 },
    { id: 249, debugname: 'guam_leaf', name: 'Guam leaf', cost: 3, stackable: false, members: true, certlink: -1, certtemplate: -1, wearpos: -1, wearpos2: -1, wearpos3: -1 },
    { id: 1525, debugname: 'unidentified_snake_weed', name: 'Herb', cost: 0, stackable: false, members: true, certlink: -1, certtemplate: -1, wearpos: -1, wearpos2: -1, wearpos3: -1 },
    { id: 1526, debugname: 'snake_weed', name: 'Snake weed', cost: 0, stackable: false, members: true, certlink: -1, certtemplate: -1, wearpos: -1, wearpos2: -1, wearpos3: -1 },
];
assert.equal(identifiedHerbLevelDefault(content), 3);
assert.deepEqual(parseParamDefinitions(fs.readFileSync(path.join(herblore, 'configs/identifying/identify.param'), 'utf8')).get('identified_herb_level'), { type: 'int', default: '3' });
const herbs = extractHerbFacts(content, herbItems);
assert.equal(herbs.herbs.length, 2);
assert.deepEqual(herbs.herbs[0], { key: 'guam', name: 'Guam leaf', id: 249, unidId: 199, level: 3, level_source: 'identified_herb_level_default', source_identified: 'guam_leaf', source_unidentified: 'unidentified_guam' });
assert.equal(herbs.herbs[0].level, 3, 'guam uses identify.param default, not cost=999');
assert.equal(herbs.herbs[1].key, 'snake weed');
assert.equal(herbs.herbs[1].level_source, 'identified_herb_level');

const dropContent = fs.mkdtempSync(path.join(os.tmpdir(), 'game-data-drop-fixture-'));
const dropScripts = path.join(dropContent, 'scripts/drop tables/scripts');
const npcConfigs = path.join(dropContent, 'scripts/_unpack/225');
fs.mkdirSync(dropScripts, { recursive: true });
fs.mkdirSync(npcConfigs, { recursive: true });
fs.writeFileSync(path.join(npcConfigs, 'all.npc'), `[giant]
name=Giant
param=death_drop,big_bones
[mossgiant]
name=Moss giant
param=death_drop,big_bones
[firegiant]
name=Fire giant
param=death_drop,big_bones
[green_dragon]
name=Green dragon
param=death_drop,dragon_bones
`);
fs.writeFileSync(path.join(dropScripts, 'giant.rs2'), `[ai_queue3,giant]
obj_add(npc_coord, npc_param(death_drop), 1, 100);
obj_add(npc_coord, coins, 1, 100);
obj_add(npc_coord, ~outer, 100);
`);
fs.writeFileSync(path.join(dropScripts, 'moss_giant.rs2'), `[ai_queue3,mossgiant]
obj_add(npc_coord, npc_param(death_drop), 1, 100);
obj_add(npc_coord, ~outer, 100);
`);
fs.writeFileSync(path.join(dropScripts, 'fire_giant.rs2'), `[ai_queue3,firegiant]
obj_add(npc_coord, npc_param(death_drop), 1, 100);
obj_add(npc_coord, cert_silver_ore, 1, 100);
`);
fs.writeFileSync(path.join(dropScripts, 'green_dragon.rs2'), `[ai_queue3,green_dragon]
obj_add(npc_coord, npc_param(death_drop), 1, 100);
obj_add(npc_coord, dragonhide_green, 1, 100);
`);
fs.writeFileSync(path.join(dropScripts, 'shared_droptables.rs2'), `[proc,outer]()(namedobj, int)
return (~inner);
[proc,inner]()(namedobj, int)
def_namedobj $drop = rune_spear;
return (keyhalf1, 1);
return (keyhalf2, 1);
return ($drop, 1);
`);
const dropItems = [
    { id: 532, debugname: 'big_bones', name: 'Big bones', cost: 0, stackable: false, members: false, certlink: -1, certtemplate: -1, wearpos: -1, wearpos2: -1, wearpos3: -1 },
    { id: 536, debugname: 'dragon_bones', name: 'Dragon bones', cost: 0, stackable: false, members: true, certlink: -1, certtemplate: -1, wearpos: -1, wearpos2: -1, wearpos3: -1 },
    { id: 995, debugname: 'coins', name: 'Coins', cost: 1, stackable: true, members: false, certlink: -1, certtemplate: -1, wearpos: -1, wearpos2: -1, wearpos3: -1 },
    { id: 985, debugname: 'keyhalf1', name: 'Half of a key', cost: 0, stackable: false, members: true, certlink: -1, certtemplate: -1, wearpos: -1, wearpos2: -1, wearpos3: -1 },
    { id: 987, debugname: 'keyhalf2', name: 'Half of a key', cost: 0, stackable: false, members: true, certlink: -1, certtemplate: -1, wearpos: -1, wearpos2: -1, wearpos3: -1 },
    { id: 1247, debugname: 'rune_spear', name: 'Rune spear', cost: 0, stackable: false, members: true, certlink: -1, certtemplate: -1, wearpos: 3, wearpos2: -1, wearpos3: -1 },
    { id: 1753, debugname: 'dragonhide_green', name: 'Dragonhide', cost: 0, stackable: false, members: true, certlink: -1, certtemplate: -1, wearpos: -1, wearpos2: -1, wearpos3: -1 },
    { id: 443, debugname: 'silver_ore', name: 'Silver ore', cost: 0, stackable: false, members: false, certlink: -1, certtemplate: -1, wearpos: -1, wearpos2: -1, wearpos3: -1 },
];
const dropNpcs = [
    { id: 117, debugname: 'giant', name: 'Giant' },
    { id: 110, debugname: 'mossgiant', name: 'Moss giant' },
    { id: 112, debugname: 'firegiant', name: 'Fire giant' },
    { id: 941, debugname: 'green_dragon', name: 'Green dragon' },
];
const drops = extractDropFacts(dropContent, dropItems, dropNpcs);
assert.equal(drops.length, 4);
assert.deepEqual(drops.find((row) => row.name === 'Giant')?.display_names, ['Big bones', 'Coins', 'Half of a key', 'Rune spear']);
assert.deepEqual(
    drops.find((row) => row.name === 'Giant')?.items.filter((item) => item.name === 'Half of a key').map((item) => [item.alias, item.id]),
    [['keyhalf1', 985], ['keyhalf2', 987]],
    'display names collapse aliases only at the foreign API boundary',
);
assert.deepEqual(drops.find((row) => row.name === 'Fire giant')?.items, [
    { alias: 'big_bones', id: 532, name: 'Big bones' },
    { alias: 'silver_ore', id: 443, name: 'Silver ore' },
]);
assert.deepEqual(drops.find((row) => row.name === 'Green dragon')?.display_names, ['Dragon bones', 'Dragonhide']);

fs.rmSync(path.join(dropScripts, 'shared_droptables.rs2'));
assert.throws(
    () => extractDropFacts(dropContent, dropItems, dropNpcs),
    /missing content input .*shared_droptables\.rs2/,
    'a missing recursive source must fail closed',
);
fs.writeFileSync(path.join(dropScripts, 'shared_droptables.rs2'), `[proc,outer]()(namedobj, int)
return (~missing_proc);
`);
assert.throws(
    () => extractDropFacts(dropContent, dropItems, dropNpcs),
    /missing drop block proc:missing_proc/,
    'an unresolved recursive join must fail closed',
);

const prayerContent = fs.mkdtempSync(path.join(os.tmpdir(), 'game-data-prayer-fixture-'));
const prayerDir = path.join(prayerContent, 'scripts/skill_prayer');
fs.mkdirSync(path.join(prayerDir, 'configs'), { recursive: true });
fs.mkdirSync(path.join(prayerDir, 'interfaces'), { recursive: true });
fs.mkdirSync(path.join(prayerContent, 'pack'), { recursive: true });
fs.writeFileSync(path.join(prayerDir, 'configs/prayers.constant'), '^prayer_thickskin = 1\n^prayer_strengthburst = 4\n');
fs.writeFileSync(path.join(prayerDir, 'configs/prayers.dbrow'), `[prayer_thick_skin]
data=prayer,^prayer_thickskin
data=name,Thick Skin
data=level,1
[prayer_strength_burst]
data=prayer,^prayer_strengthburst
data=name,Burst of Strength
data=level,4
`);
fs.writeFileSync(path.join(prayerDir, 'interfaces/prayer.if'), `[prayer_thickskin]
script1op1=pushvar,prayer0
[prayer_strengthburst]
script1op1=pushvar,prayer1
`);
fs.writeFileSync(path.join(prayerContent, 'pack/interface.pack'), '5609=prayer:prayer_thickskin\n5610=prayer:prayer_strengthburst\n');
fs.writeFileSync(path.join(prayerContent, 'pack/varp.pack'), '83=prayer0\n84=prayer1\n');
assert.throws(
    () => extractPrayerFacts(prayerContent),
    /expected 15 rows, got 2/,
    'partial prayer tables must fail closed',
);
fs.writeFileSync(path.join(prayerDir, 'configs/prayers.constant'), [...Array.from({ length: 15 }, (_, index) => `^prayer_row${index} = ${index + 1}`)].join('\n') + '\n');
let dbrow = '';
let prayerIf = '';
let iface = '';
let varp = '';
for (let index = 0; index < 15; index += 1) {
    const constant = `prayer_row${index}`;
    const name = index === 0 ? 'Thick Skin' : index === 14 ? 'Protect from Melee' : `Prayer ${index}`;
    const level = index === 0 ? 1 : index === 14 ? 43 : index + 1;
    dbrow += `[row_${index}]
data=prayer,^${constant}
data=name,${name}
data=level,${level}
`;
    prayerIf += `[${constant}]
script1op1=pushvar,prayer${index}
`;
    iface += `${5609 + index}=prayer:${constant}\n`;
    varp += `${83 + index}=prayer${index}\n`;
}
fs.writeFileSync(path.join(prayerDir, 'configs/prayers.dbrow'), dbrow);
fs.writeFileSync(path.join(prayerDir, 'interfaces/prayer.if'), prayerIf);
fs.writeFileSync(path.join(prayerContent, 'pack/interface.pack'), iface);
fs.writeFileSync(path.join(prayerContent, 'pack/varp.pack'), varp);
const prayerFacts = extractPrayerFacts(prayerContent);
assert.equal(prayerFacts.prayers.length, 15);
assert.equal(prayerFacts.prayers[0].button_com, 5609);
assert.equal(prayerFacts.prayers[14].varp, 97);
assert.throws(
    () => parsePrayerInterface('[a]\nscript1op1=pushvar,x\n[a]\nscript1op1=pushvar,y\n'),
    /duplicate section a/,
    'duplicate prayer.if sections must fail closed',
);

fs.writeFileSync(path.join(prayerDir, 'configs/prayers.dbrow'), `[bad]
data=prayer,^prayer_missing
data=name,Missing
data=level,1
`);
fs.writeFileSync(path.join(prayerDir, 'configs/prayers.constant'), '^prayer_thickskin = 1\n');
assert.throws(
    () => extractPrayerFacts(prayerContent),
    /unknown prayer constant prayer_missing/,
    'prayer constant must exist in prayers.constant',
);
fs.writeFileSync(path.join(prayerDir, 'interfaces/prayer.if'), `[prayer_thickskin]
script1op1=pushvar,prayer0
`);
fs.writeFileSync(path.join(prayerDir, 'configs/prayers.dbrow'), `[bad]
data=prayer,^prayer_thickskin
data=name,Thick Skin
data=level,1
`);
fs.writeFileSync(path.join(prayerDir, 'configs/prayers.constant'), '^prayer_thickskin = 1\n');
fs.writeFileSync(path.join(prayerContent, 'pack/interface.pack'), '5609=prayer:prayer_thickskin\n');
fs.writeFileSync(path.join(prayerContent, 'pack/varp.pack'), '83=prayer0\n');
assert.throws(
    () => extractPrayerFacts(prayerContent),
    /expected 15 rows, got 1/,
    'single-row fixture stays fail-closed until complete',
);

assert.deepEqual(parseInvShopStock('[pickaxeshop]\nstock1=bronze_pickaxe,6,50\n', 'pickaxeshop').map((row) => row.alias), ['bronze_pickaxe']);
assert.throws(() => parseInvShopStock('[other]\nstock1=a,1,1\n', 'pickaxeshop'), /missing stock rows/);
const locSection = '==== LOC ====\n0 47 62: 2662 10 1\n';
assert.deepEqual(parseJm2LocPlacements(locSection, 2662), [{ plane: 0, lx: 47, lz: 62, loc_id: 2662, shape: 10, angle: 1 }]);
assert.deepEqual(parseJm2LocPlacements('==== LOC ====\n', 2662), []);
assert.equal(parseJm2LocPlacements('==== NPC ====\n0 10 20: 2662\n', 2662).length, 0);
assert.equal(parseJm2LocPlacements('==== OBJ ====\n0 1 2: 2662 5\n', 2662).length, 0);
assert.throws(() => parseJm2LocPlacements('==== LOC ====\n0 99 99: 2662 10 1\n', 2662), /out of range/);
assert.throws(() => parseJm2LocPlacements('==== LOC ====\n0 47 62: 2662 10 1 extra\n', 2662), /extra tokens/);
assert.throws(
    () => extractFlourSixFacts(
        (() => {
            const dup = fs.mkdtempSync(path.join(os.tmpdir(), 'game-data-flour-dup-'));
            fs.mkdirSync(path.join(dup, 'scripts/quests/quest_murder/configs'), { recursive: true });
            fs.mkdirSync(path.join(dup, 'scripts/general/configs'), { recursive: true });
            fs.mkdirSync(path.join(dup, 'pack'), { recursive: true });
            fs.mkdirSync(path.join(dup, 'maps'), { recursive: true });
            fs.writeFileSync(path.join(dup, 'scripts/general/configs/quest.enum'), '[quest_names_enum]\nval=37,Murder Mystery\n');
            fs.writeFileSync(path.join(dup, 'scripts/quests/quest_murder/configs/quest_murder.loc'), '[flourbarrel]\nname=Barrel of flour\n');
            fs.writeFileSync(path.join(dup, 'pack/loc.pack'), '2662=flourbarrel\n');
            fs.writeFileSync(path.join(dup, 'pack/obj.pack'), '1931=pot_empty\n1933=pot_flour\n');
            fs.writeFileSync(path.join(dup, 'maps/m42_55.jm2'), '==== LOC ====\n0 47 62: 2662 10 1\n0 48 62: 2662 10 1\n');
            return dup;
        })(),
        [
            { id: 1931, debugname: 'pot_empty', name: 'Pot', cost: 1, stackable: false, members: false, certlink: -1, certtemplate: -1, wearpos: -1, wearpos2: -1, wearpos3: -1 },
            { id: 1933, debugname: 'pot_flour', name: 'Pot of flour', cost: 1, stackable: false, members: false, certlink: -1, certtemplate: -1, wearpos: -1, wearpos2: -1, wearpos3: -1 },
        ],
    ),
    /expected one .* LOC placement/,
);
assert.equal(parseMapsquarePath('maps/m45_75.jm2').mx, 45);
assert.throws(() => parseQuestEnumEntry('val=37,Murder Mystery\n', 'Murder Mystery'), /\[quest_names_enum\]/);
assert.equal(parseQuestEnumEntry('[quest_names_enum]\nval=37,Murder Mystery\n', 'Murder Mystery'), 'val=37,Murder Mystery');

const toolFlour = fs.mkdtempSync(path.join(os.tmpdir(), 'game-data-tool-flour-'));
fs.mkdirSync(path.join(toolFlour, 'scripts/areas/area_falador/configs'), { recursive: true });
fs.mkdirSync(path.join(toolFlour, 'scripts/skill_runecraft/configs'), { recursive: true });
fs.mkdirSync(path.join(toolFlour, 'maps'), { recursive: true });
fs.mkdirSync(path.join(toolFlour, 'pack'), { recursive: true });
fs.writeFileSync(path.join(toolFlour, 'scripts/areas/area_falador/configs/dwarven_mine.inv'), '[pickaxeshop]\nstock1=bronze_pickaxe,6,50\n');
fs.writeFileSync(path.join(toolFlour, 'scripts/areas/area_falador/configs/dwarven_mine.npc'), '[nurmof]\nname=Nurmof\nparam=owned_shop,pickaxeshop\n');
fs.writeFileSync(path.join(toolFlour, 'scripts/skill_runecraft/configs/runecraft.constant'), '^essence_mine_to_aubury = 0_50_53_53_9\n');
fs.writeFileSync(path.join(toolFlour, 'maps/m45_75.jm2'), '==== LOC ====\n0 5 50: 2492 10\n');
fs.writeFileSync(path.join(toolFlour, 'pack/npc.pack'), '594=nurmof\n553=aubury\n');
fs.writeFileSync(path.join(toolFlour, 'pack/loc.pack'), '2492=blankrunestone_exit_portal\n');
assert.throws(
    () => extractNurmofEssenceFacts(toolFlour, [{ id: 1265, debugname: 'bronze_pickaxe', name: 'Bronze pickaxe', cost: 1, stackable: false, members: false, certlink: -1, certtemplate: -1, wearpos: -1, wearpos2: -1, wearpos3: -1 }], [{ id: 594, debugname: 'nurmof', name: 'Nurmof' }]),
    /expected 6 stock rows/,
    'partial pickaxeshop must fail closed',
);

fs.mkdirSync(path.join(toolFlour, 'scripts/quests/quest_murder/configs'), { recursive: true });
fs.mkdirSync(path.join(toolFlour, 'scripts/general/configs'), { recursive: true });
fs.writeFileSync(path.join(toolFlour, 'scripts/general/configs/quest.enum'), '[quest_names_enum]\nval=37,Murder Mystery\n');
fs.writeFileSync(path.join(toolFlour, 'scripts/quests/quest_murder/configs/quest_murder.loc'), '[flourbarrel]\nname=Barrel of flour\n');
fs.writeFileSync(path.join(toolFlour, 'pack/loc.pack'), '2662=flourbarrel\n');
fs.writeFileSync(path.join(toolFlour, 'pack/obj.pack'), '1931=pot_empty\n1933=pot_flour\n');
fs.writeFileSync(path.join(toolFlour, 'maps/m42_55.jm2'), '==== LOC ====\n0 47 62: 2662 10 1\n');
assert.throws(
    () => extractFlourSixFacts(toolFlour, [{ id: 1931, debugname: 'pot_empty', name: 'Pot', cost: 1, stackable: false, members: false, certlink: -1, certtemplate: -1, wearpos: -1, wearpos2: -1, wearpos3: -1 }]),
    /pot_flour join/,
    'missing pot_flour must fail closed',
);

const equipmentItems = [
    { alias: 'shortbow', id: 841, name: 'Shortbow', cost: 1, stackable: false, members: false, certificate_link: -1, certificate_template: -1, wear_position: 3, wear_position_2: -1, wear_position_3: -1 },
    { alias: 'cert_shortbow', id: 842, name: 'Shortbow', cost: 1, stackable: false, members: false, certificate_link: 841, certificate_template: 799, wear_position: -1, wear_position_2: -1, wear_position_3: -1 },
    { alias: 'unstrung_shortbow', id: 50, name: 'Shortbow', cost: 1, stackable: false, members: false, certificate_link: -1, certificate_template: -1, wear_position: -1, wear_position_2: -1, wear_position_3: -1 },
    { alias: 'black_dagger', id: 1217, name: 'Black dagger', cost: 1, stackable: false, members: false, certificate_link: -1, certificate_template: -1, wear_position: 3, wear_position_2: -1, wear_position_3: -1 },
    { alias: 'deathdagger', id: 746, name: 'Black dagger', cost: 1, stackable: false, members: false, certificate_link: -1, certificate_template: -1, wear_position: 3, wear_position_2: -1, wear_position_3: -1 },
    { alias: 'bronze_arrow', id: 882, name: 'Bronze arrow', cost: 1, stackable: true, members: false, certificate_link: -1, certificate_template: -1, wear_position: 13, wear_position_2: -1, wear_position_3: -1 },
];
const equipmentPack = new Map<string, number>([
    ['shortbow', 841],
    ['cert_shortbow', 842],
    ['unstrung_shortbow', 50],
    ['black_dagger', 1217],
    ['deathdagger', 746],
    ['bronze_arrow', 882],
]);
const frozenText = fs.readFileSync(path.join(repoRoot, 'tools/game-data/equipment-names.frozen.ts'), 'utf8');
assert.equal(parseFrozenEquipmentSingleQuoted("'Karil\\'s crossbow'", 0).value, "Karil's crossbow");
const frozenFamilies = parseFrozenEquipmentNameArrays(frozenText);
assert.equal(frozenFamilies.crossbows.at(-1), "Karil's crossbow");
assert.deepEqual(frozenFamilies.bolts, [
    'Bronze bolts', 'Iron bolts', 'Steel bolts', 'Black bolts', 'Mithril bolts', 'Adamant bolts', 'Rune bolts',
    'Broad bolts', 'Bone bolts',
]);
const curated = loadEquipmentNamesCurated();
assert.deepEqual(curated.families.crossbows, frozenFamilies.crossbows);
assert.deepEqual(curated.families.bows, frozenFamilies.bows);
assert.equal(curated.families.crossbows.includes('Karil\\'), false);

function isolatedEquipmentRoot(mutate?: (curatedJson: ReturnType<typeof loadEquipmentNamesCurated>) => ReturnType<typeof loadEquipmentNamesCurated> | void) {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'game-data-equipment-isolated-'));
    fs.mkdirSync(path.join(dir, 'tools/game-data'), { recursive: true });
    const next = structuredClone(curated);
    mutate?.(next);
    fs.writeFileSync(path.join(dir, 'tools/game-data/equipment-names.curated.json'), `${JSON.stringify(next, null, 2)}\n`);
    fs.copyFileSync(path.join(repoRoot, 'tools/game-data/equipment-names.frozen.ts'), path.join(dir, 'tools/game-data/equipment-names.frozen.ts'));
    return dir;
}

const isolated = isolatedEquipmentRoot();
assert.deepEqual(loadEquipmentNamesCurated(isolated).families, frozenFamilies);
const isolatedFacts = extractEquipmentNamesFacts(equipmentItems, equipmentPack, isolated);
assert.equal(isolatedFacts.crossbows.at(-1)?.requested_name, "Karil's crossbow");
assert.equal(isolatedFacts.equipment_evidence.path, 'tools/game-data/equipment-names.frozen.ts');

const curatedOnly = fs.mkdtempSync(path.join(os.tmpdir(), 'game-data-equipment-curated-only-'));
fs.mkdirSync(path.join(curatedOnly, 'tools/game-data'), { recursive: true });
fs.copyFileSync(path.join(repoRoot, 'tools/game-data/equipment-names.curated.json'), path.join(curatedOnly, 'tools/game-data/equipment-names.curated.json'));
assert.throws(
    () => loadEquipmentNamesCurated(curatedOnly),
    /equipment evidence missing/,
    'loader must not require a sibling checkout; missing local evidence fails closed',
);

const malformed = isolatedEquipmentRoot((copy) => {
    copy.families.crossbows[9] = 'Karil\\';
});
assert.throws(
    () => loadEquipmentNamesCurated(malformed),
    /crossbows\[9\].*Karil/,
    'malformed known curated Karil must fail closed before selected matching',
);

const wrongOrder = isolatedEquipmentRoot((copy) => {
    const [first, second] = copy.families.bows;
    copy.families.bows[0] = second!;
    copy.families.bows[1] = first!;
});
assert.throws(
    () => loadEquipmentNamesCurated(wrongOrder),
    /bows\[0\]/,
    'wrong curated order must fail closed against frozen evidence',
);

const unknown = isolatedEquipmentRoot((copy) => {
    copy.families.bows.push('Laser gun');
});
assert.throws(
    () => loadEquipmentNamesCurated(unknown),
    /bows length 13 != frozen 12/,
    'unknown curated membership must fail closed',
);

const curatedCopy = fs.mkdtempSync(path.join(os.tmpdir(), 'game-data-equipment-curated-'));
fs.mkdirSync(path.join(curatedCopy, 'tools/game-data'), { recursive: true });
fs.writeFileSync(path.join(curatedCopy, 'tools/game-data/equipment-names.curated.json'), JSON.stringify({
    ...curated,
    families: { ...curated.families, melee_weapons: ['Dup', 'Dup'] },
}, null, 2));
assert.throws(
    () => loadEquipmentNamesCurated(curatedCopy),
    /duplicate melee_weapons name Dup/,
    'curated duplicate family rows must fail closed before evidence compare',
);

const equipmentFacts = extractEquipmentNamesFacts(equipmentItems, equipmentPack);
const shortbow = equipmentFacts.bows.find((row) => row.requested_name === 'Shortbow');
assert.equal(shortbow?.disposition, 'resolved');
assert.equal(shortbow?.alias, 'shortbow');
assert.equal(shortbow?.id, 841);
assert.equal(shortbow?.disambiguation, 'exclude_bank_note_prefer_wearable');
const blackDagger = equipmentFacts.melee_weapons.find((row) => row.requested_name === 'Black dagger');
assert.equal(blackDagger?.disposition, 'resolved');
assert.equal(blackDagger?.alias, 'black_dagger');
assert.equal(blackDagger?.id, 1217);
assert.equal(blackDagger?.disambiguation, 'prefer_standard_pack_alias_black_dagger');
const dragonArrow = equipmentFacts.arrows.find((row) => row.requested_name === 'Dragon arrow');
assert.equal(dragonArrow?.disposition, 'absent');
assert.equal(dragonArrow?.absent_class, 'no_exact_selected_match');
const karil = equipmentFacts.crossbows.find((row) => row.requested_name === "Karil's crossbow");
assert.equal(karil?.disposition, 'absent');
assert.equal(karil?.absent_class, 'no_exact_selected_match');
assert.equal(equipmentFacts.exact_name_join.bolts.generic_is_substitute, false);

const noteOnly = extractEquipmentNamesFacts([
    { alias: 'cert_shortbow', id: 842, name: 'Shortbow', cost: 1, stackable: false, members: false, certificate_link: 841, certificate_template: 799, wear_position: -1, wear_position_2: -1, wear_position_3: -1 },
], new Map([['cert_shortbow', 842]]));
assert.equal(noteOnly.bows.find((row) => row.requested_name === 'Shortbow')?.absent_class, 'exact_match_ineligible');
assert.equal(noteOnly.arrows.find((row) => row.requested_name === 'Dragon arrow')?.absent_class, 'no_exact_selected_match');

const genericBolts = extractEquipmentNamesFacts([
    { alias: 'bolt', id: 877, name: 'Bolts', cost: 1, stackable: true, members: false, certificate_link: -1, certificate_template: -1, wear_position: 13, wear_position_2: -1, wear_position_3: -1 },
], new Map([['bolt', 877]]));
assert.equal(genericBolts.bolts.find((row) => row.requested_name === 'Bronze bolts')?.absent_class, 'no_exact_selected_match');
assert.equal(genericBolts.bolts.some((row) => row.requested_name === 'Bolts' || row.alias === 'bolt'), false);
assert.equal(genericBolts.exact_name_join.bolts.generic_display_name, 'Bolts');

assert.throws(
    () => extractEquipmentNamesFacts([
        { alias: 'bronze_scimitar', id: 1, name: 'Bronze scimitar', cost: 1, stackable: false, members: false, certificate_link: -1, certificate_template: -1, wear_position: 3, wear_position_2: -1, wear_position_3: -1 },
        { alias: 'bronze_scimitar_dup', id: 2, name: 'Bronze scimitar', cost: 1, stackable: false, members: false, certificate_link: -1, certificate_template: -1, wear_position: 3, wear_position_2: -1, wear_position_3: -1 },
    ], new Map([['bronze_scimitar', 1], ['bronze_scimitar_dup', 2]])),
    /ambiguous joins Bronze scimitar/,
    'duplicate wieldable display names must fail closed at generate time',
);
assert.throws(
    () => extractEquipmentNamesFacts(equipmentItems, new Map([['shortbow', 999], ['black_dagger', 1217], ['deathdagger', 746], ['bronze_arrow', 882]])),
    /obj\.pack shortbow=999/,
    'selected pack id mismatch must fail closed',
);
assert.throws(
    () => extractEquipmentNamesFacts(equipmentItems, undefined as unknown as Map<string, number>),
    /missing required obj\.pack/,
    'missing required pack must fail closed',
);
assert.throws(
    () => extractEquipmentNamesFacts(equipmentItems, new Map([['shortbow', 841], ['black_dagger', 1217], ['bronze_arrow', 882]])),
    /ambiguous joins Black dagger/,
    'black dagger preference without independent pack support stays ambiguous',
);
assert.equal(
    joinEquipmentName('arrows', 'Dragon arrow', [], equipmentPack).absent_class,
    'no_exact_selected_match',
);

const gatherTreeLocs = ['achey.loc', 'burnt.loc', 'hollow.loc', 'magic.loc', 'maple.loc', 'normal.loc', 'oak.loc', 'willow.loc', 'yew.loc'];
function writeGatherFixture(rootDir: string, mutate?: (files: Record<string, string>) => void) {
    const files: Record<string, string> = {
        'scripts/skill_mining/configs/mine.dbrow': `[copper_rock_table]
data=rock,copperrock1
data=rock,macro_copperrock1
data=ore_name,copper
data=rock_output,copper_ore
data=rock_level,1
data=rock_exp,175
[gem_rock]
data=rock,gemrock
data=ore_name,gems
data=rock_level,40
[limestone_rock3]
data=rock,loc_4027
data=ore_name,limestone
data=rock_output,limestone
data=rock_level,10
[limestone_rock2]
data=rock,loc_4028
data=ore_name,limestone
data=rock_output,limestone
data=rock_level,10
[limestone_rock1]
data=rock,loc_4029
data=ore_name,limestone
data=rock_output,limestone
data=rock_level,10
[desertrescue_rock]
data=rock,punishrocks
data=ore_name,rock
data=rock_output,thpunishrock
data=rock_level,1
[rune_essence_table]
data=rock,blankrunestone
data=ore_name,rune stones
data=rock_output,blankrune
data=rock_level,1
`,
        'scripts/skill_woodcutting/configs/trees.dbrow': `[normal_tree_table]
data=tree,tree
data=levelrequired,0
data=product,logs
data=productexp,250
[jungle_tree_table]
data=tree,kharazi_jungle_tree1
data=levelrequired,0
data=product,logs
[achey_tree_table]
data=tree,achey_tree
data=levelrequired,0
data=product,achey_tree_logs
[burnt_tree_table]
data=tree,deadtree_burnt
data=levelrequired,0
data=product,charcoal
`,
        'scripts/skill_mining/configs/rocks.loc': `[copperrock1]
name=Rocks
param=next_loc_stage_mining,rocks1
[macro_copperrock1]
name=Rocks
category=mining_rock_macro_gas
[gemrock]
name=Rocks
param=next_loc_stage_mining,rocks1
[blankrunestone]
name=Rune Essence
[loc_4027]
name=Rocks
param=next_loc_stage_mining,loc_4028
[loc_4028]
name=Rocks
param=next_loc_stage_mining,loc_4029
[loc_4029]
name=Rocks
param=next_loc_stage_mining,loc_4030
`,
        'scripts/skill_woodcutting/configs/trees/normal.loc': `[tree]
name=Tree
param=next_loc_stage,treestump2
`,
        'scripts/skill_woodcutting/configs/trees/achey.loc': `[achey_tree]
name=Tree
param=next_loc_stage,achey_tree_stump
`,
        'scripts/skill_woodcutting/configs/trees/burnt.loc': `[deadtree_burnt]
name=Tree
param=next_loc_stage,deadtree_burnt_stump
`,
        'scripts/skill_fishing/configs/fishing.npc': `[spot_a]
name=Fishing spot
op1=Lure
op3=Bait
category=freshfish
param=fishing_movement_enum,fishing_movement_gnome_stronghold_enum
[spot_b]
name=Fishing spot
op1=Lure
op3=Bait
category=freshfish
param=fishing_movement_enum,fishing_movement_other_enum
[lava]
name=Fishing spot
op1=Bait
op3=hidden
[member_a]
name=Fishing spot
op1=Net
op3=Harpoon
category=memberfish
[member_b]
name=Fishing spot
op1=Net
op3=Harpoon
category=memberfish
`,
        'pack/loc.pack': `450=rocks1
1306=tree
1342=treestump2
1356=deadtree_burnt
3370=achey_tree
2090=copperrock1
2091=macro_copperrock1
2111=gemrock
2491=blankrunestone
2704=punishrocks
3371=achey_tree_stump
1359=deadtree_burnt_stump
4027=loc_4027
4028=loc_4028
4029=loc_4029
4030=loc_4030
4818=kharazi_jungle_tree1
`,
        'pack/obj.pack': `436=copper_ore
973=charcoal
1436=blankrune
1511=logs
1855=thpunishrock
2862=achey_tree_logs
3211=limestone
`,
        'scripts/_unpack/225/all.loc': `[punishrocks]
name=Rocks
param=next_loc_stage_mining,rocks1
`,
    };
    for (const file of gatherTreeLocs) {
        const relative = `scripts/skill_woodcutting/configs/trees/${file}`;
        files[relative] ??= '// selected tree loc; no jungle.loc\n';
    }
    mutate?.(files);
    for (const [relative, body] of Object.entries(files)) {
        const absolute = path.join(rootDir, relative);
        fs.mkdirSync(path.dirname(absolute), { recursive: true });
        fs.writeFileSync(absolute, body);
    }
}
function gatherFixture(mutate?: (files: Record<string, string>) => void) {
    const rootDir = fs.mkdtempSync(path.join(os.tmpdir(), 'gather-methods-'));
    writeGatherFixture(rootDir, mutate);
    return rootDir;
}

const gatherRoot = gatherFixture();
const gathered = extractGatherMethodsFacts(gatherRoot, 274);
assert.equal(gathered.mining.length > 0 && gathered.woods.length > 0 && gathered.fishing.length > 0, true, 'extracted rows are required');
const copper = gathered.mining.find((row) => row.table === 'copper_rock_table');
assert.equal(copper?.resource_key, 'copper');
assert.equal(copper?.resource_key === 'Rocks', false);
assert.deepEqual(copper?.loc_ids.map((loc) => loc.alias), ['copperrock1', 'macro_copperrock1']);
assert.equal(copper?.loc_ids.find((loc) => loc.alias === 'copperrock1')?.id, 2090);
assert.equal(copper?.qualification, 'partial');
assert.ok(copper?.missing_transform.includes('macro_copperrock1'));
assert.equal(copper?.empty_ids.some((loc) => loc.alias === 'rocks1' && loc.id === 450), true);
assert.equal(copper?.publication, undefined);
assert.equal('published_for_placement' in (copper ?? {}), false);
assert.equal(copper?.output?.alias, 'copper_ore');
assert.equal('exp' in (copper ?? {}), false);
const gem = gathered.mining.find((row) => row.table === 'gem_rock');
assert.equal(gem?.resource_key, 'gems');
assert.equal(gem?.output, null);
assert.equal(gem?.qualification, 'partial');
assert.ok(gem?.partial_sides.includes('output'));
const limestone = gathered.mining.filter((row) => row.resource_key === 'limestone');
assert.deepEqual(limestone.map((row) => row.table), ['limestone_rock3', 'limestone_rock2', 'limestone_rock1']);
assert.equal(new Set(limestone.map((row) => row.loc_ids[0]?.id)).size, 3);
const essence = gathered.mining.find((row) => row.table === 'rune_essence_table');
assert.equal(essence?.output?.alias, 'blankrune');
assert.equal(essence?.output?.id, 1436);
assert.ok(essence?.missing_transform.includes('blankrunestone'));
const desert = gathered.mining.find((row) => row.table === 'desertrescue_rock');
assert.equal(desert?.resource_key, 'rock');
assert.equal(desert?.empty_ids.length, 0);
assert.equal(gathered.mining.filter((row) => row.resource_key === 'rock').length, 1);
const normalWood = gathered.woods.find((row) => row.resource_key === 'normal');
assert.equal(normalWood?.table, 'normal_tree_table');
assert.equal(normalWood?.publication, 'published');
assert.equal(normalWood?.resource_key === 'Tree', false);
assert.equal(normalWood?.qualification, 'complete');
const jungle = gathered.woods.find((row) => row.resource_key === 'jungle');
assert.equal(jungle?.publication, 'unpublished');
assert.equal(jungle?.empty_ids.length, 0);
assert.equal(jungle?.loc_ids[0]?.alias, 'kharazi_jungle_tree1');
assert.equal(jungle?.qualification, 'partial');
assert.equal(gathered.woods.find((row) => row.resource_key === 'achey')?.publication, 'conditional');
assert.equal(gathered.woods.find((row) => row.resource_key === 'burnt')?.publication, 'unpublished');
assert.equal(gathered.coverage.filter((row) => row.class === 'conditional').map((row) => row.resource_key).includes('achey'), true);
assert.equal(gathered.fishing.length, 3, 'dedupe type plus actions, not one row per npc block');
assert.equal(gathered.fishing.some((row) => 'loc_ids' in row || 'fishing_movement_enum' in row), false);
const lavaSpot = gathered.fishing.find((row) => row.primary_op === 'Bait' && row.pair_op === 'hidden');
assert.equal(lavaSpot?.category, 'unknown');
assert.equal(lavaSpot?.qualification, 'partial');
assert.equal(lavaSpot?.level, null);
assert.equal(lavaSpot?.output, null);
assert.equal(gathered.coverage.filter((row) => row.class === 'revision-absent').map((row) => row.alias).join(','), 'dungeon_tree_closed,karam_dungeon_exit');
assert.equal(gathered.coverage.every((row) => row.class !== 'revision-absent' || row.copied === false), true);
const gatheredIds = [...gathered.mining, ...gathered.woods].flatMap((row) => [...row.loc_ids, ...row.empty_ids].map((loc) => loc.id));
assert.equal(gatheredIds.includes(5083) || gatheredIds.includes(5084), false);

assert.throws(() => extractGatherMethodsFacts(gatherFixture((files) => { files['scripts/skill_mining/configs/mine.dbrow'] = '// no tables\n'; }), 274), /no extracted/);
assert.throws(() => extractGatherMethodsFacts(gatherFixture((files) => { delete files['scripts/skill_mining/configs/mine.dbrow']; }), 274), /mine\.dbrow/);
assert.throws(() => extractGatherMethodsFacts(gatherFixture((files) => { delete files['scripts/skill_woodcutting/configs/trees.dbrow']; }), 274), /trees\.dbrow/);
assert.throws(() => extractGatherMethodsFacts(gatherFixture((files) => { delete files['pack/loc.pack']; }), 274), /loc\.pack/);
assert.throws(() => extractGatherMethodsFacts(gatherFixture((files) => { delete files['pack/obj.pack']; }), 274), /obj\.pack/);
assert.throws(() => extractGatherMethodsFacts(gatherFixture((files) => { delete files['scripts/skill_mining/configs/rocks.loc']; }), 274), /rocks\.loc/);
assert.throws(() => extractGatherMethodsFacts(gatherFixture((files) => { delete files['scripts/skill_woodcutting/configs/trees/oak.loc']; }), 274), /oak\.loc/);
assert.throws(() => extractGatherMethodsFacts(gatherFixture((files) => { delete files['scripts/skill_fishing/configs/fishing.npc']; }), 274), /fishing\.npc/);
assert.throws(() => extractGatherMethodsFacts(gatherFixture((files) => {
    files['scripts/skill_mining/configs/mine.dbrow'] += 'data=rock,missing_from_pack\n';
}), 274), /failed join/);
assert.throws(() => extractGatherMethodsFacts(gatherFixture((files) => {
    files['scripts/skill_mining/configs/rocks.loc'] = '[copperrock1]\nname=Rocks\ncategory=mining_rock_normal\n';
}), 274), /next_loc_stage_mining/);
assert.doesNotThrow(() => extractGatherMethodsFacts(gatherRoot, 274));
const withoutJungleLoc = extractGatherMethodsFacts(gatherRoot, 274);
assert.equal(withoutJungleLoc.woods.some((row) => row.resource_key === 'jungle'), true);
assert.equal(extractGatherMethodsFacts(gatherRoot, 289).coverage.some((row) => row.class === 'revision-absent'), false);

const pin274 = extractGatherMethodsFacts('/Users/acfrazier/experiments/Server/content', 274);
const pin289 = extractGatherMethodsFacts('/Users/acfrazier/experiments/lostcity-289/content', 289);
assert.equal(pin274.mining.length, 17);
assert.equal(pin289.mining.length, 17);
assert.equal(pin274.woods.length, 10);
assert.equal(pin289.woods.length, 10);
assert.equal(pin274.fishing.length, 9);
assert.equal(pin289.fishing.length, 9);
assert.deepEqual(pin274.fishing.map((row) => [row.category, row.primary_op, row.pair_op]), pin289.fishing.map((row) => [row.category, row.primary_op, row.pair_op]));
for (const facts of [pin274, pin289]) {
    assert.equal(facts.mining.some((row) => row.publication !== undefined || 'published_for_placement' in row), false);
    assert.equal(facts.mining.some((row) => row.resource_key === 'Rocks' || row.table === 'Rocks'), false);
    assert.equal(facts.woods.some((row) => row.resource_key === 'Tree'), false);
    assert.equal(facts.fishing.some((row) => 'loc_ids' in row || row.category === 'Fishing spot' || 'fishing_movement_enum' in row), false);
    assert.equal(facts.fishing.every((row) => row.level === null && row.output === null && row.qualification === 'partial'), true);
    const published = facts.woods.filter((row) => row.publication === 'published').map((row) => row.resource_key);
    assert.deepEqual(published, ['normal', 'oak', 'willow', 'maple', 'yew', 'magic']);
    assert.deepEqual(facts.woods.filter((row) => row.publication === 'conditional').map((row) => row.resource_key), ['achey', 'hollow']);
    assert.deepEqual(facts.woods.filter((row) => row.publication === 'unpublished').map((row) => row.resource_key), ['jungle', 'burnt']);
    assert.equal(facts.mining.filter((row) => row.resource_key === 'limestone').length, 3);
    const gemRow = facts.mining.find((row) => row.table === 'gem_rock');
    assert.equal(gemRow?.resource_key, 'gems');
    assert.equal(gemRow?.output, null);
    assert.equal(gemRow?.qualification, 'partial');
    const essenceRow = facts.mining.find((row) => row.table === 'rune_essence_table');
    assert.equal(essenceRow?.output?.alias, 'blankrune');
    assert.equal(essenceRow?.output?.id, 1436);
    const copperRow = facts.mining.find((row) => row.table === 'copper_rock_table');
    assert.equal(copperRow?.loc_ids.find((loc) => loc.alias === 'copperrock1')?.id, 2090);
    assert.equal((copperRow?.loc_ids.length ?? 0) > 2, true);
    const ironRow = facts.mining.find((row) => row.table === 'iron_rock_table');
    assert.equal(ironRow?.loc_ids.find((loc) => loc.alias === 'ironrock1')?.id, 2092);
    assert.equal(ironRow?.loc_ids.find((loc) => loc.alias === 'ironrock2')?.id, 2093);
    const desertRow = facts.mining.find((row) => row.table === 'desertrescue_rock');
    assert.equal(desertRow?.empty_ids.length, 0);
    assert.equal(desertRow?.resource_key, 'rock');
    const lavaRow = facts.fishing.find((row) => row.category === 'unknown');
    assert.equal(lavaRow?.primary_op, 'Bait');
    assert.equal(lavaRow?.pair_op, 'hidden');
    assert.equal(facts.fishing.filter((row) => row.category === 'memberfish').length, 1);
    assert.equal(facts.coverage.filter((row) => row.class === 'conditional').map((row) => row.resource_key).sort().join(','), 'achey,hollow');
}
assert.equal(pin274.coverage.filter((row) => row.class === 'revision-absent').map((row) => `${row.alias}:${row.other_pin_id}`).join(','), 'dungeon_tree_closed:5083,karam_dungeon_exit:5084');
assert.equal(pin289.coverage.some((row) => row.class === 'revision-absent'), false);
const pin274Ids = [...pin274.mining, ...pin274.woods].flatMap((row) => [...row.loc_ids, ...row.empty_ids].map((loc) => loc.id));
assert.equal(pin274Ids.includes(5083) || pin274Ids.includes(5084), false);
assert.equal(pin274.mining.find((row) => row.table === 'coal_rock_table')?.loc_ids.some((loc) => loc.alias === 'misc_dummy_coalrock1'), false);
assert.equal(pin289.mining.find((row) => row.table === 'coal_rock_table')?.loc_ids.some((loc) => loc.alias === 'misc_dummy_coalrock1' && loc.id === 4676), true);
assert.equal(pin289.woods.find((row) => row.resource_key === 'normal')?.loc_ids.some((loc) => loc.alias === 'mm_bush_kharazi_jungle_tree1'), true);
assert.equal(pin289.woods.find((row) => row.resource_key === 'normal')?.empty_ids.some((loc) => loc.alias.includes('mm_bush')), false);
assert.equal(pin274.woods.find((row) => row.resource_key === 'normal')?.qualification, 'complete');
assert.equal(pin289.woods.find((row) => row.resource_key === 'normal')?.qualification, 'partial');

console.log('generate fixture passed');
