import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { assertPinned, assertRs2b0tPinned, contentDirt, engineDirt, assertTrioGiverNpcJoins, assertTrioGiverPins, assertTalkKeyNpcJoins, assertTalkKeyPins, assertTrailPins, extractDropFacts, extractFacts, extractEquipmentNamesFacts, extractFlourSixFacts, extractTalkKeyFacts, extractTrailFacts, extractTrioGiversFacts, extractHerbFacts, extractMagicFacts, extractAutocastControls, extractDuelControls, extractNurmofEssenceFacts, extractPrayerFacts, extractSpecialControls, extractTeleportSpells, herbKeyFromName, identifiedHerbLevelDefault, joinEquipmentName, loadEquipmentNamesCurated, parseFrozenEquipmentNameArrays, parseFrozenEquipmentSingleQuoted, parseIdentifyHerbPairs, parseJm2LinkBelow, parseJm2NpcPlacements, parseTalkKeyHandlers, parseTalkKeyKeeperArms, parseTrailEnumAliases, parseTrailObjBlocks, parseTrioGiverHandlers, parseInvShopStock, parseObjSections, parseParamDefinitions, parsePrayerInterface, parseQuestEnumEntry } from './generate.ts';
import { parseJm2LocPlacements, parseMapsquarePath, parsePack, parseRows } from './extractors/common.ts';
import { extractGatheringFamily, GATHERING_SCHEMA, type GatheringFacts, type GatheringFamily, type Know, type MethodWire, type TargetWire } from './extractors/gathering.ts';
import { extractQuestIdentityFacts } from './extractors/quests.ts';
import type { TrioGiverFacts, TalkKeyFacts } from './generate.ts';
import { familyBytes, familyInputs, generateSelected, revisions, requestedRevisions, type SelectedBuild } from './generate.ts';
import { sha256, sourceFile } from './extractors/common.ts';

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
fs.writeFileSync(path.join(content, 'pack/interface.pack'), `328=combat_staff_2\n349=combat_staff_2:auto_toggle\n353=combat_staff_2:auto_choose\n1829=staff_spells\n1830=staff_spells:ssb0\n6575=duel_select_type\n6412=duel_confirm\n6733=duel_win\n6674=duel_select_type:accept\n6520=duel_confirm:accept\n6671=duel_select_type:otherplayer\n6684=duel_select_type:status\n6571=duel_confirm:status\n6700=duel_select_type:inv\n6701=duel_select_type:otherinv\n6500=duel_confirm:inv\n6501=duel_confirm:otherinv\n6676=duel_select_type:obstacles\n`);
fs.writeFileSync(path.join(content, 'pack/varp.pack'), `108=attackstyle_magic\n286=dueloptions\n`);
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
assert.equal(duel.select_mine, 6700);
assert.equal(duel.select_theirs, 6701);
assert.equal(duel.confirm_mine, 6500);
assert.equal(duel.confirm_theirs, 6501);
assert.equal(duel.obstacles, 6676);
assert.equal(duel.options_varp, 286);
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
const kqConfigs = path.join(dropContent, 'scripts/areas/area_kalphite/configs');
fs.mkdirSync(dropScripts, { recursive: true });
fs.mkdirSync(npcConfigs, { recursive: true });
fs.mkdirSync(kqConfigs, { recursive: true });
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
fs.writeFileSync(path.join(kqConfigs, 'kalphite.npc'), `[kalphite_flyingqueen]
name=Kalphite Queen
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
fs.writeFileSync(path.join(dropScripts, 'kalphite_queen.rs2'), `[ai_queue3,kalphite_flyingqueen]
obj_add(npc_coord, rune_spear, 1, 100);
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
    { id: 1160, debugname: 'kalphite_flyingqueen', name: 'Kalphite Queen' },
];
const drops = extractDropFacts(dropContent, dropItems, dropNpcs);
assert.equal(drops.length, 5);
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
assert.deepEqual(drops.find((row) => row.name === 'Kalphite Queen')?.display_names, ['Rune spear']);

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
assert.deepEqual(parseJm2LocPlacements(locSection, new Set([2662])), [{ plane: 0, lx: 47, lz: 62, loc_id: 2662, shape: 10, angle: 1 }]);
assert.deepEqual(parseJm2LocPlacements('==== LOC ====\n', new Set([2662])), []);
assert.equal(parseJm2LocPlacements('==== NPC ====\n0 10 20: 2662\n', new Set([2662])).length, 0);
assert.equal(parseJm2LocPlacements('==== OBJ ====\n0 1 2: 2662 5\n', new Set([2662])).length, 0);
assert.throws(() => parseJm2LocPlacements('==== LOC ====\n0 99 99: 2662 10 1\n', new Set([2662])), /out of range/);
assert.throws(() => parseJm2LocPlacements('==== LOC ====\n0 47 62: 2662 10 1 extra\n', new Set([2662])), /extra tokens/);
assert.deepEqual(
    parseJm2LocPlacements('==== LOC ====\n0 47 62: 2662 10 1\n0 5 6: 1306\n1 1 1: 9999\n', new Set([2662, 1306])),
    [
        { plane: 0, lx: 47, lz: 62, loc_id: 2662, shape: 10, angle: 1 },
        { plane: 0, lx: 5, lz: 6, loc_id: 1306, shape: 0, angle: 0 },
    ],
);
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

/** Minimal tracked content tree for the write-gate and pipeline tests below: packs, one map, one script. */
function gateContentFixture() {
    const rootDir = fs.mkdtempSync(path.join(os.tmpdir(), 'game-data-pin-content-'));
    const files: Record<string, string> = {
        'pack/loc.pack': '1306=tree\n',
        'pack/obj.pack': '1511=logs\n',
        'maps/m42_55.jm2': '==== LOC ====\n0 47 62: 1306 10 1\n',
        'scripts/example/example.rs2': '[proc,example]\nreturn;\n',
    };
    for (const [relative, body] of Object.entries(files)) {
        const absolute = path.join(rootDir, relative);
        fs.mkdirSync(path.dirname(absolute), { recursive: true });
        fs.writeFileSync(absolute, body);
    }
    execFileSync('git', ['init', '-q'], { cwd: rootDir });
    execFileSync('git', ['add', '-A'], { cwd: rootDir });
    return rootDir;
}


// write gate: generate refuses the same dirty placement inputs verify does — the whole
// scanned maps tree, not just the two maps content_files names, plus the published pack
const gateEngine = fs.mkdtempSync(path.join(os.tmpdir(), 'game-data-pin-engine-'));
fs.writeFileSync(path.join(gateEngine, 'pin.txt'), 'engine fixture\n');
fs.writeFileSync(path.join(gateEngine, '.gitignore'), 'data/pack/\n');
execFileSync('git', ['init', '-q'], { cwd: gateEngine });
execFileSync('git', ['add', '-A'], { cwd: gateEngine });
const gateContent = gateContentFixture();
fs.writeFileSync(path.join(gateContent, 'maps/m43_55.jm2'), '==== LOC ====\n0 1 1: 1306\n');
fs.writeFileSync(path.join(gateContent, '.gitignore'), 'pack/param.pack\npack/unknown.pack\n.DS_Store\n');
execFileSync('git', ['add', '-A'], { cwd: gateContent });
for (const dir of [gateEngine, gateContent]) execFileSync('git', ['-c', 'user.email=fixture@invalid', '-c', 'user.name=Fixture', '-c', 'commit.gpgsign=false', 'commit', '-qm', 'fixture'], { cwd: dir });
const gateHead = (dir: string) => execFileSync('git', ['-C', dir, 'rev-parse', 'HEAD'], { encoding: 'utf8' }).trim();
const gateSpec = { revision: 274, engine: gateEngine, content: gateContent, expectedEngine: gateHead(gateEngine), expectedContent: gateHead(gateContent) } as Parameters<typeof assertPinned>[0];
assert.equal(assertPinned(gateSpec).contentCommit, gateSpec.expectedContent, 'a clean pinned tree passes the write gate');
for (const relative of ['scripts/quests/example/journal.rs2', 'scripts/shared/include.constant', 'pack/obj.pack']) {
    const file = path.join(gateContent, relative);
    const original = fs.existsSync(file) ? fs.readFileSync(file) : undefined;
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file, 'untracked selected input\n');
    assert.throws(() => assertPinned(gateSpec), /relevant content inputs are dirty/, relative);
    if (original) fs.writeFileSync(file, original);
    else fs.rmSync(file);
}
const engineLimit = path.join(gateEngine, 'src/network/game/client/ClientGameProtCategory.ts');
fs.mkdirSync(path.dirname(engineLimit), { recursive: true });
fs.writeFileSync(engineLimit, 'export const USER_EVENT_LIMIT = 5;\n');
assert.match(engineDirt(gateEngine), /ClientGameProtCategory\.ts/);
assert.throws(() => assertPinned(gateSpec), /relevant engine inputs are dirty/);
fs.rmSync(engineLimit);
fs.appendFileSync(path.join(gateContent, 'maps/m43_55.jm2'), '0 2 2: 1306\n');
assert.throws(() => assertPinned(gateSpec), /relevant content inputs are dirty/, 'a modified map outside content_files fails the write gate');
fs.writeFileSync(path.join(gateContent, 'maps/m43_55.jm2'), '==== LOC ====\n0 1 1: 1306\n');
fs.writeFileSync(path.join(gateContent, 'maps/m44_55.jm2'), '==== LOC ====\n0 1 1: 1306\n');
assert.throws(() => assertPinned(gateSpec), /relevant content inputs are dirty/, 'an untracked map fails the write gate');
fs.rmSync(path.join(gateContent, 'maps/m44_55.jm2'));
const originalLocPack = fs.readFileSync(path.join(gateContent, 'pack/loc.pack'));
fs.appendFileSync(path.join(gateContent, 'pack/loc.pack'), '9999=extra\n');
assert.throws(() => assertPinned(gateSpec), /relevant content inputs are dirty/, 'a dirty published pack fails the write gate');
fs.writeFileSync(path.join(gateContent, 'pack/loc.pack'), originalLocPack);

// Ignored generated files have explicit pins; neither dirt nor identity may
// silently depend on whatever generated bytes happen to be in the checkout.
const paramPack = path.join(gateContent, 'pack/param.pack');
fs.writeFileSync(paramPack, '0=param\n');
assert.throws(() => assertPinned(gateSpec), /unrecognized ignored input.*param\.pack/);
gateSpec.generatedInputs = {
    engine_commit: gateSpec.expectedEngine, content_commit: gateSpec.expectedContent,
    content: [sourceFile(gateContent, 'pack/param.pack')], engine: [],
};
assertPinned(gateSpec);
fs.appendFileSync(paramPack, '1=dirty\n');
assert.throws(() => assertPinned(gateSpec), /generated input hash mismatch.*param\.pack/);
fs.writeFileSync(paramPack, '0=param\n');
fs.writeFileSync(path.join(gateContent, 'pack/unknown.pack'), '0=unknown\n');
assert.throws(() => assertPinned(gateSpec), /unrecognized ignored input.*unknown\.pack/);
fs.rmSync(path.join(gateContent, 'pack/unknown.pack'));
const packedConfig = path.join(gateEngine, 'data/pack/client/config');
fs.mkdirSync(path.dirname(packedConfig), { recursive: true });
fs.writeFileSync(packedConfig, 'packed config');
assert.throws(() => assertPinned(gateSpec), /unrecognized ignored input.*config/);
gateSpec.generatedInputs.engine.push(sourceFile(gateEngine, 'data/pack/client/config'));
assertPinned(gateSpec);
fs.appendFileSync(packedConfig, 'dirty');
assert.throws(() => assertPinned(gateSpec), /generated input hash mismatch.*config/);
fs.rmSync(packedConfig);
assert.throws(() => assertPinned(gateSpec), /generated input unavailable.*config/);
fs.writeFileSync(packedConfig, 'packed config');

// Real cache-identity admission, fixture source pin, and bounded producer bodies:
// refusal must leave the last complete asset directory byte-for-byte intact.
const selectedPin = revisions.find(spec => spec.revision === 289)!;
const pipelineOutput = fs.mkdtempSync(path.join(os.tmpdir(), 'selected-build-'));
const pipelineSpec = {
    ...selectedPin, content: gateContent, expectedContent: gateSpec.expectedContent,
    generatedInputs: {
        ...selectedPin.generatedInputs!, content_commit: gateSpec.expectedContent,
        content: gateSpec.generatedInputs.content,
    },
    output: path.join(pipelineOutput, '289.json'),
};
const liveDirectory = path.join(pipelineOutput, '289');
const navFile = path.join(pipelineOutput, 'nav.bin');
const flagsFile = path.join(pipelineOutput, 'flags.bin');
fs.writeFileSync(navFile, 'nav');
fs.writeFileSync(flagsFile, 'flags');
const build: SelectedBuild = {
    quests: () => ({ schema: 2, payload: { quest: 'cook' } }),
    gathering: () => ({ schema: 3, payload: { resource: 'oak' } }),
    bake: async families => ({
        nav: navFile, flags: flagsFile,
        quest_facts_sha256: families.quests.sha256, quest_extractor_schema: families.quests.schema,
    }),
    core: () => ({
        bytes: Buffer.from('{"core":"fixture"}\n'),
        action_limits: { user_events_per_tick: 5, source: { file: 'limits.ts', first: 1, last: 1 } },
    }),
};
try {
    const published = await generateSelected(pipelineSpec, build);
    const manifestBytes = fs.readFileSync(published.path, 'utf8');
    const manifest = JSON.parse(manifestBytes);
    // The exported serializer is the one the pipeline uses: byte-identical for both families.
    const upstream = familyInputs(pipelineSpec, assertPinned(pipelineSpec));
    assert.equal(fs.readFileSync(path.join(liveDirectory, 'gathering.json'), 'utf8'), familyBytes(upstream, { schema: 3, payload: { resource: 'oak' } }, 'gathering'));
    assert.equal(fs.readFileSync(path.join(liveDirectory, 'quests.json'), 'utf8'), familyBytes(upstream, { schema: 2, payload: { quest: 'cook' } }, 'quests'));
    assert.equal(sha256(path.join(liveDirectory, 'gathering.json')).sha256, manifest.families.gathering.sha256, 'the manifest digests the exact family bytes');
    assert.throws(() => familyBytes(upstream, { schema: 0, payload: {} }, 'gathering'), /invalid extractor schema/);
    assert.equal(published.sha256, sha256(published.path).sha256);
    assert.equal(manifestBytes.includes(published.sha256), false, 'manifest digest must be external');
    assert.equal(sha256(path.join(liveDirectory, 'quests.json')).sha256, manifest.families.quests.sha256);
    assert.equal(manifest.nav.quest_facts_sha256, manifest.families.quests.sha256);
    const before = fs.readdirSync(liveDirectory).map(name => [name, fs.readFileSync(path.join(liveDirectory, name))]);
    for (const mismatch of ['digest', 'schema']) {
        await assert.rejects(generateSelected(pipelineSpec, {
            ...build,
            bake: async families => ({
                ...await build.bake(families),
                ...(mismatch === 'digest' ? { quest_facts_sha256: families.gathering.sha256 } : { quest_extractor_schema: 99 }),
            }),
        }), /nav quest-family digest\/schema mismatch/);
        assert.deepEqual(fs.readdirSync(liveDirectory).map(name => [name, fs.readFileSync(path.join(liveDirectory, name))]), before);
    }
    await assert.rejects(generateSelected(pipelineSpec, {
        ...build,
        bake: async families => {
            fs.appendFileSync(paramPack, '1=mid-build-change\n');
            return build.bake(families);
        },
    }), /generated input hash mismatch.*param\.pack/);
    assert.deepEqual(fs.readdirSync(liveDirectory).map(name => [name, fs.readFileSync(path.join(liveDirectory, name))]), before);
    fs.writeFileSync(paramPack, '0=param\n');
    fs.writeFileSync(path.join(gateContent, 'scripts/.DS_Store'), 'finder metadata');
    const withJunk = await generateSelected(pipelineSpec, build);
    assert.equal(withJunk.sha256, published.sha256, 'platform junk cannot change family or manifest identity');
    fs.writeFileSync(navFile, 'new nav bytes');
    const rebaked = await generateSelected(pipelineSpec, build);
    const next = JSON.parse(fs.readFileSync(rebaked.path, 'utf8'));
    assert.equal(next.families.quests.sha256, manifest.families.quests.sha256, 'downstream nav cannot feed a family digest');
    assert.notEqual(next.nav.sha256, manifest.nav.sha256);
    assert.equal(fs.readdirSync(pipelineOutput).some(name => name.startsWith('.289-')), false, 'no staging or backup assets remain');
} finally {
    fs.rmSync(pipelineOutput, { recursive: true, force: true });
    fs.rmSync(gateEngine, { recursive: true, force: true });
    fs.rmSync(gateContent, { recursive: true, force: true });
}
assert.deepEqual(requestedRevisions(['--revision', '289']), [289]);
assert.throws(() => requestedRevisions(['--revision', '999']), /usage/);

function writeQuestFixture(rootDir: string, mutate?: (files: Record<string, string>) => void) {
    const files: Record<string, string> = {
        'scripts/general/scripts/quests.rs2': `~send_quest_progress_colour($component, $progress, $complete_progress);
%qp = $questpointrecount;
~send_quest_progress_colour(questlist:runemysteries, %runemysteries, ^runemysteries_complete);
~send_quest_progress_colour(questlist:cook, %cookquest, ^cook_complete);
~send_quest_progress_colour(questlist:zanaris, %zanaris, ^zanaris_complete);
~send_quest_progress_colour(questlist:waterfall, %waterfall_quest, ^waterfall_complete);
~send_quest_progress_colour(questlist:murder, %murderquest, ^murder_complete);
~send_quest_progress_colour(questlist:death, %death_equiproom, ^death_complete);
~send_quest_progress_colour(questlist:itexam, ~itexam_progress, ^itexam_complete);
~send_quest_progress_colour(questlist:blackarmgang, %phoenixgang, ^phoenixgang_complete);
~send_quest_progress_colour(questlist:blackarmgang, %blackarmgang, ^blackarmgang_complete);
~send_quest_progress_colour(questlist:elemental_workshop, %elemental_workshop_bits, sub(pow(2, ^elemental_workshop_complete), 1));
~send_quest_progress_colour(questlist:legends, %legendsquest, ^legends_complete);
~send_quest_progress_colour(questlist:regicide, %regicide_quest, ^regicide_complete);
`,
        'scripts/general/configs/quest.constant': `^cook_complete = 2
^cookquest_complete = 7
^cook_questpoints = 1
^runemysteries_complete = 6
^runemysteries_questpoints = 1
^murder_complete = 2
^murder_questpoints = 3
^waterfall_complete = 10
^waterfall_questpoints = 1
^death_equiproom_complete = 999
^death_complete = 80
^death_questpoints = 1
^zanaris_complete = 6
^zanaris_questpoints = 3
`,
        'scripts/player/interfaces/questlist.if': `[death]
y=1
text=Death Plateau
[zanaris]
y=2
text=Lost City
[cook]
y=99
text=Cook's Assistant
[runemysteries]
y=3
text=Rune Mysteries Quest
[murder]
y=4
text=Murder Mystery
[waterfall]
y=5
text=Waterfall Quest
`,
        'pack/varp.pack': `29=cookquest
63=runemysteries
192=murderquest
65=waterfall_quest
219=death
314=death_equiproom
147=zanaris
101=qp
`,
        'scripts/general/configs/quest.enum': `[quest_names_enum]
val=1,Cook's Assistant
val=13,Rune Mysteries Quest
val=37,Murder Mystery
val=50,Waterfall Quest
val=55,Death Plateau
val=34,Lost City
`,
        'scripts/quests/quest_cook/scripts/quest_cook.rs2': `if(inv_total(inv, pot_flour) > 0 & inv_total(inv, egg) > 0 & inv_total(inv, bucket_milk) > 0) {
}
inv_del(inv, egg, 1);
inv_del(inv, bucket_milk, 1);
inv_del(inv, pot_flour, 1);
`,
        'scripts/quests/quest_waterfall/scripts/quest_waterfall.rs2': `// rope comment is not a requirement list
if(last_useitem ! rope) {
}
if(last_useitem ! rope) {
}
`,
        'scripts/quests/quest_zanaris/scripts/quest_zanaris.rs2': `if(stat(woodcutting) < 36) {
}
if(stat(crafting) < 31) {
}
if(inv_total(inv, axe) > 0) {
}
inv_del(inv, knife, 1);
if(last_useitem ! dramen_branch) {
}
`,
        'scripts/quests/quest_death/scripts/death_sherpa.rs2': `inv_del(inv, death_climbingboots, 1);
inv_del(inv, death_secretwaymap, 1);
inv_del(inv, trout, 1);
`,
    };
    mutate?.(files);
    for (const [relative, body] of Object.entries(files)) {
        const absolute = path.join(rootDir, relative);
        fs.mkdirSync(path.dirname(absolute), { recursive: true });
        fs.writeFileSync(absolute, body);
    }
}
function questFixture(mutate?: (files: Record<string, string>) => void) {
    const rootDir = fs.mkdtempSync(path.join(os.tmpdir(), 'quest-identity-'));
    writeQuestFixture(rootDir, mutate);
    return rootDir;
}
function assertQuestRows(facts: ReturnType<typeof extractQuestIdentityFacts>) {
    assert.deepEqual(facts.rows.map((row) => row.id), ['cook', 'runemysteries', 'murder', 'waterfall', 'death', 'zanaris']);
    assert.deepEqual(facts.rows.map((row) => [row.display, row.varp, row.varp_id, row.complete, row.quest_points]), [
        ["Cook's Assistant", 'cookquest', 29, 2, 1],
        ['Rune Mysteries Quest', 'runemysteries', 63, 6, 1],
        ['Murder Mystery', 'murderquest', 192, 2, 3],
        ['Waterfall Quest', 'waterfall_quest', 65, 10, 1],
        ['Death Plateau', 'death_equiproom', 314, 80, 1],
        ['Lost City', 'zanaris', 147, 6, 3],
    ]);
    assert.equal(facts.rows.every((row) => row.requirements.qualification === 'partial' && row.requirements.unknown_as_satisfied === false && row.unknown_sides.length === 0), true);
    assert.equal(facts.rows.every((row) => !('enum_index' in row) && !('engine_id' in row) && typeof row.complete === 'number'), true);
    assert.equal(facts.rows.some((row) => row.varp_id === 101 || row.varp_id === 219 || row.varp === 'death'), false);
    assert.deepEqual(facts.rows[0].requirements.items, [
        { alias: 'egg', quantity: 1, kind: 'inv' },
        { alias: 'bucket_milk', quantity: 1, kind: 'inv' },
        { alias: 'pot_flour', quantity: 1, kind: 'inv' },
    ]);
    assert.equal(facts.rows[0].requirements.empty_must_have, false);
    for (const id of ['runemysteries', 'murder', 'death']) {
        const row = facts.rows.find((entry) => entry.id === id);
        assert.equal(row?.requirements.empty_must_have, true);
        assert.deepEqual(row?.requirements.items, []);
        assert.deepEqual(row?.requirements.skills, []);
    }
    assert.deepEqual(facts.rows.find((row) => row.id === 'waterfall')?.requirements.items, [{ alias: 'rope', quantity: null, kind: 'use-site' }]);
    assert.deepEqual(facts.rows.find((row) => row.id === 'zanaris')?.requirements.skills, [{ skill: 'woodcutting', level: 36 }, { skill: 'crafting', level: 31 }]);
    assert.deepEqual(facts.rows.find((row) => row.id === 'zanaris')?.requirements.items, []);
    const blob = JSON.stringify(facts);
    assert.equal(blob.includes('family-unavailable') || blob.includes('quest_prereqs') || blob.includes('death_climbingboots') || blob.includes('Egg') || blob.includes('Lost City Of Zanaris'), false);
    assert.equal('quest_prereqs' in facts, false);
}

const questRoot = questFixture();
const quest274 = extractQuestIdentityFacts(questRoot, 274);
assertQuestRows(quest274);
assert.equal(quest274.coverage.length, 1);
assert.deepEqual(quest274.coverage[0], { class: 'revision-absent', alias: 'routequest', on_revision: 274, other_pin_id: 387, copied: false, reason: '289-only quest, not copied onto 274' });
assert.equal('display' in quest274.coverage[0] || 'complete' in quest274.coverage[0] || 'quest_points' in quest274.coverage[0], false);
const quest289 = extractQuestIdentityFacts(questRoot, 289);
assertQuestRows(quest289);
assert.deepEqual(quest289.coverage, []);
assert.doesNotThrow(() => extractQuestIdentityFacts(questRoot, 274));

assert.throws(() => extractQuestIdentityFacts(questFixture((files) => { delete files['scripts/general/scripts/quests.rs2']; }), 274), /quests\.rs2/);
assert.throws(() => extractQuestIdentityFacts(questFixture((files) => { delete files['scripts/general/configs/quest.constant']; }), 274), /quest\.constant/);
assert.throws(() => extractQuestIdentityFacts(questFixture((files) => { delete files['scripts/player/interfaces/questlist.if']; }), 274), /questlist\.if/);
assert.throws(() => extractQuestIdentityFacts(questFixture((files) => { delete files['pack/varp.pack']; }), 274), /varp\.pack/);
assert.throws(() => extractQuestIdentityFacts(questFixture((files) => { delete files['scripts/general/configs/quest.enum']; }), 274), /quest\.enum/);
assert.throws(() => extractQuestIdentityFacts(questFixture((files) => { delete files['scripts/quests/quest_cook/scripts/quest_cook.rs2']; }), 274), /quest_cook\.rs2/);
assert.throws(() => extractQuestIdentityFacts(questFixture((files) => {
    files['pack/varp.pack'] = files['pack/varp.pack'].replace('314=death_equiproom\n', '');
}), 274), /absent from varp\.pack/);
assert.throws(() => extractQuestIdentityFacts(questFixture((files) => {
    files['scripts/general/scripts/quests.rs2'] = files['scripts/general/scripts/quests.rs2'].replace('%cookquest', '~cook_progress');
}), 274), /proc operand/);
assert.throws(() => extractQuestIdentityFacts(questFixture((files) => {
    files['scripts/general/scripts/quests.rs2'] += '~send_quest_progress_colour(questlist:cook, %cookquest, ^cook_complete);\n';
}), 274), /dual binding/);
assert.throws(() => extractQuestIdentityFacts(questFixture((files) => {
    files['scripts/general/scripts/quests.rs2'] = files['scripts/general/scripts/quests.rs2'].replace('^cook_complete);', 'sub(^cook_complete, 0));');
}), 274), /computed complete/);
assert.throws(() => extractQuestIdentityFacts(questFixture((files) => {
    files['scripts/general/scripts/quests.rs2'] = files['scripts/general/scripts/quests.rs2'].replace('%death_equiproom, ^death_complete', '%death_equiproom, ^death_equiproom_complete');
}), 274), /constant stem/);
assert.throws(() => extractQuestIdentityFacts(questFixture((files) => {
    files['scripts/player/interfaces/questlist.if'] = files['scripts/player/interfaces/questlist.if'].replace("text=Rune Mysteries Quest", 'text=Rune Mysteries');
}), 274), /display mismatch/);
assert.throws(() => extractQuestIdentityFacts(questFixture((files) => {
    files['scripts/player/interfaces/questlist.if'] = files['scripts/player/interfaces/questlist.if'].replace('text=Lost City', 'text=Lost City Of Zanaris');
}), 274), /display mismatch/);
assert.throws(() => extractQuestIdentityFacts(questFixture((files) => {
    files['scripts/general/scripts/quests.rs2'] = '// no colour calls\n';
}), 274), /no extracted rows/);
assert.throws(() => extractQuestIdentityFacts(questFixture((files) => {
    files['pack/varp.pack'] += '387=routequest\n';
}), 274), /routequest is in varp\.pack/);

const withExtras = extractQuestIdentityFacts(questFixture((files) => {
    files['scripts/general/scripts/quests.rs2'] += `~send_quest_progress_colour(questlist:routequest, %routequest, ^routequest_complete);
~send_quest_progress_colour(questlist:misc, %misc_quest, ^misc_complete);
~send_quest_progress_colour(questlist:troll_love, %troll_love, ^troll_love_complete);
~send_quest_progress_colour(questlist:mm, %mm_main, ^mm_complete);
`;
    files['pack/varp.pack'] += '387=routequest\n359=misc_quest\n385=troll_love\n365=mm_main\n';
    files['scripts/player/interfaces/questlist.if'] += '[routequest]\ny=372\ntext=In Search of the Myreque\n';
    files['scripts/general/configs/quest.enum'] += 'val=99,In Search of the Myreque\n';
    files['scripts/general/configs/quest.constant'] += '^routequest_complete = 105\n^routequest_questpoints = 2\n';
}), 289);
assertQuestRows(withExtras);
assert.deepEqual(withExtras.coverage, []);
assert.equal(withExtras.rows.some((row) => row.id === 'routequest' || row.display === 'In Search of the Myreque' || row.complete === 105 || row.varp_id === 387), false);
assert.equal(withExtras.rows.some((row) => row.id === 'misc' || row.id === 'troll_love' || row.id === 'mm'), false);

const pinQuest274 = extractQuestIdentityFacts('/Users/acfrazier/experiments/Server/content', 274);
const pinQuest289 = extractQuestIdentityFacts('/Users/acfrazier/experiments/lostcity-289/content', 289);
assertQuestRows(pinQuest274);
assertQuestRows(pinQuest289);
assert.deepEqual(pinQuest274.rows.map((row) => [row.id, row.varp, row.varp_id, row.complete, row.quest_points]), pinQuest289.rows.map((row) => [row.id, row.varp, row.varp_id, row.complete, row.quest_points]));
assert.equal(pinQuest274.coverage.length, 1);
assert.equal(pinQuest274.coverage[0].alias, 'routequest');
assert.equal(pinQuest274.coverage[0].other_pin_id, 387);
assert.equal(pinQuest274.coverage[0].copied, false);
assert.equal(pinQuest274.coverage[0].class, 'revision-absent');
assert.deepEqual(pinQuest289.coverage, []);
assert.equal(pinQuest274.rows.some((row) => row.display === 'In Search of the Myreque' || row.complete === 105), false);
assert.equal(pinQuest289.rows.some((row) => row.id === 'routequest' || row.id === 'misc' || row.id === 'troll_love' || row.id === 'mm'), false);


// ---- trails ------------------------------------------------------------------

const TRAIL_CONFIG = 'scripts/minigames/game_trail/configs';
const TRAIL_FIXTURE_IDS: Record<string, number> = {
    trail_clue_easy_simple001: 2677,
    trail_clue_easy_vague003: 2678,
    trail_clue_easy_map001: 2713,
    trail_clue_easy_map001_casket: 2714,
    trail_clue_medium_anagram001: 2841,
    trail_clue_medium_anagram001_challenge: 2842,
    trail_clue_medium_anagram002_challenge: 2844,
    trail_clue_medium_anagram003_challenge: 2846,
    trail_clue_medium_anagram006_challenge: 2850,
    trail_clue_medium_anagram007_challenge: 2852,
    trail_clue_medium_anagram008_challenge: 2854,
    trail_clue_medium_map002: 2831,
    trail_clue_medium_map002_casket: 2830,
    trail_clue_hard_riddle004: 2778,
    trail_clue_hard_riddle004_casket: 2779,
    trail_clue_hard_sextant004: 3600,
    trail_clue_hard_sextant004_casket: 3534,
    trail_clue_hard_sextant016: 3587,
    trail_clue_hard_sextant016_casket: 3531,
    trail_clue_hard_sextant017: 3532,
    trail_clue_hard_sextant017_casket: 3533,
    trail_clue_hard_sextant026: 3552,
    trail_clue_hard_sextant026_casket: 3551,
    trail_clue_hard_sextant028: 3554,
    trail_clue_hard_sextant028_casket: 3555,
};
const trailFixtureItems = Object.entries(TRAIL_FIXTURE_IDS).map(([debugname, id]) => ({
    id,
    debugname,
    name: debugname.endsWith('_casket') ? 'Casket' : 'Clue scroll',
    cost: 1,
    stackable: false,
    members: true,
    certlink: -1,
    certtemplate: -1,
    wearpos: -1,
    wearpos2: 0,
    wearpos3: 0,
})) as any;

function writeTrailFixture(rootDir: string, mutate?: (files: Record<string, string>) => void) {
    const files: Record<string, string> = {
        'pack/obj.pack': `${Object.entries(TRAIL_FIXTURE_IDS).map(([alias, id]) => `${id}=${alias}`).join('\n')}\n`,
        [`${TRAIL_CONFIG}/trail_easy.enum`]: `[trail_easy_enum]
inputtype=int
outputtype=namedobj
val=0,trail_clue_easy_simple001
val=1,trail_clue_easy_vague003
val=2,trail_clue_easy_map001
`,
        [`${TRAIL_CONFIG}/trail_medium.enum`]: `[trail_medium_enum]
inputtype=int
outputtype=namedobj
val=0,trail_clue_medium_anagram001
`,
        [`${TRAIL_CONFIG}/trail_hard.enum`]: `[trail_hard_enum]
inputtype=int
outputtype=namedobj
val=0,trail_clue_hard_sextant004
val=1,trail_clue_hard_riddle004
val=2,trail_clue_hard_sextant016
val=3,trail_clue_hard_sextant017
val=4,trail_clue_hard_sextant028
`,
        [`${TRAIL_CONFIG}/trail_easy.obj`]: `[trail_clue_easy_simple001]
name=Clue scroll
category=trail_clue_easy
param=trail_desc,Search the chest in the|Duke of Lumbridge's bedroom.
param=trail_coord,1_50_50_9_18
param=trail_loc,^true
param=trail_loc,^false

[trail_clue_easy_vague003]
name=Clue scroll
category=trail_clue_easy
param=trail_desc,Dig near the drawers.

[trail_clue_easy_map001]
name=Clue scroll
category=trail_clue_easy
param=trail_coord,0_49_52_41_32
param=trail_casket,trail_clue_easy_map001_casket
`,
        [`${TRAIL_CONFIG}/trail_medium.obj`]: `[trail_clue_medium_anagram001]
name=Clue scroll
category=trail_clue_medium
param=trail_desc,Speak to Hazelmere.

[trail_clue_medium_anagram001_challenge]
name=Challenge scroll
category=trail_clue_medium
param=trail_challenge_answer,6859

[trail_clue_medium_anagram002_challenge]
name=Challenge scroll
category=trail_clue_medium
param=trail_challenge_answer,9

[trail_clue_medium_anagram003_challenge]
name=Challenge scroll
category=trail_clue_medium
param=trail_challenge_answer,40

[trail_clue_medium_anagram006_challenge]
name=Challenge scroll
category=trail_clue_medium
param=trail_challenge_answer,5

[trail_clue_medium_anagram007_challenge]
name=Challenge scroll
category=trail_clue_medium
param=trail_challenge_answer,48

[trail_clue_medium_anagram008_challenge]
name=Challenge scroll
category=trail_clue_medium
param=trail_challenge_answer,5096

[trail_clue_medium_map002]
name=Clue scroll
category=trail_clue_medium
param=trail_coord,0_42_53_14_36
param=trail_casket,trail_clue_medium_map002_casket
`,
        [`${TRAIL_CONFIG}/trail_hard.obj`]: `[trail_clue_hard_sextant004]
name=Clue scroll
category=trail_clue_hard
param=trail_sextant,yes
param=trail_casket,trail_clue_hard_sextant004_casket

[trail_clue_hard_riddle004]
name=Clue scroll
category=trail_clue_hard
param=trail_desc,Speak to the keeper of my trail.
param=trail_casket,trail_clue_hard_riddle004_casket

[trail_clue_hard_sextant016]
name=Clue scroll
category=trail_clue_hard
param=trail_sextant,yes
param=trail_coord,0_43_45_23_11
param=trail_casket,trail_clue_hard_sextant016_casket

[trail_clue_hard_sextant017]
name=Clue scroll
category=trail_clue_hard
param=trail_sextant,yes
param=trail_casket,trail_clue_hard_sextant016_casket
param=trail_guardian,trail_hard2

[trail_clue_hard_sextant028]
name=Clue scroll
category=trail_clue_hard
param=trail_desc,02 degrees 46 minutes North|29 degrees 11 minutes East
param=trail_sextant,yes
param=trail_casket,trail_clue_hard_sextant028_casket
param=trail_coord,0_52_50_46_50

[trail_clue_hard_sextant026]
name=Clue scroll
category=trail_clue_hard

[trail_clue_hard_sextant026_casket]
name=Casket
category=trail_casket_hard
`,
        [`${TRAIL_CONFIG}/trail_casket.obj`]: `[trail_clue_hard_sextant004_casket]
name=Casket
category=trail_casket_hard
`,
    };
    mutate?.(files);
    for (const [relative, body] of Object.entries(files)) {
        const absolute = path.join(rootDir, relative);
        fs.mkdirSync(path.dirname(absolute), { recursive: true });
        fs.writeFileSync(absolute, body);
    }
}
function trailFixture(mutate?: (files: Record<string, string>) => void) {
    const rootDir = fs.mkdtempSync(path.join(os.tmpdir(), 'trail-inventory-'));
    writeTrailFixture(rootDir, mutate);
    return rootDir;
}
function trailRow(facts: ReturnType<typeof extractTrailFacts>, alias: string) {
    const found = facts.rows.find((row) => row.alias === alias);
    assert.ok(found, `missing trail row ${alias}`);
    return found;
}
const TRAIL_HANDLER_CONSTANT_COORDS = ['0_51_54_45_47', '1_42_53_14_17', '1_40_51_14_62'];

const trailFacts = extractTrailFacts(trailFixture(), trailFixtureItems);
assert.doesNotThrow(() => assertTrailPins(trailFacts, 274));

// repeated param= keys survive as a list, not last-write-wins
assert.deepEqual(trailRow(trailFacts, 'trail_clue_easy_simple001').params, [
    { key: 'trail_desc', value: "Search the chest in the|Duke of Lumbridge's bedroom." },
    { key: 'trail_coord', value: '1_50_50_9_18' },
    { key: 'trail_loc', value: '^true' },
    { key: 'trail_loc', value: '^false' },
]);
// ^true and yes stay raw strings
assert.equal(trailRow(trailFacts, 'trail_clue_hard_sextant016').params.some((param) => param.key === 'trail_sextant' && param.value === 'yes'), true);
assert.equal(trailRow(trailFacts, 'trail_clue_hard_sextant017').params.some((param) => param.key === 'trail_guardian' && param.value === 'trail_hard2'), true);
// a block without trail_sextant omits the key: absent is not false
assert.equal(trailRow(trailFacts, 'trail_clue_easy_simple001').params.some((param) => param.key === 'trail_sextant'), false);
assert.equal(trailFacts.rows.some((row) => row.alias === 'trail_clue_medium_anagram001' && row.params.some((param) => param.key === 'trail_sextant')), false);
assert.equal(trailFacts.rows.every((row) => row.params.every((param) => typeof param.value === 'string' && typeof param.key === 'string')), true);

// the six answers are a sibling list on the challenge aliases, never a membership row
assert.deepEqual(trailFacts.challenge_answers, [
    { alias: 'trail_clue_medium_anagram001_challenge', id: 2842, answer: '6859' },
    { alias: 'trail_clue_medium_anagram002_challenge', id: 2844, answer: '9' },
    { alias: 'trail_clue_medium_anagram003_challenge', id: 2846, answer: '40' },
    { alias: 'trail_clue_medium_anagram006_challenge', id: 2850, answer: '5' },
    { alias: 'trail_clue_medium_anagram007_challenge', id: 2852, answer: '48' },
    { alias: 'trail_clue_medium_anagram008_challenge', id: 2854, answer: '5096' },
]);
assert.equal(trailFacts.rows.some((row) => row.alias.endsWith('_challenge')), false);
assert.equal(trailFacts.rows.some((row) => row.params.some((param) => param.key === 'trail_challenge_answer')), false);
// the answer is not copied onto the parent that shares its prefix, and no link is invented
assert.deepEqual(trailRow(trailFacts, 'trail_clue_medium_anagram001').params, [{ key: 'trail_desc', value: 'Speak to Hazelmere.' }]);
assert.equal(JSON.stringify(trailFacts).includes('"parent"'), false);
assert.equal(JSON.stringify(trailFacts).includes('npc'), false);

// the casket alias is the param value, not the suffix-matching pack alias
assert.deepEqual(trailRow(trailFacts, 'trail_clue_hard_sextant016_casket'), { alias: 'trail_clue_hard_sextant016_casket', id: 3531, role: 'casket', params: [] });
assert.equal(trailFacts.rows.filter((row) => row.id === 3531).length, 1);
assert.deepEqual(trailRow(trailFacts, 'trail_clue_hard_sextant017').params.filter((param) => param.key === 'trail_casket'), [{ key: 'trail_casket', value: 'trail_clue_hard_sextant016_casket' }]);
assert.equal(trailFacts.rows.some((row) => row.id === 3533 || row.alias === 'trail_clue_hard_sextant017_casket'), false);
// a casket with its own obj block is still a row without params
assert.deepEqual(trailRow(trailFacts, 'trail_clue_hard_sextant004_casket').params, []);
assert.equal(trailRow(trailFacts, 'trail_clue_hard_sextant004_casket').role, 'casket');
// a named casket with no obj block is still a row: alias, id, no params
assert.deepEqual(trailRow(trailFacts, 'trail_clue_hard_riddle004_casket'), { alias: 'trail_clue_hard_riddle004_casket', id: 2779, role: 'casket', params: [] });

// blocks that no enum row lists are not members, and neither are their caskets
for (const alias of ['trail_clue_medium_map002', 'trail_clue_medium_map002_casket', 'trail_clue_hard_sextant026', 'trail_clue_hard_sextant026_casket']) {
    assert.equal(trailFacts.rows.some((row) => row.alias === alias), false);
}

// membership is not support: no support field, no coverage label, no class list
assert.equal(JSON.stringify(trailFacts).includes('supported'), false);
assert.equal(JSON.stringify(trailFacts).includes('facts-verified-both'), false);
assert.equal(JSON.stringify(trailFacts).includes('coverage'), false);
assert.equal(trailFacts.rows.every((row) => Object.keys(row).every((key) => ['alias', 'id', 'role', 'params', 'access'].includes(key))), true);
assert.equal(trailFacts.rows.some((row) => 'display' in row || 'name' in row || 'enum_index' in row), false);

// one access qualification, and it is not "open"
const constrainedRows = trailFacts.rows.filter((row) => 'access' in row);
assert.deepEqual(constrainedRows.map((row) => [row.alias, row.id, row.access]), [['trail_clue_hard_sextant028', 3554, 'constrained']]);
assert.equal(constrainedRows[0].role, 'clue');
assert.equal(JSON.stringify(trailFacts).includes('"open"'), false);

// a constant-table coordinate is not selected when the obj param is absent
assert.deepEqual(trailRow(trailFacts, 'trail_clue_easy_vague003').params, [{ key: 'trail_desc', value: 'Dig near the drawers.' }]);
assert.equal(trailFacts.rows.some((row) => row.params.some((param) => param.key === 'handler_coord' || param.key === 'coord')), false);
assert.equal(TRAIL_HANDLER_CONSTANT_COORDS.some((coord) => JSON.stringify(trailFacts).includes(coord)), false);

// fail closed: a missing required input, a missing join, and an id mismatch
assert.throws(() => extractTrailFacts(trailFixture((files) => { delete files[`${TRAIL_CONFIG}/trail_easy.enum`]; }), trailFixtureItems), /trail_easy\.enum/);
assert.throws(() => extractTrailFacts(trailFixture((files) => { delete files[`${TRAIL_CONFIG}/trail_casket.obj`]; }), trailFixtureItems), /trail_casket\.obj/);
assert.throws(() => extractTrailFacts(trailFixture((files) => { delete files['pack/obj.pack']; }), trailFixtureItems), /pack\/obj\.pack/);
assert.throws(() => extractTrailFacts(trailFixture((files) => {
    files['pack/obj.pack'] = files['pack/obj.pack'].replace('3554=trail_clue_hard_sextant028\n', '');
}), trailFixtureItems), /pack\/obj\.pack lacks trail_clue_hard_sextant028/);
assert.throws(() => extractTrailFacts(trailFixture((files) => {
    files['pack/obj.pack'] = files['pack/obj.pack'].replace('2714=trail_clue_easy_map001_casket\n', '');
}), trailFixtureItems), /pack\/obj\.pack lacks trail_clue_easy_map001_casket/);
assert.throws(() => extractTrailFacts(trailFixture((files) => {
    files['pack/obj.pack'] = files['pack/obj.pack'].replace('2779=trail_clue_hard_riddle004_casket', '9999=trail_clue_hard_riddle004_casket');
}), trailFixtureItems), /disagrees with decoded item id/);
assert.throws(() => extractTrailFacts(trailFixture((files) => {
    files[`${TRAIL_CONFIG}/trail_easy.enum`] += 'val=9,trail_clue_easy_missing\n';
}), trailFixtureItems), /has no obj block/);
assert.throws(() => extractTrailFacts(trailFixture(), trailFixtureItems.filter((item: any) => item.debugname !== 'trail_clue_hard_sextant028')), /has no decoded item row/);

// A fixture that drops trail_clue_hard_riddle004_casket publishes one row fewer:
// that is the rejected corpus aggregate (186 clues plus 70 caskets), and the
// producer's own pin check refuses it.
const droppedCasketFixture = extractTrailFacts(trailFixture((files) => {
    files[`${TRAIL_CONFIG}/trail_hard.obj`] = (files[`${TRAIL_CONFIG}/trail_hard.obj`] as string).replace('param=trail_casket,trail_clue_hard_riddle004_casket\n', '');
}), trailFixtureItems);
assert.equal(droppedCasketFixture.rows.length, trailFacts.rows.length - 1);
assert.equal(droppedCasketFixture.rows.some((row) => row.alias === 'trail_clue_hard_riddle004_casket'), false);
assert.throws(() => assertTrailPins(droppedCasketFixture, 274), /missing trail_clue_hard_riddle004_casket/);

const pinTrail274Root = '/Users/acfrazier/experiments/Server/content';
const pinTrail289Root = '/Users/acfrazier/experiments/lostcity-289/content';
function pinDecodedItems(revision: number) {
    const payload = JSON.parse(fs.readFileSync(path.join(repoRoot, `crates/api/data/game-data/${revision}.json`), 'utf8')) as { items: any[] };
    return payload.items.filter((item: any) => item.alias !== null).map((item: any) => ({ id: item.id, debugname: item.alias, name: item.name, cost: item.cost, stackable: item.stackable, members: item.members, certlink: item.certificate_link, certtemplate: item.certificate_template, wearpos: item.wear_position, wearpos2: item.wear_position_2, wearpos3: item.wear_position_3 })) as any;
}
function pinTrailInputs(root: string) {
    const enums = ['easy', 'medium', 'hard'].flatMap((tier) => parseTrailEnumAliases(fs.readFileSync(path.join(root, `${TRAIL_CONFIG}/trail_${tier}.enum`), 'utf8')));
    const blocks = new Map<string, { key: string; value: string }[]>();
    for (const tier of ['easy', 'medium', 'hard']) {
        for (const [alias, params] of parseTrailObjBlocks(fs.readFileSync(path.join(root, `${TRAIL_CONFIG}/trail_${tier}.obj`), 'utf8'))) blocks.set(alias, params);
    }
    const caskets: string[] = [];
    for (const alias of enums) {
        for (const param of blocks.get(alias) ?? []) {
            if (param.key === 'trail_casket' && !caskets.includes(param.value)) caskets.push(param.value);
        }
    }
    return { enums, caskets };
}

const pinTrail274 = extractTrailFacts(pinTrail274Root, pinDecodedItems(274));
const pinTrail289 = extractTrailFacts(pinTrail289Root, pinDecodedItems(289));
assert.deepEqual(pinTrail274, pinTrail289);
assert.doesNotThrow(() => assertTrailPins(pinTrail274, 274));
assert.doesNotThrow(() => assertTrailPins(pinTrail289, 289));
const pinTrailInputs274 = pinTrailInputs(pinTrail274Root);
assert.equal(pinTrailInputs274.enums.includes('trail_clue_medium_map002'), false);
assert.equal(pinTrailInputs274.enums.includes('trail_clue_hard_sextant026'), false);
assert.equal(pinTrail274.rows.filter((row) => row.role === 'clue').length, pinTrailInputs274.enums.length);
assert.equal(pinTrail274.rows.filter((row) => row.role === 'casket').length, pinTrailInputs274.caskets.length);
assert.equal(pinTrail274.rows.length, pinTrailInputs274.enums.length + pinTrailInputs274.caskets.length);
assert.equal(new Set(pinTrail274.rows.map((row) => row.alias)).size, pinTrail274.rows.length);
assert.equal(pinTrail274.rows.filter((row) => row.id === 3531).length, 1);
assert.equal(trailRow(pinTrail274, 'trail_clue_hard_riddle004_casket').id, 2779);
assert.equal(trailRow(pinTrail274, 'trail_clue_hard_sextant016_casket').id, 3531);
assert.equal(trailRow(pinTrail274, 'trail_clue_hard_sextant017').params.some((param) => param.key === 'trail_casket' && param.value === 'trail_clue_hard_sextant016_casket'), true);
assert.equal(pinTrail274.rows.some((row) => row.id === 3533), false);
assert.equal(trailRow(pinTrail274, 'trail_clue_easy_simple001').params.some((param) => param.key === 'trail_loc' && param.value === '^true'), true);
assert.equal(trailRow(pinTrail274, 'trail_clue_hard_sextant028').params.some((param) => param.key === 'trail_sextant' && param.value === 'yes'), true);
assert.equal(trailRow(pinTrail274, 'trail_clue_medium_anagram001').params.some((param) => param.key === 'trail_sextant'), false);
assert.deepEqual(pinTrail274.rows.filter((row) => 'access' in row).map((row) => [row.alias, row.id, row.access]), [['trail_clue_hard_sextant028', 3554, 'constrained']]);
assert.equal(pinTrail274.rows.every((row) => Object.keys(row).every((key) => ['alias', 'id', 'role', 'params', 'access'].includes(key))), true);
assert.equal(JSON.stringify(pinTrail274).includes('facts-verified-both'), false);
assert.equal(JSON.stringify(pinTrail274).includes('supported'), false);
assert.equal(TRAIL_HANDLER_CONSTANT_COORDS.some((coord) => JSON.stringify(pinTrail274).includes(coord)), false);
assert.equal(pinTrail274.challenge_answers.some((entry) => typeof entry.answer !== 'string'), false);
assert.equal(pinTrail274.rows.some((row) => ['trail_clue_medium_map002', 'trail_clue_hard_sextant026'].includes(row.alias)), false);
assert.equal(pinTrail274.rows.filter((row) => row.alias.startsWith('trail_clue_hard_riddle0') && row.role === 'casket' && row.id === 2779).length, 1);

// Kharazi and Tirannwn members are inventoried as plain rows: no crossing class,
// no jungle-cut or seam fact, no guardian display name, and no puzzle pieces.
for (const [alias, id] of [['trail_clue_hard_sextant017', 3532], ['trail_clue_hard_sextant018', 3534], ['trail_clue_hard_sextant019', 3536], ['trail_clue_hard_sextant031', 3560], ['trail_clue_hard_sextant032', 3562], ['trail_clue_hard_riddle018', 3564]] as const) {
    const row = trailRow(pinTrail274, alias);
    assert.equal(row.id, id);
    assert.equal(row.role, 'clue');
    assert.equal('access' in row, false);
}
assert.equal(pinTrail274.rows.some((row) => row.alias.endsWith('_puzzlebox')), false);
assert.equal(JSON.stringify(pinTrail274).includes('trail_puzzle'), false);
assert.equal(JSON.stringify(pinTrail274).includes('"npc"'), false);
assert.equal(JSON.stringify(pinTrail274).includes('regicide'), false);
assert.equal(JSON.stringify(pinTrail274).includes('legends'), false);

/* ------------------------------------------------------------------ *
 * talk_key: opnpc1 talk steps, trail_checkmediumdrop keepers, jm2 spawns
 * ------------------------------------------------------------------ */

// NPC placements are the `==== NPC ====` section only, with exactly one data token
assert.deepEqual(parseJm2NpcPlacements('==== NPC ====\n0 10 20: 669\n1 20 30: 0\n'), [
    { plane: 0, lx: 10, lz: 20, npc_id: 669 },
    { plane: 1, lx: 20, lz: 30, npc_id: 0 },
]);
assert.deepEqual(parseJm2NpcPlacements('==== LOC ====\n0 47 62: 2662 10 1\n'), [], 'a LOC row is not an NPC placement');
assert.deepEqual(
    [...parseJm2LinkBelow('==== MAP ====\n0 1 1: h1 f2\n1 1 1: h1 f2\n1 2 3: h1 f1\n1 4 5: f3\n==== LOC ====\n1 6 6: 2728\n')],
    ['1,1', '4,5'],
    'LINK_BELOW is the level-1 MAP flag 0x2 only',
);
{
    // The rs2b0t sources must be the pinned blobs: a local edit (or another commit) refuses.
    const pinRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'rs2b0t-pin-'));
    const source = path.join(pinRoot, 'src/bot/data/cookLocations.ts');
    fs.mkdirSync(path.dirname(source), { recursive: true });
    fs.writeFileSync(source, 'export const MAX_SURFACE_CHEB = 20;\n');
    const blob = execFileSync('git', ['hash-object', source], { encoding: 'utf8' }).trim();
    const pin = { commit: 'fixture', blobs: { 'src/bot/data/cookLocations.ts': blob } };
    assertRs2b0tPinned(pinRoot, pin);
    fs.writeFileSync(source, 'export const MAX_SURFACE_CHEB = 21;\n');
    assert.throws(() => assertRs2b0tPinned(pinRoot, pin), /dirty or not the pinned export/);
    fs.rmSync(pinRoot, { recursive: true, force: true });

    // Any .loc/.npc config under scripts/ and the loc/npc packs are selected-content inputs.
    const content = fs.mkdtempSync(path.join(os.tmpdir(), 'content-dirt-'));
    const git = (...args: string[]) => execFileSync('git', ['-C', content, ...args], { encoding: 'utf8' });
    git('init', '-q');
    const loc = path.join(content, 'scripts/areas/area_x/configs/x.loc');
    fs.mkdirSync(path.dirname(loc), { recursive: true });
    fs.writeFileSync(loc, '[range]\nname=Range\n');
    fs.mkdirSync(path.join(content, 'pack'));
    fs.writeFileSync(path.join(content, 'pack/npc.pack'), '0=man\n');
    git('add', '-A');
    git('-c', 'user.email=t@t', '-c', 'user.name=t', 'commit', '-q', '-m', 'fixture');
    assert.equal(contentDirt(content), '', 'a clean tree has no dirt');
    fs.writeFileSync(loc, '[range]\nname=Oven\n');
    assert.match(contentDirt(content), /x\.loc/, 'an edited nested .loc is dirty');
    fs.writeFileSync(loc, '[range]\nname=Range\n');
    fs.writeFileSync(path.join(content, 'scripts/new.npc'), '[a]\n');
    assert.match(contentDirt(content), /new\.npc/, 'an untracked .npc is dirty');
    fs.rmSync(path.join(content, 'scripts/new.npc'));
    fs.writeFileSync(path.join(content, 'pack/npc.pack'), '0=woman\n');
    assert.match(contentDirt(content), /npc\.pack/, 'an edited npc.pack is dirty');
    fs.rmSync(content, { recursive: true, force: true });
}
assert.deepEqual(parseJm2NpcPlacements('==== OBJ ====\n0 47 62: 2662\n==== NPC ====\n0 1 2: 5\n'), [{ plane: 0, lx: 1, lz: 2, npc_id: 5 }], 'only the NPC section is a placement source');
assert.deepEqual(parseJm2NpcPlacements('==== NPC ====\n'), []);
assert.throws(() => parseJm2NpcPlacements('==== NPC ====\n0 10 20: 669 10 1\n'), /extra tokens/, 'a shape and an angle are not NPC row tokens');
assert.throws(() => parseJm2NpcPlacements('==== NPC ====\n0 10 20: 669 1\n'), /extra tokens/);
assert.throws(() => parseJm2NpcPlacements('==== NPC ====\n0 10 20:\n'), /missing npc id/);
assert.throws(() => parseJm2NpcPlacements('==== NPC ====\n4 10 20: 669\n'), /plane out of range/);
assert.throws(() => parseJm2NpcPlacements('==== NPC ====\n0 64 20: 669\n'), /local coords out of range/);
assert.throws(() => parseJm2NpcPlacements('==== NPC ====\n0 10 20 669\n'), /malformed row/);

const TALK_KEY_CONFIG = 'scripts/minigames/game_trail/configs';
const TALK_KEY_MEDIUM = 'scripts/minigames/game_trail/scripts/medium/trail_clue_medium.rs2';
const TALK_KEY_IDS: Record<string, number> = {
    trail_clue_easy_simple005: 2681,
    trail_clue_easy_simple008: 2684,
    trail_clue_easy_vague012: 3496,
    trail_clue_hard_riddle012: 2792,
    trail_clue_medium_anagram001: 2841,
    trail_clue_medium_riddle001: 2831,
    trail_clue_medium_riddle001_key: 2832,
    trail_clue_medium_riddle002: 2833,
    trail_clue_medium_riddle002_key: 2834,
    trail_clue_medium_riddle004: 2837,
    trail_clue_medium_riddle004_key: 2838,
    trail_clue_medium_riddle005: 2839,
    trail_clue_medium_riddle005_key: 2840,
};
const talkKeyItems = Object.entries(TALK_KEY_IDS).map(([debugname, id]) => ({
    id,
    debugname,
    name: debugname.endsWith('_key') ? 'Key' : 'Clue scroll',
    cost: 1,
    stackable: false,
    members: true,
    certlink: -1,
    certtemplate: -1,
    wearpos: -1,
    wearpos2: 0,
    wearpos3: 0,
}));

function writeTalkKeyFixture(rootDir: string, mutate?: (files: Record<string, string>) => void) {
    const medium = `[proc,trail_checkmediumdrop]
if(npc_type = black_heather & inv_total(inv, trail_clue_medium_riddle001) > 0 & ~obj_gettotal(trail_clue_medium_riddle001_key) = 0) {
    obj_add(npc_coord, trail_clue_medium_riddle001_key, 1, ^lootdrop_duration);
} else if(npc_type = ardougne_guard & inv_total(inv, trail_clue_medium_riddle002) > 0 & ~obj_gettotal(trail_clue_medium_riddle002_key) = 0) {
    obj_add(npc_coord, trail_clue_medium_riddle002_key, 1, ^lootdrop_duration);
} else if(npc_category = chicken & inv_total(inv, trail_clue_medium_riddle004) > 0 & ~obj_gettotal(trail_clue_medium_riddle004_key) = 0) {
    obj_add(npc_coord, trail_clue_medium_riddle004_key, 1, ^lootdrop_duration);
} else if(compare(npc_name, "Man") = 0 & inv_total(inv, trail_clue_medium_riddle005) > 0 & ~obj_gettotal(trail_clue_medium_riddle005_key) = 0) {
    obj_add(npc_coord, trail_clue_medium_riddle005_key, 1, ^lootdrop_duration);
}

// A loc-search label is not a keeper arm.
[label,riddle_loc_medium_exp001]
if(inv_total(inv, trail_clue_medium_riddle001) > 0) {
    ~mesbox("The chest is locked!|Property of Black Heather.");
}
`;
    const files: Record<string, string> = {
        'pack/obj.pack': `${Object.entries(TALK_KEY_IDS).map(([alias, id]) => `${id}=${alias}`).join('\n')}\n`,
        'pack/npc.pack': '0=hans\n32=ardougne_guard\n202=black_heather\n376=captain_tobias\n669=grandtree_hazelmere\n804=tanner\n',
        [`${TALK_KEY_CONFIG}/trail_easy.enum`]: `[trail_easy_enum]
val=0,trail_clue_easy_simple005
val=1,trail_clue_easy_simple008
val=2,trail_clue_easy_vague012
`,
        [`${TALK_KEY_CONFIG}/trail_medium.enum`]: `[trail_medium_enum]
val=0,trail_clue_medium_anagram001
val=1,trail_clue_medium_riddle001
val=2,trail_clue_medium_riddle002
val=3,trail_clue_medium_riddle004
val=4,trail_clue_medium_riddle005
`,
        [`${TALK_KEY_CONFIG}/trail_hard.enum`]: `[trail_hard_enum]
val=0,trail_clue_hard_riddle012
`,
        [TALK_KEY_MEDIUM]: medium,
        'scripts/areas/area_gnome/scripts/hazelmere.rs2': `[opnpc1,grandtree_hazelmere]
if(inv_total(inv, trail_clue_medium_anagram001) > 0) {
    @trail_hazelmere;
}

[label,trail_hazelmere]
if(inv_total(inv, trail_clue_medium_anagram001_challenge) > 0) {
    ~chatnpc("<p,neutral>Blah, blah?");
}
`,
        'scripts/areas/area_lumbridge/scripts/hans.rs2': `[opnpc1,hans]
if(inv_total(inv, trail_clue_easy_simple005) > 0) {
    @trail_hans;
}
if(inv_total(inv, trail_clue_hard_riddle012) > 0) {
    @trail_hans;
}
if(inv_total(inv, trail_clue_hard_riddle012_puzzlebox) > 0) {
    @trail_hans;
}
`,
        'scripts/areas/area_port_sarim/scripts/sailors.rs2': `[opnpc1,_sailor]
if(map_members = ^true & npc_type = captain_tobias) {
    if(inv_total(inv, trail_clue_easy_vague012) > 0) {
        ~chatnpc("<p,happy>Well done, matey! Here you go!");
        return;
    }
}
@karamja_sailor_dialogue("Karamja", 1_46_49_12_7, ^sail_port_sarim_to_karamja);
`,
        'scripts/areas/area_alkharid/scripts/tanner.rs2': `[opnpc1,tanner]
if(inv_total(inv, trail_clue_easy_simple008) > 0) {
    @trail_tanner;
}

// Trade is opnpc3 and is not a talk-to clue anchor.
[opnpc3,tanner]
if(inv_total(inv, trail_clue_easy_simple008) > 0) {
    ~chatnpc("<p,neutral>Greetings.");
}
`,
        'scripts/areas/area_gnome/configs/gnome.npc': `[grandtree_hazelmere]
name=Hazelmere
`,
        'scripts/areas/area_lumbridge/configs/lumbridge.npc': `[hans]
name=Hans
`,
        'scripts/areas/area_port_sarim/configs/port_sarim.npc': `[captain_tobias]
name=Captain Tobias
`,
        'scripts/areas/area_alkharid/configs/alkharid.npc': `[tanner]
name=Tanner
[black_heather]
name=Black Heather
[ardougne_guard]
name=Guard
`,
        'maps/m41_48.jm2': '==== NPC ====\n1 54 14: 669\n',
        'maps/m47_50.jm2': '==== NPC ====\n0 20 16: 376\n',
        'maps/m47_57.jm2': '==== NPC ====\n0 31 52: 202\n',
        'maps/m50_50.jm2': '==== NPC ====\n0 7 33: 0\n',
        // 2662 is a LOC row and must stay out; tanner and the guard each spawn twice
        'maps/m42_55.jm2': '==== LOC ====\n0 47 62: 2662 10 1\n==== NPC ====\n0 10 20: 804\n0 11 20: 804\n0 12 20: 32\n0 13 20: 32\n',
    };
    mutate?.(files);
    for (const [relative, body] of Object.entries(files)) {
        const absolute = path.join(rootDir, relative);
        fs.mkdirSync(path.dirname(absolute), { recursive: true });
        fs.writeFileSync(absolute, body);
    }
}
function talkKeyFixture(mutate?: (files: Record<string, string>) => void) {
    const rootDir = fs.mkdtempSync(path.join(os.tmpdir(), 'talk-key-'));
    writeTalkKeyFixture(rootDir, mutate);
    execFileSync('git', ['init', '-q'], { cwd: rootDir });
    execFileSync('git', ['add', '-A'], { cwd: rootDir });
    return rootDir;
}
function talkKeyRow(facts: TalkKeyFacts, alias: string) {
    const found = facts.talk.find((row) => row.alias === alias);
    assert.ok(found, `missing talk step ${alias}`);
    return found;
}
function talkKeyKeeper(facts: TalkKeyFacts, alias: string) {
    const found = facts.keys.find((row) => row.alias === alias);
    assert.ok(found, `missing key keeper ${alias}`);
    return found;
}

const talkKeyExtract = extractTalkKeyFacts(talkKeyFixture(), talkKeyItems);
const talkKeyFacts = talkKeyExtract.facts;

// opnpc1 only, membership only: one row per step, the puzzlebox and challenge extras dropped
assert.deepEqual(talkKeyFacts.talk.map((row) => row.alias), [
    'trail_clue_easy_simple005',
    'trail_clue_easy_simple008',
    'trail_clue_easy_vague012',
    'trail_clue_hard_riddle012',
    'trail_clue_medium_anagram001',
]);
assert.deepEqual(talkKeyFacts.keys.map((row) => row.alias), [
    'trail_clue_medium_riddle001',
    'trail_clue_medium_riddle002',
    'trail_clue_medium_riddle004',
    'trail_clue_medium_riddle005',
]);
assert.equal(talkKeyFacts.talk.some((row) => row.alias.endsWith('_puzzlebox') || row.alias.endsWith('_challenge')), false);
assert.equal(JSON.stringify(talkKeyFacts).includes('puzzlebox'), false);
assert.equal(JSON.stringify(talkKeyFacts).includes('_challenge'), false);
assert.equal(JSON.stringify(talkKeyFacts).includes('_sailor'), false);

// the exemplar: Hazelmere 669 from the jm2 NPC section, not from a frozen table
assert.deepEqual(talkKeyRow(talkKeyFacts, 'trail_clue_medium_anagram001'), {
    alias: 'trail_clue_medium_anagram001',
    id: 2841,
    npc: { alias: 'grandtree_hazelmere', id: 669, name: 'Hazelmere' },
    spawn: { x: 2678, z: 3086, plane: 1 },
});
// Hans owns two membership clues: two rows, one npc, one spawn
const hansRows = talkKeyFacts.talk.filter((row) => row.npc.alias === 'hans');
assert.deepEqual(hansRows, [
    { alias: 'trail_clue_easy_simple005', id: 2681, npc: { alias: 'hans', id: 0, name: 'Hans' }, spawn: { x: 3207, z: 3233, plane: 0 } },
    { alias: 'trail_clue_hard_riddle012', id: 2792, npc: { alias: 'hans', id: 0, name: 'Hans' }, spawn: { x: 3207, z: 3233, plane: 0 } },
]);
// the `_sailor` header is a trigger: the identity is the inner npc_type
assert.deepEqual(talkKeyRow(talkKeyFacts, 'trail_clue_easy_vague012'), {
    alias: 'trail_clue_easy_vague012',
    id: 3496,
    npc: { alias: 'captain_tobias', id: 376, name: 'Captain Tobias' },
    spawn: { x: 3028, z: 3216, plane: 0 },
});
// two jm2 hits: the identity row stays, the spawn is omitted, coverage records why
const tannerRow = talkKeyRow(talkKeyFacts, 'trail_clue_easy_simple008');
assert.equal('spawn' in tannerRow, false);
assert.equal(tannerRow.npc.alias, 'tanner');
assert.deepEqual(talkKeyFacts.coverage, [
    { class: 'unknown', family: 'keys', alias: 'trail_clue_medium_riddle002', reason: 'non-unique jm2 NPC spawn' },
    { class: 'unknown', family: 'keys', alias: 'trail_clue_medium_riddle004', reason: 'keeper is a category, not one packed npc id' },
    { class: 'unknown', family: 'keys', alias: 'trail_clue_medium_riddle005', reason: 'keeper is a name match, not one packed npc id' },
    { class: 'unknown', family: 'talk', alias: 'trail_clue_easy_simple008', reason: 'non-unique jm2 NPC spawn' },
]);
// keeper is a discriminated union: type carries the pack join, category and name never do
assert.deepEqual(talkKeyKeeper(talkKeyFacts, 'trail_clue_medium_riddle001'), {
    alias: 'trail_clue_medium_riddle001',
    id: 2831,
    key_alias: 'trail_clue_medium_riddle001_key',
    key_id: 2832,
    keeper: { kind: 'type', alias: 'black_heather', id: 202, name: 'Black Heather' },
    spawn: { x: 3039, z: 3700, plane: 0 },
});
assert.deepEqual(talkKeyKeeper(talkKeyFacts, 'trail_clue_medium_riddle002'), {
    alias: 'trail_clue_medium_riddle002',
    id: 2833,
    key_alias: 'trail_clue_medium_riddle002_key',
    key_id: 2834,
    keeper: { kind: 'type', alias: 'ardougne_guard', id: 32, name: 'Guard' },
});
assert.deepEqual(talkKeyKeeper(talkKeyFacts, 'trail_clue_medium_riddle004').keeper, { kind: 'category', category: 'chicken' });
assert.deepEqual(talkKeyKeeper(talkKeyFacts, 'trail_clue_medium_riddle005').keeper, { kind: 'name', name: 'Man' });
assert.equal(JSON.stringify(talkKeyFacts).includes('"chicken_id"'), false);
assert.equal(talkKeyFacts.keys.every((row) => row.keeper.kind !== 'type' || ('id' in row.keeper && 'alias' in row.keeper)), true);
assert.equal(talkKeyFacts.keys.filter((row) => row.keeper.kind !== 'type').every((row) => !('id' in row.keeper) && !('alias' in row.keeper)), true);
// every published step is a membership clue, and a key step keeps its own key object
assert.equal(talkKeyFacts.talk.every((row) => row.id !== row.npc.id), true);
assert.equal(talkKeyFacts.keys.every((row) => row.id !== row.key_id), true);
assert.equal(talkKeyFacts.talk.filter((row) => row.npc.alias === 'sailor').length, 0);
assert.equal(JSON.stringify(talkKeyFacts).includes('2662'), false, 'a LOC row is not an NPC spawn');
assert.equal(JSON.stringify(talkKeyFacts).includes('3039'), true, 'the black_heather keeper spawn is the NPC section row');

// decode corroboration: the packed id, alias, and .npc name all have to agree with npc.dat
const talkKeyNpcs = [
    { id: 0, debugname: 'hans', name: 'Hans' },
    { id: 32, debugname: 'ardougne_guard', name: 'Guard' },
    { id: 202, debugname: 'black_heather', name: 'Black Heather' },
    { id: 376, debugname: 'captain_tobias', name: 'Captain Tobias' },
    { id: 669, debugname: 'grandtree_hazelmere', name: 'Hazelmere' },
    { id: 804, debugname: 'tanner', name: 'Tanner' },
];
assert.doesNotThrow(() => assertTalkKeyNpcJoins(talkKeyFacts, talkKeyNpcs));
assert.throws(() => assertTalkKeyNpcJoins(talkKeyFacts, talkKeyNpcs.map((npc) => (npc.debugname === 'grandtree_hazelmere' ? { ...npc, id: 670 } : npc))), /disagrees with decoded npc id/);
assert.throws(() => assertTalkKeyNpcJoins(talkKeyFacts, talkKeyNpcs.map((npc) => (npc.debugname === 'hans' ? { ...npc, name: 'Hanz' } : npc))), /disagrees with decoded npc name/);
assert.throws(() => assertTalkKeyNpcJoins(talkKeyFacts, talkKeyNpcs.filter((npc) => npc.debugname !== 'black_heather')), /npc\.dat lacks black_heather/);

// provenance: the scanned trees and the pinned inputs are the family's invalidation identity
assert.equal(talkKeyExtract.inputs.maps_directory, 'maps');
assert.equal(talkKeyExtract.inputs.scripts.files, 5);
assert.equal(talkKeyExtract.inputs.npc_configs.files, 4);
assert.equal(talkKeyExtract.inputs.npc_pack.path, 'pack/npc.pack');
assert.equal(talkKeyExtract.inputs.trail_clue_medium.path, TALK_KEY_MEDIUM);
assert.notEqual(extractTalkKeyFacts(talkKeyFixture((files) => { files['scripts/areas/area_lumbridge/scripts/hans.rs2'] += '\n[label,hans_extra]\n'; }), talkKeyItems).inputs.scripts.sha256, talkKeyExtract.inputs.scripts.sha256, 'the scripts digest must move with a scanned script');
assert.notEqual(extractTalkKeyFacts(talkKeyFixture((files) => { files['scripts/areas/area_lumbridge/configs/lumbridge.npc'] += '[nobody]\nname=Nobody\n'; }), talkKeyItems).inputs.npc_configs.sha256, talkKeyExtract.inputs.npc_configs.sha256, 'the npc config digest must move with a scanned config');
assert.notEqual(extractTalkKeyFacts(talkKeyFixture((files) => { files['pack/npc.pack'] += '9999=nobody\n'; }), talkKeyItems).inputs.npc_pack.sha256, talkKeyExtract.inputs.npc_pack.sha256, 'the npc pack digest must move with the pack');
assert.notEqual(extractTalkKeyFacts(talkKeyFixture((files) => { files['maps/m41_48.jm2'] += '0 1 1: 5\n'; }), talkKeyItems).inputs.maps.sha256, talkKeyExtract.inputs.maps.sha256, 'the maps digest must move with the maps tree');
assert.equal(talkKeyExtract.inputs.maps.files, 5);

// fail closed: missing joins, missing sections, missing names, malformed arms, truncated trees
assert.throws(() => extractTalkKeyFacts(talkKeyFixture((files) => { delete files['pack/npc.pack']; }), talkKeyItems), /pack\/npc\.pack/);
assert.throws(() => extractTalkKeyFacts(talkKeyFixture((files) => { files['pack/npc.pack'] = files['pack/npc.pack'].replace('669=grandtree_hazelmere\n', ''); }), talkKeyItems), /pack\/npc\.pack lacks grandtree_hazelmere/);
assert.throws(() => extractTalkKeyFacts(talkKeyFixture((files) => { files['scripts/areas/area_gnome/configs/gnome.npc'] = '[somebody_else]\nname=Nobody\n'; }), talkKeyItems), /missing \[grandtree_hazelmere\]/);
assert.throws(() => extractTalkKeyFacts(talkKeyFixture((files) => { files['scripts/areas/area_gnome/configs/gnome.npc'] = '[grandtree_hazelmere]\nop1=Talk-to\n'; }), talkKeyItems), /has no name/);
assert.throws(() => extractTalkKeyFacts(talkKeyFixture((files) => { files['pack/obj.pack'] = files['pack/obj.pack'].replace('2832=trail_clue_medium_riddle001_key\n', ''); }), talkKeyItems), /pack\/obj\.pack lacks trail_clue_medium_riddle001_key/);
assert.throws(() => extractTalkKeyFacts(talkKeyFixture((files) => { files['maps/m41_48.jm2'] = '==== LOC ====\n0 1 1: 2662 10 1\n'; }), talkKeyItems), /npc 669 has no jm2 NPC spawn/, 'a selected identity with no NPC row must fail');
assert.throws(() => extractTalkKeyFacts(talkKeyFixture((files) => { files['maps/m50_50.jm2'] = '==== NPC ====\n0 7 33: 0 10\n'; }), talkKeyItems), /extra tokens/);
assert.throws(() => extractTalkKeyFacts(talkKeyFixture((files) => { files['scripts/areas/area_port_sarim/scripts/sailors.rs2'] = '[opnpc1,_sailor]\nif(inv_total(inv, trail_clue_easy_vague012) > 0) {\n    return;\n}\n'; }), talkKeyItems), /exactly one inner npc_type/);
assert.throws(() => extractTalkKeyFacts(talkKeyFixture((files) => { files[TALK_KEY_MEDIUM] = '[label,riddle_loc_medium_exp001]\n~mesbox("The chest is locked.");\n'; }), talkKeyItems), /missing \[proc,trail_checkmediumdrop\]/);
assert.throws(() => extractTalkKeyFacts(talkKeyFixture((files) => { files[TALK_KEY_MEDIUM] = files[TALK_KEY_MEDIUM].replace('} else if(npc_category = chicken', '} else if(npc_category2 = chicken'); }), talkKeyItems), /malformed trail_checkmediumdrop arm/);
assert.throws(() => extractTalkKeyFacts(talkKeyFixture((files) => { files[TALK_KEY_MEDIUM] = files[TALK_KEY_MEDIUM].replace('& ~obj_gettotal(trail_clue_medium_riddle002_key) = 0', ''); }), talkKeyItems), /must guard the key it adds/);
assert.throws(() => extractTalkKeyFacts(talkKeyFixture((files) => { files[TALK_KEY_MEDIUM] = files[TALK_KEY_MEDIUM].replace('inv_total(inv, trail_clue_medium_riddle004) > 0', 'inv_total(inv, trail_clue_medium_riddle099) > 0'); }), talkKeyItems), /keeper clue trail_clue_medium_riddle099 is not a membership alias/);
const truncatedTalkKeyMaps = talkKeyFixture();
fs.rmSync(path.join(truncatedTalkKeyMaps, 'maps/m50_50.jm2'));
assert.throws(() => extractTalkKeyFacts(truncatedTalkKeyMaps, talkKeyItems), /maps\/m50_50\.jm2: tracked map missing/);
const truncatedTalkKeyScripts = talkKeyFixture();
fs.rmSync(path.join(truncatedTalkKeyScripts, 'scripts/areas/area_lumbridge/scripts/hans.rs2'));
assert.throws(() => extractTalkKeyFacts(truncatedTalkKeyScripts, talkKeyItems), /hans\.rs2: tracked \.rs2 file missing from the content tree/);
const truncatedTalkKeyConfigs = talkKeyFixture();
fs.rmSync(path.join(truncatedTalkKeyConfigs, 'scripts/areas/area_lumbridge/configs/lumbridge.npc'));
assert.throws(() => extractTalkKeyFacts(truncatedTalkKeyConfigs, talkKeyItems), /lumbridge\.npc: tracked \.npc file missing from the content tree/);

// both pins: 289 is the selected source, 274 corroborates identity and spawn
const pinTalkKey274Root = '/Users/acfrazier/experiments/Server/content';
const pinTalkKey289Root = '/Users/acfrazier/experiments/lostcity-289/content';
const pinTalkKey274 = extractTalkKeyFacts(pinTalkKey274Root, pinDecodedItems(274));
const pinTalkKey289 = extractTalkKeyFacts(pinTalkKey289Root, pinDecodedItems(289));
assert.deepEqual(pinTalkKey274.facts, pinTalkKey289.facts, '274 must corroborate the selected 289 identities and spawns');
assert.doesNotThrow(() => assertTalkKeyPins(pinTalkKey274.facts, 274));
assert.doesNotThrow(() => assertTalkKeyPins(pinTalkKey289.facts, 289));
assert.equal(pinTalkKey289.facts.talk.length, 47);
assert.equal(pinTalkKey289.facts.talk.filter((row) => row.spawn !== undefined).length, 42);
assert.equal(pinTalkKey289.facts.keys.length, 7);
assert.equal(pinTalkKey289.facts.keys.filter((row) => row.spawn !== undefined).length, 2);
assert.equal(pinTalkKey289.facts.coverage.length, 10);
assert.equal(pinTalkKey289.inputs.trail_clue_medium.sha256, 'c2d685a9906ec9396ce191b024cee79dd7d8b47b692b309806ea7c0c12dbc9f6');
for (const row of pinTalkKey289.facts.coverage) {
    const published = row.family === 'talk' ? pinTalkKey289.facts.talk.some((entry) => entry.alias === row.alias) : pinTalkKey289.facts.keys.some((entry) => entry.alias === row.alias);
    assert.equal(published, true, `coverage must belong to a published ${row.family} step`);
}
assert.equal(pinTalkKey289.facts.coverage.some((row) => row.alias.endsWith('_challenge') || row.alias.endsWith('_puzzlebox')), false);
assert.equal(pinTalkKey289.facts.talk.some((row) => row.alias.endsWith('_puzzlebox') || row.alias.endsWith('_challenge')), false);
assert.equal(pinTalkKey289.facts.keys.filter((row) => row.keeper.kind === 'category').map((row) => row.keeper.category).sort().join(','), 'chicken,pirate');
assert.equal(pinTalkKey289.facts.keys.filter((row) => row.keeper.kind === 'name').map((row) => row.keeper.name).join(','), 'Man');
assert.equal(talkKeyRow(pinTalkKey289.facts, 'trail_clue_medium_anagram001').npc.id, 669);
assert.equal(talkKeyRow(pinTalkKey289.facts, 'trail_clue_easy_vague012').npc.alias, 'captain_tobias');
assert.equal(pinTalkKey289.facts.talk.filter((row) => row.npc.id === 0).length, 2);
const pinTalkKeyBlob = JSON.stringify(pinTalkKey289.facts);
for (const banned of ['TALK_ANCHORS', 'KILL_ANCHORS', 'RIDDLE_KEY_COORDS', 'HARD_SPECIAL_COORDS']) {
    assert.equal(pinTalkKeyBlob.includes(banned), false, `talk_key must never publish ${banned}`);
}

/* ------------------------------------------------------------------ *
 * trio_givers: the closed giver set, packed ids, owning .npc names, jm2 tiles
 * ------------------------------------------------------------------ */

// A giver identity is an `opnpc1` header on an exact path: another verb, another
// part, or a label is not one.
assert.deepEqual(parseTrioGiverHandlers('[opnpc1,observatory_professor]\n@professor_initial;\n[opnpc1,observatory_professor2]\n'), ['observatory_professor', 'observatory_professor2']);
assert.deepEqual(parseTrioGiverHandlers('[opnpc2,murphy]\n[apnpc1,brother_kojo]\n[label,professor_glass]\n'), [], 'a trade, approach, or label handler is not the talk-to identity');
assert.throws(() => parseTrioGiverHandlers('[opnpc1]\n'), /malformed opnpc1 header/);
assert.throws(() => parseTrioGiverHandlers('[opnpc1,murphy,npc]\n'), /malformed opnpc1 header/);

const TRIO_GIVER_HANDLERS: Record<string, string> = {
    observatory_professor: 'scripts/quests/quest_itgronigen/scripts/observatory_professor.rs2',
    murphy: 'scripts/minigames/game_trawler/scripts/murphy.rs2',
    brother_kojo: 'scripts/areas/area_ardougne_east/scripts/brother_kojo.rs2',
};
const TRIO_GIVER_CONFIGS: Record<string, string> = {
    observatory_professor: 'scripts/quests/quest_itgronigen/configs/quest_itgronigen.npc',
    murphy: 'scripts/minigames/game_trawler/configs/trawler.npc',
    brother_kojo: 'scripts/areas/area_ardougne_east/configs/ardougne_east.npc',
};

function writeTrioGiverFixture(rootDir: string, mutate?: (files: Record<string, string>) => void) {
    const files: Record<string, string> = {
        // The lookalikes are packed and configured, and none of them is published.
        'pack/npc.pack': '223=brother_kojo\n463=murphy\n464=murphy_normal\n465=murphy_halfsunk\n488=observatory_professor\n489=observatory_professor2\n',
        [TRIO_GIVER_HANDLERS.observatory_professor]: `[opnpc1,observatory_professor]
@professor_initial;

[label,professor_initial]
~chatnpc("<p,neutral>Bring me a sextant, a watch and a chart.");

[opnpc1,observatory_professor2]
@professor_initial;
`,
        [TRIO_GIVER_HANDLERS.murphy]: `[opnpc1,murphy]
@murphy_could_i_help;

[label,murphy_could_i_help]
~chatnpc("<p,neutral>Do you want to go to the trawler?");

[opnpc2,murphy_normal]
~chatnpc("<p,neutral>Not the giver.");
`,
        [TRIO_GIVER_HANDLERS.brother_kojo]: `[opnpc1,brother_kojo]
~chatnpc("<p,neutral>Take a watch from the pedestal.");
`,
        [TRIO_GIVER_CONFIGS.observatory_professor]: `[observatory_professor]
name=Observatory professor
[observatory_professor2]
name=Observatory professor
`,
        [TRIO_GIVER_CONFIGS.murphy]: `[murphy]
name=Murphy
[murphy_normal]
name=Murphy
`,
        [TRIO_GIVER_CONFIGS.brother_kojo]: `[brother_kojo]
name=Brother Kojo
`,
        // One tile per giver, derived from the mapsquare name: a LOC row is not a spawn.
        'maps/m10_20.jm2': '==== LOC ====\n0 6 50: 488 10 1\n==== NPC ====\n0 1 2: 488\n',
        'maps/m11_21.jm2': '==== NPC ====\n1 3 4: 463\n',
        'maps/m12_22.jm2': '==== NPC ====\n0 5 6: 223\n',
    };
    mutate?.(files);
    for (const [relative, body] of Object.entries(files)) {
        const absolute = path.join(rootDir, relative);
        fs.mkdirSync(path.dirname(absolute), { recursive: true });
        fs.writeFileSync(absolute, body);
    }
}
function trioGiverFixture(mutate?: (files: Record<string, string>) => void) {
    const rootDir = fs.mkdtempSync(path.join(os.tmpdir(), 'trio-givers-'));
    writeTrioGiverFixture(rootDir, mutate);
    execFileSync('git', ['init', '-q'], { cwd: rootDir });
    execFileSync('git', ['add', '-A'], { cwd: rootDir });
    return rootDir;
}
function trioGiverRow(facts: TrioGiverFacts, alias: string) {
    const found = facts.rows.find((row) => row.alias === alias);
    assert.ok(found, `missing giver ${alias}`);
    return found;
}

const trioGiverExtract = extractTrioGiversFacts(trioGiverFixture());
const trioGiverFacts = trioGiverExtract.facts;

// The closed set in file order: one row per giver, one packed id, one .npc name,
// one world tile converted from the mapsquare filename and the local coordinates.
assert.deepEqual(trioGiverFacts, {
    rows: [
        { alias: 'observatory_professor', id: 488, name: 'Observatory professor', spawn: { x: 641, z: 1282, plane: 0 } },
        { alias: 'murphy', id: 463, name: 'Murphy', spawn: { x: 707, z: 1348, plane: 1 } },
        { alias: 'brother_kojo', id: 223, name: 'Brother Kojo', spawn: { x: 773, z: 1414, plane: 0 } },
    ],
    coverage: [],
});
assert.deepEqual(trioGiverRow(trioGiverFacts, 'observatory_professor').spawn, { x: 641, z: 1282, plane: 0 }, 'the tile comes from the jm2 NPC row, not from the LOC row on the same map');
assert.equal(JSON.stringify(trioGiverFacts).includes('observatory_professor2'), false);
assert.equal(JSON.stringify(trioGiverFacts).includes('murphy_normal'), false);
assert.equal(JSON.stringify(trioGiverFacts).includes('489'), false);
assert.equal(JSON.stringify(trioGiverFacts).includes('464'), false);
assert.equal(trioGiverFacts.rows.every((row) => Object.keys(row).every((key) => ['alias', 'id', 'name', 'spawn'].includes(key))), true);
assert.equal(trioGiverFacts.rows.every((row) => Object.keys(row.spawn ?? {}).sort().join(',') === 'plane,x,z'), true, 'a spawn is world x/z/plane, never a level');

// Two jm2 hits keep the row and omit the tile: first-in-file is never taken.
const multiSpawnTrioGivers = extractTrioGiversFacts(trioGiverFixture((files) => { files['maps/m11_21.jm2'] = '==== NPC ====\n1 3 4: 463\n1 3 9: 463\n'; }));
assert.deepEqual(multiSpawnTrioGivers.facts.rows[1], { alias: 'murphy', id: 463, name: 'Murphy' });
assert.deepEqual(multiSpawnTrioGivers.facts.coverage, [{ class: 'unknown', family: 'trio_givers', alias: 'murphy', reason: 'non-unique jm2 NPC spawn' }]);
assert.equal(JSON.stringify(multiSpawnTrioGivers.facts).includes('"spawn":null'), false);
assert.throws(() => assertTrioGiverPins(multiSpawnTrioGivers.facts, 289), /three unique jm2 spawns/);
assert.throws(() => assertTrioGiverPins(trioGiverFacts, 289), /must stay the selected jm2 tile/);

// decode corroboration: the packed alias, id, and .npc name must all agree with npc.dat
const trioGiverNpcs = [
    { id: 223, debugname: 'brother_kojo', name: 'Brother Kojo' },
    { id: 463, debugname: 'murphy', name: 'Murphy' },
    { id: 488, debugname: 'observatory_professor', name: 'Observatory professor' },
];
assert.doesNotThrow(() => assertTrioGiverNpcJoins(trioGiverFacts, trioGiverNpcs));
assert.throws(() => assertTrioGiverNpcJoins(trioGiverFacts, trioGiverNpcs.map((npc) => (npc.debugname === 'murphy' ? { ...npc, id: 464 } : npc))), /disagrees with decoded npc id/);
assert.throws(() => assertTrioGiverNpcJoins(trioGiverFacts, trioGiverNpcs.map((npc) => (npc.debugname === 'brother_kojo' ? { ...npc, name: 'Brother Kojo?' } : npc))), /disagrees with decoded npc name/);
assert.throws(() => assertTrioGiverNpcJoins(trioGiverFacts, trioGiverNpcs.filter((npc) => npc.debugname !== 'observatory_professor')), /npc\.dat lacks observatory_professor/);

// provenance: the closed handler and config set, the pack, and the maps inventory
assert.equal(trioGiverExtract.inputs.maps_directory, 'maps');
assert.equal(trioGiverExtract.inputs.maps.files, 3);
assert.deepEqual(trioGiverExtract.inputs.handlers.map((entry) => entry.path), [TRIO_GIVER_HANDLERS.observatory_professor, TRIO_GIVER_HANDLERS.murphy, TRIO_GIVER_HANDLERS.brother_kojo]);
assert.deepEqual(trioGiverExtract.inputs.npc_configs.map((entry) => entry.path), [TRIO_GIVER_CONFIGS.observatory_professor, TRIO_GIVER_CONFIGS.murphy, TRIO_GIVER_CONFIGS.brother_kojo]);
assert.equal(trioGiverExtract.inputs.npc_pack.path, 'pack/npc.pack');
assert.notEqual(extractTrioGiversFacts(trioGiverFixture((files) => { files[TRIO_GIVER_HANDLERS.murphy] += '\n[label,murphy_extra]\n'; })).inputs.handlers[1].sha256, trioGiverExtract.inputs.handlers[1].sha256, 'the handler digest must move with the scanned script');
assert.notEqual(extractTrioGiversFacts(trioGiverFixture((files) => { files[TRIO_GIVER_CONFIGS.brother_kojo] += '[nobody]\nname=Nobody\n'; })).inputs.npc_configs[2].sha256, trioGiverExtract.inputs.npc_configs[2].sha256, 'the config digest must move with the scanned config');
assert.notEqual(extractTrioGiversFacts(trioGiverFixture((files) => { files['pack/npc.pack'] += '9999=nobody\n'; })).inputs.npc_pack.sha256, trioGiverExtract.inputs.npc_pack.sha256, 'the npc pack digest must move with the pack');
assert.notEqual(extractTrioGiversFacts(trioGiverFixture((files) => { files['maps/m12_22.jm2'] += '0 1 1: 5\n'; })).inputs.maps.sha256, trioGiverExtract.inputs.maps.sha256, 'the maps digest must move with the maps tree');

// fail closed: missing files, missing joins, missing names, a lost header, zero hits, truncated trees
assert.throws(() => extractTrioGiversFacts(trioGiverFixture((files) => { delete files['pack/npc.pack']; })), /pack\/npc\.pack: required file missing/);
assert.throws(() => extractTrioGiversFacts(trioGiverFixture((files) => { files['pack/npc.pack'] = files['pack/npc.pack'].replace('463=murphy\n', ''); })), /pack\/npc\.pack lacks murphy/);
assert.throws(() => extractTrioGiversFacts(trioGiverFixture((files) => { files[TRIO_GIVER_CONFIGS.murphy] = '[murphy_normal]\nname=Murphy\n'; })), /missing \[murphy\]/);
assert.throws(() => extractTrioGiversFacts(trioGiverFixture((files) => { files[TRIO_GIVER_CONFIGS.murphy] = '[murphy]\nop1=Talk-to\n'; })), /has no name/);
assert.throws(() => extractTrioGiversFacts(trioGiverFixture((files) => { delete files[TRIO_GIVER_HANDLERS.murphy]; })), /murphy\.rs2: required file missing/);
assert.throws(() => extractTrioGiversFacts(trioGiverFixture((files) => { delete files[TRIO_GIVER_CONFIGS.brother_kojo]; })), /ardougne_east\.npc: required file missing/);
assert.throws(() => extractTrioGiversFacts(trioGiverFixture((files) => { files[TRIO_GIVER_HANDLERS.brother_kojo] = '[opnpc1,brother_kojo2]\n@kojo;\n'; })), /does not declare \[opnpc1,brother_kojo\]/);
assert.throws(() => extractTrioGiversFacts(trioGiverFixture((files) => { files[TRIO_GIVER_HANDLERS.observatory_professor] = '[opnpc1,observatory_professor2]\n@professor_initial;\n'; })), /does not declare \[opnpc1,observatory_professor\]/);
assert.throws(() => extractTrioGiversFacts(trioGiverFixture((files) => { files[TRIO_GIVER_HANDLERS.murphy] += '\n[opnpc1,brother_kojo]\n@kojo;\n'; })), /also declares \[opnpc1,brother_kojo\]/, 'one giver script must not claim another closed identity');
assert.throws(() => extractTrioGiversFacts(trioGiverFixture((files) => { files['maps/m11_21.jm2'] = '==== NPC ====\n1 3 4: 999\n'; })), /murphy npc 463 has no jm2 NPC spawn/, 'zero hits is not an unknown tile');
assert.throws(() => extractTrioGiversFacts(trioGiverFixture((files) => { files['maps/m11_21.jm2'] = '==== LOC ====\n1 3 4: 463\n'; })), /murphy npc 463 has no jm2 NPC spawn/, 'a LOC row is not an NPC spawn');
assert.throws(() => extractTrioGiversFacts(trioGiverFixture((files) => { files['maps/m11_21.jm2'] = '==== NPC ====\n1 3 4: 463 10\n'; })), /extra tokens/);
const truncatedTrioGiverMaps = trioGiverFixture();
fs.rmSync(path.join(truncatedTrioGiverMaps, 'maps/m12_22.jm2'));
assert.throws(() => extractTrioGiversFacts(truncatedTrioGiverMaps), /maps\/m12_22\.jm2: tracked map missing/);

// both pins: 289 is the selected source, 274 corroborates identity, name, and tile
const pinTrioGiver274Root = '/Users/acfrazier/experiments/Server/content';
const pinTrioGiver289Root = '/Users/acfrazier/experiments/lostcity-289/content';
const pinTrioGiver274 = extractTrioGiversFacts(pinTrioGiver274Root);
const pinTrioGiver289 = extractTrioGiversFacts(pinTrioGiver289Root);
assert.deepEqual(pinTrioGiver274.facts, pinTrioGiver289.facts, '274 must corroborate the selected 289 giver identities and tiles');
assert.doesNotThrow(() => assertTrioGiverPins(pinTrioGiver274.facts, 274));
assert.doesNotThrow(() => assertTrioGiverPins(pinTrioGiver289.facts, 289));
assert.deepEqual(pinTrioGiver289.facts.rows.map((row) => [row.alias, row.id, row.name]), [
    ['observatory_professor', 488, 'Observatory professor'],
    ['murphy', 463, 'Murphy'],
    ['brother_kojo', 223, 'Brother Kojo'],
]);
assert.deepEqual(pinTrioGiver289.facts.rows.map((row) => row.spawn), [
    { x: 2438, z: 3186, plane: 0 },
    { x: 2668, z: 3162, plane: 0 },
    { x: 2569, z: 3249, plane: 0 },
]);
assert.equal(pinTrioGiver289.facts.coverage.length, 0);
assert.equal(pinTrioGiver289.inputs.handlers.length, 3);
assert.equal(pinTrioGiver289.inputs.npc_configs.length, 3);
assert.equal(pinTrioGiver289.inputs.maps.files, 534);
assert.equal(pinTrioGiver274.inputs.maps.files, 483);
assert.equal(parseTrioGiverHandlers(fs.readFileSync(path.join(pinTrioGiver289Root, TRIO_GIVER_HANDLERS.observatory_professor), 'utf8')).includes('observatory_professor2'), true, 'the selected professor script declares a lookalike that must never be published');
assert.equal(JSON.stringify(pinTrioGiver289.facts).includes('observatory_professor2'), false);
assert.equal(JSON.stringify(pinTrioGiver289.facts).includes('murphy_'), false, 'the trawler states are display states, not giver identities');
assert.throws(() => assertTrioGiverPins({ rows: [{ alias: 'observatory_professor2', id: 489, name: 'Observatory professor', spawn: { x: 2438, z: 3186, plane: 0 } }], coverage: [] }, 289), /three closed givers/);
const pinTrioGiverBlob = JSON.stringify(pinTrioGiver289.facts);
for (const banned of ['TALK_ANCHORS', 'KILL_ANCHORS', 'RIDDLE_KEY_COORDS', 'HARD_SPECIAL_COORDS', 'frozen', 'invented', 'family-unavailable']) {
    assert.equal(pinTrioGiverBlob.includes(banned), false, `trio_givers must never publish ${banned}`);
}

// ---- gathering family (M-306 / M-215): real pinned content, plus real-script fixtures with one drifted construct ----

const gatheringPins = [
    { revision: 274, root: '/Users/acfrazier/experiments/Server/content' },
    { revision: 289, root: '/Users/acfrazier/experiments/lostcity-289/content' },
];

function gatherView(family: GatheringFamily) {
    const alias = new Map<string, string>();
    for (const row of family.payload.entities) {
        const [kind, id, name] = row.split(' ');
        alias.set(`${kind}:${id}`, name);
    }
    return {
        name: (kind: string, id: number) => alias.get(`${kind}:${id}`) ?? `?${id}`,
        method(id: string): MethodWire {
            const found = family.payload.methods.find((each) => each.id === id);
            assert.ok(found, `missing gather method ${id}`);
            return found;
        },
    };
}
function knownValue<T>(cell: Know<T>, label: string): T {
    if (cell.state === 'unknown') throw new Error(`${label}: unknown (${cell.gap.code})`);
    return cell.value;
}
const targetsOf = (method: MethodWire, cls?: TargetWire['class']) => knownValue(method.targets, `${method.id} targets`).filter((target) => cls === undefined || target.class === cls);
function listContentFiles(root: string, dir: string, extension: string, out: string[] = []) {
    for (const entry of fs.readdirSync(path.join(root, dir), { withFileTypes: true }).sort((a, b) => (a.name < b.name ? -1 : 1))) {
        if (entry.isDirectory()) listContentFiles(root, `${dir}/${entry.name}`, extension, out);
        else if (entry.name.endsWith(extension)) out.push(`${dir}/${entry.name}`);
    }
    return out;
}
/** Loc aliases whose config section matches `line`, read independently of the extractor. */
function locSectionsWith(root: string, line: RegExp) {
    const found = new Set<string>();
    for (const file of listContentFiles(root, 'scripts', '.loc')) {
        let current = '';
        for (const raw of fs.readFileSync(path.join(root, file), 'utf8').split(/\r?\n/)) {
            const text = raw.trim();
            if (text.startsWith('[') && text.endsWith(']')) current = text.slice(1, -1);
            else if (current && line.test(text)) found.add(current);
        }
    }
    return found;
}
/** `file:line:plane:lx:lz:id` for every map row of `section` whose id is wanted. */
function mapRowKeys(root: string, section: 'LOC' | 'NPC', ids: ReadonlySet<number>) {
    const keys = new Set<string>();
    for (const name of fs.readdirSync(path.join(root, 'maps')).filter((each) => each.endsWith('.jm2')).sort()) {
        let inside = false;
        fs.readFileSync(path.join(root, 'maps', name), 'utf8').split(/\r?\n/).forEach((raw, index) => {
            if (raw.startsWith('====')) inside = raw.trim() === `==== ${section} ====`;
            else if (inside) {
                const match = /^(\d+) (\d+) (\d+): (\d+)/.exec(raw);
                if (match && ids.has(Number(match[4]))) keys.add(`maps/${name}:${index + 1}:${match[1]}:${match[2]}:${match[3]}:${match[4]}`);
            }
        });
    }
    return keys;
}
/** The same keys read from the family's compact placement rows. */
function familyRowKeys(family: GatheringFamily, kind: 'l' | 'n', ids: ReadonlySet<number>) {
    const keys = new Set<string>();
    for (const file of family.payload.placements) {
        for (const row of file.rows) {
            const [rowLine, plane, lx, lz, entity] = row.split(' ');
            if (entity[0] === kind && ids.has(Number(entity.slice(1)))) keys.add(`${file.file}:${rowLine}:${plane}:${lx}:${lz}:${entity.slice(1)}`);
        }
    }
    return keys;
}

for (const { revision, root } of gatheringPins) {
    const family = extractGatheringFamily(root);
    const view = gatherView(family);
    const locPack = parsePack(fs.readFileSync(path.join(root, 'pack/loc.pack'), 'utf8'));
    const facts: GatheringFacts = family.payload;
    assert.equal(family.schema, GATHERING_SCHEMA);
    assert.equal(JSON.stringify(extractGatheringFamily(root).payload), JSON.stringify(facts), `${revision} extraction is deterministic`);
    assert.deepEqual(family.summary.methods, { woodcutting: 10, mining: 15, fishing: 15 }, `${revision} method inventory`);

    // M-215 R2/R3: handler-less locs are proven inert (no alias or category oploc handler;
    // a global handler fails closed at index time, so reaching here proves there is none
    // and the engine runs its default no-op): they are excluded from the mining
    // catalogue entirely — no method target, no loose row, no entity join. Every other
    // rock loc the content offers a Mine op on is classified or left Unknown with a gap code.
    const rockClass = new Map<number, string>();
    for (const method of facts.methods.filter((each) => each.skill === 'mining')) for (const target of method.targets.state === 'unknown' ? [] : method.targets.value) rockClass.set(target.id, target.class);
    for (const loose of facts.loose.filter((each) => each.skill === 'mining')) rockClass.set(loose.id, loose.class);
    const mineOps = locSectionsWith(root, /^op[1-5]=mine$/i);
    const inert = revision === 289
        ? ['newbierocks1', ...Array.from({ length: 21 }, (_, index) => `loc_${4976 + index}`)]
        : ['newbierocks1', 'castlewars_blocked_tunnel_1', 'castlewars_blocked_tunnel_2'];
    for (const alias of inert) {
        const id = locPack.get(alias)!;
        assert.ok(!rockClass.has(id), `${revision} ${alias} is excluded, not Unknown`);
        assert.ok(!facts.loose.some((each) => each.id === id), `${revision} ${alias} has no loose row`);
        assert.ok(!facts.entities.some((row) => row.startsWith('loc ') && row.split(' ')[2] === alias), `${revision} ${alias} is not joined`);
    }
    assert.ok(facts.loose.filter((each) => each.skill === 'mining').every((each) => each.gap?.code !== 'no-handler'), `${revision} no mining row claims no-handler`);
    for (const alias of mineOps) {
        if (inert.includes(alias)) continue;
        assert.ok(rockClass.has(locPack.get(alias)!), `${revision} ${alias} has a class or a gap`);
    }
    for (const alias of locSectionsWith(root, /^category=mining_rock_macro_gas$/)) assert.equal(rockClass.get(locPack.get(alias)!), 'hazard', `${revision} ${alias} is a gas hazard, never a resource`);
    for (const alias of locSectionsWith(root, /^param=mining_rock_empty,1$/)) assert.equal(rockClass.get(locPack.get(alias)!), 'depleted', `${revision} ${alias} is depleted`);
    const rocks = family.summary.rocks;
    assert.equal(rocks.population, rocks.resource + rocks.depleted + rocks.hazard + rocks.unclassified, `${revision} every rock accounted exactly once`);
    assert.equal(rocks.population, revision === 289 ? 62 : 59, `${revision} handler-less locs are not population`);
    assert.equal(rocks.resource, 29, `${revision} resource rocks`);
    assert.equal(rocks.hazard, 22, `${revision} gas rocks`);
    assert.equal(rocks.depleted, 5, `${revision} depleted rocks`);
    assert.equal(rocks.unclassified, revision === 289 ? 6 : 3, `${revision} unclassified rocks`);
    assert.deepEqual(rocks.unclassified_reasons, { 'custom-handler': revision === 289 ? 6 : 3 }, `${revision} every unclassified rock has a real handler`);
    for (const loose of facts.loose.filter((each) => each.class === 'unclassified')) {
        assert.ok(loose.gap && loose.gap.code && loose.gap.sources.length > 0, `${revision} ${view.name(loose.kind, loose.id)} unclassified rows carry a gap and its sources`);
    }
    const dummy = facts.loose.find((each) => each.kind === 'loc' && view.name('loc', each.id) === 'misc_dummy_coalrock1');
    assert.equal(dummy?.gap?.code, revision === 289 ? 'custom-handler' : undefined, `${revision} the Miscellania dummy rock has its own handler, so it is no coal target`);
    assert.equal(targetsOf(view.method('mining.coal'), 'resource').some((target) => view.name('loc', target.id) === 'misc_dummy_coalrock1'), false);
    // M-215: the tutorial Mine handlers provably yield copper/tin ore, so those rocks are typed
    // resources of their ore's method, with an unknown custom respawn and a targets caveat.
    for (const [methodId, ore, tutorial] of [['mining.copper', 'copper', 'newbiecopperrock'], ['mining.tin', 'tin', 'newbietinrock']] as const) {
        const method = view.method(methodId);
        assert.deepEqual(targetsOf(method, 'resource').map((target) => view.name('loc', target.id)), [`${ore}rock1`, `${ore}rock2`, tutorial]);
        const custom = targetsOf(method, 'resource').find((target) => view.name('loc', target.id) === tutorial)!;
        assert.equal(custom.respawn.state === 'unknown' && custom.respawn.gap.code, 'custom-deplete', `${revision} ${tutorial} respawn is custom, never zero`);
        assert.equal(method.targets.state === 'partial' && method.targets.gaps.some((each) => each.code === 'custom-handler-target'), true, `${revision} ${methodId} targets carry the custom-handler caveat`);
        assert.equal(facts.loose.some((each) => view.name('loc', each.id) === tutorial), false, `${revision} ${tutorial} is attached, not loose`);
    }

    // Distinct per-target respawn (limestone 10/20/40) and no respawn at all for rune essence.
    const limestone = targetsOf(view.method('mining.limestone'), 'resource');
    assert.deepEqual(limestone.map((target) => knownValue(target.respawn, 'limestone respawn')?.raw), [10, 20, 40]);
    assert.deepEqual(targetsOf(view.method('mining.limestone'), 'depleted').map((target) => view.name('loc', target.id)), ['loc_4030']);
    assert.equal(knownValue(targetsOf(view.method('mining.rune stones'))[0].respawn, 'essence respawn'), null);
    const iron = targetsOf(view.method('mining.iron'), 'resource');
    assert.deepEqual(iron.map((target) => view.name('loc', target.id)), ['ironrock1', 'ironrock2']);
    const ironRespawn = knownValue(iron[0].respawn, 'iron respawn');
    assert.equal(ironRespawn?.raw, 20);
    assert.deepEqual(knownValue(ironRespawn!.scale, 'iron scale'), { rule: 'scale_by_playercount', min_ticks: 10, max_ticks: 20, sources: ['scripts/general/scripts/player_count.rs2:1-4'] });
    const normalTrees = targetsOf(view.method('woodcutting.normal'), 'resource')[0];
    assert.deepEqual(knownValue(knownValue(normalTrees.respawn, 'tree respawn')!.scale, 'tree scale').min_ticks, 60, `${revision} zero-rate trees scale over 120..199`);
    assert.equal(knownValue(normalTrees.respawn, 'tree respawn')!.raw, 0, `${revision} the content's zero is preserved`);

    // Tools: pickaxe use gates are Mining levels, axes have none, wield gates are Attack.
    const tools = (id: string) => knownValue(view.method(id).tools, `${id} tools`).map((tool) => [view.name('obj', tool.item), tool.use_gate, tool.wield_gate] as const);
    assert.deepEqual(tools('mining.iron')[0], ['rune_pickaxe', { skill: 14, level: 41 }, { skill: 0, level: 40 }]);
    assert.deepEqual(tools('woodcutting.oak')[0], ['rune_axe', null, { skill: 0, level: 40 }]);
    assert.equal(tools('woodcutting.oak').every(([, use]) => use === null), true);

    // Fishing: method, tool, bait and level-gated product set come from the handler, not a category/op pair.
    const fishing = (id: string) => {
        const method = view.method(id);
        return {
            tools: knownValue(method.tools, `${id} tools`).map((tool) => view.name('obj', tool.item)),
            consumes: knownValue(method.consumes, `${id} consumes`).map((each) => `${view.name('obj', each.item)}x${each.count}`),
            products: knownValue(method.products, `${id} products`).map((each) => `${view.name('obj', each.item)}@${each.level}`),
            op: method.op,
        };
    };
    assert.deepEqual(fishing('fishing.freshfish.op1'), { tools: ['fly_fishing_rod'], consumes: ['featherx1'], products: ['raw_trout@20', 'raw_salmon@30'], op: { slot: 1, label: 'Lure' } });
    assert.deepEqual(fishing('fishing.freshfish.op3'), { tools: ['fishing_rod'], consumes: ['fishing_baitx1'], products: ['raw_pike@25'], op: { slot: 3, label: 'Bait' } });
    assert.deepEqual(fishing('fishing.saltfish.op1'), { tools: ['net'], consumes: [], products: ['raw_shrimp@0', 'raw_anchovies@15'], op: { slot: 1, label: 'Net' } });
    assert.deepEqual(fishing('fishing.saltfish.op3'), { tools: ['fishing_rod'], consumes: ['fishing_baitx1'], products: ['raw_sardine@5', 'raw_herring@10'], op: { slot: 3, label: 'Bait' } });
    assert.deepEqual(fishing('fishing.rarefish.op1').tools, ['lobster_pot']);
    assert.deepEqual(fishing('fishing.rarefish.op3').products, ['raw_tuna@35', 'raw_swordfish@50']);
    assert.deepEqual(fishing('fishing.memberfish.op1').products.slice(-2), ['raw_cod@23', 'raw_bass@46']);
    assert.deepEqual(fishing('fishing.0_45_152_lavafish.op1').tools, ['oily_fishing_rod']);
    const bigNet = view.method('fishing.memberfish.op1').requirements;
    assert.equal(bigNet.state, revision === 289 ? 'partial' : 'known', `${revision} the 289 monkey-form gate is not modelled, so big-net requirements are partial there; 274 has no such gate`);
    assert.equal(bigNet.state !== 'partial' || bigNet.gaps.every((each) => each.code === 'monkey-form-forbidden'), true);
    assert.equal(targetsOf(view.method('fishing.freshfish.op1'), 'hazard').map((target) => view.name('npc', target.id)).join(), 'macro_whirlpool_freshfish', `${revision} whirlpool spots are hazards, not fishing targets`);

    // Placements are the map rows themselves.
    for (const [id, kind] of [['mining.iron', 'l'], ['mining.coal', 'l'], ['woodcutting.oak', 'l'], ['fishing.freshfish.op1', 'n']] as const) {
        const ids = new Set(targetsOf(view.method(id), 'resource').map((target) => target.id));
        const expected = mapRowKeys(root, kind === 'l' ? 'LOC' : 'NPC', ids);
        assert.ok(expected.size > 0, `${revision} ${id} is placed`);
        assert.deepEqual([...familyRowKeys(family, kind, ids)].sort(), [...expected].sort(), `${revision} ${id} placements equal the content's map rows`);
    }
    const gasIds = new Set(facts.methods.filter((each) => each.skill === 'mining').flatMap((each) => targetsOf(each, 'hazard').map((target) => target.id)));
    assert.equal(familyRowKeys(family, 'l', gasIds).size, 0, `${revision} hazards are never published as placements`);

    // Unknown is not empty: a custom-handler wood publishes neither targets nor spots.
    const jungle = view.method('woodcutting.jungle');
    assert.equal(jungle.targets.state, 'unknown');
    assert.equal(jungle.spots.state, 'unknown');
    assert.equal(jungle.targets.state === 'unknown' && jungle.targets.gap.code, 'no-resource-target');
    assert.equal(facts.loose.filter((each) => each.skill === 'woodcutting').every((each) => each.gap?.code === 'custom-handler' && each.gap.sources.some((source) => source.includes('jungle_tree.rs2'))), true);

    // Content-defined zones: 289 Miscellania intercepts coal, trees and fish, but not iron; 274 has no such content.
    const intercepted = facts.zones.filter((zone) => zone.effect === 'yield-intercepted').flatMap((zone) => zone.methods);
    if (revision === 289) assert.equal(intercepted.includes('mining.coal') && intercepted.includes('woodcutting.oak') && intercepted.includes('fishing.freshfish.op1'), true);
    else assert.deepEqual(intercepted, [], '274 content has no yield intercepts, so none are invented');
    assert.equal(intercepted.includes('mining.iron') || intercepted.includes('fishing.memberfish.op1'), false, `${revision} iron and big-net yields are not intercepted`);
    assert.equal(facts.zones.some((zone) => zone.effect === 'product-substituted' && zone.methods.includes('mining.gold') && !zone.methods.includes('mining.iron')), true);
}

// Real script text, tiny maps: mutate exactly one construct and prove the extractor degrades honestly.
const realGathering = gatheringPins[1].root;
function gatheringFixture(mutate?: (rootDir: string) => void) {
    const rootDir = fs.mkdtempSync(path.join(os.tmpdir(), 'gathering-fixture-'));
    fs.cpSync(path.join(realGathering, 'scripts'), path.join(rootDir, 'scripts'), { recursive: true });
    fs.mkdirSync(path.join(rootDir, 'pack'));
    for (const pack of ['loc', 'npc', 'obj']) fs.copyFileSync(path.join(realGathering, `pack/${pack}.pack`), path.join(rootDir, `pack/${pack}.pack`));
    const loc = parsePack(fs.readFileSync(path.join(rootDir, 'pack/loc.pack'), 'utf8'));
    const npc = parsePack(fs.readFileSync(path.join(rootDir, 'pack/npc.pack'), 'utf8'));
    fs.mkdirSync(path.join(rootDir, 'maps'));
    fs.writeFileSync(path.join(rootDir, 'maps/m50_50.jm2'), `==== LOC ====\n0 10 10: ${loc.get('ironrock1')} 10 0\n0 11 10: ${loc.get('ironrock2')} 10 1\n0 12 10: ${loc.get('copperrock1')} 10 0\n==== NPC ====\n0 20 20: ${npc.get('0_48_53_freshfish')}\n`);
    mutate?.(rootDir);
    execFileSync('git', ['init', '-q'], { cwd: rootDir });
    if (fs.existsSync(path.join(rootDir, 'maps'))) execFileSync('git', ['add', 'maps'], { cwd: rootDir });
    return rootDir;
}
function replaceIn(rootDir: string, relative: string, from: string, to: string) {
    const file = path.join(rootDir, relative);
    const before = fs.readFileSync(file, 'utf8');
    const after = before.replace(from, to);
    assert.notEqual(after, before, `${relative}: fixture mutation did not apply`);
    fs.writeFileSync(file, after);
}
const mineScript = 'scripts/skill_mining/scripts/mining.rs2';
const fishScript = 'scripts/skill_fishing/scripts/fishing_spots/freshfish.rs2';

const baseline = extractGatheringFamily(gatheringFixture());
assert.equal(family289Rows(baseline, 'mining.iron'), 2, 'the fixture places exactly its two iron rocks');
function family289Rows(family: GatheringFamily, method: string) {
    return family.summary.placements.by_method[method] ?? 0;
}
assert.equal(family289Rows(baseline, 'mining.copper'), 1);
assert.equal(family289Rows(baseline, 'fishing.freshfish.op1'), 1);
assert.equal(family289Rows(baseline, 'fishing.freshfish.op3'), 1);
assert.equal(baseline.payload.placements.length, 1);

// A drifted label is not silently trusted: the first direct-output yield (the iron label) changed.
const drifted = extractGatheringFamily(gatheringFixture((rootDir) => replaceIn(rootDir, mineScript, 'inv_add(inv, db_getfield($data, mining_table:rock_output, 0), 1);', 'inv_add(inv, db_getfield($data, mining_table:rock_output, 0), 2);')));
const driftedView = gatherView(drifted);
assert.equal(drifted.payload.methods.find((each) => each.id === 'mining.iron')?.targets.state, 'unknown', 'a drifted handler leaves iron unclassified, never an empty known list');
assert.equal(drifted.payload.methods.find((each) => each.id === 'mining.iron')?.spots.state, 'unknown');
assert.equal(drifted.payload.loose.some((each) => driftedView.name('loc', each.id) === 'ironrock1' && each.gap?.code === 'mining-handler-unrecognized'), true);
assert.equal(family289Rows(drifted, 'mining.iron'), 0);
assert.equal(family289Rows(drifted, 'mining.copper'), 1, 'unrelated labels still classify');

// A player-count scale that no longer parses keeps the raw content value and reports the scale as unknown.
const unscaled = extractGatheringFamily(gatheringFixture((rootDir) => replaceIn(rootDir, 'scripts/general/scripts/player_count.rs2', '4000, $base', '3000, $base')));
const copperRespawn = knownValue(targetsOf(gatherView(unscaled).method('mining.copper'), 'resource')[0].respawn, 'copper respawn');
assert.equal(copperRespawn?.raw, 10);
assert.equal(copperRespawn?.scale.state, 'unknown');

// A depleting rock without a next stage, or a row without its rate, has an unknown respawn, not a zero one.
const noStage = extractGatheringFamily(gatheringFixture((rootDir) => replaceIn(rootDir, 'scripts/skill_mining/configs/rocks.loc', 'category=mining_rock_normal\nparam=next_loc_stage_mining,rocks1\nparam=macro_gas,macro_copperrock1', 'category=mining_rock_normal\nparam=macro_gas,macro_copperrock1')));
const noStageView = gatherView(noStage);
const copperTargets = targetsOf(noStageView.method('mining.copper'), 'resource');
assert.equal(copperTargets.find((target) => noStageView.name('loc', target.id) === 'copperrock1')?.respawn.state, 'unknown');
assert.equal(copperTargets.find((target) => noStageView.name('loc', target.id) === 'copperrock2')?.respawn.state, 'known');
const noRate = extractGatheringFamily(gatheringFixture((rootDir) => replaceIn(rootDir, 'scripts/skill_mining/configs/mine.dbrow', 'data=rock_respawnrate,10\n\n[tin_rock_table]', '\n[tin_rock_table]')));
const noRateRespawn = targetsOf(gatherView(noRate).method('mining.copper'), 'resource')[0].respawn;
assert.equal(noRateRespawn.state === 'unknown' && noRateRespawn.gap.code, 'respawn-rate-missing');

// A quest handler that takes over one rock alias removes it from the ore's targets and records why.
const takenOver = extractGatheringFamily(gatheringFixture((rootDir) => fs.appendFileSync(path.join(rootDir, 'scripts/areas/area_miscellania/scripts/miner_magnus.rs2'), '\n[oploc1,ironrock1]\nmes("Someone else uses this rock.");\n')));
const takenView = gatherView(takenOver);
assert.deepEqual(targetsOf(takenView.method('mining.iron'), 'resource').map((target) => takenView.name('loc', target.id)), ['ironrock2']);
assert.equal(takenOver.payload.loose.find((each) => takenView.name('loc', each.id) === 'ironrock1')?.gap?.code, 'custom-handler');
assert.equal(family289Rows(takenOver, 'mining.iron'), 1);
// M-215: the real tutorial handler in the fixture yields copper ore, so its rock is a copper
// resource; changing the yielded item to a non-ore drops it back to Unknown with its handler reason.
const tutScript = 'scripts/tutorial/scripts/skills/tut_mining.rs2';
const baselineView = gatherView(baseline);
assert.deepEqual(targetsOf(baselineView.method('mining.copper'), 'resource').map((target) => baselineView.name('loc', target.id)), ['copperrock1', 'copperrock2', 'newbiecopperrock']);
assert.equal(baselineView.method('mining.copper').targets.state, 'partial');
const retyped = extractGatheringFamily(gatheringFixture((rootDir) => replaceIn(rootDir, tutScript, 'inv_add(inv, copper_ore, 1);', 'inv_add(inv, logs, 1);')));
const retypedView = gatherView(retyped);
assert.deepEqual(targetsOf(retypedView.method('mining.copper'), 'resource').map((target) => retypedView.name('loc', target.id)), ['copperrock1', 'copperrock2']);
assert.equal(retyped.payload.loose.find((each) => retypedView.name('loc', each.id) === 'newbiecopperrock')?.gap?.code, 'custom-handler');
assert.equal(family289Rows(retyped, 'mining.copper'), 1, 'the tutorial rock contributes no fixture placements');
// M-215 R2: promotion is fail-closed on yield cardinality. A second grant beside the unit
// copper yield — even a repeated identical one or a non-unit extra — must drop the rock back
// to Unknown instead of promoting it as a copper resource.
const doubled = extractGatheringFamily(gatheringFixture((rootDir) => replaceIn(rootDir, tutScript, 'inv_add(inv, copper_ore, 1);', 'inv_add(inv, copper_ore, 1);\ninv_add(inv, copper_ore, 1);')));
const doubledView = gatherView(doubled);
assert.deepEqual(targetsOf(doubledView.method('mining.copper'), 'resource').map((target) => doubledView.name('loc', target.id)), ['copperrock1', 'copperrock2']);
assert.equal(doubled.payload.loose.find((each) => doubledView.name('loc', each.id) === 'newbiecopperrock')?.gap?.code, 'custom-handler');
const extraGrant = extractGatheringFamily(gatheringFixture((rootDir) => replaceIn(rootDir, tutScript, 'inv_add(inv, copper_ore, 1);', 'inv_add(inv, copper_ore, 1);\ninv_add(inv, tin_ore, 2);')));
const extraView = gatherView(extraGrant);
assert.deepEqual(targetsOf(extraView.method('mining.copper'), 'resource').map((target) => extraView.name('loc', target.id)), ['copperrock1', 'copperrock2']);
assert.equal(extraGrant.payload.loose.find((each) => extraView.name('loc', each.id) === 'newbiecopperrock')?.gap?.code, 'custom-handler');

// Bait must agree between the equipment struct and the roll; an unmodelled guard makes requirements partial.
const wrongBait = extractGatheringFamily(gatheringFixture((rootDir) => replaceIn(rootDir, fishScript, '~fish_roll(raw_pike, null, fishing_rod, fishing_bait)', '~fish_roll(raw_pike, null, fishing_rod, feather)')));
const pike = gatherView(wrongBait).method('fishing.freshfish.op3');
assert.equal(pike.products.state === 'partial' && pike.products.gaps.some((each) => each.code === 'bait-disagrees-with-equipment'), true);
const gated = extractGatheringFamily(gatheringFixture((rootDir) => replaceIn(rootDir, fishScript, '// check level\nif (stat(fishing) < 25) {', 'if (%tbwt_lubufu < 3) {\n    return;\n}\n// check level\nif (stat(fishing) < 25) {')));
const gatedPike = gatherView(gated).method('fishing.freshfish.op3').requirements;
assert.equal(gatedPike.state === 'partial' && gatedPike.gaps.some((each) => each.code === 'varp-gate'), true, 'an unmodelled quest guard is a gap, never silently ignored');
assert.equal(gatherView(gated).method('fishing.freshfish.op1').requirements.state, 'known', 'other methods are unaffected');

// M-215 R3: a global `[oploc1]` handler is the engine's third dispatch tier (type, then
// category, then global per `ScriptProvider.getByTrigger`), which the no-handler exclusion
// rule does not model. Extraction fails closed on it instead of silently dropping the rock.
assert.throws(() => extractGatheringFamily(gatheringFixture((rootDir) => fs.appendFileSync(path.join(rootDir, 'scripts/areas/area_miscellania/scripts/miner_magnus.rs2'), '\n[oploc1]\nmes("Global mine handler.");\n'))), /doesn't model global handlers/);
// Required inputs fail closed.
assert.throws(() => extractGatheringFamily(gatheringFixture((rootDir) => fs.rmSync(path.join(rootDir, 'scripts/skill_mining/configs/mine.dbrow')))), /mine\.dbrow/);
assert.throws(() => extractGatheringFamily(gatheringFixture((rootDir) => fs.rmSync(path.join(rootDir, 'pack/npc.pack')))), /npc\.pack/);
assert.throws(() => extractGatheringFamily(gatheringFixture((rootDir) => fs.rmSync(path.join(rootDir, 'maps'), { recursive: true }))), /maps.*required directory missing|ENOENT/);
assert.throws(() => extractGatheringFamily(gatheringFixture((rootDir) => fs.writeFileSync(path.join(rootDir, 'maps/m50_50.jm2'), '==== LOC ====\n0 10 10: 2092 10 0 extra tokens\n'))), /tokens/);
assert.throws(() => extractGatheringFamily(gatheringFixture((rootDir) => replaceIn(rootDir, 'scripts/skill_mining/configs/mine.dbrow', 'data=rock,copperrock1', 'data=rock,not_a_packed_loc'))), /failed join/);

console.log('generate fixture passed');
