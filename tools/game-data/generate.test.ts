import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { bankCatalogJson, cookCatalogJson } from './generate.ts';
import {
    assertTrioGiverNpcJoins,
    assertTalkKeyNpcJoins,
    extractDropFacts,
    extractFacts,
    extractEquipmentNamesFacts,
    extractFlourSixFacts,
    extractTalkKeyFacts,
    extractTrailFacts,
    extractTrioGiversFacts,
    extractHerbFacts,
    extractMagicFacts,
    extractAutocastControls,
    extractDuelControls,
    extractNurmofEssenceFacts,
    extractPrayerFacts,
    extractSpecialControls,
    extractTeleportSpells,
    herbKeyFromName,
    identifiedHerbLevelDefault,
    joinEquipmentName,
    loadEquipmentNamesCurated,
    parseIdentifyHerbPairs,
    parseJm2LinkBelow,
    parseJm2NpcPlacements,
    parseTrioGiverHandlers,
    parseInvShopStock,
    parseObjSections,
    parsePrayerInterface,
    parseQuestEnumEntry,
} from './generate.ts';

import { parseJm2LocPlacements, parseMapsquarePath, parsePack, parseRows, parseParamDefinitions } from './extractors/common.ts';
import { extractGatheringFamily, compareCodepoint, gatherMethodGap, gatherResources, gatherSites, GATHERING_SCHEMA, isKnownGatherTarget, SITE_OVERRIDES, type GatheringFacts, type GatheringFamily, type Know, type MethodWire, type SiteOverride, type TargetWire } from './extractors/gathering.ts';
import { parseDbRows, parseSections } from './extractors/gathering-content.ts';
import { extractQuestIdentityFacts } from './extractors/quests.ts';
import { extractQuestStartFacts } from './extractors/quest-starts.ts';
import { extractDialogueUiFacts } from './extractors/dialogue-ui.ts';
import type { TrioGiverFacts, TalkKeyFacts } from './generate.ts';
import { DEBUG_SCHEMA_VERSION, QUESTS_SCHEMA, buildDebugArtifact, categoryName, debugArtifactBytes, extractQuestsFamily, familyBytes, parseGenerationArgs, revisions, runtimeNpcOperations } from './generate.ts';
import { extractCombatStyleFacts, parseCombatScripts, extractSpecialWeaponFacts, extractItemCombatBonus, extractMeleeWeaponTiers } from './extractors/combat.ts';
import { extractDuelRuleButtons, extractDuelZones } from './extractors/duel.ts';
import { extractNpcNamesFacts } from './extractors/npc-names.ts';
import './dialogue-ui.test.ts';
import './gather-catalog.test.ts';
import './extractors/progress-values.test.ts';
import './quest-junk.test.ts';
import './extractors/prereq-preservation.test.ts';
import './extractors/prereq-journal-proof.test.ts';
import './extractors/prereq-steps.test.ts';
import './server-ordered.test.ts';
import './npc-transforms.test.ts';
import './npc-stat-default.test.ts';
import { extractKaramjaFacts } from './extractors/karamja.ts';
import type { NpcTransformClosureEvidence } from './extractors/npc-transforms.ts';
import { extractCookSurfaces } from './generate.ts';

const repoRoot = path.resolve(import.meta.dirname, '../..');
const bankJsonFixture = bankCatalogJson([
    {
        name: 'Main bank',
        tile: { x: 3200, z: 3201, level: 0 },
        approach: { x: 3199, z: 3201, level: 0 },
        requires: { skill: { name: 'fishing', level: 30 }, quest: 'A quest', setting: 'a_setting' },
        access: { name: 'Bank booth', op: 'Bank', openFirst: { name: 'Door', op: 'Open' } },
        npcAccess: { name: 'Banker', op: 'Talk-to', choose: 'Bank' },
    },
    {
        name: 'Remote bank',
        tile: { x: 3300, z: 3301, level: 1 },
    },
]);
assert.equal(bankJsonFixture.endsWith('\n'), true, 'bank catalog JSON ends with a newline');
assert.deepEqual(JSON.parse(bankJsonFixture), {
    schema: 1,
    facts: [
        {
            name: 'Main bank',
            tile: { x: 3200, z: 3201, level: 0 },
            approach: { x: 3199, z: 3201, level: 0 },
            skill: [10, 30],
            quest: 'A quest',
            setting: 'a_setting',
            object: { name: 'Bank booth', op: 'Bank' },
            open_first: { name: 'Door', op: 'Open' },
            npc: { name: 'Banker', op: 'Talk-to' },
            choose: 'Bank',
        },
        {
            name: 'Remote bank',
            tile: { x: 3300, z: 3301, level: 1 },
            approach: null,
            skill: null,
            quest: null,
            setting: null,
            object: null,
            open_first: null,
            npc: null,
            choose: null,
        },
    ],
});
assert.throws(() => bankCatalogJson([{
    name: 'Unsupported gate',
    tile: { x: 1, z: 2, level: 0 },
    requires: { skill: { name: 'mining', level: 1 } },
}]), /bank catalog: unknown skill mining/);

const cookJsonFixture = cookCatalogJson({
    camps: [
        { bank: 'First bank', stand: { x: 1, z: 2, level: 0 }, loc: { x: 3, z: 4, level: 0 }, locName: 'Range', kind: 'oven', label: 'First range' },
        { bank: 'Second bank', stand: { x: 5, z: 6, level: 1 }, approach: { x: 4, z: 6, level: 1 }, loc: { x: 7, z: 8, level: 1 }, locName: 'Fire', kind: 'fire', label: 'Second fire' },
    ],
    kinds: { range: 'oven', fire: 'fire' },
    maxSurfaceCheb: 9,
    derivedArriveRadius: 4,
    obstacles: ['counter', 'wall'],
});
assert.equal(cookJsonFixture.endsWith('\n'), true, 'cook catalog JSON ends with a newline');
assert.deepEqual(JSON.parse(cookJsonFixture), {
    schema: 1,
    facts: {
        max_surface_cheb: 9,
        derived_arrive_radius: 4,
        default_obstacles: ['counter', 'wall'],
        camps: [
            { bank: 'First bank', stand: { x: 1, z: 2, level: 0 }, approach: null, loc: { x: 3, z: 4, level: 0 }, loc_name: 'Range', kind: 'oven', label: 'First range' },
            { bank: 'Second bank', stand: { x: 5, z: 6, level: 1 }, approach: { x: 4, z: 6, level: 1 }, loc: { x: 7, z: 8, level: 1 }, loc_name: 'Fire', kind: 'fire', label: 'Second fire' },
        ],
    },
});
assert.equal(JSON.parse(cookJsonFixture).facts.kinds, undefined, 'cook surface kinds stay internal to extraction');

assert.deepEqual(
    ['fishing.a_b.op1', 'fishing.a-b.op1', 'fishing.A.op1'].sort(compareCodepoint),
    ['fishing.A.op1', 'fishing.a-b.op1', 'fishing.a_b.op1'],
    'group members use locale-independent string ordering',
);
assert.deepEqual(
    ['\u{10000}', '\uE000'].sort(compareCodepoint),
    ['\uE000', '\u{10000}'],
    'ordering compares Unicode code points rather than locale collation or UTF-16 units',
);

{
    const npcIds = [1, 2, 3, 390, 403];
    const evidence: NpcTransformClosureEvidence = {
        schema: 1,
        source_spans: [{ file: 'scripts/test.rs2', first: 1, last: 2 }],
        npc_types: npcIds.map(npc_type_id => ({ npc_type_id, category_id: null, config_state: 'complete', source_refs: [0] })),
        actions: npcIds.map(npc_type_id => ({
            npc_type_id, slot: 1, state: 'present', label: 'Talk-to', trigger_code: 1,
            trigger_kind: 'type', trigger_target_id: npc_type_id,
            operation_fingerprint: 'a'.repeat(64), source_refs: [0],
        })),
        transform_edges: [],
        unresolved: [],
        proofs: [1, 2].map(npc_type_id => ({
            npc_type_id, slot: 1, status: npc_type_id === 1 ? 'unchanged' : 'unavailable',
            reachable_npc_type_ids: [npc_type_id], mismatched_npc_type_ids: [],
            evidence_refs: [0], unresolved_refs: [],
        })),
        engine: { complete: true, source_refs: { numeric_dispatch: [0], type_change: [0], reversion: [0], static_type_decoder: [0] } },
    };
    const target = (id: number): TargetWire => ({
        kind: 'npc', id, op: 1, class: 'resource', respawn: { state: 'known', value: null },
    });
    const fishingMethod: Pick<MethodWire, 'skill' | 'op' | 'targets'> = {
        skill: 'fishing',
        op: { slot: 1, label: 'Net' },
        targets: { state: 'known', value: [...npcIds, 999].map(target) },
    };
    const methods = [fishingMethod, fishingMethod];
    const hazardNpcs = [3];
    const runtime = runtimeNpcOperations(evidence, methods, hazardNpcs);
    assert.deepEqual(runtime.operations, [{ npc_type_id: 1, slot: 1, operation_fingerprint: 'a'.repeat(64) }]);
    assert.deepEqual(runtime.builtin_gatherer_fishing_operations, [
        { npc_type_id: 1, slot: 1, operation_fingerprint: 'a'.repeat(64) },
        { npc_type_id: 2, slot: 1, operation_fingerprint: 'a'.repeat(64) },
    ], 'static proof admits only complete ordinary NPC types, even when full closure is unavailable');
    assert.equal(runtime.schema, 2);
    assert.deepEqual(Object.keys(runtime), ['schema', 'operations', 'builtin_gatherer_fishing_operations']);
    evidence.proofs[0].unresolved_refs.push(0);
    assert.throws(() => runtimeNpcOperations(evidence, methods, hazardNpcs), /positive NPC proof disagrees/);
    evidence.proofs[0].unresolved_refs.length = 0;
    evidence.npc_types[0].config_state = 'missing';
    assert.throws(() => runtimeNpcOperations(evidence, methods, hazardNpcs), /positive NPC proof disagrees/);
    evidence.engine.complete = false;
    const incompleteRuntime = runtimeNpcOperations(evidence, methods, hazardNpcs);
    assert.deepEqual(incompleteRuntime.operations, []);
    assert.deepEqual(incompleteRuntime.builtin_gatherer_fishing_operations, []);
}


/** NPC stat defaults are decoded from the selected local engine source. */
function engineNpcTypeSource(revision: 274 | 289): string {
    const engine = revisions.find((spec) => spec.revision === revision)!.engine;
    return fs.readFileSync(path.join(engine, 'src/cache/config/NpcType.ts'), 'utf8');
}

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
const itemCategories = new Map<number, string>();
for (const [name, id] of parsePack('14=weapon_slash\n15=unstrung_bow\n')) {
    itemCategories.set(id, name);
}
const categoryObj = (id: number, category?: number) => ({
    id,
    debugname: null,
    name: null,
    cost: 0,
    stackable: false,
    members: false,
    certlink: -1,
    certtemplate: -1,
    wearpos: -1,
    wearpos2: -1,
    wearpos3: -1,
    manwear: -1,
    ...(category === undefined ? {} : { category }),
});
assert.equal(categoryName(categoryObj(2961, 14), itemCategories), 'weapon_slash');
assert.equal(categoryName(categoryObj(48, 15), itemCategories), 'unstrung_bow');
assert.equal(categoryName(categoryObj(1, -1), itemCategories), null);
assert.equal(categoryName(categoryObj(2), itemCategories), null);
assert.throws(() => categoryName(categoryObj(3, 99), itemCategories), /obj 3: category 99 has no content name/);

const content = fs.mkdtempSync(path.join(os.tmpdir(), 'game-data-fixture-'));
const pack = path.join(content, 'pack');
const consumption = path.join(content, 'scripts/player/configs/consumption');
const consumeScript = path.join(content, 'scripts/player/scripts/consumption');
const thieving = path.join(content, 'scripts/skill_thieving/configs/pickpocking');
const digsiteScript = path.join(content, 'scripts/quests/quest_itexam/scripts/digsite_workman.rs2');
const trollGuardScript = path.join(content, 'scripts/quests/quest_troll/scripts/troll_stronghold_camp_guard.rs2');
for (const file of [digsiteScript, trollGuardScript]) fs.mkdirSync(path.dirname(file), { recursive: true });
for (const directory of [pack, consumption, consumeScript, thieving]) fs.mkdirSync(directory, { recursive: true });
fs.writeFileSync(path.join(pack, 'category.pack'), '4=food\n5=potion\n6=category_5\n');
fs.writeFileSync(path.join(pack, 'enum.pack'), '0=potion_prayerrestore\n');
fs.writeFileSync(path.join(pack, 'param.pack'), [
    '0=next_obj_stage', '1=decant_potion_enum', '2=dose_count',
    '3=stabattack_anim', '4=slashattack_anim', '5=crushattack_anim',
    '6=rangeattack_anim', '7=proj_launch', '8=proj_travel', '9=attackrate',
    '10=levelrequire', '11=attackrange', '12=stabattack', '13=slashattack',
    '14=crushattack', '15=magicattack', '16=rangeattack', '17=stabdefence',
    '18=slashdefence', '19=crushdefence', '20=magicdefence', '21=rangedefence',
    '22=strengthbonus', '23=rangebonus',
].join('\n') + '\n');
fs.writeFileSync(path.join(pack, 'obj.pack'), '50=ogre_bow\n62=bronze_bow\n63=bronze_arrow\n');
fs.writeFileSync(path.join(consumeScript, 'consume.rs2'), `[opheld1,_food]
def_int $eat_delay = 2;
def_int $skill_delay = 3;
if (oc_param(last_item, next_obj_stage) = ^stale_anchovies) {
    $eat_delay = 1;
}
@player_consume_item(^consume_food, $eat_delay, $skill_delay);
[opheld1,_potion]
@player_consume_item(^consume_potion, null, null);
`);
fs.writeFileSync(path.join(consumption, 'consume_normal.dbrow'), `[food]
data=consumable,bread
data=consumable,anchovies
data=stat_heal,hitpoints,4,0
[zero]
data=consumable,zero_food
data=stat_heal,hitpoints,0,0
[unhandled]
data=consumable,cooked_chompy
data=stat_heal,hitpoints,10,0
`);
fs.writeFileSync(path.join(consumption, 'consume_effects.dbrow'), `[potion]
data=consumable,potion
data=stat_change,attack,1,5
data=stat_heal,hitpoints,5,10
data=healenergy,3
`);
fs.writeFileSync(path.join(consumption, 'consume_messages.dbrow'), `[anchovies]
data=consumable,anchovies
data=message_delay,2
[potion]
data=consumable,potion
data=message_delay,1
[cooked_chompy]
data=consumable,cooked_chompy
data=message_delay,4
`);
fs.writeFileSync(path.join(thieving, 'pickpocket.dbrow'), `[guard]
data=npc,guard1
data=npc,guard2
data=level,40
data=experience,30
data=stun_ticks,8
data=stun_damage,3
data=success_chance,1,2
data=pocket,coins
data=loot,coin,1,5,10
data=loot,coin,2,3,20
`);
fs.writeFileSync(digsiteScript, `[opnpc3,digworkman1] @pickpocket_digworkman1;
[opnpc3,digworkman2] @pickpocket_digworkman1;
if (stat (thieving) < 25) {
    return;
}
`);
fs.writeFileSync(trollGuardScript, `[opnpc3,troll_prison_guard1]
[opnpc3,troll_prison_guard2]
if (stat(thieving) < 30) {
    return;
}
`);
const facts = extractFacts(content,
    [
        { id: 1, debugname: 'bread', name: 'Bread', cost: 0, stackable: false, members: false, certlink: -1, certtemplate: -1, wearpos: -1, wearpos2: -1, wearpos3: -1, category: 4, params: new Map() },
        { id: 2, debugname: 'anchovies', name: 'Anchovies', cost: 0, stackable: false, members: false, certlink: -1, certtemplate: -1, wearpos: -1, wearpos2: -1, wearpos3: -1, category: 4, params: new Map([[0, 'stale_anchovies']]) },
        { id: 3, debugname: 'zero_food', name: 'Zero food', cost: 0, stackable: false, members: false, certlink: -1, certtemplate: -1, wearpos: -1, wearpos2: -1, wearpos3: -1, category: 4, params: new Map() },
        { id: 4, debugname: 'potion', name: 'Potion', cost: 0, stackable: false, members: false, certlink: -1, certtemplate: -1, wearpos: -1, wearpos2: -1, wearpos3: -1, category: 5, params: new Map([[1, 0], [2, 1]]) },
        { id: 5, debugname: 'coin', name: 'Coins', cost: 0, stackable: true, members: false, certlink: -1, certtemplate: -1, wearpos: -1, wearpos2: -1, wearpos3: -1 },
        { id: 6, debugname: 'stale_anchovies', name: 'Stale anchovies', cost: 0, stackable: false, members: false, certlink: -1, certtemplate: -1, wearpos: -1, wearpos2: -1, wearpos3: -1, category: 4, params: new Map() },
        { id: 7, debugname: 'cooked_chompy', name: 'Cooked chompy', cost: 0, stackable: false, members: false, certlink: -1, certtemplate: -1, wearpos: -1, wearpos2: -1, wearpos3: -1, category: 6, params: new Map() },
    ],
    [
        { id: 10, debugname: 'guard1', name: 'Guard', respawnrate: 120 },
        { id: 11, debugname: 'guard2', name: 'Guard' },
        { id: 12, debugname: 'digworkman1', name: 'Digsite workman' },
        { id: 13, debugname: 'digworkman2', name: 'Digsite workman', respawnrate: 120 },
        { id: 14, debugname: 'troll_prison_guard1', name: 'Twig' },
        { id: 15, debugname: 'troll_prison_guard2', name: 'Berry' },
    ],
);
assert.equal(facts.consumption.length, 5);
assert.equal(facts.consumption.filter((fact) => fact.qualification === 'fixed_hp_heal').length, 2);
assert.equal(facts.consumption.find((fact) => fact.item.alias === 'zero_food')?.qualification, 'not_fixed_hp_heal');
const unhandledFact = facts.consumption.find((fact) => fact.item.alias === 'cooked_chompy')!;
assert.equal(unhandledFact.effect, '');
assert.equal(unhandledFact.eat_delay_arg, null);
assert.equal(unhandledFact.skill_delay_arg, null);
assert.equal(unhandledFact.message_delay, null);
assert.equal(unhandledFact.qualification, 'not_fixed_hp_heal');
assert.deepEqual(unhandledFact.stat_heal, [{ stat: 'hitpoints', base: 10, percent: 0 }]);
assert.equal(facts.consumption.find((fact) => fact.item.alias === 'potion')?.stat_change[0].percent, 5);
const breadFact = facts.consumption.find((fact) => fact.item.alias === 'bread')!;
assert.equal(breadFact.eat_delay_arg, 2);
assert.equal(breadFact.skill_delay_arg, 3);
assert.equal(breadFact.message_delay, null);
const stageFoodFact = facts.consumption.find((fact) => fact.item.alias === 'anchovies')!;
assert.equal(stageFoodFact.next_stage, 'stale_anchovies');
assert.equal(stageFoodFact.eat_delay_arg, 1, 'the source next-stage branch overrides the ordinary food delay');
assert.equal(stageFoodFact.skill_delay_arg, 3);
assert.equal(stageFoodFact.message_delay, 2);
const potionFact = facts.consumption.find((fact) => fact.item.alias === 'potion')!;
assert.equal(potionFact.effect, 'consume_potion');
assert.equal(potionFact.eat_delay_arg, null);
assert.equal(potionFact.skill_delay_arg, null);
assert.equal(potionFact.message_delay, 1);
assert.equal(potionFact.dose_family, 'potion_prayerrestore');
assert.equal(potionFact.dose_count, 1);
const fullPickpocket = facts.pickpocket.find((fact) => !('kind' in fact))!;
assert.deepEqual(fullPickpocket.npcs.map((npc) => npc.alias), ['guard1', 'guard2']);
assert.deepEqual(
    fullPickpocket.npcs.map((npc) => npc.respawn_ticks),
    [120, 100],
    'the declared respawnrate wins and an omitted one is the engine default 100',
);
assert.equal(fullPickpocket.loot.length, 2);
assert.deepEqual(fullPickpocket.success_chance, { numerator: 1, denominator: 2 });
const levelOnlyPickpocket = facts.pickpocket.filter((fact) => 'kind' in fact);
assert.deepEqual(
    levelOnlyPickpocket.map((fact) => ({
        alias: fact.npcs[0].alias,
        id: fact.npcs[0].id,
        respawn_ticks: fact.npcs[0].respawn_ticks,
        level: fact.level,
        kind: fact.kind,
        source_file: fact.source_file,
        target_row: fact.target_row,
        source_row: fact.source_row,
    })),
    [
        { alias: 'digworkman1', id: 12, respawn_ticks: 100, level: 25, kind: 'level_only', source_file: 'scripts/quests/quest_itexam/scripts/digsite_workman.rs2', target_row: '[opnpc3,digworkman1] @pickpocket_digworkman1;', source_row: 'if (stat (thieving) < 25) {' },
        { alias: 'digworkman2', id: 13, respawn_ticks: 120, level: 25, kind: 'level_only', source_file: 'scripts/quests/quest_itexam/scripts/digsite_workman.rs2', target_row: '[opnpc3,digworkman2] @pickpocket_digworkman1;', source_row: 'if (stat (thieving) < 25) {' },
        { alias: 'troll_prison_guard1', id: 14, respawn_ticks: 100, level: 30, kind: 'level_only', source_file: 'scripts/quests/quest_troll/scripts/troll_stronghold_camp_guard.rs2', target_row: '[opnpc3,troll_prison_guard1]', source_row: 'if (stat(thieving) < 30) {' },
        { alias: 'troll_prison_guard2', id: 15, respawn_ticks: 100, level: 30, kind: 'level_only', source_file: 'scripts/quests/quest_troll/scripts/troll_stronghold_camp_guard.rs2', target_row: '[opnpc3,troll_prison_guard2]', source_row: 'if (stat(thieving) < 30) {' },
    ],
);
for (const fact of levelOnlyPickpocket) {
    for (const key of ['experience', 'stun_ticks', 'stun_damage', 'success_chance', 'loot', 'pocket']) {
        assert.equal(Object.hasOwn(fact, key), false, `${fact.npcs[0].alias} must stay level-only`);
    }
}

const combat = path.join(content, 'scripts/skill_combat/configs/magic');
const magic = path.join(content, 'scripts/skill_magic/configs');
fs.mkdirSync(combat, { recursive: true });
fs.mkdirSync(magic, { recursive: true });
const combatParamFile = path.join(content, 'scripts/skill_combat/configs/combat.param');
const combatParams = `[strengthbonus]
type=int
default=0
[rangebonus]
type=int
default=0
[attackbonus]
type=int
default=0
[stabattack]
type=int
default=0
[slashattack]
type=int
default=0
[crushattack]
type=int
default=0
[magicattack]
type=int
default=0
[rangeattack]
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
fs.writeFileSync(combatParamFile, combatParams);
fs.writeFileSync(path.join(content, 'scripts/skill_combat/configs/npc_combat.param'), '[damagetype]\ntype=int\ndefault=2\n');
fs.writeFileSync(path.join(combat, 'magic_combat_spells.dbrow'), `[magic_spell_wind_strike]
data=name,Wind Strike
data=spellcom,wind_strike_button
data=spell,^test_spell
data=levelrequired,1
data=maxhit,2
data=members,false
data=anim,^test_magic_seq
data=staffanim,^test_magic_seq
data=spotanim_origin,^spell_origin
data=spotanim_proj,^spell_projectile
data=spotanim_target,^spell_target
data=runesrequired,mindrune,1,airrune,1,null,null
data=continue_by_autocast,true
[magic_spell_iban]
data=levelrequired,50
data=spell,^test_iban
data=maxhit,10
data=members,true
data=runesrequired,firerune,1,null,null,null,null
data=continue_by_autocast,true
[magic_spell_fire_wave]
data=name,Fire Wave
data=spellcom,fire_wave_button
data=spell,^fire_wave
data=levelrequired,75
data=maxhit,15
data=members,true
data=wornrequired,staff_of_fire
data=worn_reqmessage,"You need a fire staff."
data=anim,^test_magic_seq
data=staffanim,^test_magic_seq
data=runesrequired,bloodrune,1,firerune,7,airrune,5
data=spotanim_target,fire_impact,10
data=continue_by_autocast,true
[magic_spell_crumble_test]
data=spellcom,crumble_button
data=spell,^test_crumble
data=levelrequired,39
data=maxhit,8
data=members,false
data=wornrequired,staff_of_fire
data=worn_reqmessage,"You need a fire staff."
data=spotanim_target,crumble_test_impact,5
data=runesrequired,chaosrune,1,airrune,2,earthrune,2
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
fs.writeFileSync(path.join(pack, 'interface.pack'), '3001=wind_strike_button\n3002=fire_wave_button\n3003=crumble_button\n');
fs.writeFileSync(path.join(pack, 'spotanim.pack'), '5=spell_target\n6=fire_impact\n7=crumble_test_impact\n85=failedspell_impact\n');
const autoCastDir = path.join(content, 'scripts/skill_combat/scripts/player');
fs.mkdirSync(autoCastDir, { recursive: true });
fs.writeFileSync(path.join(autoCastDir, 'auto_cast.rs2'), `[if_button,staff_spells:ssb0] @set_autocast_spell(^test_spell);\n[if_button,staff_spells:ssb1] @set_autocast_spell(^fire_wave);\n`);
const magicInterfaces = path.join(content, 'scripts/skill_magic/interfaces');
fs.mkdirSync(magicInterfaces, { recursive: true });
fs.writeFileSync(path.join(magicInterfaces, 'magic.if'), `[test_spell]\naction=Wind strike\n[fire_wave]\naction=Fire wave\n[test_crumble]\naction=Crumble test\n`);
const magicItems = [
    { id: 1, debugname: 'mindrune', name: 'Mind rune', cost: 0, stackable: true, members: false, certlink: -1, certtemplate: -1, wearpos: -1, wearpos2: -1, wearpos3: -1 },
    { id: 2, debugname: 'airrune', name: 'Air rune', cost: 0, stackable: true, members: false, certlink: -1, certtemplate: -1, wearpos: -1, wearpos2: -1, wearpos3: -1 },
    { id: 3, debugname: 'firerune', name: 'Fire rune', cost: 0, stackable: true, members: false, certlink: -1, certtemplate: -1, wearpos: -1, wearpos2: -1, wearpos3: -1 },
    { id: 4, debugname: 'bloodrune', name: 'Blood rune', cost: 0, stackable: true, members: false, certlink: -1, certtemplate: -1, wearpos: -1, wearpos2: -1, wearpos3: -1 },
    { id: 5, debugname: 'earthrune', name: 'Earth rune', cost: 0, stackable: true, members: false, certlink: -1, certtemplate: -1, wearpos: -1, wearpos2: -1, wearpos3: -1 },
    { id: 9, debugname: 'chaosrune', name: 'Chaos rune', cost: 0, stackable: true, members: false, certlink: -1, certtemplate: -1, wearpos: -1, wearpos2: -1, wearpos3: -1 },
    { id: 6, debugname: 'staff_of_fire', name: 'Staff of fire', cost: 0, stackable: false, members: false, certlink: -1, certtemplate: -1, wearpos: 3, wearpos2: -1, wearpos3: -1 },
    { id: 7, debugname: 'lava_battlestaff', name: 'Lava battlestaff', cost: 0, stackable: false, members: false, certlink: -1, certtemplate: -1, wearpos: 3, wearpos2: -1, wearpos3: -1 },
    { id: 8, debugname: 'staff_of_air', name: 'Staff of air', cost: 0, stackable: false, members: false, certlink: -1, certtemplate: -1, wearpos: 3, wearpos2: -1, wearpos3: -1 },
];
const magicFacts = extractMagicFacts(content, magicItems);
assert.equal(magicFacts.spells.length, 3, 'manual combat spells join the table; rows without a spell button stay out');
assert.equal(magicFacts.spells[0].name, 'Wind Strike');
assert.equal(magicFacts.spells[0].ssb, 0);
assert.deepEqual(magicFacts.spells[0].runes.map((rune) => [rune.name, rune.count]), [['Mind rune', 1], ['Air rune', 1]]);
assert.equal(magicFacts.spells[1].name, 'Fire Wave');
assert.equal(magicFacts.spells[1].ssb, 1);
const lava = magicFacts.staves.find((staff) => staff.name === 'Lava battlestaff');
assert.deepEqual(lava?.runes.map((rune) => rune.name).sort(), ['Earth rune', 'Fire rune']);
assert.equal(magicFacts.staves.find((staff) => staff.name === 'Staff of air')?.runes.length, 1);
assert.equal(magicFacts.spells[0].spellcom, 'wind_strike_button');
assert.equal(magicFacts.spells[0].maxhit, 2);
assert.equal(magicFacts.spells[0].members, false);
assert.equal(magicFacts.spells[1].spellcom, 'fire_wave_button');
assert.equal(magicFacts.spells[1].maxhit, 15);
assert.equal(magicFacts.spells[1].members, true);
assert.equal(magicFacts.spells[1].wornrequired, 'staff_of_fire');
assert.equal(magicFacts.spells[0].autocast_selectable, true);
assert.equal(magicFacts.spells[0].component_id, 3001);
assert.equal(magicFacts.spells[0].source_row, 'magic_spell_wind_strike');
assert.equal(magicFacts.spells[1].autocast_selectable, true);
assert.equal(magicFacts.spells[1].component_id, 3002);
assert.equal(magicFacts.spells[0].impact_spotanim, 5);
assert.equal(magicFacts.spells[1].impact_spotanim, 6);
const crumble = magicFacts.spells.find((spell) => spell.source_row === 'magic_spell_crumble_test')!;
assert.equal(crumble.name, 'Crumble test');
assert.equal(crumble.autocast_selectable, false, 'the staff chooser excludes manual-only spells');
assert.equal(crumble.ssb, -1);
assert.equal(crumble.component_id, 3003);
assert.equal(crumble.maxhit, 8);
assert.equal(crumble.members, false);
assert.deepEqual(crumble.runes.map((rune) => [rune.name, rune.count]), [['Chaos rune', 1], ['Air rune', 2], ['Earth rune', 2]]);
assert.equal(crumble.worn_reqmessage, 'You need a fire staff.');
assert.equal(crumble.impact_spotanim, 7);
assert.equal(magicFacts.failed_spell_impact, 85, 'the miss splash is one selected scalar, not per-spell data');
const npcSource = path.join(content, 'scripts/npc/config/combat.npc');
const npcScriptDir = path.join(content, 'scripts/npc/scripts');
fs.mkdirSync(path.dirname(npcSource), { recursive: true });
fs.mkdirSync(npcScriptDir, { recursive: true });
fs.writeFileSync(path.join(pack, 'npc.pack'), '100=test_npc\n477=khazard_warlord\n478=dragon_npc\n1007=zamorak_like\n1264=saradomin_like\n');
fs.writeFileSync(path.join(pack, 'seq.pack'), [
    '1=shared_seq', '2=staff_seq', '3=human_unarmedpunch', '4=human_unarmedkick',
    '5=test_attack_seq', '6=test_range_seq', '7=test_magic_seq', '8=dragon_seq',
].join('\n') + '\n');
fs.writeFileSync(path.join(pack, 'spotanim.pack'), [
    '1=arrow_launch', '2=arrow_travel', '3=spell_origin', '4=spell_projectile',
    '5=spell_target', '6=failedspell_impact', '7=firebreath_attack',
    '8=fireblast_travel', '9=fireblast_impact',
    '10=fire_impact', '11=crumble_test_impact',
    // PvP V0 special-attack spotanims (PVP-DESIGN-1026 §3.3).
    '20=sp_attack_glow_arrow_travel', '21=sp_attack_chainhit_travel_spotanim',
    '22=sp_attack_puncture_spotanim', '23=sp_attack_cleave_spotanim',
    '24=sp_attack_shatter_spotanim', '25=sp_attack_impale_spotanim',
    '26=sp_attack_shove_spotanim', '27=sp_attack_snapshot_spotanim',
    '28=sp_attack_glow_arrow_launch', '29=sp_attack_chainhit_launch_spotanim',
].join('\n') + '\n');
fs.writeFileSync(path.join(pack, 'category.pack'), '4=food\n5=potion\n10=weapon_bow\n11=weapon_staff\n12=weapon_sword\n13=weapon_2h_sword\n14=weapon_slash\n15=weapon_stab\n16=weapon_unknown\n17=weapon_unclassified\n18=weapon_crossbow\n19=weapon_thrown\n20=weapon_javelin\n21=weapon_axe\n22=arrows\n');
fs.writeFileSync(npcSource, `[test_npc]
name=Test NPC
op1=Attack
hitpoints=44
strength=24
ranged=3
category=dagannoth_mother
headicon=8
param=strengthbonus,8
param=rangebonus,4
param=attackbonus,6
param=stabdefence,-1
param=undead,^true
param=attackrate,6
param=max_dealt,3
param=poison_severity,15
param=npc_forcemulti,^true
param=attack_anim,test_attack_seq
param=magicattack_anim,test_magic_seq
param=proj_launch,arrow_launch
param=proj_travel,arrow_travel
[dragon_npc]
name=Dragon NPC
wanderrange=0
op1=Attack
hitpoints=60
param=attackrate,4
param=max_dealt,0
[zamorak_like]
name=Zamorak Like
wanderrange=6
maxrange=0
op1=Attack
hitpoints=80
[saradomin_like]
name=Saradomin Like
wanderrange=9
op1=Attack
hitpoints=120
`);
fs.writeFileSync(path.join(npcScriptDir, 'test.rs2'), `/*
[ai_applayer2,phantom]
~npc_default_attack();
*/
// [ai_opplayer2,ghost] is not a trigger
[proc,test_attack_proc]
~npc_cast_spell(^test_spell, 4);
~npc_cast_spell_with_forced_max_hit(^test_spell, 4, 12);
[ai_applayer2,test_npc]
~test_attack_proc;
~npc_default_attack();
npc_anim(^test_attack_seq);
[ai_opplayer2,test_npc]
~dagannoth_rangeattack(12, ^true);
npc_anim(^test_range_seq);
[ai_queue1,test_npc]
~npc_default_retaliate_ap();
[proc,npc_cast_spell]
return;
[ai_applayer2,saradomin_like]
~npc_cast_spell(^test_spell, 4);
[ai_opplayer2,saradomin_like]
if (~check_protect_prayer(^melee_style) = true) {
    ~npc_cast_spell(^test_spell, 4);
    return;
}
~npc_default_attack();
`);
// A global spell helper's protect check must not leak into another NPC's trigger fact.
fs.writeFileSync(path.join(npcScriptDir, 'shared.rs2'), `[proc,npc_cast_spell]
if (~check_protect_prayer(^magic_style) = true) {
    return;
}
`);
fs.writeFileSync(path.join(npcScriptDir, 'zamorak.rs2'), `[ai_applayer2,zamorak_like]
~npc_cast_spell(^test_spell, 4);
[ai_opplayer2,zamorak_like]
~npc_cast_spell(^test_spell, 4);
`);
fs.writeFileSync(path.join(npcScriptDir, 'dragon.rs2'), `%dragonresist = 1;
[ai_applayer2,dragon_npc]
~firebreath_attack();
npc_anim(^dragon_seq);
`);
const attackStylesDir = path.join(content, 'scripts/skill_combat/scripts/player');
fs.mkdirSync(attackStylesDir, { recursive: true });
fs.writeFileSync(path.join(attackStylesDir, 'player_attackstyles.rs2'), `switch_category (oc_category($obj)) {
case weapon_bow : ~.weapon_category_tab_attack(combat_bow:preview, combat_bow:name, combat_bow, $obj, null);
case weapon_staff : ~.weapon_category_tab_attack(combat_staff_2:preview, combat_staff_2:name, combat_staff_2, $obj, null);
case weapon_axe : ~.weapon_category_tab_attack(combat_axe:preview, combat_axe:name, combat_axe, $obj, null);
case weapon_2h_sword : ~.weapon_category_tab_attack(combat_heavysword:preview, combat_heavysword:name, combat_heavysword, $obj, null);
case weapon_slash : ~.weapon_category_tab_attack(combat_hacksword:preview, combat_hacksword:name, combat_hacksword, $obj, null);
case weapon_stab : ~.weapon_category_tab_attack(combat_stabsword:preview, combat_stabsword:name, combat_stabsword, $obj, null);
case weapon_unknown : ~.weapon_category_tab_attack(combat_missing:preview, combat_missing:name, combat_missing, $obj, null);
case weapon_crossbow : ~.weapon_category_tab_attack(combat_crossbow:preview, combat_crossbow:name, combat_crossbow, $obj, null);
case weapon_thrown : ~.weapon_category_tab_attack(combat_thrown:preview, combat_thrown:name, combat_thrown, $obj, null);
case weapon_javelin : ~.weapon_category_tab_attack(combat_thrown:preview, combat_thrown:name, combat_thrown, $obj, null);
case default : ~.weapon_category_tab_attack_unarmed($obj);
}
[proc,.weapon_category_tab_attack_unarmed](obj $obj)
.if_settab(combat_unarmed, 0);
`);
const modeConfigDir = path.join(content, 'scripts/skill_combat/configs');
fs.mkdirSync(modeConfigDir, { recursive: true });
fs.writeFileSync(path.join(modeConfigDir, 'combat_damagestyles.constant'), [
    '^style_melee_accurate = 0',
    '^style_melee_aggressive = 1',
    '^style_melee_defensive = 2',
    '^style_melee_controlled = 3',
    '^style_ranged_accurate = 4',
    '^style_ranged_rapid = 5',
    '^style_ranged_longrange = 6',
    '^style_magic_normal = 7',
    '^style_magic_defensive = 8',
].join('\n') + '\n');
fs.writeFileSync(path.join(modeConfigDir, 'combat_damagetypes.constant'), [
    '^damagetype_stab = 0',
    '^damagetype_slash = 1',
    '^damagetype_crush = 2',
    '^damagetype_ranged = 3',
    '^damagetype_magic = 4',
    '^damagetype_melee = 5',
].join('\n') + '\n');
fs.writeFileSync(path.join(modeConfigDir, 'combat.dbtable'), '[combat_style_table]\ncolumn=damagestyle,int,LIST\ncolumn=damagetype,int,LIST\n');
const styleRow = (name: string, styles: string[], type: string) =>
    `[${name}]\ntable=combat_style_table\n${styles.map((style) => `data=damagestyle,^${style}`).join('\n')}\n${styles.map(() => `data=damagetype,^damagetype_${type}`).join('\n')}\n`;
fs.writeFileSync(path.join(modeConfigDir, 'combat.dbrow'), [
    styleRow('weapon_2h_sword_table', ['style_melee_accurate', 'style_melee_aggressive', 'style_melee_aggressive', 'style_melee_defensive'], 'slash'),
    styleRow('weapon_slash_table', ['style_melee_accurate', 'style_melee_aggressive', 'style_melee_controlled', 'style_melee_defensive'], 'slash'),
    styleRow('weapon_unarmed_table', ['style_melee_accurate', 'style_melee_aggressive', 'style_melee_defensive'], 'crush'),
    styleRow('weapon_bow_table', ['style_ranged_accurate', 'style_ranged_rapid', 'style_ranged_longrange'], 'ranged'),
    styleRow('weapon_staff_table', ['style_magic_normal', 'style_magic_defensive'], 'magic'),
].join('\n'));
fs.writeFileSync(path.join(content, 'scripts/skill_combat/scripts/combat.rs2'), `[proc,combat_get_weapon_style_data](obj $weapon)(dbrow)
switch_category(oc_category($weapon)) {
case weapon_2h_sword : return(weapon_2h_sword_table);
case weapon_slash : return(weapon_slash_table);
case weapon_bow, weapon_crossbow, weapon_thrown, weapon_javelin : return(weapon_bow_table);
case weapon_staff : return(weapon_staff_table);
case default : return(weapon_unarmed_table);
}
[proc,combat_get_damagestyle_bonuses](int $damagestyle)(int,int,int,int)
switch_int($damagestyle) {
case ^style_melee_accurate : return(0,0,0,0);
case ^style_melee_aggressive : return(0,3,0,0);
case ^style_melee_defensive : return(0,0,3,0);
case ^style_melee_controlled : return(1,1,1,0);
case ^style_ranged_accurate : return(0,0,0,0);
case ^style_ranged_rapid : return(0,1,0,0);
case ^style_ranged_longrange : return(0,0,3,0);
case ^style_magic_normal : return(0,0,0,0);
case ^style_magic_defensive : return(0,0,3,0);
case default : return(0,0,0,0);
}
`);
fs.appendFileSync(path.join(attackStylesDir, 'player_attackstyles.rs2'), [
    '[if_button,combat_heavysword:heavy0] ~set_attackstyle(0);',
    '[if_button,combat_heavysword:heavy1] ~set_attackstyle(1);',
    '[if_button,combat_heavysword:heavy2] ~set_attackstyle(2);',
    '[if_button,combat_heavysword:heavy3] ~set_attackstyle(3);',
    '[if_button,combat_hacksword:hack0] ~set_attackstyle(0);',
    '[if_button,combat_hacksword:hack1] ~set_attackstyle(1);',
    '[if_button,combat_hacksword:hack2] ~set_attackstyle(2);',
    '[if_button,combat_hacksword:hack3] ~set_attackstyle(3);',
    '[if_button,combat_unarmed:unarmed0] ~set_attackstyle(0);',
    '[if_button,combat_unarmed:unarmed1] ~set_attackstyle(1);',
    '[if_button,combat_unarmed:unarmed2] ~set_attackstyle(2);',
    '[if_button,combat_bow:bow0] ~set_attackstyle(0);',
    '[if_button,combat_bow:bow1] ~set_attackstyle(1);',
    '[if_button,combat_bow:bow2] ~set_attackstyle(2);',
].join('\n') + '\n');
const npcTypeSource274 = engineNpcTypeSource(274);
const npcTypeSource289 = engineNpcTypeSource(289);
const combatScripts = parseCombatScripts(content);
assert.equal(combatScripts.triggers.has('phantom'), false);
assert.equal(combatScripts.triggers.has('ghost'), false);
const npcFacts274 = extractNpcNamesFacts(content, 274, npcTypeSource274, combatScripts);
const npcFacts = extractNpcNamesFacts(content, 289, npcTypeSource289, combatScripts);
assert.equal(npcFacts274.combat_facts_version, 2);
assert.equal(npcFacts.combat_facts_version, 2);
for (const [facts, expected] of [
    [npcFacts274, [['test_npc', 5, 7], ['khazard_warlord', 5, 7], ['dragon_npc', 0, 7], ['zamorak_like', 6, 0], ['saradomin_like', 9, 7]]],
    [npcFacts, [['test_npc', 5, 7], ['khazard_warlord', 5, 7], ['dragon_npc', 0, 2], ['zamorak_like', 6, 6], ['saradomin_like', 9, 11]]],
] as const) {
    for (const [config, wanderrange, maxrange] of expected) {
        const row = facts.rows.find((entry) => entry.config === config)!;
        assert.equal(row.wanderrange, wanderrange, `${config}: runtime wander default`);
        assert.equal(row.maxrange, maxrange, `${config}: runtime maxrange derivation/clamp`);
    }
}
assert.throws(() => extractNpcNamesFacts(content, 275, npcTypeSource289, combatScripts), /unsupported revision 275/);
const testNpc = npcFacts.rows.find((row) => row.config === 'test_npc')!;
assert.equal(testNpc.attackrate, 6);
assert.equal(testNpc.max_dealt, 3);
assert.equal(npcFacts.rows.find((row) => row.config === 'dragon_npc')?.max_dealt, 0);
assert.equal(npcFacts.rows.find((row) => row.config === 'khazard_warlord')?.max_dealt, 1000);
assert.equal(testNpc.headicon, 8);
assert.equal(npcFacts.rows.find((row) => row.config === 'dragon_npc')?.headicon, undefined);
assert.deepEqual(
    [
        npcFacts.rows.find((row) => row.config === 'dragon_npc')?.attack,
        npcFacts.rows.find((row) => row.config === 'dragon_npc')?.defence,
        npcFacts.rows.find((row) => row.config === 'dragon_npc')?.magic,
    ],
    [1, 1, 1],
    'omitted combat levels use the pinned NpcType defaults',
);
const dragonNpc = npcFacts.rows.find((row) => row.config === 'dragon_npc')!;
assert.deepEqual(
    [
        dragonNpc.attackbonus,
        dragonNpc.rangeattack,
        dragonNpc.magicattack,
        dragonNpc.stabdefence,
        dragonNpc.slashdefence,
        dragonNpc.crushdefence,
        dragonNpc.magicdefence,
        dragonNpc.rangedefence,
    ],
    [0, 0, 0, 0, 0, 0, 0, 0],
    'omitted NPC bonuses use the combat.param defaults',
);
assert.equal(testNpc.strength, 24);
assert.equal(testNpc.attackbonus, 6);
assert.equal(testNpc.stabdefence, -1);
assert.equal(testNpc.ranged, 3);
assert.equal(testNpc.strengthbonus, 8);
assert.equal(testNpc.rangebonus, 4);
assert.equal(testNpc.undead, 1);
assert.equal(testNpc.poison_severity, 15);
assert.equal(testNpc.forcemulti, true);
assert.equal(npcFacts.rows.find((row) => row.config === 'dragon_npc')?.poison_severity, 0);
assert.equal(npcFacts.rows.find((row) => row.config === 'dragon_npc')?.forcemulti, false);
assert.equal(testNpc.ap_attack, true);
assert.equal(testNpc.attack_kind, 'mixed');
assert.equal(testNpc.counter_protect, false);
assert.equal(testNpc.forced_max_hit, 24, 'a dagannoth double roll is one 24-damage event');
assert.equal(testNpc.bespoke, true);
assert.equal(npcFacts.rows.find((row) => row.config === 'dragon_npc')?.dragonfire, 'chromatic');
const zamorakLike = npcFacts.rows.find((row) => row.config === 'zamorak_like')!;
assert.equal(zamorakLike.attack_kind, 'magic', 'both bodies cast; an op trigger is not mixed by itself');
assert.equal(zamorakLike.counter_protect, false);
const saradominLike = npcFacts.rows.find((row) => row.config === 'saradomin_like')!;
assert.equal(saradominLike.attack_kind, 'mixed');
assert.equal(saradominLike.counter_protect, true);
fs.writeFileSync(combatParamFile, combatParams
    .replace('[strengthbonus]\ntype=int\ndefault=0', '[strengthbonus]\ntype=int\ndefault=7')
    .replace('[attackbonus]\ntype=int\ndefault=0', '[attackbonus]\ntype=int\ndefault=9'));
const changedDefaults = extractNpcNamesFacts(content, 289, npcTypeSource289, combatScripts);
assert.equal(changedDefaults.rows.find((row) => row.config === 'dragon_npc')?.strengthbonus, 7);
assert.equal(changedDefaults.rows.find((row) => row.config === 'dragon_npc')?.attackbonus, 9);
assert.equal(changedDefaults.rows.find((row) => row.config === 'test_npc')?.attackbonus, 6, 'an explicit NPC attack bonus overrides the content default');
assert.equal(changedDefaults.rows.find((row) => row.config === 'test_npc')?.strengthbonus, 8, 'an explicit NPC bonus overrides the content default');
fs.writeFileSync(combatParamFile, combatParams
    .replace('[poison_severity]\ntype=int\ndefault=0', '[poison_severity]\ntype=int\ndefault=35')
    .replace('[npc_forcemulti]\ntype=int\ndefault=0', '[npc_forcemulti]\ntype=int\ndefault=1'));
const changedPoisonDefaults = extractNpcNamesFacts(content, 289, npcTypeSource289, combatScripts);
assert.equal(changedPoisonDefaults.rows.find((row) => row.config === 'dragon_npc')?.poison_severity, 35);
assert.equal(changedPoisonDefaults.rows.find((row) => row.config === 'dragon_npc')?.forcemulti, true);
fs.writeFileSync(combatParamFile, combatParams);
const npcSourceBytes = fs.readFileSync(npcSource);
fs.writeFileSync(npcSource, npcSourceBytes.toString().replace('param=attackrate,6', 'param=attackrate,300'));
assert.throws(() => extractNpcNamesFacts(content, 289, npcTypeSource289, combatScripts), /attackrate is out of range/);
fs.writeFileSync(npcSource, npcSourceBytes);
// combat_axe is intentionally absent so a recognized but unresolved root must stay null.
fs.writeFileSync(path.join(content, 'pack/interface.pack'), `328=combat_staff_2\n349=combat_staff_2:auto_toggle\n352=combat_staff_2:auto_spell\n353=combat_staff_2:auto_choose\n900=combat_bow\n901=combat_heavysword\n902=combat_hacksword\n903=combat_stabsword\n904=combat_unarmed\n910=combat_crossbow\n911=combat_thrown\n1829=staff_spells\n1830=staff_spells:ssb0\n6575=duel_select_type\n6412=duel_confirm\n6733=duel_win\n6674=duel_select_type:accept\n6520=duel_confirm:accept\n6671=duel_select_type:otherplayer\n6684=duel_select_type:status\n6571=duel_confirm:status\n6700=duel_select_type:inv\n6701=duel_select_type:otherinv\n6500=duel_confirm:inv\n6501=duel_confirm:otherinv\n6676=duel_select_type:obstacles\n`);
const interfacePackPath = path.join(content, 'pack/interface.pack');
fs.appendFileSync(interfacePackPath, [
    '20001=combat_heavysword:heavy0',
    '20002=combat_heavysword:heavy1',
    '20003=combat_heavysword:heavy2',
    '20004=combat_heavysword:heavy3',
    '20011=combat_hacksword:hack0',
    '20012=combat_hacksword:hack1',
    '20013=combat_hacksword:hack2',
    '20014=combat_hacksword:hack3',
    '20021=combat_unarmed:unarmed0',
    '20022=combat_unarmed:unarmed1',
    '20023=combat_unarmed:unarmed2',
    '20031=combat_bow:bow0',
    '20032=combat_bow:bow1',
    '20033=combat_bow:bow2',
].join('\n') + '\n');
fs.writeFileSync(path.join(content, 'pack/varp.pack'), '43=com_mode\n');
const styleObjects = [
    { id: 50, category: 10, params: new Map([[6, 'shared_seq'], [7, 'arrow_launch'], [8, 'arrow_travel'], [9, 5], [10, 1], [11, 10]]) },
    { id: 51, category: 11, params: new Map([[3, 'staff_seq'], [9, 7]]) },
    { id: 52, category: 12, params: new Map([[3, 'shared_seq'], [9, 4]]) },
    { id: 53, category: 13, params: new Map([[3, 'shared_seq'], [9, 4]]) },
    { id: 54, category: 14, params: new Map([[3, 'shared_seq'], [9, 4]]) },
    { id: 55, category: 15, params: new Map([[3, 'shared_seq'], [9, 4]]) },
    { id: 56, category: 16, params: new Map([[3, 'shared_seq'], [9, 4]]) },
    { id: 57, category: 17, params: new Map([[3, 'shared_seq'], [9, 4]]) },
    { id: 58, category: 18, params: new Map([[3, 'shared_seq'], [9, 4], [10, 1], [11, 10]]) },
    { id: 59, category: 19, params: new Map([[3, 'shared_seq'], [9, 4], [11, 4]]) },
    { id: 60, category: 20, params: new Map([[3, 'shared_seq'], [9, 4], [10, 30], [11, 5]]) },
    { id: 61, category: 21, params: new Map([[3, 'shared_seq'], [9, 4]]) },
    { id: 62, category: 10, params: new Map([[9, 4], [10, 20], [11, 10]]) },
    { id: 63, category: 22, params: new Map([[10, 1]]) },
];
const extractStyleFacts = () => extractCombatStyleFacts(content, styleObjects, npcFacts.combatNpcs, combatScripts);
const styleFacts = extractStyleFacts();
// A pre-V0 revision omits the PvP special-attack spotanim rows.
const preV0StyleFacts = extractCombatStyleFacts(content, styleObjects, npcFacts.combatNpcs, combatScripts, false);
assert.deepEqual(
    styleFacts.style_spotanims
        .map((row) => row.spotanim_id)
        .filter((id) => !preV0StyleFacts.style_spotanims.some((row) => row.spotanim_id === id)),
    [20, 21, 22, 23, 24, 25, 26, 27, 28, 29],
    'a pre-V0 revision omits the special-attack spotanim rows',
);
const sequenceStyle = (name: string) => styleFacts.style_seqs.find((row) => row.seq_id === new Map([
    ['shared_seq', 1], ['staff_seq', 2], ['test_attack_seq', 5], ['test_range_seq', 6],
    ['test_magic_seq', 7], ['dragon_seq', 8],
]).get(name))?.style;
assert.equal(sequenceStyle('shared_seq'), 3, 'melee and ranged object facts merge into one mask');
assert.equal(sequenceStyle('staff_seq'), 1);
assert.equal(sequenceStyle('test_magic_seq'), 4);
assert.equal(sequenceStyle('dragon_seq'), 8);
const weaponMetadata = new Map(styleFacts.weapon_styles.map((row) => [
    row.obj_id, [row.obj_id, row.style, row.attackrate, row.category, row.tab] as const,
] as const));
assert.deepEqual([...weaponMetadata.values()], [
    [50, 2, 5, 1, 12], [51, 4, 7, 5, 15], [52, 1, 4, 0, 0],
    [53, 1, 4, 0, 1], [54, 1, 4, 0, 6], [55, 1, 4, 0, 9],
    [56, 1, 4, 0, null], [57, 1, 4, 0, 0], [58, 2, 4, 2, 13],
    [59, 2, 4, 3, 14], [60, 2, 4, 4, 14], [61, 1, 4, 0, null],
    [62, 2, 4, 1, 12],
]);
const weaponSlotsFor = (obj_id: number) => styleFacts.weapon_styles
    .filter((row) => row.obj_id === obj_id)
    .map((row) => [row.style_slot, row.damagetype, row.damagestyle, row.style_bonus]);
assert.equal(styleFacts.weapon_styles.length, 40, 'weapon style facts retain each content slot');
assert.deepEqual(weaponSlotsFor(50), [
    [0, 3, 4, [0, 0, 0]], [1, 3, 5, [0, 1, 0]], [2, 3, 6, [0, 0, 3]],
]);
assert.deepEqual(weaponSlotsFor(51), [
    [0, 4, 7, [0, 0, 0]], [1, 4, 8, [0, 0, 3]],
]);
assert.deepEqual(weaponSlotsFor(53), [
    [0, 1, 0, [0, 0, 0]], [1, 1, 1, [0, 3, 0]],
    [2, 1, 1, [0, 3, 0]], [3, 1, 2, [0, 0, 3]],
]);
// Old: an omitted weapon rate was always 4. New: only the declared default applies.
const defaultRateStyles = () => extractCombatStyleFacts(
    content, [{ id: 64, category: 13, params: new Map() }], npcFacts.combatNpcs, combatScripts,
).weapon_styles;
const declaredRate = '[attackrate]\ntype=int\ndefault=4';
assert.ok(combatParams.includes(declaredRate), 'weapon-rate fixture has a declared default');
try {
    fs.writeFileSync(combatParamFile, combatParams.replace(declaredRate, '[attackrate]\ntype=int\ndefault=6'));
    const rows = defaultRateStyles();
    assert.ok(rows.length > 0, 'the fixture has selected weapon style slots');
    assert.ok(rows.every((row) => row.attackrate === 6), 'omitted weapon attackrate uses declared 6, not product 4');
    for (const badDefault of ['[attackrate]\ntype=int', '[attackrate]\ntype=int\ndefault=', '[attackrate]\ntype=int\ndefault=300']) {
        fs.writeFileSync(combatParamFile, combatParams.replace(declaredRate, badDefault));
        assert.throws(defaultRateStyles, /attackrate has no valid content default/);
    }
} finally {
    fs.writeFileSync(combatParamFile, combatParams);
}

// Every revision retains the declared default and validates explicit wire rates.
fs.writeFileSync(combatParamFile, combatParams
    .replace(declaredRate, '[attackrate]\ntype=int\ndefault=6')
    + '[attackrange]\ntype=int\ndefault=0\n[levelrequire]\ntype=int\ndefault=0\n');
try {
    for (const revision of [274, 289, 377]) {
        const weapon = (rate: number | string | undefined) => ({
            id: 52,
            category: 12,
            params: new Map<number, number | string>(rate === undefined ? [] : [[9, rate]]),
        });
        const extractRate = (rate: number | string | undefined) =>
            extractCombatStyleFacts(content, [weapon(rate)], [], combatScripts, true, revision).weapon_styles[0]!.attackrate;
        assert.equal(extractRate(undefined), 6, `${revision}: selected attackrate fallback`);
        for (const rate of [0, 4, 255]) {
            assert.equal(extractRate(rate), rate, `${revision}: valid explicit attackrate ${rate}`);
        }
        assert.throws(() => extractRate('4'), /obj 52: attackrate is not numeric/, `${revision}: reject a string rate`);
        for (const rate of [-1, 256, 1.5, NaN, Infinity, -Infinity]) {
            assert.throws(
                () => extractRate(rate),
                new RegExp(`obj 52: attackrate ${String(rate).replace('.', '\\.')} is out of range`),
                `${revision}: reject invalid explicit attackrate ${rate}`,
            );
        }
    }
} finally {
    fs.writeFileSync(combatParamFile, combatParams);
}

const combatParamIds = parsePack(fs.readFileSync(path.join(pack, 'param.pack'), 'utf8'));
const itemCombatBonuses = extractItemCombatBonus(content, [
    {
        id: 70, wearpos: 1,
        params: new Map([
            [combatParamIds.get('stabattack')!, -12],
            [combatParamIds.get('slashattack')!, 8],
            [combatParamIds.get('rangeattack')!, 6],
            [combatParamIds.get('rangedefence')!, -7],
        ]),
    },
    { id: 71, wearpos: 0, params: new Map() },
    { id: 72, params: new Map([[combatParamIds.get('stabattack')!, 30]]) },
]);
assert.deepEqual(itemCombatBonuses, [
    {
        obj_id: 70, strengthbonus: 0, rangebonus: 0,
        stabattack: -12, slashattack: 8, crushattack: 0, magicattack: 0, rangeattack: 6,
        stabdefence: 0, slashdefence: 0, crushdefence: 0, magicdefence: 0, rangedefence: -7,
    },
    {
        obj_id: 71, strengthbonus: 0, rangebonus: 0,
        stabattack: 0, slashattack: 0, crushattack: 0, magicattack: 0, rangeattack: 0,
        stabdefence: 0, slashdefence: 0, crushdefence: 0, magicdefence: 0, rangedefence: 0,
    },
], 'one signed-bonus row is emitted for each wearable; non-wearable objects are skipped');
assert.throws(
    () => extractItemCombatBonus(content, [{
        id: 73, wearpos: 0,
        params: new Map([[combatParamIds.get('stabattack')!, -32769]]),
    }]),
    /out of i16 range/,
);
assert.deepEqual(styleFacts.combat_tabs.map((row) => [row.tab, row.root_id]), [
    [0, 904], [1, 901], [6, 902], [9, 903], [12, 900], [13, 910], [14, 911], [15, 328],
]);

const rangedModeRowsFor = (facts: typeof styleFacts, tab: number) => facts.ranged_modes
    .filter((row) => row.tab === tab)
    .map((row) => [row.slot, row.mode, row.button]);
assert.deepEqual(rangedModeRowsFor(styleFacts, 12), [
    [0, 0, 20031], [1, 1, 20032], [2, 2, 20033],
]);
const modeRowsFor = (facts: typeof styleFacts, tab: number) => facts.melee_modes
    .filter((row) => row.tab === tab)
    .map((row) => [row.slot, row.mode, row.button]);
assert.deepEqual(modeRowsFor(styleFacts, 1), [
    [0, 0, 20001], [1, 1, 20002], [2, 1, 20003], [3, 2, 20004],
]);
assert.deepEqual(modeRowsFor(styleFacts, 6), [
    [0, 0, 20011], [1, 1, 20012], [2, 3, 20013], [3, 2, 20014],
]);
assert.equal(modeRowsFor(styleFacts, 12).length, 0, 'ranged modes are not melee mode choices');
assert.equal(styleFacts.melee_mode_varp, 43);
assert.equal(rangedModeRowsFor(styleFacts, 1).length, 0, 'melee modes are not ranged mode choices');
assert.equal(styleFacts.ranged_mode_varp, 43);
assert.deepEqual(styleFacts.ranged_weapons.map((row) => [
    row.obj_id, row.attackrange, row.levelrequire, row.ammo_family,
]), [
    [50, 10, 1, 'ogre_arrow'], [58, 10, 1, 'bolt'], [59, 4, 0, 'thrown'],
    [60, 5, 30, 'javelin'], [62, 10, 20, 'arrow'],
]);
assert.deepEqual(styleFacts.ranged_ammo.map((row) => [row.obj_id, row.levelrequire, row.family]), [
    [63, 1, 'arrow'],
]);

const interfacePackOriginal = fs.readFileSync(interfacePackPath);
fs.writeFileSync(
    interfacePackPath,
    interfacePackOriginal.toString().replace('20001=combat_heavysword:heavy0\n', ''),
);
const missingButtonFacts = extractStyleFacts();
assert.deepEqual(modeRowsFor(missingButtonFacts, 1), [], 'an unresolved control withholds that tab mode choices');
assert.equal(missingButtonFacts.combat_tabs.some((row) => row.tab === 1), true, 'root facts remain separate from button facts');
fs.writeFileSync(
    interfacePackPath,
    interfacePackOriginal.toString().replace('901=combat_heavysword\n', ''),
);
const missingRootFacts = extractStyleFacts();
assert.deepEqual(modeRowsFor(missingRootFacts, 1), [], 'an unresolved root withholds its mode choices');
assert.equal(missingRootFacts.combat_tabs.some((row) => row.tab === 1), false);
fs.writeFileSync(interfacePackPath, interfacePackOriginal);
const launch = styleFacts.style_spotanims.find((row) => row.spotanim_id === 1);
const travel = styleFacts.style_spotanims.find((row) => row.spotanim_id === 2);
const spellTarget = styleFacts.style_spotanims.find((row) => row.spotanim_id === 5);
assert.deepEqual(launch && [launch.style, launch.where], [2, 'attacker']);
assert.deepEqual(travel && [travel.style, travel.where], [2, 'projectile']);
assert.deepEqual(spellTarget && [spellTarget.style, spellTarget.where], [4, 'on_us']);
fs.mkdirSync(path.join(content, 'pack'), { recursive: true });
fs.writeFileSync(path.join(content, 'pack/varp.pack'), `108=attackstyle_magic\n286=dueloptions\n`);
const autocast = extractAutocastControls(content);
assert.equal(autocast.staff_tab_root, 328);
assert.equal(autocast.spell_text_component, 352);
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
param=identified_herb_level,5
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
assert.deepEqual(herbs.herbs[0], { key: 'guam', name: 'Guam leaf', id: 249, unidId: 199, level: 3 });
assert.equal(herbs.herbs[0].level, 3, 'guam uses identify.param default, not cost=999');
assert.equal(herbs.herbs[1].key, 'snake weed');
assert.equal(herbs.herbs[1].level, 5, 'an explicit herbs.obj level overrides the identify.param default');

const herb377Content = fs.mkdtempSync(path.join(os.tmpdir(), 'game-data-herb377-fixture-'));
fs.cpSync(herblore, path.join(herb377Content, 'scripts/skill_herblore'), { recursive: true });
fs.mkdirSync(path.join(herb377Content, 'scripts/_unpack/377'), { recursive: true });
const [guam377Config, snake377Config] = fs.readFileSync(path.join(herblore, 'configs/herbs.obj'), 'utf8').split('[snake_weed]');
assert.ok(snake377Config);
fs.writeFileSync(path.join(herb377Content, 'scripts/skill_herblore/configs/herbs.obj'), guam377Config);
fs.writeFileSync(path.join(herb377Content, 'scripts/_unpack/377/all.obj'), `[snake_weed]${snake377Config}`);
assert.deepEqual(extractHerbFacts(herb377Content, herbItems, 377), herbs, '377 joins its unpacked quest herbs without substituting legacy levels');
fs.rmSync(path.join(herb377Content, 'scripts/_unpack/377/all.obj'));
assert.throws(() => extractHerbFacts(herb377Content, herbItems, 377), /all\.obj/, '377 requires its actual quest-herb input');
fs.rmSync(herb377Content, { recursive: true });

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

const drop377Content = fs.mkdtempSync(path.join(os.tmpdir(), 'game-data-drop377-fixture-'));
fs.cpSync(path.join(dropContent, 'scripts/drop tables'), path.join(drop377Content, 'scripts/drop_tables'), { recursive: true });
fs.mkdirSync(path.join(drop377Content, 'scripts/_unpack/377'), { recursive: true });
fs.mkdirSync(path.join(drop377Content, 'scripts/npc/configs'), { recursive: true });
fs.mkdirSync(path.join(drop377Content, 'scripts/areas/area_kalphite/configs'), { recursive: true });
const [giantDropConfigs, greenDragonDropConfig] = fs.readFileSync(path.join(npcConfigs, 'all.npc'), 'utf8').split('[green_dragon]');
assert.ok(greenDragonDropConfig);
fs.writeFileSync(path.join(drop377Content, 'scripts/_unpack/377/all.npc'), giantDropConfigs);
fs.writeFileSync(path.join(drop377Content, 'scripts/npc/configs/dragons.npc'), `[green_dragon]${greenDragonDropConfig}`);
fs.copyFileSync(path.join(kqConfigs, 'kalphite.npc'), path.join(drop377Content, 'scripts/areas/area_kalphite/configs/kalphite.npc'));
const moss377File = path.join(drop377Content, 'scripts/drop_tables/scripts/moss_giant.rs2');
const moss377Source = fs.readFileSync(moss377File, 'utf8');
const moss377Wrapped = `[ai_queue3,mossgiant] @moss_drop;\n[label,moss_drop]\n${moss377Source.replace(/^\[ai_queue3,mossgiant\]\r?\n/, '')}`;
fs.writeFileSync(moss377File, moss377Wrapped);
assert.deepEqual(extractDropFacts(drop377Content, dropItems, dropNpcs, 377), drops, '377 joins its own drop-table and split NPC sources');
fs.writeFileSync(moss377File, '[ai_queue3,mossgiant] @missing_drop;\n');
assert.throws(() => extractDropFacts(drop377Content, dropItems, dropNpcs, 377), /missing drop block label:missing_drop/, '377 cannot silently drop an unresolved label');
fs.writeFileSync(moss377File, moss377Wrapped);
fs.rmSync(path.join(drop377Content, 'scripts/npc/configs/dragons.npc'));
assert.throws(() => extractDropFacts(drop377Content, dropItems, dropNpcs, 377), /missing content input scripts\/npc\/configs\/dragons\.npc/, '377 cannot fall back to legacy dragon config');
fs.rmSync(drop377Content, { recursive: true });

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
fs.writeFileSync(path.join(prayerDir, 'configs/prayers.constant'), '^prayer_alpha = 1\n^prayer_beta = 2\n');
fs.writeFileSync(path.join(prayerDir, 'configs/prayers.dbrow'), `[alpha]
data=prayer,^prayer_alpha
data=name,Prayer Alpha
data=level,2
[beta]
data=prayer,^prayer_beta
data=name,Prayer Beta
data=level,4
`);
fs.writeFileSync(path.join(prayerDir, 'interfaces/prayer.if'), `[prayer_alpha]
script1op1=pushvar,prayer_alpha
[prayer_beta]
script1op1=pushvar,prayer_beta
`);
fs.writeFileSync(path.join(prayerContent, 'pack/interface.pack'), '501=prayer:prayer_alpha\n502=prayer:prayer_beta\n');
fs.writeFileSync(path.join(prayerContent, 'pack/varp.pack'), '41=prayer_alpha\n42=prayer_beta\n');
const prayerFacts = extractPrayerFacts(prayerContent);
assert.deepEqual(prayerFacts.prayers.map(({ name, level, button_com, varp }) => ({ name, level, button_com, varp })), [
    { name: 'Prayer Alpha', level: 2, button_com: 501, varp: 41 },
    { name: 'Prayer Beta', level: 4, button_com: 502, varp: 42 },
]);
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
assert.throws(
    () => extractPrayerFacts(prayerContent),
    /unknown prayer constant prayer_missing/,
    'prayer constant must exist in prayers.constant',
);
fs.writeFileSync(path.join(prayerDir, 'configs/prayers.dbrow'), `[only]
data=prayer,^prayer_alpha
data=name,Prayer Alpha
data=level,2
`);
const singlePrayer = extractPrayerFacts(prayerContent);
assert.equal(singlePrayer.prayers.length, 1, 'one valid extracted prayer is not rejected by a publication-count pin');
assert.equal(singlePrayer.prayers[0].button_com, 501);
fs.writeFileSync(path.join(prayerDir, 'configs/prayers.dbrow'), '');
assert.throws(() => extractPrayerFacts(prayerContent), /missing prayer rows/);

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
const toolPickaxes = [{ id: 1265, debugname: 'bronze_pickaxe', name: 'Bronze pickaxe', cost: 1, stackable: false, members: false, certlink: -1, certtemplate: -1, wearpos: -1, wearpos2: -1, wearpos3: -1 }];
const toolNurmof = { id: 594, debugname: 'nurmof', name: 'Nurmof' };
assert.throws(
    () => extractNurmofEssenceFacts(toolFlour, toolPickaxes, [toolNurmof]),
    /aubury: pack\/cache join mismatch 553\/undefined/,
    'a missing decoded Aubury must fail the pack/cache join',
);
const singleStock = extractNurmofEssenceFacts(
    toolFlour,
    toolPickaxes,
    [toolNurmof, { id: 553, debugname: 'aubury', name: 'Aubury' }],
);
assert.deepEqual(
    singleStock.pickaxes,
    [{ alias: 'bronze_pickaxe', id: 1265, name: 'Bronze pickaxe', base_cost: 1, shop_baseline_qty: 6 }],
    'one valid extracted stock row is not rejected by a publication-count pin',
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
const curated = loadEquipmentNamesCurated();

function isolatedEquipmentRoot(families: typeof curated.families = structuredClone(curated.families)) {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'game-data-equipment-isolated-'));
    fs.mkdirSync(path.join(dir, 'tools/game-data'), { recursive: true });
    fs.writeFileSync(path.join(dir, 'tools/game-data/equipment-names.curated.json'), `${JSON.stringify({ schema: curated.schema, families }, null, 2)}\n`);
    return dir;
}

const isolated = isolatedEquipmentRoot();
assert.deepEqual(loadEquipmentNamesCurated(isolated).families, curated.families);
const isolatedFacts = extractEquipmentNamesFacts(equipmentItems, equipmentPack, isolated);
assert.equal(isolatedFacts.crossbows.at(-1)?.requested_name, "Karil's crossbow");

const unknown = isolatedEquipmentRoot({
    ...curated.families,
    bows: [...curated.families.bows, 'Laser gun'],
});
const unknownFacts = extractEquipmentNamesFacts([], new Map(), unknown);
assert.equal(unknownFacts.bows.at(-1)?.requested_name, 'Laser gun');
assert.equal(unknownFacts.bows.at(-1)?.disposition, 'absent');

const curatedCopy = fs.mkdtempSync(path.join(os.tmpdir(), 'game-data-equipment-curated-'));
fs.mkdirSync(path.join(curatedCopy, 'tools/game-data'), { recursive: true });
fs.writeFileSync(path.join(curatedCopy, 'tools/game-data/equipment-names.curated.json'), JSON.stringify({
    schema: curated.schema,
    families: { ...curated.families, melee_weapons: ['Dup', 'Dup'] },
}, null, 2));
assert.throws(
    () => loadEquipmentNamesCurated(curatedCopy),
    /duplicate melee_weapons name Dup/,
    'curated duplicate family rows are malformed',
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

// revision 377 derives the same equipment rows; only the frozen-bolt note is revision-specific
const equipmentFacts377 = extractEquipmentNamesFacts(equipmentItems, equipmentPack, repoRoot, 377);
assert.deepEqual(equipmentFacts377.bows.map((row) => row.requested_name), equipmentFacts.bows.map((row) => row.requested_name));
assert.equal(equipmentFacts377.exact_name_join.bolts.note, 'Frozen bolt names are exact queries. Each row derives its selected identity or absence; generic Bolts / Bolts(p) are never substituted for a tier name.');

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
const shortbow377 = equipmentItems.find(item => item.alias === 'shortbow')!;
const ambiguous377Equipment = extractEquipmentNamesFacts(
    [...equipmentItems, { ...shortbow377, id: 19000, alias: 'quest_shortbow' }],
    new Map([...equipmentPack, ['quest_shortbow', 19000]]), undefined, 377,
).bows.find(row => row.requested_name === 'Shortbow')!;
assert.equal(ambiguous377Equipment.disposition, 'ambiguous');
assert.equal(ambiguous377Equipment.id, undefined, '377 ambiguity does not invent a selected id');
assert.equal(ambiguous377Equipment.alias, undefined, '377 ambiguity is not a silent preferred variant');
assert.deepEqual(ambiguous377Equipment.candidates?.map(row => row.alias), ['shortbow', 'quest_shortbow']);
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

assert.deepEqual(parseGenerationArgs(['--stage', '1']), { stage: 1, revisions: [274, 289] });
assert.deepEqual(parseGenerationArgs(['--revision', '289', '--stage', '2']), { stage: 2, revisions: [289] });
assert.deepEqual(parseGenerationArgs(['--stage', '2', '--revision', '377']), { stage: 2, revisions: [377] });
assert.throws(() => parseGenerationArgs([]), /usage/);
assert.throws(() => parseGenerationArgs(['--stage', '1', '--revision', '999']), /usage/);

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
^cookquest_not_started = 0
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
        'pack/npc.pack': `100=cook
`,
        'pack/loc.pack': `200=cook_start
`,
        'scripts/areas/area_test/scripts/start.rs2': `[opnpc1,cook]
if (%cookquest = ^cookquest_not_started) {
    @cook_start;
}
[label,cook_start]
%cookquest = 1;
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

function addCookColourMetadata(files: Record<string, string>) {
    files['scripts/general/scripts/quests.rs2'] += `[proc,send_quest_progress_colour](component $component, int $progress, int $complete_progress)
if ($progress = 0) {
    if_setcolour($component, ^red_rgb);
} else if ($progress >= $complete_progress) {
    if_setcolour($component, ^green_rgb);
} else {
    if_setcolour($component, ^yellow_rgb);
}
`;
    files['scripts/quests/quest_cook/configs/quest_cook.varp'] = '[cookquest]\ntransmit=no\n';
    files['scripts/quests/quest_cook/scripts/complete.rs2'] = '[oploc3,cook_finish]\nif (%cookquest = 1) { %cookquest = ^cook_complete; }\n';
}
function assertQuestRows(facts: ReturnType<typeof extractQuestIdentityFacts>) {
    const byId = Object.fromEntries(facts.rows.map((row) => [row.id, row]));
    assert.deepEqual(['cook', 'runemysteries', 'murder', 'waterfall', 'death', 'zanaris'].map((id) => byId[id]?.id), ['cook', 'runemysteries', 'murder', 'waterfall', 'death', 'zanaris']);
    assert.deepEqual(['cook', 'runemysteries', 'murder', 'waterfall', 'death', 'zanaris'].map((id) => [byId[id].display, byId[id].varp, byId[id].varp_id, byId[id].complete, byId[id].quest_points]), [
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

if (process.env.GAME_DATA_289_CONTENT) {
    const real289 = extractQuestIdentityFacts(process.env.GAME_DATA_289_CONTENT, 289);
    const hero = real289.rows.find((row) => row.id === 'hero');
    assert.ok(hero, 'the real 289 quest rows include Hero');
    assert.equal(hero.colour_authoritative, false, 'Hero keeps weak raw colour authority for the queued Grip reset');
    assert.ok(hero.completion_range, 'Hero has a full post-completion range');
    assert.ok(
        hero.completion_range.min <= 5 && hero.completion_range.max >= 5,
        'Hero ordinary range retains the queued Grip reset to 5',
    );
    assert.deepEqual(
        hero.completion_range_after_login,
        { min: 15, max: 15 },
        'Hero fresh-login range excludes the pre-completion Grip reset',
    );
}

for (const [writer, expected] of [
    ['^cook_complete', { min: 0, max: 2 }],
    ['5', { min: 0, max: 5 }],
    ['~runtime_cook_progress', null],
] as const) {
    const root = questFixture((files) => {
        files['scripts/general/configs/quest.constant'] += '^cook_unrelated_bit = 21\n';
        files['scripts/quests/quest_cook/scripts/quest_cook_progress.rs2'] = `[proc,quest_cook_progress]
%cookquest = ${writer};
%cook_bits = ^cook_unrelated_bit;
`;
    });
    try {
        const row = extractQuestIdentityFacts(root, 289).rows.find((entry) => entry.id === 'cook');
        assert.deepEqual(
            row?.stage_domain,
            expected,
            `progress domain follows its actual writer ${writer}, not another varp's bit index`,
        );
    } finally {
        fs.rmSync(root, { recursive: true, force: true });
    }
}

for (const targets of ['%cookquest, $other', '$other, %cookquest']) {
    const root = questFixture((files) => {
        files['scripts/quests/quest_cook/scripts/quest_cook_progress.rs2'] = `[proc,quest_cook_progress]
${targets} = ~two_values;
`;
    });
    try {
        const row = extractQuestIdentityFacts(root, 289).rows.find((entry) => entry.id === 'cook');
        assert.equal(row?.stage_domain, null, `an unresolved multi-output write stays unknown: ${targets}`);
    } finally {
        fs.rmSync(root, { recursive: true, force: true });
    }
}

for (const [entry, writer, maximum, authoritative] of [
    ['if (%cookquest < ^cook_complete) { @cook_revisit; }', '1', 2, true],
    ['if (%cookquest >= 1) { @cook_revisit; }', '1', 2, false],
    ['if (%cookquest >= ^cook_complete) { return; }\n@cook_revisit;', '1', 2, true],
    ['', '1', 2, false],
    ['@cook_revisit;', '^cook_complete', 2, true],
    ['if (%cookquest = 5) { @cook_revisit; }', '1', 5, false],
] as const) {
    const root = questFixture((files) => {
        addCookColourMetadata(files);
        files['scripts/quests/quest_cook/scripts/quest_cook_progress.rs2'] = `[oploc1,cook_start]
${entry}
[label,cook_revisit]
~update_questlist;
%cookquest = ${writer};
`;
        if (maximum > 2) {
            files['scripts/quests/quest_cook/scripts/quest_cook_progress.rs2'] += `[oploc2,cook_start]
if (%cookquest = 0) { %cookquest = ${maximum}; }
`;
        }
    });
    try {
        const row = extractQuestIdentityFacts(root, 289).rows.find((entry) => entry.id === 'cook');
        assert.deepEqual(row?.stage_domain, { min: 0, max: maximum }, 'post-completion reachability is independent of the value domain');
        assert.equal(
            row?.colour_authoritative,
            authoritative,
            `colour authority follows the content caller guards, not a hand-maintained varp list: ${entry} / ${writer}`,
        );
    } finally {
        fs.rmSync(root, { recursive: true, force: true });
    }
}
for (const [guard, authoritative, completionRange, afterLoginRange] of [
    [
        '%blackarmgang < ^blackarmgang_complete & %phoenixgang < ^phoenixgang_complete',
        true,
        { min: 2, max: 2 },
        { min: 2, max: 2 },
    ],
    [
        '%phoenixgang < ^phoenixgang_complete',
        false,
        { min: 1, max: 2 },
        { min: 1, max: 2 },
    ],
    [
        '%phoenixgang < ^phoenixgang_complete & npc_find(coord, cook, 10, 0) = true',
        false,
        { min: 1, max: 2 },
        { min: 1, max: 2 },
    ],
] as const) {
    const root = questFixture((files) => {
        addCookColourMetadata(files);
        files['scripts/general/configs/quest.constant'] += '^blackarmgang_complete = 12\n^phoenixgang_complete = 34\n';
        files['scripts/quests/quest_cook/scripts/requirements.rs2'] = `[proc,has_cook_quest_requirements]()(boolean)
if (%blackarmgang < ^blackarmgang_complete & %phoenixgang < ^phoenixgang_complete) {
    return(false);
}
return(true);
`;
        files['scripts/areas/area_test/scripts/prerequisite-writer.rs2'] = `[opnpc1,prerequisite_writer]
if (${guard}) {
    %cookquest = 1;
}
`;
    });
    try {
        const row = extractQuestIdentityFacts(root, 289).rows.find((entry) => entry.id === 'cook');
        assert.equal(row?.colour_authoritative, authoritative, `post-completion writer uses its full content prerequisite: ${guard}`);
        assert.deepEqual(row?.completion_range, completionRange, `post-completion range includes reachable writes: ${guard}`);
        assert.deepEqual(row?.completion_range_after_login, afterLoginRange, `fresh-login range preserves prerequisite reachability: ${guard}`);
    } finally {
        fs.rmSync(root, { recursive: true, force: true });
    }
}

for (const fixture of [
    {
        name: 'pre-completion queue',
        schedule: 'if (%cookquest < ^cook_complete) {\n    queue(reset_cook, 30, 0);\n}',
        afterLogin: { min: 2, max: 2 },
    },
    {
        name: 'post-completion queue',
        schedule: 'if (%cookquest >= ^cook_complete) {\n    queue(reset_cook, 30, 0);\n}',
        afterLogin: { min: 1, max: 2 },
    },
    {
        name: 'dynamic queue target',
        schedule: 'queue($dynamic_target, 30, 0);',
        afterLogin: { min: 1, max: 2 },
    },
]) {
    const root = questFixture((files) => {
        addCookColourMetadata(files);
        files['scripts/areas/area_test/scripts/queued-reset.rs2'] = `[opnpc1,schedule_reset]
${fixture.schedule}
[queue,reset_cook]
%cookquest = 1;
`;
    });
    try {
        const row = extractQuestIdentityFacts(root, 289).rows.find((entry) => entry.id === 'cook');
        assert.equal(row?.colour_authoritative, false, `${fixture.name}: ordinary authority retains queued writes`);
        assert.deepEqual(row?.completion_range, { min: 1, max: 2 }, `${fixture.name}: ordinary range includes pending queued resets`);
        assert.deepEqual(row?.completion_range_after_login, fixture.afterLogin, `${fixture.name}: fresh-login range uses empty-queue reachability`);
    } finally {
        fs.rmSync(root, { recursive: true, force: true });
    }
}

{
    const root = questFixture((files) => {
        addCookColourMetadata(files);
        files['scripts/general/configs/quest.constant'] += '^left_complete = 12\n^right_complete = 34\n';
        files['scripts/quests/quest_cook/scripts/requirements.rs2'] = `[proc,has_cook_quest_requirements]()(boolean)
if (%leftgang < ^left_complete & %rightgang < ^right_complete) {
    return(false);
}
return(true);
`;
        files['scripts/quests/quest_cook/scripts/progress-bounds.rs2'] = `[proc,left_progress]()
def_int $option = ~p_choice2("Choose", 1, "Other", 2);
switch_int($option) {
    case 1:
        if (%leftgang < ^left_complete) %leftgang = ^left_complete;
    case 2:
        %leftgang = 1;
}
[proc,right_progress]()
%rightgang = ^right_complete;
[opnpc1,old_stage_writer]
if (%cookquest < 2) {
    switch_int(~old_stage_choice) {
        case 1:
            if (%cookquest < 2) %cookquest = 1;
    }
}
[proc,old_stage_choice]()(int)
%cookquest = 1;
return(1);
`;
        files['scripts/areas/area_test/scripts/bounded-writer.rs2'] = `[opnpc1,bounded_writer]
if (%leftgang != ^left_complete & %rightgang < ^right_complete) {
    %cookquest = 1;
}
`;
    });
    try {
        const row = extractQuestIdentityFacts(root, 289).rows.find((entry) => entry.id === 'cook');
        assert.equal(row?.colour_authoritative, true, 'content-derived prerequisite varp domains exclude the apparent reset');
        assert.deepEqual(row?.completion_range, { min: 2, max: 2 });
    } finally {
        fs.rmSync(root, { recursive: true, force: true });
    }
}

for (const progressWriter of [
    `if (%leftgang >= ^left_complete) {
    return;
}
while (true) {
    %leftgang = add(%leftgang, 1);
}`,
    'if (true) { %leftgang = ^left_complete; } %leftgang += 1;',
]) {
    const root = questFixture((files) => {
        addCookColourMetadata(files);
        files['scripts/general/configs/quest.constant'] += '^left_complete = 12\n^right_complete = 34\n';
        files['scripts/quests/quest_cook/scripts/requirements.rs2'] = `[proc,has_cook_quest_requirements]()(boolean)
if (%leftgang < ^left_complete & %rightgang < ^right_complete) {
    return(false);
}
return(true);
`;
        files['scripts/quests/quest_cook/scripts/progress-unbounded.rs2'] = `[proc,left_progress]()
${progressWriter}
[proc,right_progress]()
%rightgang = ^right_complete;
`;
        files['scripts/areas/area_test/scripts/unbounded-writer.rs2'] = `[opnpc1,unbounded_writer]
if (%leftgang != ^left_complete & %rightgang < ^right_complete) {
    %cookquest = 1;
}
`;
    });
    try {
        const row = extractQuestIdentityFacts(root, 289).rows.find((entry) => entry.id === 'cook');
        assert.equal(row?.colour_authoritative, false, 'an unbounded or unaccounted prerequisite writer keeps the later reset possible');
        assert.deepEqual(row?.completion_range, { min: 1, max: 2 });
    } finally {
        fs.rmSync(root, { recursive: true, force: true });
    }
}

{
    const root = questFixture((files) => {
        addCookColourMetadata(files);
        files['scripts/areas/area_test/scripts/transitive-reset.rs2'] = `[opnpc1,transitive_reset]
if (%cookquest = ^cook_complete) {
    %cookquest = 1;
}
if (%cookquest = 1) {
    %cookquest = 0;
}
`;
    });
    try {
        const row = extractQuestIdentityFacts(root, 289).rows.find((entry) => entry.id === 'cook');
        assert.equal(row?.colour_authoritative, false, 'a reset reachable after the first post-completion reset keeps authority weak');
        assert.deepEqual(row?.completion_range, { min: 0, max: 2 }, 'completion range closes over transitive resets');
    } finally {
        fs.rmSync(root, { recursive: true, force: true });
    }
}

const questStarts = extractQuestStartFacts(questRoot, quest289.rows, 289);
assert.equal(questStarts.schema, 1);
assert.equal(questStarts.revision, 289);
assert.deepEqual(questStarts.rows, [{
    quest: 'cook',
    target: { kind: 'npc', id: 100 },
    op: 1,
}]);
assert.doesNotThrow(() => extractQuestIdentityFacts(questRoot, 274));

const constantJournalRoot = questFixture((files) => {
    files['scripts/player/interfaces/questlist.if'] += '[priest]\ny=6\ntext=The Restless Ghost\n';
    files['scripts/general/scripts/quests.rs2'] += '~send_quest_progress_colour(questlist:priest, %prieststart, ^priest_complete);\n';
    files['scripts/general/configs/quest.constant'] += '^priest_complete = 5\n^priest_questpoints = 1\n';
    files['pack/varp.pack'] += '107=prieststart\n';
    files['scripts/quests/quest_priest/scripts/priest_journal.rs2'] = `[if_button,questlist:priest]
def_string $x;
def_string $quest_title = "The Restless Ghost";
if (%prieststart = 0) {
    $x = "Speak to Father Aereck.";
    ~quest_journal($quest_title, $x);
    return;
}
~quest_journal($quest_title, "Quest complete.");
`;
    files['scripts/quests/quest_cook/scripts/cook_journal.rs2'] = `[if_button,questlist:cook]
~quest_journal("Cook's Assistant", "Speak to the Cook.");
`;
});
for (const revision of [274, 289]) {
    const rows = extractQuestIdentityFacts(constantJournalRoot, revision).rows;
    assert.equal(rows.find((row) => row.id === 'priest')?.journal_title, 'The Restless Ghost');
    assert.equal(rows.find((row) => row.id === 'priest')?.journal_script, 'scripts/quests/quest_priest/scripts/priest_journal.rs2');
    assert.equal(rows.find((row) => row.id === 'cook')?.journal_title, "Cook's Assistant");
}
fs.rmSync(constantJournalRoot, { recursive: true, force: true });

// Rule 1: a proc progress operand resolves to the one varp its body returns; a proc name never lands in `varp`.
const procOperandRoot = questFixture((files) => {
    files['scripts/player/interfaces/questlist.if'] += '[itexam]\ny=7\ntext=Digsite Quest\n';
    files['pack/varp.pack'] += '131=itexamlevel\n';
    files['scripts/general/configs/quest.constant'] += '^itexam_complete = 9\n^itexam_questpoints = 2\n';
    files['scripts/quests/quest_itexam/scripts/quest_itexam.rs2'] = `[proc,itexam_progress]()(int)
return (getbit_range(%itexamlevel, 0, 3));
`;
});
for (const revision of [274, 289]) {
    const facts = extractQuestIdentityFacts(procOperandRoot, revision);
    const row = facts.rows.find((entry) => entry.id === 'itexam');
    assert.deepEqual([row?.varp, row?.varp_id, row?.complete, row?.quest_points], ['itexamlevel', 131, 9, 2]);
    assert.equal(JSON.stringify(facts.rows).includes('~itexam_progress'), false, 'a proc call is never stored as the varp');
}
fs.rmSync(procOperandRoot, { recursive: true, force: true });

// A bare `%varp` return resolves the same way.
const procBareRoot = questFixture((files) => {
    files['scripts/player/interfaces/questlist.if'] += '[itexam]\ny=7\ntext=Digsite Quest\n';
    files['pack/varp.pack'] += '131=itexamlevel\n';
    files['scripts/general/configs/quest.constant'] += '^itexam_complete = 9\n';
    files['scripts/quests/quest_itexam/scripts/quest_itexam.rs2'] = `[proc,itexam_progress]()(int)
return (%itexamlevel);
`;
});
const bareProcRow = extractQuestIdentityFacts(procBareRoot, 289).rows.find((entry) => entry.id === 'itexam');
assert.deepEqual([bareProcRow?.varp, bareProcRow?.varp_id, bareProcRow?.complete], ['itexamlevel', 131, 9], 'a bare-varp return resolves');
fs.rmSync(procBareRoot, { recursive: true, force: true });

// Rule 1 refusal: anything that is not a bare `%varp` or a `getbit_range` of
// one varp stays unresolved and prints exactly one warning.
function refusedProcRow(extra: Record<string, string>) {
    const root = questFixture((files) => {
        files['scripts/player/interfaces/questlist.if'] += '[itexam]\ny=7\ntext=Digsite Quest\n';
        files['pack/varp.pack'] += '131=itexamlevel\n';
        files['scripts/general/configs/quest.constant'] += '^itexam_complete = 9\n';
        Object.assign(files, extra);
    });
    const lines: string[] = [];
    const consoleWarn = console.warn;
    console.warn = (...args: unknown[]) => { lines.push(args.map(String).join(' ')); };
    try {
        const row = extractQuestIdentityFacts(root, 289).rows.find((entry) => entry.id === 'itexam');
        assert.deepEqual([row?.varp, row?.varp_id, row?.complete], ['~itexam_progress', -1, 9], 'a refused proc keeps the operand text and the resolved complete');
        assert.equal(lines.length, 1, `one warning for the refused proc: ${lines.join(' | ')}`);
        assert.match(lines[0], /^quest_identity: itexam /);
        assert.match(lines[0], /progress operand ~itexam_progress/);
    } finally {
        console.warn = consoleWarn;
        fs.rmSync(root, { recursive: true, force: true });
    }
}
const itexamProcFile = 'scripts/quests/quest_itexam/scripts/quest_itexam.rs2';
// A computed return (`add(%foo, 1)`) is not a progress read.
refusedProcRow({ [itexamProcFile]: `[proc,itexam_progress]()(int)\nreturn (add(%itexamlevel, 1));\n` });
// Two `[proc,itexam_progress]` definitions are ambiguous.
refusedProcRow({ [itexamProcFile]: `[proc,itexam_progress]()(int)\nreturn (%itexamlevel);\n`, 'scripts/quests/quest_itexam/scripts/itexam_shadow.rs2': `[proc,itexam_progress]()(int)\nreturn (getbit_range(%itexamlevel, 0, 3));\n` });
// Two returns are not one progress read.
refusedProcRow({ [itexamProcFile]: `[proc,itexam_progress]()(int)\nif (%itexamlevel = 0) {\n    return (0);\n}\nreturn (getbit_range(%itexamlevel, 0, 3));\n` });
// One return naming two varps is not one progress read.
refusedProcRow({ [itexamProcFile]: `[proc,itexam_progress]()(int)\nreturn (add(%itexamlevel, %cogquest));\n` });

// A bit range followed by more arithmetic is not only a bit range.
refusedProcRow({ [itexamProcFile]: `[proc,itexam_progress]()(int)\nreturn (getbit_range(%itexamlevel, 0, 3) + (1));\n` });

// Rule 2: a complete constant absent from quest.constant resolves in the quest's own .constant file.
const questLocalConstantRoot = questFixture((files) => {
    files['scripts/general/scripts/quests.rs2'] += '~send_quest_progress_colour(questlist:mm, %mm_main, ^mm_complete);\n';
    files['scripts/player/interfaces/questlist.if'] += '[mm]\ny=8\ntext=Monkey Madness\n';
    files['pack/varp.pack'] += '365=mm_main\n';
    files['scripts/quests/quest_mm/configs/quest_mm.constant'] = `^mm_received_reward = 8
^mm_complete = 9
`;
});
for (const revision of [274, 289]) {
    const row = extractQuestIdentityFacts(questLocalConstantRoot, revision).rows.find((entry) => entry.id === 'mm');
    assert.deepEqual([row?.varp, row?.varp_id, row?.complete], ['mm_main', 365, 9]);
}
fs.rmSync(questLocalConstantRoot, { recursive: true, force: true });

// Rule 2 refusal: two different quest-local values for the same constant stay unresolved.
const contradictoryConstantRoot = questFixture((files) => {
    files['scripts/general/scripts/quests.rs2'] += '~send_quest_progress_colour(questlist:mm, %mm_main, ^mm_complete);\n';
    files['scripts/player/interfaces/questlist.if'] += '[mm]\ny=8\ntext=Monkey Madness\n';
    files['pack/varp.pack'] += '365=mm_main\n';
    files['scripts/quests/quest_mm/configs/quest_mm.constant'] = '^mm_complete = 9\n';
    files['scripts/quests/quest_mm/configs/quest_mm_reward.constant'] = '^mm_complete = 10\n';
});
const constantLines: string[] = [];
const constantWarn = console.warn;
console.warn = (...args: unknown[]) => { constantLines.push(args.map(String).join(' ')); };
try {
    const row = extractQuestIdentityFacts(contradictoryConstantRoot, 289).rows.find((entry) => entry.id === 'mm');
    assert.deepEqual([row?.varp, row?.varp_id, row?.complete], ['mm_main', 365, 0], 'a contradictory local constant keeps 0');
    assert.equal(constantLines.length, 1, `one warning for the contradictory constant: ${constantLines.join(' | ')}`);
    assert.match(constantLines[0], /^quest_identity: mm /);
    assert.match(constantLines[0], /complete constant \^mm_complete is not in quest\.constant or the quest's own \.constant/);
} finally {
    console.warn = constantWarn;
    fs.rmSync(contradictoryConstantRoot, { recursive: true, force: true });
}

// Rule 3: an operand that still cannot be resolved keeps today's row values and prints one warning naming the quest.
const unresolvedRoot = questFixture((files) => {
    files['scripts/player/interfaces/questlist.if'] += '[elemental_workshop]\ny=9\ntext=Elemental Workshop\n[itexam]\ny=7\ntext=Digsite Quest\n';
    files['pack/varp.pack'] += '299=elemental_workshop_bits\n';
    files['scripts/general/configs/quest.constant'] += '^itexam_complete = 9\n^itexam_questpoints = 2\n';
});
const warnings: string[] = [];
const consoleWarn = console.warn;
console.warn = (...args: unknown[]) => { warnings.push(args.map(String).join(' ')); };
const unresolvedFacts = (() => {
    try {
        return extractQuestIdentityFacts(unresolvedRoot, 289);
    } finally {
        console.warn = consoleWarn;
    }
})();
const elemental = unresolvedFacts.rows.find((entry) => entry.id === 'elemental_workshop');
assert.deepEqual([elemental?.varp, elemental?.varp_id, elemental?.complete], ['elemental_workshop_bits', 299, 0], 'a computed complete operand keeps 0');
const unresolvedItexam = unresolvedFacts.rows.find((entry) => entry.id === 'itexam');
assert.deepEqual([unresolvedItexam?.varp, unresolvedItexam?.varp_id, unresolvedItexam?.complete], ['~itexam_progress', -1, 9], 'a missing proc keeps the operand text');
assert.equal(warnings.length, 2, 'one warning line per unresolved quest');
assert.match(warnings[0], /^quest_identity: elemental_workshop /);
assert.match(warnings[0], /complete operand/);
assert.match(warnings[1], /^quest_identity: itexam /);
assert.match(warnings[1], /progress operand ~itexam_progress/);
fs.rmSync(unresolvedRoot, { recursive: true, force: true });

for (const body of [
    `[proc,unrelated]
def_string $title = "Not Cook's Assistant";
[if_button,questlist:cook]
~quest_journal($title, "");
`,
    `[if_button,questlist:cook]
def_string $title = ~runtime_title;
~quest_journal($title, "");
`,
    `[if_button,questlist:cook]
def_string $title = "Cook's Assistant";
if (%cookquest = 2) {
    $title = "A different title";
}
~quest_journal($title, "");
`,
]) {
    const root = questFixture((files) => {
        files['scripts/quests/quest_cook/scripts/cook_journal.rs2'] = body;
    });
    const row = extractQuestIdentityFacts(root, 289).rows.find((entry) => entry.id === 'cook');
    assert.equal(row?.journal_title, null, 'nonconstant or foreign-script bindings stay unknown');
    assert.equal(row?.journal_script, null);
    fs.rmSync(root, { recursive: true, force: true });
}

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
assert.equal(withExtras.rows.some((row) => row.id === 'routequest' && row.display === 'In Search of the Myreque'), true);
assert.equal(withExtras.rows.some((row) => row.id === 'misc' || row.id === 'troll_love' || row.id === 'mm'), false);
// ---- quests family serialization: only the schema and actual facts ----
{
    assert.equal(QUESTS_SCHEMA, 2, 'quests family schema includes static observation metadata');
    const draft = extractQuestsFamily(questRoot, 289);
    assert.equal(draft.schema, QUESTS_SCHEMA);
    assert.deepEqual(draft.payload, quest289, 'the family payload is the existing identity extract, unchanged');
    const payload = JSON.parse(JSON.stringify(draft.payload)) as { rows: { id: string }[]; coverage: unknown[] };
    assert.deepEqual(Object.keys(payload).sort(), ['coverage', 'rows'], 'static identity rows plus coverage only');
    const rowKeys = ['id', 'component', 'display', 'varp', 'varp_id', 'complete', 'quest_points', 'colour_authoritative', 'completion_range', 'completion_range_after_login', 'stage_domain', 'transmission', 'unknown_sides', 'requirements', 'members', 'kind', 'journal_title', 'journal_script'];
    for (const row of payload.rows) assert.deepEqual(Object.keys(row).sort(), [...rowKeys].sort(), `quest row ${row.id} carries static metadata, not per-account progress`);
    const first = familyBytes(draft, 'quests');
    const envelope = JSON.parse(first);
    assert.deepEqual(Object.keys(envelope), ['schema', 'facts']);
    assert.equal(envelope.schema, QUESTS_SCHEMA);
    assert.deepEqual(envelope.facts, quest289);
    const otherRoot = questFixture();
    try {
        assert.equal(familyBytes(extractQuestsFamily(otherRoot, 289), 'quests'), first, 'independent fixtures serialize deterministically');
    } finally {
        fs.rmSync(otherRoot, { recursive: true, force: true });
    }
    assert.throws(() => familyBytes({ schema: 0, payload: {} }, 'quests'), /invalid extractor schema/);
}



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
const trailFixtureItems: Parameters<typeof extractTrailFacts>[1] = Object.entries(TRAIL_FIXTURE_IDS).map(([debugname, id]) => ({
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
    manwear: -1,
}));

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
const expectedTrailChallengeAnswers = [
    { alias: 'trail_clue_medium_anagram001_challenge', id: 2842, answer: '6859' },
    { alias: 'trail_clue_medium_anagram002_challenge', id: 2844, answer: '9' },
    { alias: 'trail_clue_medium_anagram003_challenge', id: 2846, answer: '40' },
    { alias: 'trail_clue_medium_anagram006_challenge', id: 2850, answer: '5' },
    { alias: 'trail_clue_medium_anagram007_challenge', id: 2852, answer: '48' },
    { alias: 'trail_clue_medium_anagram008_challenge', id: 2854, answer: '5096' },
];
assert.deepEqual(trailFacts.challenge_answers, expectedTrailChallengeAnswers);
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

const repackedConstrainedId = 9001;
const repackedTrailFacts = extractTrailFacts(trailFixture((files) => {
    files['pack/obj.pack'] = files['pack/obj.pack'].replace('3554=trail_clue_hard_sextant028\n', `${repackedConstrainedId}=trail_clue_hard_sextant028\n`);
}), trailFixtureItems.map((item: Parameters<typeof extractTrailFacts>[1][number]) =>
    Object.assign({}, item, item.debugname === 'trail_clue_hard_sextant028' ? { id: repackedConstrainedId } : {})));
assert.equal(trailRow(repackedTrailFacts, 'trail_clue_hard_sextant028').id, repackedConstrainedId);
assert.equal(trailRow(repackedTrailFacts, 'trail_clue_hard_sextant028').access, 'constrained');

for (const revision of [274, 289] as const) {
    const withoutConstrained = extractTrailFacts(trailFixture((files) => {
        files[`${TRAIL_CONFIG}/trail_hard.enum`] =
            files[`${TRAIL_CONFIG}/trail_hard.enum`].replace('val=4,trail_clue_hard_sextant028\n', '');
    }), trailFixtureItems, revision);
    assert.equal(withoutConstrained.rows.some((row) => row.access !== undefined), false,
        `${revision}: an absent constrained clue must not reject the corpus`);

    const repeatedConstrained = extractTrailFacts(trailFixture((files) => {
        files[`${TRAIL_CONFIG}/trail_hard.enum`] += 'val=5,trail_clue_hard_sextant028\n';
    }), trailFixtureItems, revision);
    assert.deepEqual(repeatedConstrained.rows.filter((row) => row.access !== undefined)
        .map((row) => row.access), ['constrained', 'constrained'],
        `${revision}: annotate every matching membership row without a count pin`);
}

// revision 377 derives the same membership but never inherits the 289 constrained-clue annotation
const selectedTrailAnswerParam = 5;
const selectedTrailAnswers = new Map(expectedTrailChallengeAnswers.map(({ alias, answer }) => [alias, Number(answer)]));
const selectedTrailItems = trailFixtureItems.map((item) => {
    const answer = selectedTrailAnswers.get(item.debugname!);
    return { ...item, params: answer === undefined ? undefined : new Map([[selectedTrailAnswerParam, answer]]) };
});
const selectedTrailContent = trailFixture((files) => {
    files['pack/param.pack'] = `${selectedTrailAnswerParam}=trail_challenge_answer\n`;
    files['scripts/_unpack/377/all.obj'] = ['easy', 'medium', 'hard', 'casket']
        .map((tier) => files[`${TRAIL_CONFIG}/trail_${tier}.obj`]).join('\n');
});
const trailFacts377 = extractTrailFacts(selectedTrailContent, selectedTrailItems, 377);
assert.equal(trailFacts377.rows.some((row) => 'access' in row), false);
assert.deepEqual(trailFacts377.rows.map((row) => row.alias).sort(), trailFacts.rows.map((row) => row.alias).sort());
assert.deepEqual(trailFacts377.challenge_answers, expectedTrailChallengeAnswers);
fs.rmSync(selectedTrailContent, { recursive: true });

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

// Removing a source object block removes its derived casket row; there is no corpus-count pin.
const droppedCasketFixture = extractTrailFacts(trailFixture((files) => {
    files[`${TRAIL_CONFIG}/trail_hard.obj`] = (files[`${TRAIL_CONFIG}/trail_hard.obj`] as string).replace('param=trail_casket,trail_clue_hard_riddle004_casket\n', '');
}), trailFixtureItems);
assert.equal(droppedCasketFixture.rows.length, trailFacts.rows.length - 1);
assert.equal(droppedCasketFixture.rows.some((row) => row.alias === 'trail_clue_hard_riddle004_casket'), false);

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

const talkKeyFacts = extractTalkKeyFacts(talkKeyFixture(), talkKeyItems);

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
assert.throws(() => extractTalkKeyFacts(truncatedTalkKeyMaps, talkKeyItems), /npc 0 has no jm2 NPC spawn/);
const truncatedTalkKeyConfigs = talkKeyFixture();
fs.rmSync(path.join(truncatedTalkKeyConfigs, 'scripts/areas/area_lumbridge/configs/lumbridge.npc'));
assert.throws(() => extractTalkKeyFacts(truncatedTalkKeyConfigs, talkKeyItems), /missing \[hans\]/);


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
    return rootDir;
}
function trioGiverRow(facts: TrioGiverFacts, alias: string) {
    const found = facts.rows.find((row) => row.alias === alias);
    assert.ok(found, `missing giver ${alias}`);
    return found;
}

const trioGiverFacts = extractTrioGiversFacts(trioGiverFixture());

const trioGiver377Root = trioGiverFixture((files) => {
    files['scripts/_unpack/377/all.npc'] = Object.values(TRIO_GIVER_CONFIGS).map(file => files[file]).join('\n');
    for (const file of Object.values(TRIO_GIVER_CONFIGS)) delete files[file];
});
const trioGiver377 = extractTrioGiversFacts(trioGiver377Root, 377);
assert.deepEqual(trioGiver377, trioGiverFacts, '377 joins its actual unpacked NPC configs without legacy per-area paths');
fs.rmSync(path.join(trioGiver377Root, 'scripts/_unpack/377/all.npc'));
assert.throws(() => extractTrioGiversFacts(trioGiver377Root, 377), /all\.npc: required file missing/);
fs.rmSync(trioGiver377Root, { recursive: true });

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
assert.deepEqual(multiSpawnTrioGivers.rows[1], { alias: 'murphy', id: 463, name: 'Murphy' });
assert.deepEqual(multiSpawnTrioGivers.coverage, [{ class: 'unknown', family: 'trio_givers', alias: 'murphy', reason: 'non-unique jm2 NPC spawn' }]);
assert.equal(JSON.stringify(multiSpawnTrioGivers).includes('"spawn":null'), false);

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
assert.throws(() => extractTrioGiversFacts(truncatedTrioGiverMaps), /brother_kojo npc 223 has no jm2 NPC spawn/);


// ---- gathering family: local source grammar and synthetic placements ----

const hemensterFishingScript = 'scripts/quests/quest_fishingcompo/scripts/hemenster_fishing.rs2';
const hemensterSpot = '0_41_53_sinisterfishspot';
const hemensterCarpMethod = `fishing.${hemensterSpot}.op1`;

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
/** Resource rock names the assertions are about: newer content adds unnamed quest veins (`loc_<id>`, Haunted Mine) to every ore row. */
const namedRocks = (view: { name(kind: string, id: number): string; method(id: string): MethodWire }, id: string) => targetsOf(view.method(id), 'resource')
    .map((target) => view.name('loc', target.id)).filter((name) => !/^loc_\d+$/.test(name));
function listContentFiles(root: string, dir: string, extension: string, out: string[] = []) {
    for (const entry of fs.readdirSync(path.join(root, dir), { withFileTypes: true }).sort((a, b) => (a.name < b.name ? -1 : 1))) {
        if (entry.isDirectory()) listContentFiles(root, `${dir}/${entry.name}`, extension, out);
        else if (entry.name.endsWith(extension)) out.push(`${dir}/${entry.name}`);
    }
    return out;
}


// Real script text with synthetic maps: mutate one construct and prove ordinary extractor behavior.
const realGathering = revisions.find((spec) => spec.revision === 289)!.content;
function gatheringFixture(mutate?: (rootDir: string) => void) {
    const rootDir = fs.mkdtempSync(path.join(os.tmpdir(), 'gathering-fixture-'));
    fs.cpSync(path.join(realGathering, 'scripts'), path.join(rootDir, 'scripts'), { recursive: true });
    fs.mkdirSync(path.join(rootDir, 'pack'));
    for (const pack of ['loc', 'npc', 'obj', 'seq']) fs.copyFileSync(path.join(realGathering, `pack/${pack}.pack`), path.join(rootDir, `pack/${pack}.pack`));
    const loc = parsePack(fs.readFileSync(path.join(rootDir, 'pack/loc.pack'), 'utf8'));
    const npc = parsePack(fs.readFileSync(path.join(rootDir, 'pack/npc.pack'), 'utf8'));
    fs.mkdirSync(path.join(rootDir, 'maps'));
    fs.writeFileSync(path.join(rootDir, 'maps/m50_50.jm2'), `==== LOC ====\n0 10 10: ${loc.get('ironrock1')} 10 0\n0 11 10: ${loc.get('ironrock2')} 10 1\n0 12 10: ${loc.get('copperrock1')} 10 0\n==== NPC ====\n0 20 20: ${npc.get('0_48_53_freshfish')}\n`);
    mutate?.(rootDir);
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
const gatheringEnvelope = JSON.parse(familyBytes(baseline, 'gathering'));
assert.deepEqual(Object.keys(gatheringEnvelope), ['schema', 'facts']);
assert.equal(gatheringEnvelope.schema, GATHERING_SCHEMA);
assert.deepEqual(gatheringEnvelope.facts, baseline.payload, 'family serialization keeps semantic evidence in actual facts');
assert.equal(family289Rows(baseline, 'mining.iron'), 2, 'the fixture places exactly its two iron rocks');
function family289Rows(family: GatheringFamily, method: string) {
    return family.summary.placements.by_method[method] ?? 0;
}
assert.equal(family289Rows(baseline, 'mining.copper'), 1);
assert.equal(family289Rows(baseline, 'fishing.freshfish.op1'), 1);
assert.equal(family289Rows(baseline, 'fishing.freshfish.op3'), 1);
assert.equal(baseline.payload.placements.length, 1);
assert.equal(baseline.payload.methods.some((method) => method.id === hemensterCarpMethod), true, 'the anchored direct-yield extractor admits the selected quest handler');

const woodcutScript = 'scripts/skill_woodcutting/scripts/woodcut.rs2';
function structWoodcuttingFixture(mutate?: (rootDir: string) => void) {
    return gatheringFixture((rootDir) => {
        const script = path.join(rootDir, woodcutScript);
        const before = fs.readFileSync(script, 'utf8');
        const after = before.replaceAll('anim(oc_param($axe, woodcutting_anim), 0)', 'anim(struct_param(oc_param($axe, woodcutting_struct), skill_anim), 0)');
        assert.notEqual(after, before, 'the selected woodcutting loop uses its tool animation');
        fs.writeFileSync(script, after);
        const structs: string[] = [];
        for (const relative of listContentFiles(rootDir, 'scripts', '.obj')) {
            const file = path.join(rootDir, relative);
            const source = fs.readFileSync(file, 'utf8');
            const changed = source.replace(/^param=woodcutting_anim,([^\r\n]+)$/gm, (_line, animation: string) => {
                const alias = `fixture_${animation}`;
                structs.push(`[${alias}]\nparam=skill_anim,${animation}\n`);
                return `param=woodcutting_struct,${alias}`;
            });
            if (changed !== source) fs.writeFileSync(file, changed);
        }
        fs.writeFileSync(path.join(rootDir, 'scripts/skill_woodcutting/configs/fixture.struct'), structs.join('\n'));
        mutate?.(rootDir);
    });
}
assert.deepEqual(extractGatheringFamily(structWoodcuttingFixture(), 377).payload, baseline.payload,
    '377 resolves each axe animation through its selected struct without changing the gathering facts');
assert.throws(() => extractGatheringFamily(structWoodcuttingFixture()), /unrecognized loop/,
    'historical revisions do not admit the 377 woodcutting shape');
assert.throws(() => extractGatheringFamily(structWoodcuttingFixture((rootDir) => {
    fs.rmSync(path.join(rootDir, 'scripts/skill_woodcutting/configs/fixture.struct'));
}), 377), /expected one skill_anim parameter/,
    '377 cannot substitute a legacy animation when the selected tool struct is missing');

const mixedIron = extractGatheringFamily(structWoodcuttingFixture((rootDir) => {
    replaceIn(rootDir, 'scripts/skill_mining/configs/rocks.loc',
        'recol1d=6242\ncategory=mining_rock_fast\nparam=next_loc_stage_mining,rocks1',
        'recol1d=6242\ncategory=mining_rock_normal\nparam=next_loc_stage_mining,rocks1');
}), 377);
const ironTargetIds = (family: GatheringFamily) => family.payload.methods.filter((method) => method.resources.includes('iron'))
    .flatMap((method) => targetsOf(method, 'resource').map((target) => target.id)).sort((a, b) => a - b);
const mixedIronMethods = mixedIron.payload.methods.filter((method) => method.resources.includes('iron'));
assert.deepEqual(mixedIronMethods.map((method) => [method.id, method.tick_arm.delay_drop]).sort(),
    [['mining.iron', true], ['mining.iron.get_ore_fast', false]]);
assert.deepEqual(ironTargetIds(mixedIron), ironTargetIds(baseline),
    'each incompatible 377 iron target stays available with exactly its own tick timing');

// Revision 289 splits a row by real handler too (the Haunted Mine iron rocks run mining_rock_normal inside the
// iron row that regular iron runs as mining_rock_fast): one method per handler, each target on its own handler.
const mixedIron289 = extractGatheringFamily(gatheringFixture((rootDir) => {
    replaceIn(rootDir, 'scripts/skill_mining/configs/rocks.loc',
        'recol1d=6242\ncategory=mining_rock_fast\nparam=next_loc_stage_mining,rocks1',
        'recol1d=6242\ncategory=mining_rock_normal\nparam=next_loc_stage_mining,rocks1');
}));
const mixedIron289View = gatherView(mixedIron289);
const mixedIron289Methods = mixedIron289.payload.methods.filter((method) => method.resources.includes('iron'));
assert.deepEqual(mixedIron289Methods.map((method) => [method.id, method.tick_arm.delay_drop, method.tick_arm.depletes]).sort(),
    [['mining.iron', true, true], ['mining.iron.get_ore_fast', false, true]], '289 gives each iron handler its own method');
const iron289Names = (id: string) => targetsOf(mixedIron289View.method(id), 'resource').map((target) => mixedIron289View.name('loc', target.id));
assert.ok(iron289Names('mining.iron').includes('ironrock1'), 'the rock whose category is mining_rock_normal runs the delayed method');
assert.ok(iron289Names('mining.iron.get_ore_fast').includes('ironrock2'), 'the rock whose category is mining_rock_fast runs the immediate method');
assert.ok(!iron289Names('mining.iron').includes('ironrock2') && !iron289Names('mining.iron.get_ore_fast').includes('ironrock1'), 'no rock appears under another handler');
assert.deepEqual(ironTargetIds(mixedIron289), ironTargetIds(baseline), 'splitting by handler neither adds nor drops an iron target');
assert.deepEqual(gatherResources(mixedIron289.payload, new Map([[440, 'Iron ore']])).filter((row) => row.key === 'iron')
    .map((row) => [row.method, row.methods, row.selectable]), [['mining.iron', ['mining.iron', 'mining.iron.get_ore_fast'], true]],
    'the iron resource row keeps both methods under one selectable option');
assert.deepEqual(mixedIron289.payload.methods.filter((method) => method.resources.includes('copper')).map((method) => method.id), ['mining.copper'],
    'an unmixed row stays one method');


function renameWaterfallSpot(rootDir: string) {
    for (const relative of ['pack/loc.pack', 'scripts/skill_fishing/scripts/fishing_spots/waterfall.rs2',
        ...listContentFiles(rootDir, 'scripts', '.loc')]) {
        const file = path.join(rootDir, relative);
        const before = fs.readFileSync(file, 'utf8');
        const after = before.replace(/\bloc_2027\b/g, 'fishing_spot_2027');
        if (after !== before) fs.writeFileSync(file, after);
    }
}
const namedWaterfall = extractGatheringFamily(structWoodcuttingFixture(renameWaterfallSpot), 377);
for (const slot of [1, 2]) {
    assert.deepEqual(namedWaterfall.payload.methods.find((method) => method.id === `fishing.fishing_spot_2027.op${slot}`)?.tick_arm,
        baseline.payload.methods.find((method) => method.id === `fishing.loc_2027.op${slot}`)!.tick_arm,
        '377 named waterfall entries retain source-checked timing and roll blocks');
}
assert.throws(() => extractGatheringFamily(gatheringFixture(renameWaterfallSpot)), /no reviewed fishing tick specification/,
    'historical revisions do not admit the 377 fishing entry identity');
const waterfallScript = 'scripts/skill_fishing/scripts/fishing_spots/waterfall.rs2';
for (const [from, to] of [
    ['p_oploc(2);\n        return;', 'p_oploc(2);'],
    ['p_oploc(2);\n        return;', 'p_oploc(1);\n        return;'],
    ['if (random(5) = 0)', 'if (random(6) = 0)'],
]) {
    assert.throws(
        () => extractGatheringFamily(gatheringFixture((rootDir) => replaceIn(rootDir, waterfallScript, from, to))),
        /waterfall recast no longer suppresses one roll animation/,
        'the Sometimes roll signal requires the authored early return and recast operation',
    );
}

const changedContestYield = extractGatheringFamily(gatheringFixture((rootDir) =>
    replaceIn(rootDir, hemensterFishingScript, 'inv_add(inv, raw_giant_carp, 1);', 'inv_add(inv, raw_sardine, 1);')));
assert.equal(changedContestYield.payload.methods.some((method) => method.id === hemensterCarpMethod), false, 'a changed direct yield is not promoted');
const changedContestLoose = changedContestYield.payload.loose.find((each) => each.gap?.code === 'quest-handler-shape');
assert.ok(changedContestLoose);
assert.equal(gatherView(changedContestYield).name('npc', changedContestLoose.id), hemensterSpot);
const missingContestSource = extractGatheringFamily(gatheringFixture((rootDir) =>
    fs.rmSync(path.join(rootDir, hemensterFishingScript))));
assert.equal(missingContestSource.payload.methods.some((method) => method.id === hemensterCarpMethod), false, 'a missing quest handler is not promoted');
const missingContestView = gatherView(missingContestSource);
const missingContestLoose = missingContestSource.payload.loose.find((each) =>
    each.gap?.code === 'no-handler' && missingContestView.name('npc', each.id) === hemensterSpot);
assert.ok(missingContestLoose);
assert.equal(missingContestView.name('npc', missingContestLoose.id), hemensterSpot);

const sourceNpcPack = parsePack(fs.readFileSync(path.join(realGathering, 'pack/npc.pack'), 'utf8'));
const sourceObjPack = parsePack(fs.readFileSync(path.join(realGathering, 'pack/obj.pack'), 'utf8'));
const treeRows = parseDbRows('scripts/skill_woodcutting/configs/trees.dbrow', fs.readFileSync(path.join(realGathering, 'scripts/skill_woodcutting/configs/trees.dbrow'), 'utf8'))
    .filter((row) => row.table === 'woodcutting_trees');
const treeAliases = new Set(treeRows.flatMap((row) => (row.data.get('tree') ?? []).map((data) => data.values[0])));
const treeConfigFiles = listContentFiles(realGathering, 'scripts/skill_woodcutting/configs/trees', '.loc');
const treeSections = treeConfigFiles.flatMap((file) => parseSections(file, fs.readFileSync(path.join(realGathering, file), 'utf8')));
const treeEntSections = treeSections.filter((section) => treeAliases.has(section.name) && section.params.has('ent') && section.params.get('ent')?.[0] !== 'null');
const treeEntSection = treeEntSections[0];
assert.ok(treeEntSection, 'a selected tree loc publishes its content ent param');
const treeEntAlias = treeEntSection.params.get('ent')![0];
const treeEntId = sourceNpcPack.get(treeEntAlias);
assert.ok(treeEntId !== undefined, 'tree ent aliases join through npc.pack');
assert.ok(baseline.payload.hazard_npcs.includes(treeEntId), 'tree ent ids are in the family hazard row');
const baselineHazardIds = new Set(baseline.payload.hazard_npcs);
const replacementNpc = [...sourceNpcPack].find(([, id]) => !baselineHazardIds.has(id));
assert.ok(replacementNpc, 'the source pack has an NPC type not already classified as a hazard');
const changedTreeEnt = extractGatheringFamily(gatheringFixture((rootDir) => {
    const lines = fs.readFileSync(path.join(rootDir, treeEntSection.span.file), 'utf8').split(/\r?\n/);
    const at = lines.findIndex((line, index) => index >= treeEntSection.span.first - 1 && index < treeEntSection.span.last && line.startsWith('param=ent,'));
    assert.ok(at >= 0, 'the selected tree section still has its ent param');
    lines[at] = `param=ent,${replacementNpc[0]}`;
    fs.writeFileSync(path.join(rootDir, treeEntSection.span.file), lines.join('\n'));
}));
assert.ok(changedTreeEnt.payload.hazard_npcs.includes(replacementNpc[1]), 'changing a tree ent param changes the published NPC ids');

const treeEntAliases = new Set(treeEntSections.map((section) => section.params.get('ent')![0]));
const fishingNpcSections = parseSections('scripts/skill_fishing/configs/fishing.npc', fs.readFileSync(path.join(realGathering, 'scripts/skill_fishing/configs/fishing.npc'), 'utf8'));
const whirlpoolLinks = new Set(fishingNpcSections.flatMap((section) => section.params.get('whirlpool') ?? []));
const whirlpoolConfig = listContentFiles(realGathering, 'scripts', '.npc')
    .flatMap((file) => parseSections(file, fs.readFileSync(path.join(realGathering, file), 'utf8')))
    .find((section) => whirlpoolLinks.has(section.name) && section.params.get('is_whirlpool')?.[0] === '^true' && !treeEntAliases.has(section.name));
assert.ok(whirlpoolConfig, 'a fishing spot links to an NPC carrying is_whirlpool');
const whirlpoolId = sourceNpcPack.get(whirlpoolConfig.name);
assert.ok(whirlpoolId !== undefined && baseline.payload.hazard_npcs.includes(whirlpoolId), 'the linked whirlpool NPC is published');
const noWhirlpool = extractGatheringFamily(gatheringFixture((rootDir) => {
    const lines = fs.readFileSync(path.join(rootDir, whirlpoolConfig.span.file), 'utf8').split(/\r?\n/);
    const at = lines.findIndex((line, index) => index >= whirlpoolConfig.span.first - 1 && index < whirlpoolConfig.span.last && line === 'param=is_whirlpool,^true');
    assert.ok(at >= 0, 'the linked NPC still carries its whirlpool flag');
    lines[at] = 'param=is_whirlpool,^false';
    fs.writeFileSync(path.join(rootDir, whirlpoolConfig.span.file), lines.join('\n'));
}));
assert.equal(noWhirlpool.payload.hazard_npcs.includes(whirlpoolId), false, 'a fishing NPC without is_whirlpool is not a hazard');

const baselineGemIds = baseline.payload.incidental_gem_ids;
assert.ok(baselineGemIds.length > 0, 'the mining gem table has named item outputs');
assert.deepEqual(baselineGemIds, [...new Set(baselineGemIds)].sort((a, b) => a - b), 'gem ids are sorted and unique');
const gemScript = fs.readFileSync(path.join(realGathering, mineScript), 'utf8');
const gemProcAt = gemScript.indexOf('[proc,mining_gem_table]');
assert.ok(gemProcAt >= 0, 'the mining gem proc is present');
const gemProc = gemScript.slice(gemProcAt);
const firstGemOutput = /\breturn\s*\(\s*([A-Za-z0-9_]+)\s*\)\s*;/.exec(gemProc);
assert.ok(firstGemOutput, 'the mining gem proc returns a named item');
const replacementItem = [...sourceObjPack].find(([, id]) => !baselineGemIds.includes(id));
assert.ok(replacementItem, 'the source pack has an object id not already in the gem table');
const changedGemTable = extractGatheringFamily(gatheringFixture((rootDir) => {
    const file = path.join(rootDir, mineScript);
    const source = fs.readFileSync(file, 'utf8');
    const start = source.indexOf('[proc,mining_gem_table]');
    const changedProc = source.slice(start).replace(firstGemOutput[0], firstGemOutput[0].replace(firstGemOutput[1], replacementItem[0]));
    assert.notEqual(changedProc, source.slice(start), 'the gem table return mutation applies');
    fs.writeFileSync(file, source.slice(0, start) + changedProc);
}));
assert.ok(changedGemTable.payload.incidental_gem_ids.includes(replacementItem[1]), 'the selected mining_gem_table return controls the published object ids');

// A drifted fast-handler yield affects only that method; the normal iron method stays separate.
const drifted = extractGatheringFamily(gatheringFixture((rootDir) => replaceIn(rootDir, mineScript,
    'inv_add(inv, db_getfield($data, mining_table:rock_output, 0), 1);',
    'inv_add(inv, db_getfield($data, mining_table:rock_output, 0), 2);')));
const driftedView = gatherView(drifted);
assert.equal(drifted.payload.methods.find((each) => each.id === 'mining.iron')?.tick_arm.delay_drop, true,
    'the unchanged normal handler stays separate');
assert.equal(drifted.payload.methods.some((each) => each.id === 'mining.iron.get_ore_fast'), false,
    'a drifted fast handler is not promoted');
assert.equal(drifted.payload.loose.some((each) => driftedView.name('loc', each.id) === 'ironrock1' && each.gap?.code === 'mining-handler-unrecognized'), true,
    'the changed fast-handler rock is unclassified');
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
assert.ok(noRateRespawn.state === 'unknown' && noRateRespawn.gap.sources.some((source) => source.startsWith('scripts/skill_mining/configs/mine.dbrow:')), 'respawn gap cites the missing-rate row');

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
const carpMethod = baselineView.method(hemensterCarpMethod);
assert.ok(carpMethod.sources.some((source) => source.startsWith(`${hemensterFishingScript}:`)), 'method source citations remain in family facts');
const carpRequirements = knownValue(carpMethod.requirements, `${carpMethod.id} requirements`);
assert.ok(carpRequirements.some((requirement) => requirement.kind === 'skill' && requirement.source.startsWith(`${hemensterFishingScript}:`)), 'requirement retains its source span');
assert.ok(carpMethod.requirements.state === 'partial' && carpMethod.requirements.gaps.some((each) => each.code === 'varp-gate' && each.sources.some((source) => source.startsWith(`${hemensterFishingScript}:`))), 'semantic requirement gaps retain source spans');

const copperRespawnEvidence = knownValue(targetsOf(baselineView.method('mining.copper'), 'resource')[0]!.respawn, 'copper respawn evidence');
assert.ok(copperRespawnEvidence, 'depleting copper target retains its respawn fact');
assert.match(copperRespawnEvidence.source, /^scripts\/skill_mining\/configs\/mine\.dbrow:\d+-\d+$/, 'respawn fact cites its source row');
const copperScaleEvidence = knownValue(copperRespawnEvidence.scale, 'copper respawn scale');
assert.ok(copperScaleEvidence.sources.some((source) => source.startsWith('scripts/general/scripts/player_count.rs2:')), 'respawn scaling cites its source formula');

assert.ok(baseline.payload.zones.length > 0, 'content-defined gathering zones are extracted');
assert.ok(baseline.payload.zones.every((zone) => zone.sources.length > 0), 'every gathering zone retains semantic source citations');
const freshfishEntity = baseline.payload.entities.find((entity) => entity.endsWith(' 0_48_53_freshfish'));
assert.ok(freshfishEntity, 'the packed freshfish entity fact is retained');
const freshfishId = Number(freshfishEntity.split(' ')[1]);
assert.ok(baseline.payload.placements.some((file) => file.file === 'maps/m50_50.jm2' && file.rows.some((row) => row.split(' ')[4] === `n${freshfishId}`)), 'the synthetic NPC placement row is retained with its entity fact');
assert.deepEqual(namedRocks(baselineView, 'mining.copper'), ['copperrock1', 'copperrock2', 'newbiecopperrock']);
assert.equal(baselineView.method('mining.copper').targets.state, 'partial');
const retyped = extractGatheringFamily(gatheringFixture((rootDir) => replaceIn(rootDir, tutScript, 'inv_add(inv, copper_ore, 1);', 'inv_add(inv, logs, 1);')));
const retypedView = gatherView(retyped);
assert.deepEqual(namedRocks(retypedView, 'mining.copper'), ['copperrock1', 'copperrock2']);
assert.equal(retyped.payload.loose.find((each) => retypedView.name('loc', each.id) === 'newbiecopperrock')?.gap?.code, 'custom-handler');
assert.equal(family289Rows(retyped, 'mining.copper'), 1, 'the tutorial rock contributes no fixture placements');
// M-215 R2: promotion is fail-closed on yield cardinality. A second grant beside the unit
// copper yield — even a repeated identical one or a non-unit extra — must drop the rock back
// to Unknown instead of promoting it as a copper resource.
const doubled = extractGatheringFamily(gatheringFixture((rootDir) => replaceIn(rootDir, tutScript, 'inv_add(inv, copper_ore, 1);', 'inv_add(inv, copper_ore, 1);\ninv_add(inv, copper_ore, 1);')));
const doubledView = gatherView(doubled);
assert.deepEqual(namedRocks(doubledView, 'mining.copper'), ['copperrock1', 'copperrock2']);
assert.equal(doubled.payload.loose.find((each) => doubledView.name('loc', each.id) === 'newbiecopperrock')?.gap?.code, 'custom-handler');
const extraGrant = extractGatheringFamily(gatheringFixture((rootDir) => replaceIn(rootDir, tutScript, 'inv_add(inv, copper_ore, 1);', 'inv_add(inv, copper_ore, 1);\ninv_add(inv, tin_ore, 2);')));
const extraView = gatherView(extraGrant);
assert.deepEqual(namedRocks(extraView, 'mining.copper'), ['copperrock1', 'copperrock2']);
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





// The debug artifact shape: schema 1, no base item name duplication, strict bytes.
const debugBuilt = buildDebugArtifact(289, { commands: [{ name: '~extra' }], names: { npc: [{ id: 1, alias: 'man', name: 'Man' }] } });
assert.equal(debugBuilt.schema_version, DEBUG_SCHEMA_VERSION);
assert.equal(debugBuilt.revision, 289);
assert.deepEqual(JSON.parse(debugArtifactBytes(debugBuilt)), { schema_version: 1, revision: 289, debug_commands: [{ name: '~extra' }], debug_names: { npc: [{ id: 1, alias: 'man', name: 'Man' }] } });
assert.throws(() => buildDebugArtifact(289, { commands: [], names: { obj: [{ id: 1, alias: 'coin', name: 'Coins' }] } }), /must not duplicate base item names/);
const tamperedSchema = JSON.parse(debugArtifactBytes(debugBuilt));
tamperedSchema.schema_version = 2;
assert.throws(() => debugArtifactBytes(tamperedSchema), /invalid debug artifact schema/);
// ---- gather_resources labels and admission (synthetic methods; no content) ----

const synRespawn = {
    state: 'known' as const,
    value: {
        raw: 10,
        scale: { state: 'known' as const, value: { rule: 'scale_by_playercount', min_ticks: 5, max_ticks: 10, sources: [] as string[] } },
        source: 'mine.dbrow:1-2',
    },
};
const synTarget = (id: number, respawn: TargetWire['respawn'] = synRespawn): TargetWire => ({ kind: 'loc', id, op: 1, class: 'resource', respawn });

const synTickArm = (): MethodWire['tick_arm'] => ({
    kind: 'none',
    roll_signal: 'never',
    delay_drop: false,
    depletes: false,
    native_arm: null,
    act_min_d: 0,
    act_silent: false,
    no_gain: false,
    skill_anim: null,
    roll_anim: null,
    cast_anim: null,
    fletch_anim: 1,
    tool_facts: [],
});
function synMethod(
    id: string,
    skill: MethodWire['skill'],
    resources: string[],
    products: MethodWire['products'],
    requirements: MethodWire['requirements'],
    extras?: Partial<Pick<MethodWire, 'targets' | 'tools' | 'consumes' | 'spots' | 'forbidden_states'>>,
): MethodWire {
    const knownEmpty = { state: 'known' as const, value: [] };
    return {
        id,
        skill,
        resources,
        op: { slot: 1, label: 'Mine' },
        targets: { state: 'known' as const, value: [synTarget(2092)] },
        products,
        tools: knownEmpty,
        consumes: knownEmpty,
        tick_arm: synTickArm(),
        requirements,
        spots: { state: 'known' as const, value: null },
        sources: [],
        forbidden_states: [],
        ...extras,
    };
}

const synPartialGap = (code: string) => ({ state: 'partial' as const, value: [], gaps: [{ code, sources: [] as string[] }] });
const synUnknownGap = (code: string) => ({ state: 'unknown' as const, gap: { code, sources: [] as string[] } });
const synKnownIds = (ids: { item: number; level: number }[]) => ({ state: 'known' as const, value: ids });
const synFacts = (methods: MethodWire[]): GatheringFacts => ({ entities: [], methods, loose: [], zones: [], movements: [], placements: [] });

const synOak = synMethod('woodcutting.oak', 'woodcutting', ['oak'], synKnownIds([{ item: 1521, level: 15 }]), synKnownIds([]));
assert.equal(gatherMethodGap(synOak), null, 'a fully known method is admitted');
assert.equal(isKnownGatherTarget(synTarget(2090)), true, 'a row with a respawn fact is a Known target');
assert.equal(isKnownGatherTarget(synTarget(3042, synUnknownGap('custom-deplete'))), false, 'a tutorial-gate row without respawn is excluded per target');
const synCopper = synMethod('mining.copper', 'mining', ['copper'], synPartialGap('incidental-gem-roll'), synKnownIds([]), {
    targets: {
        state: 'partial' as const,
        value: [synTarget(2090), synTarget(2091), synTarget(3042, synUnknownGap('custom-deplete'))],
        gaps: [{ code: 'custom-handler-target', sources: [] as string[] }],
    },
});
assert.equal(gatherMethodGap(synCopper), null, 'excluded tutorial rows never block: Known ore rows admit the method');
const synTakenOver = synMethod('mining.iron', 'mining', ['iron'], synKnownIds([{ item: 440, level: 15 }]), synKnownIds([]), {
    targets: {
        state: 'partial' as const,
        value: [synTarget(1, synUnknownGap('custom-deplete'))],
        gaps: [{ code: 'custom-handler-target', sources: [] as string[] }],
    },
});
assert.equal(gatherMethodGap(synTakenOver), 'custom-handler-target', 'zero Known targets refuses with the cell gap code');
const synJungle = synMethod('woodcutting.jungle', 'woodcutting', ['jungle'], synUnknownGap('no-resource-target'), synUnknownGap('no-resource-target'), {
    targets: synUnknownGap('no-resource-target'),
    tools: synUnknownGap('no-resource-target'),
    consumes: synUnknownGap('no-resource-target'),
    spots: synUnknownGap('no-resource-target'),
});
assert.equal(gatherMethodGap(synJungle), 'no-resource-target', 'an unknown cell refuses with its gap code');
const synKarambwan = synMethod('fishing.category_633.op1', 'fishing', ['tbwt_raw_karambwan'], synPartialGap('inventory-effect'), synPartialGap('varp-gate'));
assert.equal(gatherMethodGap(synKarambwan), 'varp-gate', 'requirements block before products under the design cell order');
const synBigNet = synMethod('fishing.memberfish.op1', 'fishing', ['raw_mackerel'], synKnownIds([{ item: 353, level: 16 }]), synKnownIds([]), { forbidden_states: ['monkey-form'] });
assert.equal(gatherMethodGap(synBigNet), null, 'a forbidden state is never a gap: the row stays selectable and the Gatherer refuses at begin');
assert.deepEqual(gatherResources(synFacts([synBigNet]), new Map()).map((row) => [row.selectable, row.forbidden_states]), [[true, ['monkey-form']]], 'forbidden states reach the row and keep it selectable');

const synNames = new Map([[1521, 'Oak logs'], [335, 'Raw trout'], [331, 'Raw salmon']]);
assert.deepEqual(gatherResources(synFacts([synOak]), synNames), [
    { skill: 'woodcutting', method: 'woodcutting.oak', methods: ['woodcutting.oak'], aliases: [], key: 'oak', resources: ['oak'], label: 'Oak logs', level: 15, selectable: true, forbidden_states: [], gap: null },
]);
assert.deepEqual(gatherResources(synFacts([synJungle]), new Map()), [
    { skill: 'woodcutting', method: 'woodcutting.jungle', methods: ['woodcutting.jungle'], aliases: [], key: 'jungle', resources: ['jungle'], label: 'Jungle', level: 0, selectable: false, forbidden_states: [], gap: 'no-resource-target' },
], 'a refused method keeps its row with a humanized fallback label');
const synIron = synMethod('mining.iron', 'mining', ['iron'], synKnownIds([{ item: 440, level: 15 }]), synKnownIds([]));
const synIronWall = synMethod('mining.iron.method_2', 'mining', ['iron'], synKnownIds([{ item: 440, level: 35 }]), synKnownIds([]), { forbidden_states: ['monkey-form'] });
assert.deepEqual(gatherResources(synFacts([synIron, synIronWall]), new Map([[440, 'Iron ore']])), [
    {
        skill: 'mining', method: 'mining.iron', methods: ['mining.iron', 'mining.iron.method_2'],
        aliases: [], key: 'iron', resources: ['iron'], label: 'Iron ore', level: 15,
        selectable: true, forbidden_states: ['monkey-form'], gap: null,
    },
], 'distinct source-proved methods share one resource option and retain both method IDs');
assert.deepEqual(
    gatherResources(synFacts([synIron, { ...synIronWall, tools: synUnknownGap('wall-tool') }]), new Map([[440, 'Iron ore']]))
        .map((row) => [row.selectable, row.gap]),
    [[false, 'wall-tool']],
    'a grouped resource cannot hide a refused method',
);
const synFresh = synMethod('fishing.freshfish.op1', 'fishing', ['raw_trout', 'raw_salmon'], synKnownIds([{ item: 335, level: 20 }, { item: 331, level: 30 }]), synKnownIds([]));
const synFreshAlias = synMethod('fishing.freshfish.op2', 'fishing', ['raw_salmon', 'raw_trout'], synKnownIds([{ item: 331, level: 30 }, { item: 335, level: 20 }]), synKnownIds([]));
const freshGroups = gatherResources(synFacts([synFresh, synFreshAlias]), synNames);
assert.deepEqual(freshGroups, [
    {
        skill: 'fishing',
        method: 'fishing.freshfish.op1',
        methods: ['fishing.freshfish.op1', 'fishing.freshfish.op2'],
        aliases: ['fishing.freshfish.op1', 'fishing.freshfish.op2'],
        key: 'fishing.mine.tool_none.products_331_335',
        resources: ['raw_salmon', 'raw_trout'],
        label: 'Raw trout / Raw salmon — Mine (Unknown tool)',
        level: 20,
        selectable: true,
        forbidden_states: [],
        gap: null,
    },
], 'fishing keys sort product ids, while members and legacy ids form one option');
assert.deepEqual(
    gatherResources(synFacts([synFreshAlias, synFresh]), synNames),
    freshGroups,
    'fishing group keys and rows do not depend on product discovery order',
);
const cappedProducts = synMethod(
    'fishing.capped.op1',
    'fishing',
    ['p1', 'p2', 'p3', 'p4', 'p5'],
    synKnownIds([
        { item: 1, level: 1 },
        { item: 2, level: 2 },
        { item: 3, level: 3 },
        { item: 4, level: 4 },
        { item: 5, level: 5 },
    ]),
    synKnownIds([]),
);
const cappedNames = new Map([[1, 'One'], [2, 'Two'], [3, 'Three'], [4, 'Four'], [5, 'Five']]);
assert.equal(
    gatherResources(synFacts([cappedProducts]), cappedNames)[0]?.label,
    'One / Two / Three +2 more — Mine (Unknown tool)',
    'fishing labels cap product names at three',
);
const duplicateLabelA = synMethod('fishing.same.op1', 'fishing', ['raw_trout'], synKnownIds([{ item: 335, level: 20 }]), synKnownIds([]), {
    tools: synKnownIds([{ item: 10, level: 1 }]),
});
const duplicateLabelB = synMethod('fishing.same.op2', 'fishing', ['raw_trout'], synKnownIds([{ item: 335, level: 20 }]), synKnownIds([]), {
    tools: synKnownIds([{ item: 11, level: 1 }]),
});
assert.throws(
    () => gatherResources(synFacts([duplicateLabelA, duplicateLabelB]), new Map([[335, 'Raw trout'], [10, 'Shared tool'], [11, 'Shared tool']])),
    /fishing labels are not unique/,
);
const synUnnamed = synMethod('woodcutting.oak', 'woodcutting', ['oak'], synKnownIds([{ item: 99999, level: 15 }]), synKnownIds([]));
assert.equal(gatherResources(synFacts([synUnnamed]), new Map())[0]?.label, 'Oak', 'an unjoined product id falls back instead of inventing a name');

// ---- gather_sites negatives: fixture joins fail closed, never guess ----
function siteFixtureContent(labels: string[], fishingNpc: string): string {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'gather-sites-fixture-'));
    fs.mkdirSync(path.join(dir, 'maps'), { recursive: true });
    fs.mkdirSync(path.join(dir, 'scripts/skill_fishing/configs'), { recursive: true });
    fs.writeFileSync(path.join(dir, 'maps/labels.txt'), `${labels.join('\n')}\n`);
    fs.writeFileSync(path.join(dir, 'scripts/skill_fishing/configs/fishing.npc'), `${fishingNpc}\n`);
    return dir;
}
function synSiteFacts(methods: MethodWire[], entities: string[], movements: GatheringFacts['movements'], placements: GatheringFacts['placements']): GatheringFacts {
    return { entities, methods, loose: [], zones: [], movements, placements, hazard_npcs: [], incidental_gem_ids: [] };
}
function synMovement(npc: number, region: GatheringFacts['movements'][number]['region']): GatheringFacts['movements'][number] {
    return { npc, region };
}
const synKnownBox = (minX: number, minZ: number, maxX: number, maxZ: number): GatheringFacts['movements'][number]['region'] => ({ state: 'known', value: { min_x: minX, min_z: minZ, max_x: maxX, max_z: maxZ, level: 0 } });
const synNpcTarget = (id: number): TargetWire => ({ kind: 'npc', id, op: 1, class: 'resource', respawn: synRespawn });
const synPlacements = (file: string, rows: string[]): GatheringFacts['placements'][number] => ({ file, rows });
const synTestFish = (id: string, targets: TargetWire[], item = 9001): MethodWire =>
    synMethod(id, 'fishing', ['raw_test'], synKnownIds([{ item, level: 1 }]), synKnownIds([]), { targets: { state: 'known', value: targets } });
const synFishNames = new Map([[9001, 'Testfish'], [9002, 'Secondfish']]);
// No overrides: the synthetic fixtures test the walk, not the shipped table.
const noOverrides: readonly SiteOverride[] = [];

// A geographic enum stem names the NPC camp inside its own movement box.
{
    const content = siteFixtureContent(['=Test Water,3210,3210,0'], '[testspot]\nparam=fishing_movement_enum,fishing_movement_test_water_enum');
    const facts = synSiteFacts([synTestFish('fishing.test.op1', [synNpcTarget(5)])], ['npc 5 testspot'], [synMovement(5, synKnownBox(3200, 3200, 3220, 3220))], [synPlacements('maps/m50_50.jm2', ['1 0 10 10 n5 0'])]);
    const result = gatherSites(facts, gatherResources(facts, synFishNames), content, [], noOverrides);
    assert.deepEqual(result.rows, [{ id: 'fishing.test_water', skill: 'fishing', label: 'Test Water · Mine', region: { min_x: 3210, min_z: 3210, max_x: 3210, max_z: 3210, level: 0 }, keys: [{ key: 'fishing.mine.tool_none.products_9001', count: 1 }], requires: [] }]);
}
// The resolution rule enumerates method targets: a missing, null or unknown movement box throws.
{
    const content = siteFixtureContent(['=Test Water,3210,3210,0'], '[testspot]\nparam=fishing_movement_enum,fishing_movement_test_water_enum');
    const method = synTestFish('fishing.test.op1', [synNpcTarget(5)]);
    const rows = gatherResources(synSiteFacts([method], ['npc 5 testspot'], [], []), synFishNames);
    assert.throws(() => gatherSites(synSiteFacts([method], ['npc 5 testspot'], [], [synPlacements('maps/m50_50.jm2', ['1 0 10 10 n5 0'])]), rows, content, [], noOverrides), /no known movement box/, 'an entirely missing movement row throws');
    assert.throws(() => gatherSites(synSiteFacts([method], ['npc 5 testspot'], [synMovement(5, { state: 'known', value: null })], []), rows, content, [], noOverrides), /no known movement box/, 'a null movement box throws');
    assert.throws(() => gatherSites(synSiteFacts([method], ['npc 5 testspot'], [synMovement(5, synUnknownGap('movement-enum-missing'))], []), rows, content, [], noOverrides), /no known movement box/, 'an unknown movement box throws');
}
// Two stems humanising to one name throw, even before any placement is read.
{
    const content = siteFixtureContent([`=Seers' Village,2714,3529,0`], '[spot_a]\nparam=fishing_movement_enum,fishing_movement_seers_village_enum\n[spot_b]\nparam=fishing_movement_enum,fishing_movement_seersvillage_enum');
    const methods = [synTestFish('fishing.a.op1', [synNpcTarget(5)]), synTestFish('fishing.b.op1', [synNpcTarget(6)], 9002)];
    const facts = synSiteFacts(methods, ['npc 5 spot_a', 'npc 6 spot_b'], [synMovement(5, synKnownBox(2700, 3520, 2730, 3540)), synMovement(6, synKnownBox(2700, 3520, 2730, 3540))], []);
    assert.throws(() => gatherSites(facts, gatherResources(facts, synFishNames), content, [], noOverrides), /collide/, 'two stems humanising to one name throw');
}
// Two geographic names in one camp throw; a spawn outside its own box is ignored for direct naming.
{
    const content = siteFixtureContent(['=Test Water,3210,3210,0'], '[pool_a]\nparam=fishing_movement_enum,fishing_movement_test_water_enum\n[pool_b]\nparam=fishing_movement_enum,fishing_movement_far_water_enum');
    const method = synTestFish('fishing.test.op1', [synNpcTarget(5), synNpcTarget(6)]);
    const clashing = synSiteFacts([method], ['npc 5 pool_a', 'npc 6 pool_b'], [synMovement(5, synKnownBox(3200, 3200, 3220, 3220)), synMovement(6, synKnownBox(3200, 3200, 3220, 3220))], [synPlacements('maps/m50_50.jm2', ['1 0 10 10 n5 0', '2 0 12 10 n6 0'])]);
    assert.throws(() => gatherSites(clashing, gatherResources(clashing, synFishNames), content, [], noOverrides), /ambiguous/, 'two direct names in one site throw');
    const outside = synSiteFacts([method], ['npc 5 pool_a', 'npc 6 pool_b'], [synMovement(5, synKnownBox(3200, 3200, 3220, 3220)), synMovement(6, synKnownBox(0, 0, 10, 10))], [synPlacements('maps/m50_50.jm2', ['1 0 10 10 n5 0', '2 0 12 10 n6 0'])]);
    const outsideResult = gatherSites(outside, gatherResources(outside, synFishNames), content, [], noOverrides);
    assert.equal(outsideResult.rows[0]?.label, 'Test Water · Mine', 'a spawn outside its own box takes the one inside name');
}

// A loc camp never borrows a nearby NPC's enum, even a geographic one.
{
    const content = siteFixtureContent(['=Test Water,3210,3210,0'], '[other]\nparam=fishing_movement_enum,fishing_movement_other_enum');
    const method = synMethod('fishing.test.op1', 'fishing', ['raw_test'], synKnownIds([{ item: 9001, level: 1 }]), synKnownIds([]), { targets: { state: 'known', value: [synTarget(2092)] } });
    const facts = synSiteFacts([method], [], [], [synPlacements('maps/m50_50.jm2', ['1 0 10 10 l2092 0'])]);
    const result = gatherSites(facts, gatherResources(facts, synFishNames), content, [], noOverrides);
    assert.equal(result.rows[0]?.label, 'Test Water · Mine', 'a loc camp never borrows a nearby NPC name');
}
// A spawn outside its own box with no inside name falls back to the place.
{
    const content = siteFixtureContent(['=Test Water,3210,3210,0'], '[testspot]\nparam=fishing_movement_enum,fishing_movement_test_water_enum');
    const facts = synSiteFacts([synTestFish('fishing.test.op1', [synNpcTarget(5)])], ['npc 5 testspot'], [synMovement(5, synKnownBox(0, 0, 10, 10))], [synPlacements('maps/m50_50.jm2', ['1 0 10 10 n5 0'])]);
    const result = gatherSites(facts, gatherResources(facts, synFishNames), content, [], noOverrides);
    assert.equal(result.rows[0]?.label, 'Test Water · Mine', 'an all-outside camp keeps its place name');
}
// A distant surface site uses the nearest label with a bearing and distance.
{
    const content = siteFixtureContent(['=Far Place,100,100,0'], '[unrelated]');
    const facts = synSiteFacts([synTestFish('fishing.test.op1', [synNpcTarget(5)])], ['npc 5 testspot'], [synMovement(5, synKnownBox(3200, 3200, 3220, 3220))], [synPlacements('maps/m50_50.jm2', ['1 0 10 10 n5 0'])]);
    const result = gatherSites(facts, gatherResources(facts, synFishNames), content, [], noOverrides);
    assert.equal(result.rows.length, 1, 'a surface site far from every label is still named');
    assert.match(result.rows[0]!.label, /^Far Place [A-Z]+\d+ · /, 'the far site takes the nearest label with a bearing and distance');
}
// A one-tile underground fragment is below the three-tile woodcutting minimum and is dropped, as on the surface.
{
    const content = siteFixtureContent(['=Test Water,3210,3210,0'], '[unrelated]');
    const oak = synMethod('woodcutting.oak', 'woodcutting', ['oak'], synKnownIds([{ item: 1521, level: 15 }]), synKnownIds([]), { targets: { state: 'known', value: [synTarget(2092)] } });
    const deep = synSiteFacts([oak], [], [], [synPlacements('maps/m50_100.jm2', ['1 0 10 0 l2092 0'])]);
    assert.deepEqual(gatherSites(deep, gatherResources(deep, new Map([[1521, 'Oak logs']])), content, [], noOverrides).rows, [], 'a one-tile underground fragment is dropped');
}
// A special-area cluster with no label within 64 tiles is never offered.
{
    const content = siteFixtureContent(['=Test Water,3210,3210,0'], '[testspot]\nparam=fishing_movement_enum,fishing_movement_test_water_enum');
    const facts = synSiteFacts([synTestFish('fishing.test.op1', [synNpcTarget(5)])], ['npc 5 testspot'], [synMovement(5, synKnownBox(3200, 4480, 3220, 4500))], [synPlacements('maps/m50_70.jm2', ['1 0 10 10 n5 0'])]);
    const result = gatherSites(facts, gatherResources(facts, synFishNames), content, [], noOverrides);
    assert.deepEqual(result.rows, [], 'a special-area cluster with no label within 64 tiles is not offered');
}
// Ordinals rank by distance, then count: the nearer part keeps the bare name.
{
    const content = siteFixtureContent(['=P,0,0,0'], '[unrelated]');
    const oak = synMethod('woodcutting.oak', 'woodcutting', ['oak'], synKnownIds([{ item: 1521, level: 15 }]), synKnownIds([]), { targets: { state: 'known', value: [synTarget(7)] } });
    const facts = synSiteFacts([oak], [], [], [synPlacements('maps/m0_0.jm2', ['1 0 20 0 l7 0', '2 0 21 0 l7 0', '3 0 40 0 l7 0', '4 0 41 0 l7 0', '5 0 42 0 l7 0'])]);
    const result = gatherSites(facts, gatherResources(facts, new Map([[1521, 'Oak logs']])), content, [], noOverrides);
    assert.deepEqual(result.rows.map((row) => [row.id, row.label]), [['woodcutting.p.e', 'P E20 · Oak logs 2'], ['woodcutting.p.e.2', 'P E40 (2) · Oak logs 3']]);
}

// Place names sort before their bearing siblings, independently of the label separator.
{
    const content = siteFixtureContent(['=Test Water,3210,3210,0'], '[unrelated]');
    const oak = synMethod(
        'woodcutting.oak',
        'woodcutting',
        ['oak'],
        synKnownIds([{ item: 1521, level: 15 }]),
        synKnownIds([]),
        { targets: { state: 'known', value: [synTarget(7), synTarget(8)] } },
    );
    const facts = synSiteFacts(
        [oak],
        [],
        [],
        [synPlacements('maps/m50_50.jm2', ['1 0 10 10 l7 0', '2 0 50 10 l8 0'])],
    );
    const result = gatherSites(facts, gatherResources(facts, new Map([[1521, 'Oak logs']])), content, [], noOverrides);
    assert.deepEqual(
        result.rows.map((row) => row.label),
        ['Test Water · Oak logs 1', 'Test Water E40 · Oak logs 1'],
    );
}

// A category-suffixed enum stem is not a direct name.
{
    const content = siteFixtureContent(['=Test Water,3210,3210,0'], '[coalfish]\nparam=fishing_movement_enum,fishing_movement_coalfish_enum');
    const facts = synSiteFacts([synTestFish('fishing.test.op1', [synNpcTarget(5)])], ['npc 5 coalfish'], [synMovement(5, synKnownBox(3200, 3200, 3220, 3220))], [synPlacements('maps/m50_50.jm2', ['1 0 10 10 n5 0'])]);
    const result = gatherSites(facts, gatherResources(facts, synFishNames), content, [], noOverrides);
    assert.equal(result.rows[0]?.label, 'Test Water · Mine', 'a category-suffixed enum stem is not a direct name');
}
{
    const content = fs.mkdtempSync(path.join(os.tmpdir(), 'game-data-karamja-fixture-'));
    const write = (relative: string, text: string) => {
        const file = path.join(content, relative);
        fs.mkdirSync(path.dirname(file), { recursive: true });
        fs.writeFileSync(file, text);
    };
    write('pack/loc.pack', '700=bananacrate\n701=bananatreefull\n702=bananatreeone\n703=bananatreeempty\n');
    write('pack/npc.pack', '500=luthas\n');
    write('scripts/areas/area_karamja/configs/plantation.loc', [
        '[bananatreefull]', 'category=banana_tree', 'param=next_loc_stage,bananatreeone',
        '[bananatreeone]', 'category=banana_tree', 'param=next_loc_stage,bananatreeempty',
        '[bananatreeempty]',
    ].join('\n'));
    write('scripts/areas/area_karamja/scripts/banana_tree.rs2', [
        '[oploc1,_banana_tree]',
        'loc_change(loc_param(next_loc_stage), 500);',
        'inv_add(inv, banana, 1);',
        '[oploc1,bananatreeempty]',
    ].join('\n'));
    write('scripts/quests/quest_hunt/scripts/banana_crate.rs2', [
        '[oploc1,bananacrate]',
        'if (%crate_bananas = 0 & %crate_rum = 0) {',
        '    mes("The crate is completely empty.");',
        '}',
        'if (%crate_bananas = 7) {',
        '    mes("The crate is full of bananas.");',
        '}',
    ].join('\n'));
    const luthasPath = 'scripts/quests/quest_hunt/scripts/luthas.rs2';
    const luthasScript = [
        '[opnpc1,luthas]',
        'if (testbit(%hunt_store_employed, ^hunt_not_started) = ^false) {',
        '    @multi2("Offer employment?", luthas_employment, "Other choice", other);',
        '}',
        'if (%crate_bananas = 7) {',
        '    mes("Luthas hands you 43 coins.");',
        '    inv_add(inv, coins, 43);',
        '    @multi4("Another crate?", again, "Thanks!", thanks, "Delivery?", delivery, "Other?", other);',
        '}',
        '@multi4("Repeat?", repeat, "Not yet.", incomplete, "Delivery?", delivery, "Other?", other);',
    ].join('\n');
    write(luthasPath, luthasScript);
    write('maps/m45_49.jm2', [
        '==== LOC ====',
        '0 1 2: 700 10 0',
        '0 4 5: 701 10 0',
        '0 5 6: 702 10 0',
        '==== NPC ====',
    ].join('\n'));
    const npcNames = { rows: [{ id: 500, config: 'luthas', display: 'Fixture Luthas', ops: ['Talk-to'] }] };
    const locNames = { rows: [
        { id: 700, config: 'bananacrate', display: 'Crate', ops: ['Search'] },
        { id: 701, config: 'bananatreefull', display: 'Banana Tree', ops: ['Search'] },
        { id: 702, config: 'bananatreeone', display: 'Banana Tree', ops: ['Search'] },
        { id: 703, config: 'bananatreeempty', display: 'Banana Tree', ops: ['Search'] },
    ] };
    const npcPlacements = { rows: [{ npc_id: 500, x: 2882, z: 3139, plane: 0, mapsquare: 'm45_49' }] };
    const result = extractKaramjaFacts(content, npcNames, locNames, npcPlacements);
    assert.deepEqual(result.facts, {
        luthas_spawn: { config: 'luthas', x: 2882, z: 3139, plane: 0 },
        crate_spawn: { config: 'bananacrate', x: 2881, z: 3138, plane: 0 },
        banana_tree_configs: ['bananatreefull', 'bananatreeone'],
        banana_tree_spawns: [
            { config: 'bananatreefull', x: 2884, z: 3141, plane: 0 },
            { config: 'bananatreeone', x: 2885, z: 3142, plane: 0 },
        ],
        crate_capacity: 7,
        coin_payout: 43,
        dialogue: { employment: 'Offer employment?', paid: 'Thanks!', incomplete: 'Not yet.' },
    });
    assert.throws(
        () => extractKaramjaFacts(content, npcNames, locNames, { rows: [] }),
        /expected one Luthas NPC placement, got 0/,
        'missing content placement refuses instead of guessing an anchor',
    );
    write(luthasPath, luthasScript.replace('inv_add(inv, coins, 43);', 'inv_add(inv, coins, 44);'));
    assert.throws(
        () => extractKaramjaFacts(content, npcNames, locNames, npcPlacements),
        /Luthas coin message\/grant disagree \(43\/44\)/,
        'the full-crate payout is the amount the content actually grants',
    );
    write(luthasPath, luthasScript);
    const legacyPlantation = path.join(content, 'scripts/areas/area_karamja/configs/plantation.loc');
    const unpackedPlantation = 'scripts/_unpack/377/all.loc';
    write(unpackedPlantation, fs.readFileSync(legacyPlantation, 'utf8'));
    fs.rmSync(legacyPlantation);
    const selected377 = extractKaramjaFacts(content, npcNames, locNames, npcPlacements, 377);
    assert.deepEqual(selected377.facts, result.facts, '377 uses its actual unpacked loc configs without requiring a legacy plantation path');
    fs.rmSync(path.join(content, unpackedPlantation));
    assert.throws(() => extractKaramjaFacts(content, npcNames, locNames, npcPlacements, 377), /all\.loc: required file missing/);
}

// Overrides name, require or drop a published row by id: the id and region never change.
{
    const content = siteFixtureContent(['=Test Water,3210,3210,0'], '[testspot]\nparam=fishing_movement_enum,fishing_movement_test_water_enum');
    const facts = synSiteFacts([synTestFish('fishing.test.op1', [synNpcTarget(5)])], ['npc 5 testspot'], [synMovement(5, synKnownBox(3200, 3200, 3220, 3220))], [synPlacements('maps/m50_50.jm2', ['1 0 10 10 n5 0'])]);
    const resources = gatherResources(facts, synFishNames);
    const requires = [{ kind: 'skill' as const, skill: 'fishing' as const, level: 20 }];
    const renamed = gatherSites(facts, resources, content, [], [{ id: 'fishing.test_water', cite: 'fixture', action: 'name', name: 'Renamed Water', requires }]);
    assert.deepEqual(renamed.rows, [{ id: 'fishing.test_water', skill: 'fishing', label: 'Renamed Water (Fishing 20) · Mine', region: { min_x: 3210, min_z: 3210, max_x: 3210, max_z: 3210, level: 0 }, keys: [{ key: 'fishing.mine.tool_none.products_9001', count: 1 }], requires }]);
    const dropRow = { cite: 'fixture', action: 'drop', place: 'Fixture', reason: 'fixture' } as const;
    const dropped = gatherSites(facts, resources, content, [], [{ id: 'fishing.test_water', ...dropRow }]);
    assert.deepEqual(dropped.rows, [], 'a drop row removes the published site');
    assert.deepEqual(dropped.hidden.map((row) => row.id), ['fishing.test_water'], 'a dropped site is listed as hidden with its region');
    assert.throws(() => gatherSites(facts, resources, content, [], [{ id: 'fishing.missing', ...dropRow }]), /names no published site/, 'an unmatched non-optional override throws');
    assert.equal(gatherSites(facts, resources, content, [], [{ id: 'fishing.missing', optional: true, ...dropRow }]).rows.length, 1, 'an unmatched optional override is skipped');
    assert.throws(() => gatherSites(facts, resources, content, [], [{ id: 'fishing.test_water', ...dropRow }, { id: 'fishing.test_water', cite: 'fixture', action: 'name', name: 'Other', requires: [] }]), /has two overrides/, 'two overrides for one id throw');
}
// The shipped table lists each id once and cites content as path:lines.
{
    assert.equal(new Set(SITE_OVERRIDES.map((row) => row.id)).size, SITE_OVERRIDES.length, 'each override id is listed once');
    for (const row of SITE_OVERRIDES) assert.match(row.cite, /\.(?:rs2|enum|loc|jm2):\d/, `${row.id} cites content lines`);
}

// PvP V0 fact parsers: bounded synthetic inputs exercise their grammar and structural errors.
{
    const fixture = fs.mkdtempSync(path.join(os.tmpdir(), 'duel-facts-fixture-'));
    const write = (relative: string, bytes: string) => {
        const file = path.join(fixture, relative);
        fs.mkdirSync(path.dirname(file), { recursive: true });
        fs.writeFileSync(file, bytes);
    };
    const rules = [
        'no_forfeit', 'no_movement', 'no_weapons', 'no_armour', 'no_ranged', 'no_melee', 'no_magic',
        'no_potions', 'no_food', 'no_prayer', 'obstacles', 'no_jewelry', 'flower_power', 'no_specs',
    ] as const;
    write('pack/interface.pack', rules.map((rule, index) => `${100 + index}=duel_select_type:${rule}`).join('\n') + '\n');
    write('scripts/minigames/game_duelarena/configs/duelarena.constant', rules.map((rule, bit) => `^${rule} = ${bit}`).join('\n') + '\n');
    write('scripts/minigames/game_duelarena/scripts/duel_arena_settings.rs2', rules.map((rule) => `[if_button,duel_select_type:${rule}]`).join('\n') + '\n');
    const ruleButtons = extractDuelRuleButtons(fixture);
    assert.deepEqual(ruleButtons.map((row) => row.bit), rules.map((_, bit) => bit));
    assert.deepEqual(ruleButtons.map((row) => row.component), rules.map((_, index) => 100 + index));

    const corner = (index: number) => `0_50_50_${index * 2 + 1}_${index * 2 + 2}`;
    const pair = (first: number, second: number) => `data=coord_pair,${corner(first)},${corner(second)}`;
    const fightPairs = [pair(0, 1), pair(2, 3), pair(4, 5)];
    const obstaclePairs = [pair(6, 7), pair(8, 9), pair(10, 11)];
    const zonesFile = [
        '[duel_arena_fight_zones]', ...fightPairs,
        '[duel_arena_obstacle_fight_zones]', ...obstaclePairs,
        '[duel_arena_zones]', pair(12, 13),
    ].join('\n') + '\n';
    write('scripts/minigames/game_duelarena/configs/duelarena.dbrow', zonesFile);
    write('scripts/minigames/game_duelarena/scripts/duel_arena_finish.rs2', 'map_findsquare_inzone(0_50_50_29_30, 0_50_50_31_32);\n');
    const zones = extractDuelZones(fixture);
    assert.deepEqual(zones.pens.map((pen) => pen.obstacles), [false, false, false, true, true, true]);
    assert.deepEqual(zones.pens[0], { x1: 3201, z1: 3202, x2: 3203, z2: 3204, obstacles: false });
    assert.deepEqual(zones.challenge, { x1: 3225, z1: 3226, x2: 3227, z2: 3228 });
    assert.deepEqual(zones.lobby, { x1: 3229, z1: 3230, x2: 3231, z2: 3232 });
    write('scripts/minigames/game_duelarena/configs/duelarena.dbrow', zonesFile.replace(`${obstaclePairs[2]}\n`, ''));
    assert.throws(() => extractDuelZones(fixture), /expected 3 fight and 3 obstacle pens, got 3\/2/);

    // The parser reads the strength argument from the ordinary tier gate.
    // 377 emits only handler-proven rule buttons through the same parser.
    const duel377Fixture = fs.mkdtempSync(path.join(os.tmpdir(), 'duel-rules-377-'));
    fs.mkdirSync(path.join(duel377Fixture, 'pack'), { recursive: true });
    fs.mkdirSync(path.join(duel377Fixture, 'scripts/minigames/game_duelarena/configs'), { recursive: true });
    fs.mkdirSync(path.join(duel377Fixture, 'scripts/minigames/game_duelarena/scripts'), { recursive: true });
    const duel377Constants = path.join(duel377Fixture, 'scripts/minigames/game_duelarena/configs/duelarena.constant');
    const duel377Settings = path.join(duel377Fixture, 'scripts/minigames/game_duelarena/scripts/duel_arena_settings.rs2');
    fs.writeFileSync(path.join(duel377Fixture, 'pack/interface.pack'), '101=duel_select_type:no_forfeit\n113=duel_select_type:no_specs\n');
    fs.writeFileSync(duel377Constants, '^no_forfeit = 0\n^no_specs = 13\n');
    const selected377Settings = '[if_button,duel_select_type:no_forfeit]\n~toggle_duel_setting(^no_forfeit);\n[if_button,duel_select_type:no_specs]\n~toggle_duel_setting(^no_specs);\n[proc,toggle_duel_setting](int $setting)\n%dueloptions = togglebit(%dueloptions, $setting);\n';
    fs.writeFileSync(duel377Settings, selected377Settings);
    assert.deepEqual(extractDuelRuleButtons(duel377Fixture, 377), [{ bit: 0, component: 101 }, { bit: 13, component: 113 }], '377 emits only handler-proven rule buttons, retaining their selected bit positions');
    assert.throws(() => extractDuelRuleButtons(duel377Fixture), /not the 14 known rules/, 'the legacy full-rule contract is unchanged');
    fs.writeFileSync(duel377Settings, selected377Settings.replace('~toggle_duel_setting(^no_specs);', '~toggle_duel_setting(^no_forfeit);'));
    assert.throws(() => extractDuelRuleButtons(duel377Fixture, 377), /no_specs handler does not toggle its named setting/);
    fs.writeFileSync(duel377Settings, selected377Settings.replace('%dueloptions = togglebit(%dueloptions, $setting);', '%dueloptions = 0;'));
    assert.throws(() => extractDuelRuleButtons(duel377Fixture, 377), /toggle proc does not toggle dueloptions/);
    fs.writeFileSync(duel377Settings, selected377Settings);
    fs.writeFileSync(duel377Constants, '^no_forfeit = 0\n^no_specs = 14\n');
    assert.throws(() => extractDuelRuleButtons(duel377Fixture, 377), /invalid or duplicate bit/);
    fs.rmSync(duel377Fixture, { recursive: true, force: true });
    const strengthFixture = fs.mkdtempSync(path.join(os.tmpdir(), 'melee-tier-fixture-'));
    const tierDirectory = path.join(strengthFixture, 'scripts/levelrequire/scripts');
    fs.mkdirSync(tierDirectory, { recursive: true });
    for (const file of ['tier1', 'tier5', 'tier10', 'tier20', 'tier30', 'tier40', 'tier45', 'tier50', 'tier60', 'tier70']) {
        fs.writeFileSync(path.join(tierDirectory, `${file}.rs2`), file === 'tier30'
            ? '[opheld2,test_halberd]\n@levelrequire_attack_and_strength(30, 15);\n'
            : '// no test_halberd wield row in this tier\n');
    }
    const weaponStyles = [{ obj_id: 1, category: 0 }];
    const aliases = new Map([[1, 'test_halberd']]);
    assert.deepEqual(
        extractMeleeWeaponTiers(strengthFixture, weaponStyles, aliases).rows,
        [{ obj_id: 1, tier: 30, quest_gated: false, strength: 15 }],
    );
    fs.writeFileSync(path.join(tierDirectory, 'tier30.rs2'), '[opheld2,test_halberd] @levelrequire_attack_and_strength(30);\n');
    assert.throws(
        () => extractMeleeWeaponTiers(strengthFixture, weaponStyles, aliases).rows,
        /missing the strength level/,
        'a one-argument strength gate throws',
    );
    const tier377Fixture = fs.mkdtempSync(path.join(os.tmpdir(), 'melee-tier-377-'));
    fs.mkdirSync(path.join(tier377Fixture, 'scripts/levelrequire/scripts'), { recursive: true });
    for (const tier of [1, 5, 10, 20, 30, 40, 45, 50, 60, 70]) {
        fs.writeFileSync(path.join(tier377Fixture, `scripts/levelrequire/scripts/tier${tier}.rs2`), '// no fixture gate\n');
    }
    const direct377Gate = '[opheld2,literal_maul]\nif (stat_base(attack) < 51 | stat_base(strength) < 49) {\nmes("Requirements not met.");\nreturn;\n}\n~equip(last_slot);\n';
    const direct377File = path.join(tier377Fixture, 'scripts/levelrequire/scripts/tier50.rs2');
    fs.writeFileSync(direct377File, direct377Gate
        + '[opheld2,slayer_spear]\nif (stat_base(slayer) < 55 | stat_base(attack) < 50) {\nmes("Requirements not met.");\nreturn;\n}\n~equip(last_slot);\n');
    fs.writeFileSync(path.join(tier377Fixture, 'scripts/levelrequire/scripts/tier60.rs2'), '[opheld2,quest_scimitar] @levelrequire_mm_quest_attack(61, last_slot);\n');
    fs.writeFileSync(path.join(tier377Fixture, 'scripts/levelrequire/scripts/tier70.rs2'), '[opheld2,paired_hammers] @levelrequire_attack_and_strength2(71, last_slot);\n');
    const tier377Styles = [1, 2, 3, 4, 5].map(obj_id => ({ obj_id, category: 0 }));
    const tier377Aliases = new Map([[1, 'literal_maul'], [2, 'slayer_spear'], [3, 'quest_scimitar'], [4, 'paired_hammers'], [5, 'unproved']]);
    const tiers377 = extractMeleeWeaponTiers(tier377Fixture, tier377Styles, tier377Aliases, 377);
    assert.deepEqual(tiers377.rows, [
        { obj_id: 1, tier: 51, quest_gated: false, strength: 49 },
        { obj_id: 3, tier: 61, quest_gated: true, strength: 0 },
        { obj_id: 4, tier: 71, quest_gated: false, strength: 71 },
    ], '377 direct, quest and same-level attack/strength gates derive their selected operands');
    assert.deepEqual(tiers377.unknown.map(row => row.alias), ['slayer_spear', 'unproved'], 'unsupported skills and missing gates do not acquire a false tier-0 or complete eligibility claim');
    fs.writeFileSync(direct377File, direct377Gate.replace('return;', 'mes("No refusal.");'));
    assert.equal(extractMeleeWeaponTiers(tier377Fixture, tier377Styles, tier377Aliases, 377).rows.some(row => row.obj_id === 1), false, 'a missing refusal does not prove a wield requirement');
    fs.writeFileSync(direct377File, direct377Gate.replace('stat_base(attack) < 51', 'stat_base(attack) > 51'));
    assert.equal(extractMeleeWeaponTiers(tier377Fixture, tier377Styles, tier377Aliases, 377).rows.some(row => row.obj_id === 1), false, 'an inverted condition is not a minimum tier');
    fs.rmSync(tier377Fixture, { recursive: true, force: true });
}

{
    // An unknown specwep obj (no §6.1 row) fails closed instead of emitting
    // an untyped fact.
    const fixture = fs.mkdtempSync(path.join(os.tmpdir(), 'pvp-facts-fixture-'));
    for (const [name, bytes] of [
        ['pack/param.pack', [
            '136=specwep', '137=sa_energy', '138=strengthbonus', '139=rangebonus',
            '140=stabattack', '141=slashattack', '142=crushattack', '143=magicattack', '144=rangeattack',
            '145=stabdefence', '146=slashdefence', '147=crushdefence', '148=magicdefence', '149=rangedefence',
        ].join('\n') + '\n'],
        ['pack/seq.pack', '1=puncture\n'],
        ['pack/spotanim.pack', '2=sp_attack_puncture_spotanim\n'],
        ['scripts/skill_combat/configs/combat.param', `${combatParams}[specwep]
type=int
default=0

[sa_energy]
type=int
default=1000
`],
    ] as const) {
        fs.mkdirSync(path.join(fixture, path.dirname(name)), { recursive: true });
        fs.writeFileSync(path.join(fixture, name), bytes);
    }
    const specIds = parsePack(fs.readFileSync(path.join(fixture, 'pack/param.pack'), 'utf8'));
    const mystery = [{ id: 9999, params: new Map([[specIds.get('specwep')!, 1]]) }];
    assert.throws(
        () => extractSpecialWeaponFacts(fixture, mystery, [{ obj_id: 9999, style: 1, tab: 9 }], new Map([['mystery_sword', 9999]])),
        /no §6\.1 effect row/,
        'a specwep obj without a typed §6.1 row throws',
    );
    // A missing combat tab fails closed instead of emitting tab null.
    assert.throws(
        () => extractSpecialWeaponFacts(fixture, mystery, [{ obj_id: 9999, style: 1, tab: null }], new Map([['dragon_dagger', 9999]])),
        /has no combat tab/,
        'a specwep obj without a combat tab throws',
    );
    const bonus = extractItemCombatBonus(fixture, [
        { id: 1, params: new Map([[specIds.get('sa_energy')!, 40]]) },
        { id: 2, params: new Map() },
    ]);
    assert.deepEqual(bonus, [], 'sa_energy is not a worn combat bonus');
}

{
    const content = fs.mkdtempSync(path.join(os.tmpdir(), 'cook-surfaces-377-'));
    const files = {
        'scripts/skill_cooking/scripts/cooking.rs2': '[oplocu,_cooking_oven] @attempt_cook_item(last_useitem);\n[oplocu,_cooking_fire]\n@attempt_cook_item(last_useitem);\n',
        'scripts/test.loc': '[new_oven]\nname=Selected oven\ncategory=cooking_oven\n[new_fire]\nname=Selected fire\ncategory=cooking_fire\n[legacy_name_only]\nname=Range\n',
        'pack/loc.pack': '700=new_oven\n701=new_fire\n702=legacy_name_only\n',
        'maps/m50_50.jm2': '==== LOC ====\n0 1 2: 700\n0 3 4: 701\n0 5 6: 702\n',
    };
    for (const [relative, text] of Object.entries(files)) {
        fs.mkdirSync(path.dirname(path.join(content, relative)), { recursive: true });
        fs.writeFileSync(path.join(content, relative), text);
    }

    const selected = extractCookSurfaces(content, undefined, 377);
    assert.deepEqual(selected.facts.rows, [
        { x: 3201, z: 3202, level: 0, name: 'Selected oven', debugname: 'new_oven', kind: 'oven' },
        { x: 3203, z: 3204, level: 0, name: 'Selected fire', debugname: 'new_fire', kind: 'fire' },
    ], '377 kinds and identities derive from selected categories, not frozen aliases or display names');
    fs.writeFileSync(path.join(content, 'scripts/skill_cooking/scripts/cooking.rs2'), '[oplocu,_cooking_fire] @attempt_cook_item(last_useitem);\n');
    assert.throws(() => extractCookSurfaces(content, undefined, 377), /cooking_oven cooking handler is not derived/);
    fs.rmSync(content, { recursive: true });
}

// The real 377 selected corpus exercises the same source joins, never the
// 274/289 identity/count/coordinate pins. Optional only for legacy-only runs.
if (process.env.GAME_DATA_377_CONTENT) {
    const spec377 = revisions.find(spec => spec.revision === 377)!;
    const payload = JSON.parse(fs.readFileSync(spec377.output, 'utf8'));
    assert.equal(payload.revision, 377);
    assert.deepEqual(payload.prayers, extractPrayerFacts(spec377.content).prayers);
    // GAME_DATA_377_ENGINE selects the checkout at runtime; a static import would bind another corpus.
    const selectedObjectModule: { default: { load(dir: string): void; configs: Parameters<typeof extractTrailFacts>[1] } } = await import(pathToFileURL(path.join(spec377.engine, 'src/cache/config/ObjType.ts')).href);
    selectedObjectModule.default.load(path.join(spec377.engine, 'data/pack'));
    const decoded377Items = selectedObjectModule.default.configs;
    const selectedObjectIds = new Map(decoded377Items.map((item) => [item.debugname, item.id]));
    // The engine checkout is selected by GAME_DATA_377_ENGINE at runtime.
    const selectedInventoryModule: {
        default: {
            load(dir: string): void;
            getByName(name: string): { stockobj: Uint16Array | null; stockcount: Uint16Array | null; stockrate: Int32Array | null } | null;
        };
    } = await import(pathToFileURL(path.join(spec377.engine, 'src/cache/config/InvType.ts')).href);
    selectedInventoryModule.default.load(path.join(spec377.engine, 'data/pack'));
    for (const shop of payload.shops) {
        const packed = selectedInventoryModule.default.getByName(shop.inv);
        assert.ok(packed?.stockobj && packed.stockcount && packed.stockrate, `${shop.inv}: selected packed inventory is required`);
        const stock = Array.from(packed.stockobj, (id, index) => [id, packed.stockcount![index], packed.stockrate![index]]);
        // UpdateInvFullEncoder writes id + 1 as u16; 0xffff is an empty wire slot.
        for (const [id, count, rate] of stock) {
            if (id === 0xffff) assert.deepEqual([count, rate], [0, 0], `${shop.inv}: an absent stock slot has no count or rate`);
        }
        assert.deepEqual(
            shop.items.map((row: { obj: string; baseline: number; restockTicks: number }) =>
                [selectedObjectIds.get(row.obj), row.baseline, row.restockTicks]),
            stock.filter(([id]) => id !== 0xffff),
            `${shop.inv}: source stock must match the selected server inventory`,
        );
    }
    assert.deepEqual(payload.trails, extractTrailFacts(spec377.content, decoded377Items, 377));
    const overriddenChallenge = 'trail_clue_medium_anagram003_challenge';
    assert.equal(payload.trails.challenge_answers.find((row: { alias: string }) => row.alias === overriddenChallenge)?.answer, '40', '377 answer follows the actual decoded parameter override, not the earlier raw 32');
    const challengeItem = decoded377Items.find(item => item.debugname === overriddenChallenge)!;
    const answerParamId = parsePack(fs.readFileSync(path.join(spec377.content, 'pack/param.pack'), 'utf8')).get('trail_challenge_answer')!;
    const originalParams = challengeItem.params;
    try {
        challengeItem.params = new Map(originalParams);
        challengeItem.params.set(answerParamId, -1);
        assert.throws(() => extractTrailFacts(spec377.content, decoded377Items, 377), /decoded trail_challenge_answer does not match selected source params/, 'a mismatched decoded value cannot become a selected answer');
    } finally {
        challengeItem.params = originalParams;
    }
    assert.equal(payload.trails.rows.some((row: { access?: string }) => row.access !== undefined), false, '377 must not inherit the 289 constrained-clue annotation');
    assert.deepEqual(payload.talk_key, extractTalkKeyFacts(spec377.content, decoded377Items));
    assert.deepEqual(payload.trio_givers, extractTrioGiversFacts(spec377.content, 377));
    assert.deepEqual(payload.quest_identity, extractQuestIdentityFacts(spec377.content, 377));
    assert.deepEqual(payload.quest_starts, extractQuestStartFacts(spec377.content, payload.quest_identity.rows, 377));
    assert.deepEqual(payload.teleports, extractTeleportSpells(spec377.content, decoded377Items));
    const magic377 = extractMagicFacts(spec377.content, decoded377Items);
    assert.deepEqual(payload.spells, magic377.spells);
    assert.deepEqual(payload.staves, magic377.staves);
    assert.deepEqual(payload.herbs, extractHerbFacts(spec377.content, decoded377Items, 377).herbs);
    const equipment377 = extractEquipmentNamesFacts(payload.items, parsePack(fs.readFileSync(path.join(spec377.content, 'pack/obj.pack'), 'utf8')), repoRoot, 377);
    assert.deepEqual(payload.equipment_names, JSON.parse(JSON.stringify(equipment377)), 'the wire omits undefined optional metadata');
    assert.deepEqual(payload.cook_surfaces, extractCookSurfaces(spec377.content, undefined, 377).facts);
    const aliases377 = new Map<number, string>();
    for (const item of payload.items) if (typeof item.alias === 'string') aliases377.set(item.id, item.alias);
    const tiers377 = extractMeleeWeaponTiers(spec377.content, payload.weapon_styles, aliases377, 377);
    assert.deepEqual(payload.melee_weapon_tiers, tiers377.rows);
    assert.ok(tiers377.unknown.length > 0 && payload.coverage.some((row: { family: string }) => row.family === 'melee_weapon_tiers'), 'unproved 377 melee gates need named UNKNOWN coverage');
    const ruleButtons377 = new Map(extractDuelRuleButtons(spec377.content, 377).map(row => [row.bit, row.component]));
    assert.deepEqual(payload.duel.rule_buttons, Array.from({ length: 14 }, (_, bit) => ruleButtons377.get(bit) ?? 0), 'absent selected rule handlers retain unavailable slots, not legacy components');
    assert.deepEqual(payload.duel.rule_buttons.flatMap((id: number, bit: number) => id === 0 ? [bit] : []), [2, 3, 11]);
    assert.throws(() => extractDialogueUiFacts(spec377.content), /quest completion does not open the authored quest scroll in both branches/, '377 must not weaken the single-common-root proof or select just one completion branch');
    for (const family of ['autocast', 'special', 'special_weapons', 'nurmof_essence', 'flour_six', 'dialogue_ui', 'server_ordered']) {
        assert.equal(family in payload, false, `${family}: no legacy facts may be substituted on 377`);
        assert.ok(payload.coverage.some((row: { family: string; status: string; reason: string }) => row.family === family && row.status === 'UNKNOWN' && row.reason.length > 0), `${family}: omitted family needs named UNKNOWN coverage`);
    }
}

console.log('generate fixture passed');
