import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { verifyCacheIdentity } from './cache-identity.ts';
import { extractGatherMethodsFacts, extractGatherPlacementsFacts, extractQuestIdentityFacts, extractTalkKeyFacts, extractTrailFacts, extractTrioGiversFacts, assertTalkKeyPins, assertTrioGiverPins, gatherContentFiles, questIdentityContentFiles, trailContentFiles, loadEquipmentNamesCurated, parseFrozenEquipmentNameArrays, parsePack } from './generate.ts';
import { bankCatalogRust, cookCatalogRust, extractBankCatalog, extractBankPlacements, extractCookCatalog, extractCookSurfaces } from './generate.ts';
const root = path.resolve(import.meta.dirname, '../..');
const expected: Record<number, { engine: string; content: string; engineRoot: string; contentRoot: string; cache: { cache_id: string; content_id: string; nav_sha256: string; flags_sha256: string } }> = {
    274: { engine: '4c95f87efe00b068cadbd229d94736626907bd1a', content: '000c19997e07206131bcb3c884265840efce416d', engineRoot: process.env.GAME_DATA_274_ENGINE || '/Users/acfrazier/experiments/Server/engine', contentRoot: process.env.GAME_DATA_274_CONTENT || '/Users/acfrazier/experiments/Server/content', cache: { cache_id: '4aac9b63312dcb75d5de8f686772d083ba0808c57985438246edf21ef522be1c', content_id: '0d14c891b5727142379c6d8844bd5bb1ff9874d4d996db1dc5ceea0e9062469c', nav_sha256: '05db24743e9f549ced16c1f00b87c30a390d3aaec391815da3f3563130b3bcd4', flags_sha256: '92d5dea05c886ac8720be6b47e47cbc68355a8ff42676c0886f5b7ea8343a4cb' } },
    289: { engine: 'cc359656b4acd216ca452495874b6beba9a0ac75', content: '92649430fcbc83538d8c4367ecb96cee1a67a944', engineRoot: process.env.GAME_DATA_289_ENGINE || '/Users/acfrazier/experiments/lostcity-289/engine', contentRoot: process.env.GAME_DATA_289_CONTENT || '/Users/acfrazier/experiments/lostcity-289/content', cache: { cache_id: 'c4d8ab36bcfd2a7907535b4f619e28623b0a22e98d496fd2a9620d544c5b5b09', content_id: 'cdb2f161c35239f09bf5175648e15e7dbbc4bbf9be4419cea41f7053ccf8b044', nav_sha256: '131db92e32eddcb08148909d477589544320fe7e34a422e8004e97e888032924', flags_sha256: '67e4094dff06def5cf8abc172ce751f4ca8679532ba04c1ba15ab6bf668c7a4a' } }
};
const decoderSources = ['src/cache/config/ObjType.ts', 'src/cache/config/NpcType.ts', 'src/cache/config/ConfigType.ts', 'src/cache/config/ParamHelper.ts', 'src/cache/config/ParamType.ts', 'src/cache/config/ScriptVarType.ts', 'src/io/BZip2.ts', 'src/io/Jagfile.ts', 'src/io/Packet.ts', 'src/datastruct/DoublyLinkable.ts', 'src/datastruct/LinkList.ts', 'src/datastruct/Linkable.ts', 'src/util/Environment.ts', 'src/util/Logger.ts', 'src/util/TryParse.ts', 'src/util/WorldConfig.ts'];
const contentFiles = ['scripts/player/configs/consumption/consume.dbtable', 'scripts/player/configs/consumption/consume_normal.dbrow', 'scripts/player/configs/consumption/consume_effects.dbrow', 'scripts/skill_thieving/configs/pickpocking/pickpocket.dbtable', 'scripts/skill_thieving/configs/pickpocking/pickpocket.dbrow', 'scripts/player/scripts/consumption/effects/scripts/consume_effects.rs2', 'scripts/skill_combat/configs/magic/magic_combat_spells.dbrow', 'scripts/skill_magic/configs/magic.dbtable', 'scripts/skill_magic/configs/magic_spells.dbrow', 'scripts/skill_magic/configs/magic_staff.dbrow', 'scripts/skill_combat/configs/combat.constant', 'scripts/skill_herblore/configs/herbs.obj', 'scripts/skill_herblore/configs/identifying/identify.param', 'scripts/skill_herblore/scripts/identifying/identify.rs2', 'scripts/skill_prayer/configs/prayers.dbrow', 'scripts/skill_prayer/configs/prayers.constant', 'scripts/skill_prayer/interfaces/prayer.if', 'scripts/areas/area_falador/configs/dwarven_mine.inv', 'scripts/areas/area_falador/configs/dwarven_mine.npc', 'scripts/skill_runecraft/configs/runecraft.constant', 'maps/m45_75.jm2', 'pack/npc.pack', 'scripts/quests/quest_murder/configs/quest_murder.loc', 'scripts/general/configs/quest.enum', 'maps/m42_55.jm2', 'pack/loc.pack', 'pack/obj.pack', 'pack/interface.pack', 'pack/varp.pack', 'pack/param.pack', 'scripts/_unpack/225/all.npc', 'scripts/drop tables/scripts/giant.rs2', 'scripts/drop tables/scripts/moss_giant.rs2', 'scripts/drop tables/scripts/fire_giant.rs2', 'scripts/drop tables/scripts/green_dragon.rs2', 'scripts/drop tables/scripts/shared_droptables.rs2', ...gatherContentFiles, ...questIdentityContentFiles, ...trailContentFiles];
const expectedDropNames: Record<number, Record<string, string[]>> = {
    274: {
        Giant: ['Beer', 'Big bones', 'Body talisman', 'Chaos rune', 'Chaos talisman', 'Coins', 'Cosmic rune', 'Death rune', 'Dragon spear', 'Fire rune', 'Half of a key', 'Herb', 'Iron arrow', 'Iron dagger', 'Iron full helm', 'Iron kiteshield', 'Law rune', 'Limpwurt root', 'Mind rune', 'Nature rune', 'Nature talisman', 'Rune javelin', 'Rune spear', 'Shield left half', 'Steel arrow', 'Steel longsword', 'Uncut diamond', 'Uncut emerald', 'Uncut ruby', 'Uncut sapphire', 'Water rune'],
        'Moss giant': ['Air rune', 'Big bones', 'Black sq shield', 'Blood rune', 'Chaos rune', 'Chaos talisman', 'Coal', 'Coins', 'Cosmic rune', 'Death rune', 'Dragon spear', 'Earth rune', 'Half of a key', 'Herb', 'Iron arrow', 'Law rune', 'Magic staff', 'Mithril spear', 'Mithril sword', 'Nature rune', 'Nature talisman', 'Rune javelin', 'Rune spear', 'Shield left half', 'Spinach roll', 'Steel arrow', 'Steel bar', 'Steel kiteshield', 'Steel med helm', 'Uncut diamond', 'Uncut emerald', 'Uncut ruby', 'Uncut sapphire'],
        'Fire giant': ['Adamant javelin', 'Big bones', 'Blood rune', 'Chaos rune', 'Chaos talisman', 'Coins', 'Death rune', 'Dragon med helm', 'Dragon spear', 'Dragonstone', 'Fire battlestaff', 'Fire rune', 'Half of a key', 'Herb', 'Law rune', 'Lobster', 'Mithril sq shield', 'Nature rune', 'Nature talisman', 'Rune 2h sword', 'Rune arrow', 'Rune battleaxe', 'Rune javelin', 'Rune kiteshield', 'Rune scimitar', 'Rune spear', 'Rune sq shield', 'Runite bar', 'Shield left half', 'Silver ore', 'Steel arrow', 'Steel axe', 'Steel bar', 'Strength potion(2)', 'Uncut diamond', 'Uncut emerald', 'Uncut ruby', 'Uncut sapphire'],
        'Green dragon': ['Adamant full helm', 'Adamantite ore', 'Bass', 'Chaos talisman', 'Coins', 'Dragon bones', 'Dragon spear', 'Dragonhide', 'Fire rune', 'Half of a key', 'Herb', 'Law rune', 'Mithril axe', 'Mithril kiteshield', 'Mithril spear', 'Nature rune', 'Nature talisman', 'Rune dagger', 'Rune javelin', 'Rune spear', 'Shield left half', 'Steel battleaxe', 'Steel platelegs', 'Uncut diamond', 'Uncut emerald', 'Uncut ruby', 'Uncut sapphire', 'Water rune'],
    },
    289: {
        Giant: ['Beer', 'Big bones', 'Body talisman', 'Chaos rune', 'Chaos talisman', 'Coins', 'Cosmic rune', 'Death rune', 'Dragon spear', 'Fire rune', 'Half of a key', 'Herb', 'Iron arrow', 'Iron dagger', 'Iron full helm', 'Iron kiteshield', 'Law rune', 'Limpwurt root', 'Mind rune', 'Nature rune', 'Nature talisman', 'Rune javelin', 'Rune spear', 'Shield left half', 'Steel arrow', 'Steel longsword', 'Uncut diamond', 'Uncut emerald', 'Uncut ruby', 'Uncut sapphire', 'Water rune'],
        'Moss giant': ['Air rune', 'Big bones', 'Black sq shield', 'Blood rune', 'Chaos rune', 'Chaos talisman', 'Coal', 'Coins', 'Cosmic rune', 'Death rune', 'Dragon spear', 'Earth rune', 'Half of a key', 'Herb', 'Iron arrow', 'Law rune', 'Magic staff', 'Mithril spear', 'Mithril sword', 'Nature rune', 'Nature talisman', 'Rune javelin', 'Rune spear', 'Shield left half', 'Spinach roll', 'Steel arrow', 'Steel bar', 'Steel kiteshield', 'Steel med helm', 'Uncut diamond', 'Uncut emerald', 'Uncut ruby', 'Uncut sapphire'],
        'Fire giant': ['Adamant javelin', 'Big bones', 'Blood rune', 'Chaos rune', 'Chaos talisman', 'Coins', 'Death rune', 'Dragon med helm', 'Dragon spear', 'Dragonstone', 'Fire battlestaff', 'Fire rune', 'Half of a key', 'Herb', 'Law rune', 'Lobster', 'Mithril sq shield', 'Nature rune', 'Nature talisman', 'Rune 2h sword', 'Rune arrow', 'Rune battleaxe', 'Rune javelin', 'Rune kiteshield', 'Rune scimitar', 'Rune spear', 'Rune sq shield', 'Runite bar', 'Shield left half', 'Silver ore', 'Steel arrow', 'Steel axe', 'Steel bar', 'Strength potion(2)', 'Uncut diamond', 'Uncut emerald', 'Uncut ruby', 'Uncut sapphire'],
        'Green dragon': ['Adamant full helm', 'Adamantite ore', 'Bass', 'Chaos talisman', 'Coins', 'Dragon bones', 'Dragon spear', 'Dragonhide', 'Fire rune', 'Half of a key', 'Herb', 'Law rune', 'Mithril axe', 'Mithril kiteshield', 'Mithril spear', 'Nature rune', 'Nature talisman', 'Rune dagger', 'Rune javelin', 'Rune spear', 'Shield left half', 'Steel battleaxe', 'Steel platelegs', 'Uncut diamond', 'Uncut emerald', 'Uncut ruby', 'Uncut sapphire', 'Water rune'],
    },
};
function digest(file: string) { const data = fs.readFileSync(file); return { bytes: data.length, sha256: crypto.createHash('sha256').update(data).digest('hex') }; }
function commit(dir: string) { return execFileSync('git', ['-C', dir, 'rev-parse', 'HEAD'], { encoding: 'utf8' }).trim(); }
function assertEqual(actual: unknown, expectedValue: unknown, label: string) { if (actual !== expectedValue) throw new Error(`${label}: expected ${expectedValue}, got ${actual}`); }
/** One published item row, in the writer's decoded shape. Read under the checks below. */
type PublishedItemRow = { alias: string | null; id: number; name: string | null; cost: number; stackable: boolean; members: boolean; certificate_link: number; certificate_template: number; wear_position: number; wear_position_2: number; wear_position_3: number; tradeable: boolean; stack_variant: boolean };
/** One published talk_key step spawn. `plane` is the scene plane, never `level`. */
type PublishedTalkKeySpawn = { x: number; z: number; plane: number };
/** One published talk_key talk step. */
type PublishedTalkKeyTalk = { alias: string; id: number; npc: { alias: string; id: number; name: string }; spawn?: PublishedTalkKeySpawn };
/** One published talk_key keeper, read as a union of optional fields and checked by `kind`. */
type PublishedTalkKeyKeeper = { kind: string; alias?: string; id?: number; name?: string; category?: string };
/** One published talk_key key-keeper step. */
type PublishedTalkKeyKey = { alias: string; id: number; key_alias: string; key_id: number; keeper: PublishedTalkKeyKeeper; spawn?: PublishedTalkKeySpawn };
/** One published talk_key unknown-spawn record. */
type PublishedTalkKeyCoverage = { class: string; family: string; alias: string; reason: string };
/** The published talk_key family object. */
type PublishedTalkKey = { talk: PublishedTalkKeyTalk[]; keys: PublishedTalkKeyKey[]; coverage: PublishedTalkKeyCoverage[] };
/** One published trio_givers spawn. `plane` is the scene plane, never `level`. */
type PublishedTrioGiverSpawn = { x: number; z: number; plane: number };
/** One published giver: the packed npc identity itself, with `spawn` omitted when it is not unique. */
type PublishedTrioGiverRow = { alias: string; id: number; name: string; spawn?: PublishedTrioGiverSpawn };
/** Why one published giver carries no world tile. */
type PublishedTrioGiverCoverage = { class: string; family: string; alias: string; reason: string };
/** The published trio_givers family object. */
type PublishedTrioGivers = { rows: PublishedTrioGiverRow[]; coverage: PublishedTrioGiverCoverage[] };
const manifest = JSON.parse(fs.readFileSync(path.join(root, 'crates/api/data/game-data/manifest.json'), 'utf8')) as { schema_version: number; revisions: any[] };
assertEqual(manifest.schema_version, 4, 'manifest schema');
const results = [];
const publishedTrioGivers = new Map<number, PublishedTrioGivers>();
for (const revision of [274, 289]) {
    const file = path.join(root, `crates/api/data/game-data/${revision}.json`); const payload = JSON.parse(fs.readFileSync(file, 'utf8')) as any; const pin = expected[revision]; const manifestRow = manifest.revisions.find((entry) => entry.revision === revision); if (!manifestRow) throw new Error(`${revision}: missing manifest row`);
    assertEqual(payload.schema_version, 4, `${revision} schema`); assertEqual(payload.revision, revision, `${revision} revision`); assertEqual(payload.provenance.engine_commit, pin.engine, `${revision} engine pin`); assertEqual(payload.provenance.content_commit, pin.content, `${revision} content pin`); assertEqual(commit(pin.engineRoot), pin.engine, `${revision} live engine commit`); assertEqual(commit(pin.contentRoot), pin.content, `${revision} live content commit`);
    verifyCacheIdentity(revision, pin.engineRoot, pin.cache);
    const sourcePaths = [...decoderSources, 'data/pack/server/obj.dat', 'data/pack/server/npc.dat', 'data/pack/client/config']; const dirtyEngine = execFileSync('git', ['-C', pin.engineRoot, 'status', '--porcelain', '--untracked-files=all', '--', ...sourcePaths], { encoding: 'utf8' }).trim(); if (dirtyEngine) throw new Error(`${revision}: relevant engine inputs are dirty: ${dirtyEngine}`); const dirtyContent = execFileSync('git', ['-C', pin.contentRoot, 'status', '--porcelain', '--untracked-files=all', '--', ...contentFiles, 'maps'], { encoding: 'utf8' }).trim(); if (dirtyContent) throw new Error(`${revision}: relevant content inputs are dirty: ${dirtyContent}`);
    assertEqual(JSON.stringify(payload.provenance.content_inputs.map((input: any) => input.path)), JSON.stringify(contentFiles), `${revision} complete content provenance`);
    for (const input of [...payload.provenance.inputs, ...payload.provenance.decoder_sources, ...payload.provenance.content_inputs]) { const base = payload.provenance.content_inputs.includes(input) ? pin.contentRoot : pin.engineRoot; const actual = digest(path.join(base, input.path)); assertEqual(actual.bytes, input.bytes, `${revision} ${input.path} bytes`); assertEqual(actual.sha256, input.sha256, `${revision} ${input.path} hash`); }
    const output = digest(file); assertEqual(output.bytes, manifestRow.bytes, `${revision} output bytes`); assertEqual(output.sha256, manifestRow.sha256, `${revision} output hash`); assertEqual(JSON.stringify(payload.provenance.cache_identity), JSON.stringify(pin.cache), `${revision} cache identity`);
    const byAlias = new Map(payload.items.filter((item: any) => item.alias !== null).map((item: any) => [item.alias, item])); const plate = byAlias.get('rune_platebody'); const chain = byAlias.get('rune_chainbody'); if (!plate || plate.name !== 'Rune platebody' || !chain || chain.name !== 'Rune chainbody' || plate.cost <= chain.cost) throw new Error(`${revision}: Rune platebody/chainbody value order`); if (payload.items.filter((item: any) => item.name === 'Dragonhide').length < 2) throw new Error(`${revision}: same-name Dragonhide identity`); if (new Set(payload.items.map((item: any) => item.id)).size !== payload.items.length || new Set(payload.items.map((item: any) => item.alias)).size !== payload.items.length) throw new Error(`${revision}: duplicate IDs or aliases`);
    const fixed = new Map(payload.consumption.filter((fact: any) => fact.qualification === 'fixed_hp_heal').map((fact: any) => [fact.item.alias, fact.stat_heal[0].base])); for (const [alias, heal] of [['lobster', 12], ['bread', 4], ['anchovies', 3]] as const) if (fixed.get(alias) !== heal) throw new Error(`${revision}: ${alias} fixed heal mismatch`); const guard = payload.pickpocket.find((fact: any) => fact.npcs.some((npc: any) => npc.alias === 'guard1')); if (!guard || guard.level !== 40) throw new Error(`${revision}: Guard thieving level mismatch`); if (payload.pickpocket.some((fact: any) => fact.npcs.length === 0 || fact.loot.some((loot: any) => loot.min > loot.max))) throw new Error(`${revision}: invalid pickpocket joins`);
    const drops = payload.drop_tables ?? [];
    assertEqual(drops.length, 4, `${revision} bounded drop row count`);
    assertEqual(JSON.stringify(drops.map((row: any) => row.name)), JSON.stringify(['Giant', 'Moss giant', 'Fire giant', 'Green dragon']), `${revision} bounded drop target order`);
    for (const row of drops) {
        const expectedNames = expectedDropNames[revision][row.name];
        if (!expectedNames) throw new Error(`${revision}: unexpected drop row ${row.name}`);
        assertEqual(JSON.stringify(row.display_names), JSON.stringify(expectedNames), `${revision} ${row.name} exact display names`);
        const joinedNames = [...new Set(row.items.map((item: any) => item.name))].sort((a: string, b: string) => a.localeCompare(b));
        assertEqual(JSON.stringify(row.display_names), JSON.stringify(joinedNames), `${revision} ${row.name} display-name projection`);
        for (const item of row.items) {
            const joined = byAlias.get(item.alias) as any;
            if (!joined || joined.id !== item.id || joined.name !== item.name) throw new Error(`${revision}: ${row.name} item join ${JSON.stringify(item)}`);
        }
    }
    const greenDrops = drops.find((row: any) => row.name === 'Green dragon');
    if (!greenDrops?.items.some((item: any) => item.alias === 'dragonhide_green' && item.id === 1753 && item.name === 'Dragonhide')) throw new Error(`${revision}: Green dragonhide alias/id evidence`);
    if (greenDrops.display_names.includes('Bones') || !greenDrops.display_names.includes('Dragonhide')) throw new Error(`${revision}: Green dragon invented Bones or missing Dragonhide`);
    const spells = payload.spells ?? [];
    if (spells.length !== 16 || spells[0]?.name !== 'Wind Strike' || spells[15]?.name !== 'Fire Wave') throw new Error(`${revision}: named autocast combat spells`);
    const wind = spells.find((spell: any) => spell.name === 'Wind Strike');
    if (!wind || wind.ssb !== 0 || wind.level !== 1 || JSON.stringify(wind.runes.map((rune: any) => [rune.name, rune.count])) !== JSON.stringify([['Mind rune', 1], ['Air rune', 1]])) throw new Error(`${revision}: Wind Strike runes`);
    const fireWave = spells.find((spell: any) => spell.name === 'Fire Wave');
    if (!fireWave || fireWave.ssb !== 15 || JSON.stringify(fireWave.runes.map((rune: any) => [rune.name, rune.count])) !== JSON.stringify([['Blood rune', 1], ['Fire rune', 7], ['Air rune', 5]])) throw new Error(`${revision}: Fire Wave runes`);
    const staves = payload.staves ?? [];
    if (staves.length !== 14) throw new Error(`${revision}: expected 14 staves`);
    const fireProviders = staves.filter((staff: any) => staff.runes.some((rune: any) => rune.name === 'Fire rune')).map((staff: any) => staff.name).sort();
    if (JSON.stringify(fireProviders) !== JSON.stringify(['Fire battlestaff', 'Lava battlestaff', 'Mystic fire staff', 'Mystic lava staff', 'Staff of fire'])) throw new Error(`${revision}: fire staff providers ${fireProviders}`);
    const lava = staves.find((staff: any) => staff.name === 'Lava battlestaff');
    if (!lava || JSON.stringify(lava.runes.map((rune: any) => rune.name).sort()) !== JSON.stringify(['Earth rune', 'Fire rune'])) throw new Error(`${revision}: lava staff runes`);
    if (staves.some((staff: any) => staff.name === 'Staff of air' && staff.runes.some((rune: any) => rune.name === 'Fire rune'))) throw new Error(`${revision}: Staff of air is not a fire provider`);
    const autocast = payload.autocast;
    if (!autocast || autocast.staff_tab_root !== 328 || autocast.choose_com !== 353 || autocast.spell_panel_root !== 1829 || autocast.spell_grid_base !== 1830 || autocast.toggle_com !== 349 || autocast.magic_varp !== 108 || autocast.selected_value !== 2 || autocast.armed_value !== 3) throw new Error(`${revision}: autocast controls ${JSON.stringify(autocast)}`);
    const duel = payload.duel;
    if (!duel || duel.select_modal !== 6575 || duel.confirm_modal !== 6412 || duel.win_modal !== 6733 || duel.select_accept !== 6674 || duel.confirm_accept !== 6520 || duel.select_partner !== 6671 || duel.select_status !== 6684 || duel.confirm_status !== 6571) throw new Error(`${revision}: duel controls ${JSON.stringify(duel)}`);
    const special = payload.special;
    if (!special || special.energy_varp !== 300 || special.armed_varp !== 301 || special.armed_value !== 1 || special.max_energy !== 1000 || special.arm_confirm_ticks !== 2) throw new Error(`${revision}: special controls ${JSON.stringify({ energy_varp: special?.energy_varp, armed_varp: special?.armed_varp, max_energy: special?.max_energy })}`);
    const bars = new Map((special.bars ?? []).map((bar: any) => [bar.root, bar]));
    if (bars.get('combat_blunt')?.root_id !== 425 || bars.get('combat_blunt')?.bar !== 7462 || bars.get('combat_hacksword')?.root_id !== 2423 || bars.get('combat_hacksword')?.bar !== 7587 || bars.get('combat_polearm')?.root_id !== 8460 || bars.get('combat_polearm')?.bar !== 8481 || bars.has('combat_staff_2')) throw new Error(`${revision}: special bars ${JSON.stringify(special.bars)}`);
    const byName = new Map((special.weapons ?? []).map((weapon: any) => [weapon.name, weapon.cost]));
    if (byName.get('Dragon dagger') !== 250 || byName.get('Dragon dagger(p)') !== 250 || byName.get('Magic shortbow') !== 350 || byName.get('Rune thrownaxe') !== 100 || byName.get('Dragon halberd') !== 300 || byName.has('Dragon battleaxe') || byName.has('Excalibur') || byName.has('Rune scimitar')) throw new Error(`${revision}: special weapons ${JSON.stringify(special.weapons)}`);
    const teleports = payload.teleports ?? [];
    if (teleports.length !== 7 || teleports[0]?.name !== 'Varrock' || teleports[6]?.name !== 'Trollheim') throw new Error(`${revision}: expected 7 teleports`);
    const varrock = teleports.find((row: any) => row.name === 'Varrock');
    if (!varrock || varrock.component_id !== 1164 || varrock.level !== 25 || varrock.experience !== 350 || varrock.members !== false || varrock.x !== 3213 || varrock.z !== 3424 || varrock.plane !== 0 || JSON.stringify(varrock.runes.map((rune: any) => [rune.name, rune.count])) !== JSON.stringify([['Fire rune', 1], ['Air rune', 3], ['Law rune', 1]])) throw new Error(`${revision}: Varrock teleport ${JSON.stringify(varrock)}`);
    const falador = teleports.find((row: any) => row.name === 'Falador');
    if (!falador || falador.component_id !== 1170 || falador.level !== 37) throw new Error(`${revision}: Falador teleport`);
    if (teleports.some((row: any) => row.spell === 'highlvl_alchemy' || row.spell === 'wind_strike')) throw new Error(`${revision}: target spells are not teleports`);
    const herbs = payload.herbs ?? [];
    if (herbs.length < 14) throw new Error(`${revision}: expected herb identify table, got ${herbs.length}`);
    if (payload.herb_level_default !== 3) throw new Error(`${revision}: herb_level_default ${payload.herb_level_default}`);
    const guam = herbs.find((herb: any) => herb.key === 'guam');
    if (!guam || guam.name !== 'Guam leaf' || guam.id !== 249 || guam.unidId !== 199 || guam.level !== 3 || guam.level_source !== 'identified_herb_level_default') throw new Error(`${revision}: guam herb ${JSON.stringify(guam)}`);
    const marrentill = herbs.find((herb: any) => herb.key === 'marrentill');
    if (!marrentill || marrentill.level !== 5 || marrentill.level_source !== 'identified_herb_level') throw new Error(`${revision}: marrentill ${JSON.stringify(marrentill)}`);
    const snake = herbs.find((herb: any) => herb.key === 'snake weed');
    if (!snake || snake.level !== 3 || snake.level_source !== 'identified_herb_level' || snake.unidId !== 1525 || snake.id !== 1526) throw new Error(`${revision}: snake weed ${JSON.stringify(snake)}`);
    const prayers = payload.prayers ?? [];
    if (prayers.length !== 15) throw new Error(`${revision}: expected 15 prayers, got ${prayers.length}`);
    const thick = prayers[0];
    const melee = prayers[14];
    if (!thick || thick.name !== 'Thick Skin' || thick.level !== 1 || thick.button_com !== 5609 || thick.varp !== 83 || thick.varp_alias !== 'prayer0') throw new Error(`${revision}: prayer anchor ${JSON.stringify(thick)}`);
    if (!melee || melee.name !== 'Protect from Melee' || melee.level !== 43 || melee.button_com !== 5623 || melee.varp !== 97 || melee.varp_alias !== 'prayer14') throw new Error(`${revision}: prayer tail ${JSON.stringify(melee)}`);
    for (let index = 0; index < prayers.length; index += 1) {
        const row = prayers[index];
        if (row.button_com !== 5609 + index || row.varp !== 83 + index) throw new Error(`${revision}: prayer com/varp sequence ${JSON.stringify(row)}`);
        if (!row.com_alias?.startsWith('prayer:prayer_')) throw new Error(`${revision}: bad com alias ${JSON.stringify(row)}`);
    }
    const burst = prayers.find((row: any) => row.name === 'Burst of Strength');
    if (!burst || burst.button_com !== 5610 || burst.varp !== 84 || burst.level !== 4) throw new Error(`${revision}: Burst of Strength ${JSON.stringify(burst)}`);
    const nurmof = payload.nurmof_essence;
    if (!nurmof || nurmof.npc_id !== 594 || nurmof.npc_alias !== 'nurmof' || nurmof.shop_inv !== 'pickaxeshop') throw new Error(`${revision}: nurmof essence ${JSON.stringify(nurmof)}`);
    if (nurmof.pickaxes.length !== 6) throw new Error(`${revision}: expected six pickaxes`);
    const iron = nurmof.pickaxes.find((row: any) => row.alias === 'iron_pickaxe');
    if (!iron || iron.id !== 1267 || iron.base_cost !== 140 || iron.name !== 'Iron pickaxe') throw new Error(`${revision}: iron pickaxe ${JSON.stringify(iron)}`);
    const rune = nurmof.pickaxes.find((row: any) => row.alias === 'rune_pickaxe');
    if (!rune || rune.base_cost !== 32000) throw new Error(`${revision}: rune pickaxe cost ${JSON.stringify(rune)}`);
    if (nurmof.essence_region.mapsquare_mx !== 45 || nurmof.essence_region.mapsquare_mz !== 75) throw new Error(`${revision}: essence mapsquare ${JSON.stringify(nurmof.essence_region)}`);
    if (!nurmof.essence_region.mapsquare_from_filename || nurmof.essence_region.mine_portal_loc_id !== 2492 || nurmof.essence_region.mine_portal_loc_placements < 1) throw new Error(`${revision}: essence mine portal evidence ${JSON.stringify(nurmof.essence_region)}`);
    if (nurmof.essence_region.source_constant !== undefined) throw new Error(`${revision}: essence region must not use source_constant as mine authority`);
    if (nurmof.essence_region.example_inside.x !== 2880 || nurmof.essence_region.example_inside.z !== 4800) throw new Error(`${revision}: essence example ${JSON.stringify(nurmof.essence_region.example_inside)}`);
    if (nurmof.curated_vendor_tactics.label !== 'curated' || nurmof.curated_vendor_tactics.stand.z !== 9844) throw new Error(`${revision}: curated vendor ${JSON.stringify(nurmof.curated_vendor_tactics)}`);
    if (!nurmof.aubury_travel.already_packed || nurmof.aubury_travel.npc_id !== 553 || nurmof.aubury_travel.return_anchor_role !== 'overworld_exit_after_mine_teleport') throw new Error(`${revision}: aubury travel ${JSON.stringify(nurmof.aubury_travel)}`);
    const flour = payload.flour_six;
    if (!flour || flour.quest_name !== 'Murder Mystery') throw new Error(`${revision}: flour quest ${JSON.stringify(flour)}`);
    if (flour.pot.id !== 1931 || flour.pot_flour.id !== 1933 || flour.flour_barrel.id !== 2662) throw new Error(`${revision}: flour ids ${JSON.stringify(flour)}`);
    const objectTile = flour.flour_barrel_object_tile;
    if (!objectTile || objectTile.role !== 'loc_placement' || objectTile.provenance !== 'derived' || objectTile.x !== 2735 || objectTile.z !== 3582) throw new Error(`${revision}: flour object tile ${JSON.stringify(objectTile)}`);
    const approachTile = flour.flour_barrel_approach_tile;
    if (!approachTile || approachTile.role !== 'interaction_near' || approachTile.provenance !== 'curated' || approachTile.x !== 2735 || approachTile.z !== 3581) throw new Error(`${revision}: flour approach tile ${JSON.stringify(approachTile)}`);
    if (flour.bank_tile.provenance !== 'curated' || flour.bank_tile.x !== 2725 || flour.bank_tile.z !== 3491) throw new Error(`${revision}: flour bank tile ${JSON.stringify(flour.bank_tile)}`);
    const joinedPot = byAlias.get('pot_empty') as any;
    if (!joinedPot || joinedPot.id !== flour.pot.id) throw new Error(`${revision}: flour pot join`);
    const equipment = payload.equipment_names;
    if (!equipment) throw new Error(`${revision}: missing equipment_names`);
    const curatedMembership = loadEquipmentNamesCurated();
    const frozenFamilies = parseFrozenEquipmentNameArrays(fs.readFileSync(path.join(root, 'tools/game-data/equipment-names.frozen.ts'), 'utf8'));
    const objPack = parsePack(fs.readFileSync(path.join(pin.contentRoot, 'pack/obj.pack'), 'utf8'));
    if (objPack.size === 0) throw new Error(`${revision}: empty pack/obj.pack`);
    assertEqual(equipment.equipment_source.sha256, 'ec2ab37311b6373046626f08777ebbbf5f86590e6599c7a3f906acedc79151d3', `${revision} equipment.ts pin`);
    assertEqual(equipment.equipment_source.bytes, 2360, `${revision} equipment.ts bytes`);
    assertEqual(equipment.equipment_evidence?.sha256, 'ec2ab37311b6373046626f08777ebbbf5f86590e6599c7a3f906acedc79151d3', `${revision} equipment evidence pin`);
    assertEqual(equipment.equipment_evidence?.path, 'tools/game-data/equipment-names.frozen.ts', `${revision} equipment evidence path`);
    if (!equipment.exact_name_join || equipment.exact_name_join.matching !== 'exact_display_name_only' || equipment.exact_name_join.bolts?.generic_is_substitute !== false || equipment.exact_name_join.bolts?.generic_display_name !== 'Bolts') {
        throw new Error(`${revision}: exact_name_join limitation missing ${JSON.stringify(equipment.exact_name_join)}`);
    }
    const familyCounts = {
        bows: 12,
        crossbows: 10,
        darts: 7,
        arrows: 7,
        bolts: 9,
        melee_weapons: 33,
        staffs: 15,
    };
    for (const [family, count] of Object.entries(familyCounts)) {
        const rows = equipment[family] ?? [];
        const expectedNames = frozenFamilies[family as keyof typeof frozenFamilies];
        assertEqual(rows.length, count, `${revision} equipment ${family} row count summary`);
        assertEqual(JSON.stringify(curatedMembership.families[family as keyof typeof curatedMembership.families]), JSON.stringify(expectedNames), `${revision} curated vs independently parsed frozen ${family}`);
        assertEqual(JSON.stringify(rows.map((row: any) => row.requested_name)), JSON.stringify(expectedNames), `${revision} equipment ${family} order vs frozen evidence`);
        if (rows.some((row: any) => row.disposition === 'ambiguous')) throw new Error(`${revision}: equipment ${family} must not publish ambiguous rows`);
    }
    const resolved = ['bows', 'crossbows', 'darts', 'arrows', 'bolts', 'melee_weapons', 'staffs']
        .flatMap((family) => equipment[family])
        .filter((row: any) => row.disposition === 'resolved');
    const absent = ['bows', 'crossbows', 'darts', 'arrows', 'bolts', 'melee_weapons', 'staffs']
        .flatMap((family) => equipment[family])
        .filter((row: any) => row.disposition === 'absent');
    assertEqual(resolved.length, 74, `${revision} equipment resolved count summary`);
    assertEqual(absent.length, 19, `${revision} equipment absent count summary`);
    const shortbow = equipment.bows.find((row: any) => row.requested_name === 'Shortbow');
    if (!shortbow || shortbow.disposition !== 'resolved' || shortbow.alias !== 'shortbow' || shortbow.id !== 841) throw new Error(`${revision}: Shortbow join ${JSON.stringify(shortbow)}`);
    const blackDagger = equipment.melee_weapons.find((row: any) => row.requested_name === 'Black dagger');
    if (!blackDagger || blackDagger.disposition !== 'resolved' || blackDagger.alias !== 'black_dagger' || blackDagger.id !== 1217 || blackDagger.disambiguation !== 'prefer_standard_pack_alias_black_dagger') {
        throw new Error(`${revision}: Black dagger join ${JSON.stringify(blackDagger)}`);
    }
    if (objPack.get('black_dagger') !== 1217 || objPack.get('deathdagger') !== 746) {
        throw new Error(`${revision}: black dagger pack aliases ${objPack.get('black_dagger')}/${objPack.get('deathdagger')}`);
    }
    const dragonArrow = equipment.arrows.find((row: any) => row.requested_name === 'Dragon arrow');
    if (!dragonArrow || dragonArrow.disposition !== 'absent' || dragonArrow.absent_class !== 'no_exact_selected_match') throw new Error(`${revision}: Dragon arrow absent ${JSON.stringify(dragonArrow)}`);
    const bronzeBolt = equipment.bolts.find((row: any) => row.requested_name === 'Bronze bolts');
    if (!bronzeBolt || bronzeBolt.disposition !== 'absent' || bronzeBolt.absent_class !== 'no_exact_selected_match') throw new Error(`${revision}: Bronze bolts absent ${JSON.stringify(bronzeBolt)}`);
    const bronzeCrossbow = equipment.crossbows.find((row: any) => row.requested_name === 'Bronze crossbow');
    if (!bronzeCrossbow || bronzeCrossbow.disposition !== 'absent' || bronzeCrossbow.absent_class !== 'no_exact_selected_match') throw new Error(`${revision}: Bronze crossbow absent ${JSON.stringify(bronzeCrossbow)}`);
    const karil = equipment.crossbows.find((row: any) => row.requested_name === "Karil's crossbow");
    if (!karil || karil.disposition !== 'absent' || karil.absent_class !== 'no_exact_selected_match') throw new Error(`${revision}: Karil's crossbow ${JSON.stringify(karil)}`);
    if (equipment.crossbows.some((row: any) => row.requested_name === 'Karil\\' || row.requested_name === "Karil\\")) throw new Error(`${revision}: malformed Karil membership leaked`);
    if (equipment.bolts.some((row: any) => row.requested_name === 'Bolts' || row.selected_name === 'Bolts' || row.alias === 'bolt')) {
        throw new Error(`${revision}: generic Bolts must not map onto frozen bolt tiers`);
    }
    const genericBolts = payload.items.filter((item: any) => item.name === 'Bolts');
    if (genericBolts.length === 0) throw new Error(`${revision}: selected table must contain generic Bolts so the non-mapping is testable`);
    for (const row of resolved) {
        if (!row.alias || row.id === undefined) throw new Error(`${revision}: resolved equipment missing join ${JSON.stringify(row)}`);
        const joined = payload.items.find((item: any) => item.alias === row.alias);
        if (!joined || joined.id !== row.id || joined.name !== row.selected_name) throw new Error(`${revision}: equipment join drift ${JSON.stringify(row)}`);
        const packed = objPack.get(row.alias);
        if (packed !== row.id) throw new Error(`${revision}: equipment obj.pack mismatch ${JSON.stringify(row)} pack=${packed}`);
    }
    const gather = payload.gather_methods;
    if (!gather || Array.isArray(gather)) throw new Error(`${revision}: gather_methods must be one family object, not a bare list`);
    const extracted = extractGatherMethodsFacts(pin.contentRoot, revision);
    assertEqual(JSON.stringify(gather), JSON.stringify(extracted), `${revision} gather_methods matches the writer extract`);
    assertEqual(gather.mining.length, 17, `${revision} mine tables`);
    assertEqual(gather.woods.length, 10, `${revision} tree tables`);
    assertEqual(gather.fishing.length, 9, `${revision} fishing method rows`);
    if (gather.mining.some((row: any) => row.publication !== undefined || row.published_for_placement !== undefined || row.resource_key === 'Rocks')) throw new Error(`${revision}: mining must not publish placement or use display name Rocks`);
    if (gather.woods.some((row: any) => row.resource_key === 'Tree')) throw new Error(`${revision}: Tree is not a wood key`);
    if (gather.fishing.some((row: any) => row.loc_ids || row.fishing_movement_enum || row.category === 'Fishing spot' || row.level !== null || row.output !== null || row.qualification !== 'partial')) throw new Error(`${revision}: fishing rows must be type plus actions, not a spawn tile`);
    assertEqual(JSON.stringify(gather.woods.filter((row: any) => row.publication === 'published').map((row: any) => row.resource_key)), JSON.stringify(['normal', 'oak', 'willow', 'maple', 'yew', 'magic']), `${revision} published woods`);
    assertEqual(JSON.stringify(gather.woods.filter((row: any) => row.publication === 'conditional').map((row: any) => row.resource_key)), JSON.stringify(['achey', 'hollow']), `${revision} conditional woods`);
    assertEqual(JSON.stringify(gather.woods.filter((row: any) => row.publication === 'unpublished').map((row: any) => row.resource_key)), JSON.stringify(['jungle', 'burnt']), `${revision} unpublished woods`);
    assertEqual(gather.mining.filter((row: any) => row.resource_key === 'limestone').length, 3, `${revision} limestone rows`);
    const gem = gather.mining.find((row: any) => row.table === 'gem_rock');
    if (!gem || gem.resource_key !== 'gems' || gem.output !== null || gem.qualification !== 'partial') throw new Error(`${revision}: gem_rock ${JSON.stringify(gem)}`);
    const essence = gather.mining.find((row: any) => row.table === 'rune_essence_table');
    if (!essence || essence.output?.alias !== 'blankrune' || essence.output?.id !== 1436) throw new Error(`${revision}: rune essence ${JSON.stringify(essence?.output)}`);
    const copper = gather.mining.find((row: any) => row.table === 'copper_rock_table');
    if (!copper || copper.loc_ids.find((loc: any) => loc.alias === 'copperrock1')?.id !== 2090 || copper.loc_ids.length <= 2) throw new Error(`${revision}: copper table ${JSON.stringify(copper?.loc_ids)}`);
    const ironRock = gather.mining.find((row: any) => row.table === 'iron_rock_table');
    if (!ironRock || ironRock.loc_ids.find((loc: any) => loc.alias === 'ironrock1')?.id !== 2092 || ironRock.loc_ids.find((loc: any) => loc.alias === 'ironrock2')?.id !== 2093) throw new Error(`${revision}: iron rocks`);
    const desert = gather.mining.find((row: any) => row.table === 'desertrescue_rock');
    if (!desert || desert.resource_key !== 'rock' || desert.empty_ids.length !== 0) throw new Error(`${revision}: desertrescue ${JSON.stringify(desert)}`);
    const lavaEel = gather.fishing.find((row: any) => row.category === 'unknown');
    if (!lavaEel || lavaEel.primary_op !== 'Bait' || lavaEel.pair_op !== 'hidden') throw new Error(`${revision}: lava eel ${JSON.stringify(lavaEel)}`);
    if (gather.fishing.filter((row: any) => row.category === 'memberfish').length !== 1) throw new Error(`${revision}: memberfish must dedupe to one signature`);
    if (JSON.stringify(gather.coverage.filter((row: any) => row.class === 'conditional').map((row: any) => row.resource_key).sort()) !== JSON.stringify(['achey', 'hollow'])) throw new Error(`${revision}: conditional coverage`);
    const locIds = [...gather.mining, ...gather.woods].flatMap((row: any) => [...row.loc_ids, ...row.empty_ids].map((loc: any) => loc.id));
    if (revision === 274) {
        if (locIds.includes(5083) || locIds.includes(5084)) throw new Error('274 copied 289-only locs');
        assertEqual(gather.coverage.filter((row: any) => row.class === 'revision-absent').map((row: any) => `${row.alias}:${row.other_pin_id}:${row.copied}`).join(','), 'dungeon_tree_closed:5083:false,karam_dungeon_exit:5084:false', '274 revision-absent coverage');
        if (gather.mining.find((row: any) => row.table === 'coal_rock_table').loc_ids.some((loc: any) => loc.alias === 'misc_dummy_coalrock1')) throw new Error('274 copied 289 coal dummy');
    } else {
        if (gather.coverage.some((row: any) => row.class === 'revision-absent')) throw new Error('289 must not record 274 revision-absent rows');
        if (!gather.mining.find((row: any) => row.table === 'coal_rock_table').loc_ids.some((loc: any) => loc.alias === 'misc_dummy_coalrock1' && loc.id === 4676)) throw new Error('289 coal dummy missing from loc_ids');
        const normal = gather.woods.find((row: any) => row.resource_key === 'normal');
        if (!normal.loc_ids.some((loc: any) => loc.alias === 'mm_bush_kharazi_jungle_tree1') || normal.empty_ids.some((loc: any) => String(loc.alias).includes('mm_bush'))) throw new Error('289 jungle bush transform invented');
    }
    const placements = payload.gather_placements;
    if (!placements || Array.isArray(placements) || !Array.isArray(placements.rows)) throw new Error(`${revision}: gather_placements must be one family object, not a bare list`);
    const extractedPlacements = extractGatherPlacementsFacts(pin.contentRoot, extracted.woods);
    assertEqual(JSON.stringify(placements), JSON.stringify(extractedPlacements.facts), `${revision} gather_placements matches the writer extract`);
    assertEqual(JSON.stringify(payload.provenance.placement_inputs), JSON.stringify(extractedPlacements.inputs), `${revision} placement inputs match the maps, pack, and published set`);
    assertEqual(JSON.stringify(extractedPlacements.woods.map((row) => row.resource_key)), JSON.stringify(['normal', 'oak', 'willow', 'maple', 'yew', 'magic']), `${revision} published wood order`);
    if (extractedPlacements.woods.some((row) => row.placements === 0)) throw new Error(`${revision}: every published wood needs placements`);
    assertEqual(placements.rows.length, extractedPlacements.woods.reduce((sum, row) => sum + row.placements, 0), `${revision} placement row total`);
    assertEqual(JSON.stringify(placements.coverage), JSON.stringify([{ class: 'unknown', family: 'mining', reason: 'no selected published-ore set' }]), `${revision} mining unknown coverage`);
    const placementIds = new Set(gather.woods.filter((row: any) => row.publication === 'published').flatMap((row: any) => row.loc_ids.map((loc: any) => loc.id)));
    if (placements.rows.some((row: any) => Object.keys(row).join(',') !== 'loc_id,x,z,plane')) throw new Error(`${revision}: placement rows are world loc_id/x/z/plane only`);
    if (placements.rows.some((row: any) => !placementIds.has(row.loc_id))) throw new Error(`${revision}: placement rows must be published wood ids`);
    if (placements.rows.some((row: any) => row.plane < 0 || row.plane > 3 || row.x < 0 || row.z < 0)) throw new Error(`${revision}: placement world range`);
    if (placements.rows.some((row: any) => row.loc_id === 2662 || row.loc_id === 2492)) throw new Error(`${revision}: flour and essence stay one-off, not this family`);
    if (placements.rows.some((row: any) => 'publication' in row || 'published_for_placement' in row)) throw new Error(`${revision}: placement rows must not carry a publication field`);
    for (const banned of ['curated', 'frozen', 'operator_placement', 'RIDDLE_KEY_COORDS', 'HARD_SPECIAL_COORDS', 'invented']) {
        if (JSON.stringify(placements).includes(banned)) throw new Error(`${revision}: placements published ${banned}`);
    }
    const questIdentity = payload.quest_identity;
    if (!questIdentity || Array.isArray(questIdentity) || !Array.isArray(questIdentity.rows)) throw new Error(`${revision}: quest_identity must be one family object, not a bare list`);
    if (payload.quest_prereqs !== undefined) throw new Error(`${revision}: quest_prereqs is not a second field`);
    const extractedQuest = extractQuestIdentityFacts(pin.contentRoot, revision);
    assertEqual(JSON.stringify(questIdentity), JSON.stringify(extractedQuest), `${revision} quest_identity matches the writer extract`);
    const expectedQuest = [
        ['cook', "Cook's Assistant", 'cook', 'cookquest', 29, 2, 1],
        ['runemysteries', 'Rune Mysteries Quest', 'runemysteries', 'runemysteries', 63, 6, 1],
        ['murder', 'Murder Mystery', 'murder', 'murderquest', 192, 2, 3],
        ['waterfall', 'Waterfall Quest', 'waterfall', 'waterfall_quest', 65, 10, 1],
        ['death', 'Death Plateau', 'death', 'death_equiproom', 314, 80, 1],
        ['zanaris', 'Lost City', 'zanaris', 'zanaris', 147, 6, 3],
    ];
    assertEqual(questIdentity.rows.length, 6, `${revision} quest rows`);
    assertEqual(JSON.stringify(questIdentity.rows.map((row: any) => [row.id, row.display, row.component, row.varp, row.varp_id, row.complete, row.quest_points])), JSON.stringify(expectedQuest), `${revision} quest identity rows`);
    if (questIdentity.rows.some((row: any) => row.requirements.qualification !== 'partial' || row.requirements.unknown_as_satisfied !== false || row.unknown_sides.length !== 0 || row.enum_index !== undefined || row.engine_id !== undefined || typeof row.complete !== 'number')) throw new Error(`${revision}: quest row promoted, ranged, or aliased`);
    const cookQuest = questIdentity.rows[0];
    assertEqual(JSON.stringify(cookQuest.requirements.items), JSON.stringify([{ alias: 'egg', quantity: 1, kind: 'inv' }, { alias: 'bucket_milk', quantity: 1, kind: 'inv' }, { alias: 'pot_flour', quantity: 1, kind: 'inv' }]), `${revision} cook aliases`);
    if (cookQuest.requirements.empty_must_have !== false) throw new Error(`${revision}: cook mustHave is not empty`);
    for (const id of ['runemysteries', 'murder', 'death']) {
        const row = questIdentity.rows.find((entry: any) => entry.id === id);
        if (!row.requirements.empty_must_have || row.requirements.items.length !== 0 || row.requirements.skills.length !== 0) throw new Error(`${revision}: ${id} empty mustHave`);
    }
    const waterfallQuest = questIdentity.rows.find((row: any) => row.id === 'waterfall');
    assertEqual(JSON.stringify(waterfallQuest.requirements.items), JSON.stringify([{ alias: 'rope', quantity: null, kind: 'use-site' }]), `${revision} waterfall rope`);
    const zanarisQuest = questIdentity.rows.find((row: any) => row.id === 'zanaris');
    assertEqual(JSON.stringify(zanarisQuest.requirements.skills), JSON.stringify([{ skill: 'woodcutting', level: 36 }, { skill: 'crafting', level: 31 }]), `${revision} zanaris gates`);
    if (zanarisQuest.requirements.items.length !== 0) throw new Error(`${revision}: zanaris must not add axe, knife, branch, or spirit`);
    const questBlob = JSON.stringify(questIdentity);
    for (const banned of ['family-unavailable', 'enum_index', 'Lost City Of Zanaris', 'In Search of the Myreque', 'death_climbingboots', 'death_secretwaymap', 'death_spikedboots', 'Egg', 'Pot of flour', 'Bucket of milk']) {
        if (questBlob.includes(banned)) throw new Error(`${revision}: quest identity published ${banned}`);
    }
    if (questIdentity.rows.some((row: any) => row.varp_id === 101 || row.varp_id === 219 || row.varp === 'death' || row.id === 'routequest' || row.id === 'misc' || row.id === 'troll_love' || row.id === 'mm' || row.id === 'regicide' || row.id === 'legends')) throw new Error(`${revision}: quest identity published a rejected binding`);
    if (revision === 274) {
        assertEqual(questIdentity.coverage.length, 1, '274 quest coverage');
        assertEqual(JSON.stringify(questIdentity.coverage[0]), JSON.stringify({ class: 'revision-absent', alias: 'routequest', on_revision: 274, other_pin_id: 387, copied: false, reason: '289-only quest, not copied onto 274' }), '274 routequest coverage');
        if (questIdentity.coverage[0].display !== undefined || questIdentity.coverage[0].complete !== undefined || questIdentity.coverage[0].quest_points !== undefined) throw new Error('274 copied routequest facts');
    } else {
        if (questIdentity.coverage.length !== 0) throw new Error('289 quest coverage must be empty');
        if (questIdentity.rows.length !== 6) throw new Error('289 must not gain a seventh quest row');
    }
    const trails = payload.trails;
    if (!trails || Array.isArray(trails) || !Array.isArray(trails.rows)) throw new Error(`${revision}: trails must be one family object, not a bare list`);
    const extractedTrails = extractTrailFacts(pin.contentRoot, payload.items.map((item: any) => ({ id: item.id, debugname: item.alias, name: item.name, cost: item.cost, stackable: item.stackable, members: item.members, certlink: item.certificate_link, certtemplate: item.certificate_template, wearpos: item.wear_position, wearpos2: item.wear_position_2, wearpos3: item.wear_position_3 })));
    assertEqual(JSON.stringify(trails), JSON.stringify(extractedTrails), `${revision} trails matches the writer extract`);
    const trailRows = trails.rows as any[];
    if (trailRows.some((row: any) => typeof row.alias !== 'string' || typeof row.id !== 'number' || (row.role !== 'clue' && row.role !== 'casket') || !Array.isArray(row.params))) throw new Error(`${revision}: trail row shape`);
    if (trailRows.some((row: any) => row.params.some((param: any) => typeof param.key !== 'string' || typeof param.value !== 'string'))) throw new Error(`${revision}: trail params must stay raw strings`);
    if (trailRows.some((row: any) => 'supported' in row || 'coverage' in row || 'class' in row || 'npc' in row || 'handler_coord' in row || 'coord' in row)) throw new Error(`${revision}: trails published a support, coverage, class, npc, or coordinate field`);
    const constrained = trailRows.filter((row: any) => row.access !== undefined);
    assertEqual(constrained.length, 1, `${revision} constrained trail rows`);
    assertEqual(constrained[0].alias, 'trail_clue_hard_sextant028', `${revision} constrained trail alias`);
    assertEqual(constrained[0].id, 3554, `${revision} constrained trail id`);
    assertEqual(constrained[0].access, 'constrained', `${revision} constrained trail access`);
    const riddle004Casket = trailRows.find((row: any) => row.alias === 'trail_clue_hard_riddle004_casket');
    if (!riddle004Casket || riddle004Casket.id !== 2779 || riddle004Casket.role !== 'casket' || riddle004Casket.params.length !== 0) throw new Error(`${revision}: trails must keep the selected casket 2779`);
    const sextant016Casket = trailRows.filter((row: any) => row.alias === 'trail_clue_hard_sextant016_casket');
    if (sextant016Casket.length !== 1 || sextant016Casket[0].id !== 3531) throw new Error(`${revision}: trails casket alias join`);
    if (trailRows.some((row: any) => row.id === 3533 || row.alias === 'trail_clue_hard_sextant017_casket')) throw new Error(`${revision}: trails must not emit the unnamed casket 3533`);
    const sextant017 = trailRows.find((row: any) => row.alias === 'trail_clue_hard_sextant017');
    if (!sextant017?.params.some((param: any) => param.key === 'trail_casket' && param.value === 'trail_clue_hard_sextant016_casket')) throw new Error(`${revision}: trails sextant017 casket param`);
    for (const alias of ['trail_clue_medium_map002', 'trail_clue_medium_map002_casket', 'trail_clue_hard_sextant026', 'trail_clue_hard_sextant026_casket']) {
        if (trailRows.some((row: any) => row.alias === alias)) throw new Error(`${revision}: ${alias} is not a membership row`);
    }
    const trailSimple001 = trailRows.find((row: any) => row.alias === 'trail_clue_easy_simple001');
    if (!trailSimple001?.params.some((param: any) => param.key === 'trail_loc' && param.value === '^true')) throw new Error(`${revision}: trails simple001 trail_loc must stay the raw ^true`);
    const trailAnagram001 = trailRows.find((row: any) => row.alias === 'trail_clue_medium_anagram001');
    if (trailAnagram001?.params.length !== 1 || trailAnagram001.params[0].key !== 'trail_desc' || trailAnagram001.params.some((param: any) => param.key === 'trail_sextant' || param.key === 'trail_challenge_answer')) throw new Error(`${revision}: trails anagram001 keeps only its own selected param`);
    assertEqual(trails.challenge_answers.length, 6, `${revision} trail challenge answers`);
    assertEqual(JSON.stringify(trails.challenge_answers), JSON.stringify([
        { alias: 'trail_clue_medium_anagram001_challenge', id: 2842, answer: '6859' },
        { alias: 'trail_clue_medium_anagram002_challenge', id: 2844, answer: '9' },
        { alias: 'trail_clue_medium_anagram003_challenge', id: 2846, answer: '40' },
        { alias: 'trail_clue_medium_anagram006_challenge', id: 2850, answer: '5' },
        { alias: 'trail_clue_medium_anagram007_challenge', id: 2852, answer: '48' },
        { alias: 'trail_clue_medium_anagram008_challenge', id: 2854, answer: '5096' },
    ]), `${revision} trail challenge answer rows`);
    const trailBlob = JSON.stringify(trails);
    for (const banned of ['facts-verified-both', 'facts_field_verified_both', 'family-unavailable', 'supported', 'coverage', 'regicide', 'legends', 'curated-unverified', '0_51_54_45_47', '1_42_53_14_17', '1_40_51_14_62', 'HARD_SPECIAL_COORDS', 'RIDDLE_KEY_COORDS', 'VAGUE003']) {
        if (trailBlob.includes(banned)) throw new Error(`${revision}: trails published ${banned}`);
    }
    const talkKey: PublishedTalkKey = payload.talk_key;
    if (!talkKey || Array.isArray(talkKey) || !Array.isArray(talkKey.talk) || !Array.isArray(talkKey.keys) || !Array.isArray(talkKey.coverage)) throw new Error(`${revision}: talk_key must be one family object, not a bare list`);
    const publishedItems: PublishedItemRow[] = payload.items;
    const extractedTalkKey = extractTalkKeyFacts(pin.contentRoot, publishedItems.map((item) => ({ id: item.id, debugname: item.alias, name: item.name, cost: item.cost, stackable: item.stackable, members: item.members, certlink: item.certificate_link, certtemplate: item.certificate_template, wearpos: item.wear_position, wearpos2: item.wear_position_2, wearpos3: item.wear_position_3 })));
    assertEqual(JSON.stringify(talkKey), JSON.stringify(extractedTalkKey.facts), `${revision} talk_key matches the writer extract`);
    assertEqual(JSON.stringify(payload.provenance.talk_key_inputs), JSON.stringify(extractedTalkKey.inputs), `${revision} talk key inputs match the scanned scripts, npc configs, maps, pack, and medium proc`);
    assertTalkKeyPins(extractedTalkKey.facts, revision);
    const talkRows = talkKey.talk;
    const keyRows = talkKey.keys;
    assertEqual(talkRows.length, 47, `${revision} talk_key talk steps`);
    assertEqual(talkRows.filter((row) => row.spawn !== undefined).length, 42, `${revision} talk_key unique talk spawns`);
    assertEqual(keyRows.length, 7, `${revision} talk_key key keepers`);
    assertEqual(keyRows.filter((row) => row.spawn !== undefined).length, 2, `${revision} talk_key unique keeper spawns`);
    assertEqual(talkKey.coverage.length, 10, `${revision} talk_key unknown-spawn coverage`);
    if (talkRows.some((row) => Object.keys(row).some((key) => !['alias', 'id', 'npc', 'spawn'].includes(key)))) throw new Error(`${revision}: talk_key talk rows must stay alias/id/npc/spawn`);
    if (keyRows.some((row) => Object.keys(row).some((key) => !['alias', 'id', 'key_alias', 'key_id', 'keeper', 'spawn'].includes(key)))) throw new Error(`${revision}: talk_key key rows must stay alias/id/key_alias/key_id/keeper/spawn`);
    if (talkRows.some((row) => Object.keys(row.npc).sort().join(',') !== 'alias,id,name' || row.npc.name.length === 0)) throw new Error(`${revision}: talk_key npc identity must be alias/id/name`);
    if ([...talkRows, ...keyRows].some((row) => row.spawn !== undefined && Object.keys(row.spawn).sort().join(',') !== 'plane,x,z')) throw new Error(`${revision}: talk_key spawns are world x/z/plane only`);
    if (JSON.stringify({ talk: talkRows, keys: keyRows }).includes('"spawn":null')) throw new Error(`${revision}: a non-unique spawn must omit the key, not publish null`);
    for (const row of keyRows) {
        const keeper = row.keeper;
        const keeperKeys = Object.keys(keeper).sort().join(',');
        if (keeper.kind === 'type') {
            if (keeperKeys !== 'alias,id,kind,name' || keeper.id === undefined || keeper.alias === undefined || keeper.name === undefined) throw new Error(`${revision}: talk_key type keeper must be alias/id/name`);
        } else if (keeper.kind === 'category') {
            if (keeperKeys !== 'category,kind' || keeper.category === undefined) throw new Error(`${revision}: talk_key category keeper must be the bare category`);
        } else if (keeper.kind === 'name') {
            if (keeperKeys !== 'kind,name' || keeper.name === undefined) throw new Error(`${revision}: talk_key name keeper must be the bare name`);
        } else {
            throw new Error(`${revision}: talk_key keeper kind ${keeper.kind} is not a keeper`);
        }
    }
    const coveragePairs = talkKey.coverage.map((row) => `${row.family}:${row.alias}`).sort();
    const missingSpawn = [
        ...talkRows.filter((row) => row.spawn === undefined).map((row) => `talk:${row.alias}`),
        ...keyRows.filter((row) => row.spawn === undefined).map((row) => `keys:${row.alias}`),
    ].sort();
    assertEqual(JSON.stringify(coveragePairs), JSON.stringify(missingSpawn), `${revision} talk_key coverage is exactly the steps without a unique spawn`);
    if (talkKey.coverage.some((row) => row.class !== 'unknown' || (row.family !== 'talk' && row.family !== 'keys') || row.reason.length === 0 || Object.keys(row).sort().join(',') !== 'alias,class,family,reason')) throw new Error(`${revision}: talk_key coverage rows must be named unknown talk or keys rows with a reason`);
    const talkKeyBlob = JSON.stringify(talkKey);
    for (const banned of ['TALK_ANCHORS', 'KILL_ANCHORS', 'RIDDLE_KEY_COORDS', 'HARD_SPECIAL_COORDS', 'frozen', 'invented', 'family-unavailable']) {
        if (talkKeyBlob.includes(banned)) throw new Error(`${revision}: talk_key published ${banned}`);
    }
    const trioGivers: PublishedTrioGivers = payload.trio_givers;
    if (!trioGivers || Array.isArray(trioGivers) || !Array.isArray(trioGivers.rows) || !Array.isArray(trioGivers.coverage)) throw new Error(`${revision}: trio_givers must be one family object, not a bare list`);
    const extractedTrioGivers = extractTrioGiversFacts(pin.contentRoot);
    assertEqual(JSON.stringify(trioGivers), JSON.stringify(extractedTrioGivers.facts), `${revision} trio_givers matches the writer extract`);
    assertEqual(JSON.stringify(payload.provenance.trio_givers_inputs), JSON.stringify(extractedTrioGivers.inputs), `${revision} trio giver inputs match the closed handlers, configs, maps, and pack`);
    assertTrioGiverPins(extractedTrioGivers.facts, revision);
    const giverRows = trioGivers.rows;
    assertEqual(JSON.stringify(giverRows.map((row) => [row.alias, row.id, row.name])), JSON.stringify([['observatory_professor', 488, 'Observatory professor'], ['murphy', 463, 'Murphy'], ['brother_kojo', 223, 'Brother Kojo']]), `${revision} trio_givers closed identity set`);
    if (giverRows.some((row) => Object.keys(row).some((key) => !['alias', 'id', 'name', 'spawn'].includes(key)))) throw new Error(`${revision}: trio_givers rows must be alias/id/name/spawn only`);
    if (giverRows.some((row) => row.alias === 'observatory_professor2' || row.alias.startsWith('murphy_'))) throw new Error(`${revision}: trio_givers published a lookalike alias`);
    if (giverRows.some((row) => row.spawn !== undefined && Object.keys(row.spawn).sort().join(',') !== 'plane,x,z')) throw new Error(`${revision}: trio_givers spawns are world x/z/plane only`);
    if (giverRows.some((row) => row.spawn !== undefined && (row.spawn.plane < 0 || row.spawn.plane > 3 || row.spawn.x < 0 || row.spawn.z < 0))) throw new Error(`${revision}: trio_givers spawn world range`);
    if (JSON.stringify(giverRows).includes('"spawn":null')) throw new Error(`${revision}: a non-unique spawn must omit the key, not publish null`);
    const giverCoverage = trioGivers.coverage.map((row) => row.alias).sort();
    const giverMissingSpawn = giverRows.filter((row) => row.spawn === undefined).map((row) => row.alias).sort();
    assertEqual(JSON.stringify(giverCoverage), JSON.stringify(giverMissingSpawn), `${revision} trio_givers coverage is exactly the givers without a unique spawn`);
    if (trioGivers.coverage.some((row) => row.class !== 'unknown' || row.family !== 'trio_givers' || row.reason.length === 0 || Object.keys(row).sort().join(',') !== 'alias,class,family,reason')) throw new Error(`${revision}: trio_givers coverage rows must be named unknown trio_givers rows with a reason`);
    const trioGiversBlob = JSON.stringify(trioGivers);
    for (const banned of ['TALK_ANCHORS', 'KILL_ANCHORS', 'RIDDLE_KEY_COORDS', 'HARD_SPECIAL_COORDS', 'frozen', 'invented', 'family-unavailable']) {
        if (trioGiversBlob.includes(banned)) throw new Error(`${revision}: trio_givers published ${banned}`);
    }
    const bankSource = path.join(process.env.RS2B0T ?? path.join(root, '.superpowers/release-0.1.9/reference/rs2b0t-00d39a17e0'), 'src/bot/api/bank/BankLocations.ts');
    const bankCatalog = await extractBankCatalog(pin.engineRoot, fs.readFileSync(bankSource, 'utf8'));
    const bankPlacements = extractBankPlacements(pin.contentRoot, bankCatalog);
    assertEqual(JSON.stringify(payload.bank_placements), JSON.stringify(bankPlacements.facts), `${revision} bank access placements match selected content`);
    assertEqual(JSON.stringify(payload.provenance.bank_inputs), JSON.stringify({
        catalog: { path: 'rs2b0t-00d39a17e0/src/bot/api/bank/BankLocations.ts', ...digest(bankSource) },
        ...bankPlacements.inputs,
    }), `${revision} bank catalog and placement inputs`);
    assertEqual(fs.readFileSync(path.join(root, 'crates/api/data/game-data/bank-catalog.rs'), 'utf8'), bankCatalogRust(bankCatalog), `${revision} compiled bank roster matches frozen AST`);
    const rs2b0tRoot = process.env.RS2B0T ?? path.join(root, '.superpowers/release-0.1.9/reference/rs2b0t-00d39a17e0');
    const cookFiles = ['src/bot/data/cookLocations.ts', 'src/bot/data/cookingRanges.ts', 'tools/cooking/gen-cooksurfaces.ts'];
    const [cookLocations, cookingRanges, genCookSurfaces] = cookFiles.map((file) => fs.readFileSync(path.join(rs2b0tRoot, file), 'utf8'));
    const cookCatalog = await extractCookCatalog(pin.engineRoot, { cookLocations, cookingRanges, genCookSurfaces });
    const cookSurfaces = extractCookSurfaces(pin.contentRoot, cookCatalog);
    assertEqual(JSON.stringify(payload.cook_surfaces), JSON.stringify(cookSurfaces.facts), `${revision} cook surfaces match selected content`);
    assertEqual(JSON.stringify(payload.provenance.cook_inputs), JSON.stringify({
        catalog: cookFiles.map((file) => ({ path: `rs2b0t-00d39a17e0/${file}`, ...digest(path.join(rs2b0tRoot, file)) })),
        ...cookSurfaces.inputs,
    }), `${revision} cook catalog and surface inputs`);
    assertEqual(fs.readFileSync(path.join(root, 'crates/api/data/game-data/cook-catalog.rs'), 'utf8'), cookCatalogRust(cookCatalog), `${revision} compiled cook camps match frozen AST`);
    publishedTrioGivers.set(revision, trioGivers);
    results.push({ revision, records: payload.items.length, consumption: payload.consumption.length, pickpocket: payload.pickpocket.length, drop_tables: drops.length, spells: spells.length, staves: staves.length, herbs: herbs.length, prayers: prayers.length, pickaxes: nurmof.pickaxes.length, flour_six: 6, gather_methods: { mining: gather.mining.length, woods: gather.woods.length, fishing: gather.fishing.length }, gather_placements: { rows: placements.rows.length, maps: payload.provenance.placement_inputs.maps.files, published_loc_ids: payload.provenance.placement_inputs.published_loc_ids.count, coverage: placements.coverage.length, woods: extractedPlacements.woods }, quest_identity: { rows: questIdentity.rows.length, coverage: questIdentity.coverage.length }, trails: { rows: trailRows.length, clues: trailRows.filter((row: any) => row.role === 'clue').length, caskets: trailRows.filter((row: any) => row.role === 'casket').length, challenge_answers: trails.challenge_answers.length, access_constrained: constrained.length }, talk_key: { talk: talkRows.length, talk_with_spawn: talkRows.filter((row) => row.spawn !== undefined).length, keys: keyRows.length, keys_with_spawn: keyRows.filter((row) => row.spawn !== undefined).length, coverage: talkKey.coverage.length, scripts: extractedTalkKey.inputs.scripts.files, npc_configs: extractedTalkKey.inputs.npc_configs.files }, trio_givers: { rows: giverRows.length, with_spawn: giverRows.filter((row) => row.spawn !== undefined).length, coverage: trioGivers.coverage.length, maps: extractedTrioGivers.inputs.maps.files, handlers: extractedTrioGivers.inputs.handlers.length, npc_configs: extractedTrioGivers.inputs.npc_configs.length, aliases: giverRows.map((row) => row.alias) }, equipment_names: { resolved: resolved.length, absent: absent.length, family_counts: familyCounts }, fire_staff_providers: fireProviders, autocast, duel, special: { energy_varp: special.energy_varp, armed_varp: special.armed_varp, max_energy: special.max_energy, bars: special.bars.length, weapons: special.weapons.length }, teleports: teleports.length, fixed_food_heals: Object.fromEntries(fixed), output_sha256: output.sha256, input_hashes: true, content_hashes: true, source_pins: true, dirty_gate: true, cache_identity: pin.cache });
}
const pinnedGivers274 = publishedTrioGivers.get(274); const pinnedGivers289 = publishedTrioGivers.get(289);
if (!pinnedGivers274 || !pinnedGivers289) throw new Error('trio_givers: both pins must publish the family');
if (JSON.stringify(pinnedGivers289) !== JSON.stringify(pinnedGivers274)) throw new Error('trio_givers: the two pins disagree on the selected identity, display name, or unique spawn');
const evidence = { schema_version: 4, generator: 'tools/game-data/generate.ts', verification: 'tools/game-data/verify.ts', revisions: results }; const evidencePath = path.join(root, 'docs/compat/evidence/generated-game-data/verification.json'); fs.mkdirSync(path.dirname(evidencePath), { recursive: true }); fs.writeFileSync(evidencePath, `${JSON.stringify(evidence, null, 2)}\n`); console.log(JSON.stringify(evidence));
