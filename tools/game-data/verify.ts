import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { verifyCacheIdentity } from './cache-identity.ts';
import { loadEquipmentNamesCurated, parseFrozenEquipmentNameArrays, parsePack } from './generate.ts';
const root = path.resolve(import.meta.dirname, '../..');
const expected: Record<number, { engine: string; content: string; engineRoot: string; contentRoot: string; cache: { cache_id: string; content_id: string; nav_sha256: string; flags_sha256: string } }> = {
    274: { engine: '4c95f87efe00b068cadbd229d94736626907bd1a', content: '000c19997e07206131bcb3c884265840efce416d', engineRoot: process.env.GAME_DATA_274_ENGINE || '/Users/acfrazier/experiments/Server/engine', contentRoot: process.env.GAME_DATA_274_CONTENT || '/Users/acfrazier/experiments/Server/content', cache: { cache_id: '4aac9b63312dcb75d5de8f686772d083ba0808c57985438246edf21ef522be1c', content_id: '0d14c891b5727142379c6d8844bd5bb1ff9874d4d996db1dc5ceea0e9062469c', nav_sha256: '05db24743e9f549ced16c1f00b87c30a390d3aaec391815da3f3563130b3bcd4', flags_sha256: '92d5dea05c886ac8720be6b47e47cbc68355a8ff42676c0886f5b7ea8343a4cb' } },
    289: { engine: 'cc359656b4acd216ca452495874b6beba9a0ac75', content: '92649430fcbc83538d8c4367ecb96cee1a67a944', engineRoot: process.env.GAME_DATA_289_ENGINE || '/Users/acfrazier/experiments/lostcity-289/engine', contentRoot: process.env.GAME_DATA_289_CONTENT || '/Users/acfrazier/experiments/lostcity-289/content', cache: { cache_id: 'c4d8ab36bcfd2a7907535b4f619e28623b0a22e98d496fd2a9620d544c5b5b09', content_id: 'cdb2f161c35239f09bf5175648e15e7dbbc4bbf9be4419cea41f7053ccf8b044', nav_sha256: '131db92e32eddcb08148909d477589544320fe7e34a422e8004e97e888032924', flags_sha256: '67e4094dff06def5cf8abc172ce751f4ca8679532ba04c1ba15ab6bf668c7a4a' } }
};
const decoderSources = ['src/cache/config/ObjType.ts', 'src/cache/config/NpcType.ts', 'src/cache/config/ConfigType.ts', 'src/cache/config/ParamHelper.ts', 'src/cache/config/ParamType.ts', 'src/cache/config/ScriptVarType.ts', 'src/io/BZip2.ts', 'src/io/Jagfile.ts', 'src/io/Packet.ts', 'src/datastruct/DoublyLinkable.ts', 'src/datastruct/LinkList.ts', 'src/datastruct/Linkable.ts', 'src/util/Environment.ts', 'src/util/Logger.ts', 'src/util/TryParse.ts', 'src/util/WorldConfig.ts'];
const contentFiles = ['scripts/player/configs/consumption/consume.dbtable', 'scripts/player/configs/consumption/consume_normal.dbrow', 'scripts/player/configs/consumption/consume_effects.dbrow', 'scripts/skill_thieving/configs/pickpocking/pickpocket.dbtable', 'scripts/skill_thieving/configs/pickpocking/pickpocket.dbrow', 'scripts/player/scripts/consumption/effects/scripts/consume_effects.rs2', 'scripts/skill_combat/configs/magic/magic_combat_spells.dbrow', 'scripts/skill_magic/configs/magic.dbtable', 'scripts/skill_magic/configs/magic_spells.dbrow', 'scripts/skill_magic/configs/magic_staff.dbrow', 'scripts/skill_combat/configs/combat.constant', 'scripts/skill_herblore/configs/herbs.obj', 'scripts/skill_herblore/configs/identifying/identify.param', 'scripts/skill_herblore/scripts/identifying/identify.rs2', 'scripts/skill_prayer/configs/prayers.dbrow', 'scripts/skill_prayer/configs/prayers.constant', 'scripts/skill_prayer/interfaces/prayer.if', 'scripts/areas/area_falador/configs/dwarven_mine.inv', 'scripts/areas/area_falador/configs/dwarven_mine.npc', 'scripts/skill_runecraft/configs/runecraft.constant', 'maps/m45_75.jm2', 'pack/npc.pack', 'scripts/quests/quest_murder/configs/quest_murder.loc', 'scripts/general/configs/quest.enum', 'maps/m42_55.jm2', 'pack/loc.pack', 'pack/obj.pack', 'pack/interface.pack', 'pack/varp.pack', 'pack/param.pack', 'scripts/_unpack/225/all.npc', 'scripts/drop tables/scripts/giant.rs2', 'scripts/drop tables/scripts/moss_giant.rs2', 'scripts/drop tables/scripts/fire_giant.rs2', 'scripts/drop tables/scripts/green_dragon.rs2', 'scripts/drop tables/scripts/shared_droptables.rs2'];
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
const manifest = JSON.parse(fs.readFileSync(path.join(root, 'crates/api/data/game-data/manifest.json'), 'utf8')) as { schema_version: number; revisions: any[] };
assertEqual(manifest.schema_version, 4, 'manifest schema');
const results = [];
for (const revision of [274, 289]) {
    const file = path.join(root, `crates/api/data/game-data/${revision}.json`); const payload = JSON.parse(fs.readFileSync(file, 'utf8')) as any; const pin = expected[revision]; const manifestRow = manifest.revisions.find((entry) => entry.revision === revision); if (!manifestRow) throw new Error(`${revision}: missing manifest row`);
    assertEqual(payload.schema_version, 4, `${revision} schema`); assertEqual(payload.revision, revision, `${revision} revision`); assertEqual(payload.provenance.engine_commit, pin.engine, `${revision} engine pin`); assertEqual(payload.provenance.content_commit, pin.content, `${revision} content pin`); assertEqual(commit(pin.engineRoot), pin.engine, `${revision} live engine commit`); assertEqual(commit(pin.contentRoot), pin.content, `${revision} live content commit`);
    verifyCacheIdentity(revision, pin.engineRoot, pin.cache);
    const sourcePaths = [...decoderSources, 'data/pack/server/obj.dat', 'data/pack/server/npc.dat', 'data/pack/client/config']; const dirtyEngine = execFileSync('git', ['-C', pin.engineRoot, 'status', '--porcelain', '--untracked-files=all', '--', ...sourcePaths], { encoding: 'utf8' }).trim(); if (dirtyEngine) throw new Error(`${revision}: relevant engine inputs are dirty: ${dirtyEngine}`); const dirtyContent = execFileSync('git', ['-C', pin.contentRoot, 'status', '--porcelain', '--untracked-files=all', '--', ...contentFiles], { encoding: 'utf8' }).trim(); if (dirtyContent) throw new Error(`${revision}: relevant content inputs are dirty: ${dirtyContent}`);
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
    results.push({ revision, records: payload.items.length, consumption: payload.consumption.length, pickpocket: payload.pickpocket.length, drop_tables: drops.length, spells: spells.length, staves: staves.length, herbs: herbs.length, prayers: prayers.length, pickaxes: nurmof.pickaxes.length, flour_six: 6, equipment_names: { resolved: resolved.length, absent: absent.length, family_counts: familyCounts }, fire_staff_providers: fireProviders, autocast, duel, special: { energy_varp: special.energy_varp, armed_varp: special.armed_varp, max_energy: special.max_energy, bars: special.bars.length, weapons: special.weapons.length }, teleports: teleports.length, fixed_food_heals: Object.fromEntries(fixed), output_sha256: output.sha256, input_hashes: true, content_hashes: true, source_pins: true, dirty_gate: true, cache_identity: pin.cache });
}
const evidence = { schema_version: 4, generator: 'tools/game-data/generate.ts', verification: 'tools/game-data/verify.ts', revisions: results }; const evidencePath = path.join(root, 'docs/compat/evidence/generated-game-data/verification.json'); fs.mkdirSync(path.dirname(evidencePath), { recursive: true }); fs.writeFileSync(evidencePath, `${JSON.stringify(evidence, null, 2)}\n`); console.log(JSON.stringify(evidence));
