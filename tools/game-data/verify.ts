import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { verifyCacheIdentity } from './cache-identity.ts';
import { extractTalkKeyFacts, extractTrailFacts, extractTrioGiversFacts, assertTalkKeyPins, assertTrioGiverPins, trailContentFiles, loadEquipmentNamesCurated, parseFrozenEquipmentNameArrays, assertPinned, revisions, requestedRevisions } from './generate.ts';
import { parsePack } from './extractors/common.ts';
import { parseCombatScripts } from './extractors/combat.ts';
import { extractNpcNamesFacts } from './extractors/npc-names.ts';
import { extractGatheringFamily, compareCodepoint, gatherResources, gatherSites, miningHazards, type GatherSiteWire } from './extractors/gathering.ts';
import { extractQuestIdentityFacts, questIdentityContentFiles } from './extractors/quests.ts';
import { assertRs2b0tPinned, bankCatalogRust, cookCatalogRust, extractBankCatalog, extractBankPlacements, extractCookCatalog, extractCookSurfaces, familyBytes, familyInputs, requireEnvPath, BASE_ENGINE_INPUT_PATHS, DEBUG_ENGINE_INPUT_PATHS, DEBUG_SCHEMA_VERSION, baseProvenanceInputs } from './generate.ts';
import { ENGINE_DEBUG_COMMANDS, extractDebugCatalog, engineHandlerRelative } from './extractors/debug.ts';
import { extractQuestStartFacts, questStartContentFiles } from './extractors/quest-starts.ts';
import { extractDialogueUiFacts } from './extractors/dialogue-ui.ts';
const root = path.resolve(import.meta.dirname, '../..');
const expected = Object.fromEntries(revisions.map(spec => [spec.revision, {
    engine: spec.expectedEngine, content: spec.expectedContent,
    engineRoot: spec.engine, contentRoot: spec.content, cache: spec.cacheIdentity,
}]));
const kqDropNames = ['Adamant spear', 'Amulet of power', 'Blood rune', 'Chaos talisman', 'Death rune', 'Dragon chainbody', 'Dragon spear', 'Fire rune', 'Half of a key', 'Iron arrow', 'Lava battlestaff', 'Law rune', 'Lobster', 'Mithril arrow', 'Nature rune', 'Nature talisman', 'Oyster pearls', 'Rune arrow', 'Rune axe', 'Rune chainbody', 'Rune javelin', 'Rune spear', 'Rune warhammer', 'Shield left half', 'Uncut diamond', 'Uncut emerald', 'Uncut ruby', 'Uncut sapphire', 'Wine of zamorak'];
const expectedDropNames: Record<number, Record<string, string[]>> = {
    274: {
        Giant: ['Beer', 'Big bones', 'Body talisman', 'Chaos rune', 'Chaos talisman', 'Coins', 'Cosmic rune', 'Death rune', 'Dragon spear', 'Fire rune', 'Half of a key', 'Herb', 'Iron arrow', 'Iron dagger', 'Iron full helm', 'Iron kiteshield', 'Law rune', 'Limpwurt root', 'Mind rune', 'Nature rune', 'Nature talisman', 'Rune javelin', 'Rune spear', 'Shield left half', 'Steel arrow', 'Steel longsword', 'Uncut diamond', 'Uncut emerald', 'Uncut ruby', 'Uncut sapphire', 'Water rune'],
        'Moss giant': ['Air rune', 'Big bones', 'Black sq shield', 'Blood rune', 'Chaos rune', 'Chaos talisman', 'Coal', 'Coins', 'Cosmic rune', 'Death rune', 'Dragon spear', 'Earth rune', 'Half of a key', 'Herb', 'Iron arrow', 'Law rune', 'Magic staff', 'Mithril spear', 'Mithril sword', 'Nature rune', 'Nature talisman', 'Rune javelin', 'Rune spear', 'Shield left half', 'Spinach roll', 'Steel arrow', 'Steel bar', 'Steel kiteshield', 'Steel med helm', 'Uncut diamond', 'Uncut emerald', 'Uncut ruby', 'Uncut sapphire'],
        'Fire giant': ['Adamant javelin', 'Big bones', 'Blood rune', 'Chaos rune', 'Chaos talisman', 'Coins', 'Death rune', 'Dragon med helm', 'Dragon spear', 'Dragonstone', 'Fire battlestaff', 'Fire rune', 'Half of a key', 'Herb', 'Law rune', 'Lobster', 'Mithril sq shield', 'Nature rune', 'Nature talisman', 'Rune 2h sword', 'Rune arrow', 'Rune battleaxe', 'Rune javelin', 'Rune kiteshield', 'Rune scimitar', 'Rune spear', 'Rune sq shield', 'Runite bar', 'Shield left half', 'Silver ore', 'Steel arrow', 'Steel axe', 'Steel bar', 'Strength potion(2)', 'Uncut diamond', 'Uncut emerald', 'Uncut ruby', 'Uncut sapphire'],
        'Green dragon': ['Adamant full helm', 'Adamantite ore', 'Bass', 'Chaos talisman', 'Coins', 'Dragon bones', 'Dragon spear', 'Dragonhide', 'Fire rune', 'Half of a key', 'Herb', 'Law rune', 'Mithril axe', 'Mithril kiteshield', 'Mithril spear', 'Nature rune', 'Nature talisman', 'Rune dagger', 'Rune javelin', 'Rune spear', 'Shield left half', 'Steel battleaxe', 'Steel platelegs', 'Uncut diamond', 'Uncut emerald', 'Uncut ruby', 'Uncut sapphire', 'Water rune'],
        'Kalphite Queen': kqDropNames,
    },
    289: {
        Giant: ['Beer', 'Big bones', 'Body talisman', 'Chaos rune', 'Chaos talisman', 'Coins', 'Cosmic rune', 'Death rune', 'Dragon spear', 'Fire rune', 'Half of a key', 'Herb', 'Iron arrow', 'Iron dagger', 'Iron full helm', 'Iron kiteshield', 'Law rune', 'Limpwurt root', 'Mind rune', 'Nature rune', 'Nature talisman', 'Rune javelin', 'Rune spear', 'Shield left half', 'Steel arrow', 'Steel longsword', 'Uncut diamond', 'Uncut emerald', 'Uncut ruby', 'Uncut sapphire', 'Water rune'],
        'Moss giant': ['Air rune', 'Big bones', 'Black sq shield', 'Blood rune', 'Chaos rune', 'Chaos talisman', 'Coal', 'Coins', 'Cosmic rune', 'Death rune', 'Dragon spear', 'Earth rune', 'Half of a key', 'Herb', 'Iron arrow', 'Law rune', 'Magic staff', 'Mithril spear', 'Mithril sword', 'Nature rune', 'Nature talisman', 'Rune javelin', 'Rune spear', 'Shield left half', 'Spinach roll', 'Steel arrow', 'Steel bar', 'Steel kiteshield', 'Steel med helm', 'Uncut diamond', 'Uncut emerald', 'Uncut ruby', 'Uncut sapphire'],
        'Fire giant': ['Adamant javelin', 'Big bones', 'Blood rune', 'Chaos rune', 'Chaos talisman', 'Coins', 'Death rune', 'Dragon med helm', 'Dragon spear', 'Dragonstone', 'Fire battlestaff', 'Fire rune', 'Half of a key', 'Herb', 'Law rune', 'Lobster', 'Mithril sq shield', 'Nature rune', 'Nature talisman', 'Rune 2h sword', 'Rune arrow', 'Rune battleaxe', 'Rune javelin', 'Rune kiteshield', 'Rune scimitar', 'Rune spear', 'Rune sq shield', 'Runite bar', 'Shield left half', 'Silver ore', 'Steel arrow', 'Steel axe', 'Steel bar', 'Strength potion(2)', 'Uncut diamond', 'Uncut emerald', 'Uncut ruby', 'Uncut sapphire'],
        'Green dragon': ['Adamant full helm', 'Adamantite ore', 'Bass', 'Chaos talisman', 'Coins', 'Dragon bones', 'Dragon spear', 'Dragonhide', 'Fire rune', 'Half of a key', 'Herb', 'Law rune', 'Mithril axe', 'Mithril kiteshield', 'Mithril spear', 'Nature rune', 'Nature talisman', 'Rune dagger', 'Rune javelin', 'Rune spear', 'Shield left half', 'Steel battleaxe', 'Steel platelegs', 'Uncut diamond', 'Uncut emerald', 'Uncut ruby', 'Uncut sapphire', 'Water rune'],
        'Kalphite Queen': kqDropNames,
    },
};
function digest(file: string) { const data = fs.readFileSync(file); return { bytes: data.length, sha256: crypto.createHash('sha256').update(data).digest('hex') }; }
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
type PublishedCombatSpell = {
    name: string; source_row: string; ssb: number; component_id: number;
    autocast_selectable: boolean; level: number; impact_spotanim: number;
    runes: { name: string; count: number }[];
};
function isPublishedCombatSpell(value: unknown): value is PublishedCombatSpell {
    return typeof value === 'object' && value !== null
        && 'name' in value && typeof value.name === 'string'
        && 'source_row' in value && typeof value.source_row === 'string'
        && 'ssb' in value && typeof value.ssb === 'number' && Number.isInteger(value.ssb)
        && 'component_id' in value && typeof value.component_id === 'number' && Number.isInteger(value.component_id)
        && 'autocast_selectable' in value && typeof value.autocast_selectable === 'boolean'
        && 'level' in value && typeof value.level === 'number' && Number.isInteger(value.level)
        && 'impact_spotanim' in value && typeof value.impact_spotanim === 'number' && Number.isInteger(value.impact_spotanim)
        && 'runes' in value && Array.isArray(value.runes)
        && value.runes.every((rune: unknown) => typeof rune === 'object' && rune !== null
            && 'name' in rune && typeof rune.name === 'string'
            && 'count' in rune && typeof rune.count === 'number' && Number.isInteger(rune.count));
}
const manifest = JSON.parse(fs.readFileSync(path.join(root, 'crates/api/data/game-data/manifest.json'), 'utf8')) as { schema_version: number; revisions: any[] };
assertEqual(manifest.schema_version, 4, 'manifest schema');
const results = [];
const publishedTrioGivers = new Map<number, PublishedTrioGivers>();
const publishedSiteIds = new Map<number, string[]>();
export type DebugVerifyStatus = { status: 'verified'; commands: number } | { status: 'source-rejected'; reason: string };
/** The debug family's own verification: a debug-only mismatch withholds only the Debug catalog. */
async function verifyDebugArtifact(revision: number, manifestRow: any, pin: { engine: string; content: string; engineRoot: string; contentRoot: string; cache: unknown }): Promise<DebugVerifyStatus> {
    try {
        const debugFile = path.join(root, `crates/api/data/game-data/${revision}/debug.json`);
        const debug = JSON.parse(fs.readFileSync(debugFile, 'utf8')) as any;
        assertEqual(debug.schema_version, DEBUG_SCHEMA_VERSION, `${revision} debug schema`);
        assertEqual(debug.revision, revision, `${revision} debug revision`);
        assertEqual(debug.provenance.engine_commit, pin.engine, `${revision} debug engine pin`);
        assertEqual(debug.provenance.content_commit, pin.content, `${revision} debug content pin`);
        assertEqual(JSON.stringify(debug.provenance.inputs.map((input: { path: string }) => input.path).sort()), JSON.stringify([...DEBUG_ENGINE_INPUT_PATHS].sort()), `${revision} debug engine inputs`);
        assertEqual(JSON.stringify(debug.provenance.cache_identity), JSON.stringify(pin.cache), `${revision} debug cache identity`);
        for (const input of debug.provenance.inputs) { const actual = digest(path.join(pin.engineRoot, input.path)); assertEqual(actual.bytes, input.bytes, `${revision} debug ${input.path} bytes`); assertEqual(actual.sha256, input.sha256, `${revision} debug ${input.path} hash`); }
        for (const input of debug.provenance.content_inputs) { const actual = digest(path.join(pin.contentRoot, input.path)); assertEqual(actual.bytes, input.bytes, `${revision} debug ${input.path} bytes`); assertEqual(actual.sha256, input.sha256, `${revision} debug ${input.path} hash`); }
        // The decoder lives under the revision-selected engine root, so it cannot be a static import (mirrors generate.ts).
        const cwd = process.cwd();
        process.chdir(pin.engineRoot);
        let npcs: { id: number; debugname?: string | null; name: string | null }[];
        try {
            const npcModule = (await import(pathToFileURL(path.join(pin.engineRoot, 'src/cache/config/NpcType.ts')).href)) as { default: { load(dir: string): void; configs: { id: number; debugname?: string | null; name: string | null }[] } };
            npcModule.default.load('data/pack');
            npcs = npcModule.default.configs;
        } finally {
            process.chdir(cwd);
        }
        const handlerText = fs.readFileSync(path.join(pin.engineRoot, engineHandlerRelative()), 'utf8');
        const statText = fs.readFileSync(path.join(pin.engineRoot, 'src/engine/entity/PlayerStat.ts'), 'utf8');
        const fresh = extractDebugCatalog(pin.contentRoot, handlerText, statText, npcs);
        assertEqual(JSON.stringify(debug.provenance.content_inputs), JSON.stringify(fresh.inputs), `${revision} debug content inputs match the extractor scan`);
        assertEqual(JSON.stringify(debug.debug_commands), JSON.stringify(fresh.commands), `${revision} debug commands match the writer extract`);
        assertEqual(JSON.stringify(debug.debug_names), JSON.stringify(fresh.names), `${revision} debug names match the writer extract`);
        const debugCommands = debug.debug_commands as { name: string; category: string; args: { name: string; kind: string; optional: boolean }[]; production_only: boolean }[];
        const expectedEngine = ENGINE_DEBUG_COMMANDS.filter((row) => revision !== 274 || row.name !== 'givebank').map((row) => row.name).sort();
        if (!Array.isArray(debugCommands) || debugCommands.length <= expectedEngine.length) throw new Error(`${revision}: incomplete debug command catalog`);
        const engineNames = debugCommands.filter((row) => !row.name.startsWith('~')).map((row) => row.name).sort();
        assertEqual(JSON.stringify(engineNames), JSON.stringify(expectedEngine), `${revision} engine debug command catalog`);
        if (new Set(debugCommands.map((row) => row.name)).size !== debugCommands.length || debugCommands.some((row) => !row.category || !Array.isArray(row.args))) throw new Error(`${revision}: invalid debug command rows`);
        const debugNames = debug.debug_names as Record<string, { id: number; alias: string; name: string }[]>;
        if (Object.hasOwn(debugNames, 'obj') || Object.hasOwn(debugNames, 'namedobj')) throw new Error(`${revision}: duplicate item debug-name family`);
        for (const kind of ['npc', 'loc', 'seq', 'spotanim', 'interface', 'stat', 'varp', 'inv', 'idkit']) if (!Array.isArray(debugNames?.[kind]) || debugNames[kind].length === 0) throw new Error(`${revision}: missing debug name family ${kind}`);
        assertEqual(debugNames.stat.length, 19, `${revision} stat debug names`);
        assertEqual(JSON.stringify(manifestRow.families?.debug), JSON.stringify({ path: `${revision}/debug.json`, schema: DEBUG_SCHEMA_VERSION, ...digest(debugFile) }), `${revision} manifest debug descriptor`);
        return { status: 'verified', commands: debugCommands.length };
    } catch (error) {
        return { status: 'source-rejected', reason: error instanceof Error ? error.message : String(error) };
    }
}

async function verifyRevision(revision: number) {
    const file = path.join(root, `crates/api/data/game-data/${revision}.json`); const payload = JSON.parse(fs.readFileSync(file, 'utf8')) as any; const pin = expected[revision]; const manifestRow = manifest.revisions.find((entry) => entry.revision === revision); if (!manifestRow) throw new Error(`${revision}: missing manifest row`);
    assertEqual(payload.schema_version, 4, `${revision} schema`); assertEqual(payload.revision, revision, `${revision} revision`); assertEqual(payload.provenance.engine_commit, pin.engine, `${revision} engine pin`); assertEqual(payload.provenance.content_commit, pin.content, `${revision} content pin`);
    const spec = revisions.find((each) => each.revision === revision)!;
    const pinnedCommits = assertPinned(spec);
    verifyCacheIdentity(revision, pin.engineRoot, pin.cache);
    const baseContentFiles = baseProvenanceInputs(pin.engineRoot, pin.contentRoot).content_inputs.map((input) => input.path);
    const npcNames = extractNpcNamesFacts(pin.contentRoot, revision);
    const combatScripts = parseCombatScripts(pin.contentRoot);
    const expectedContentFiles = [...new Set([
        ...baseContentFiles,
        ...(payload.quest_starts === undefined ? [] : questStartContentFiles(pin.contentRoot)),
        ...npcNames.files,
        ...combatScripts.files.map((script) => script.relative),
    ])].sort();
    assertEqual(JSON.stringify(payload.provenance.content_inputs.map((input: { path: string }) => input.path)), JSON.stringify(expectedContentFiles), `${revision} complete content provenance`);
    if (payload.debug_commands !== undefined || payload.debug_names !== undefined) throw new Error(`${revision}: the debug catalog lives in the debug family, not the core asset`);
    assertEqual(JSON.stringify(payload.dialogue_ui), JSON.stringify(extractDialogueUiFacts(pin.contentRoot)), `${revision} source-proven dialogue UI`);
    assertEqual(JSON.stringify(payload.provenance.inputs.map((input: { path: string }) => input.path)), JSON.stringify([...BASE_ENGINE_INPUT_PATHS]), `${revision} base engine inputs`);
    for (const input of [...payload.provenance.inputs, ...payload.provenance.decoder_sources, ...payload.provenance.content_inputs]) { const base = payload.provenance.content_inputs.includes(input) ? pin.contentRoot : pin.engineRoot; const actual = digest(path.join(base, input.path)); assertEqual(actual.bytes, input.bytes, `${revision} ${input.path} bytes`); assertEqual(actual.sha256, input.sha256, `${revision} ${input.path} hash`); }
    const output = digest(file); assertEqual(output.bytes, manifestRow.bytes, `${revision} output bytes`); assertEqual(output.sha256, manifestRow.sha256, `${revision} output hash`); assertEqual(JSON.stringify(payload.provenance.cache_identity), JSON.stringify(pin.cache), `${revision} cache identity`);
    const byAlias = new Map(payload.items.filter((item: any) => item.alias !== null).map((item: any) => [item.alias, item])); const plate = byAlias.get('rune_platebody'); const chain = byAlias.get('rune_chainbody'); if (!plate || plate.name !== 'Rune platebody' || !chain || chain.name !== 'Rune chainbody' || plate.cost <= chain.cost) throw new Error(`${revision}: Rune platebody/chainbody value order`); if (payload.items.filter((item: any) => item.name === 'Dragonhide').length < 2) throw new Error(`${revision}: same-name Dragonhide identity`); if (new Set(payload.items.map((item: any) => item.id)).size !== payload.items.length || new Set(payload.items.map((item: any) => item.alias)).size !== payload.items.length) throw new Error(`${revision}: duplicate IDs or aliases`);
    const isNpc = (value: unknown): value is { alias: string; id: number; name: string } =>
        typeof value === 'object'
        && value !== null
        && !Array.isArray(value)
        && 'alias' in value
        && typeof value.alias === 'string'
        && 'id' in value
        && typeof value.id === 'number'
        && 'name' in value
        && typeof value.name === 'string';
    type PickpocketCheckRow = {
        npcs: { alias: string; id: number; name: string }[];
        kind?: unknown; level?: unknown; source_file?: unknown;
        target_row?: unknown; source_row?: unknown; loot?: unknown;
    };
    const hasPickpocketNpcs = (value: unknown): value is PickpocketCheckRow =>
        typeof value === 'object' && value !== null && !Array.isArray(value)
        && 'npcs' in value && Array.isArray(value.npcs)
        && value.npcs.length > 0 && value.npcs.every(isNpc);
    const consumption: unknown[] = Array.isArray(payload.consumption) ? payload.consumption : [];
    const fixed = new Map<string, number>();
    for (const value of consumption) {
        if (
            typeof value !== 'object'
            || value === null
            || Array.isArray(value)
            || !('qualification' in value)
            || value.qualification !== 'fixed_hp_heal'
            || !('item' in value)
            || typeof value.item !== 'object'
            || value.item === null
            || Array.isArray(value.item)
            || !('alias' in value.item)
            || typeof value.item.alias !== 'string'
            || !('stat_heal' in value)
            || !Array.isArray(value.stat_heal)
        ) continue;
        const heals: unknown[] = value.stat_heal;
        const [heal] = heals;
        if (
            typeof heal === 'object'
            && heal !== null
            && !Array.isArray(heal)
            && 'base' in heal
            && typeof heal.base === 'number'
        ) fixed.set(value.item.alias, heal.base);
    }
    for (const [alias, heal] of [['lobster', 12], ['bread', 4], ['anchovies', 3]] as const) {
        if (fixed.get(alias) !== heal) throw new Error(`${revision}: ${alias} fixed heal mismatch`);
    }
    const pickpocket: unknown[] = Array.isArray(payload.pickpocket) ? payload.pickpocket : [];
    const guard = pickpocket.find((value) =>
        hasPickpocketNpcs(value) && value.npcs.some((npc) => npc.alias === 'guard1'));
    if (!hasPickpocketNpcs(guard) || guard.level !== 40) throw new Error(`${revision}: Guard thieving level mismatch`);
    const invalidPickpocket = pickpocket.some((value) => {
        if (!hasPickpocketNpcs(value)) return true;
        const npcs = value.npcs;
        if ('kind' in value && value.kind === 'level_only') {
            return npcs.length !== 1
                || !('level' in value)
                || typeof value.level !== 'number'
                || !Number.isInteger(value.level)
                || !('source_file' in value)
                || typeof value.source_file !== 'string'
                || !('target_row' in value)
                || typeof value.target_row !== 'string'
                || !('source_row' in value)
                || typeof value.source_row !== 'string'
                || ['experience', 'stun_ticks', 'stun_damage', 'success_chance', 'loot', 'pocket']
                    .some((key) => Object.hasOwn(value, key));
        }
        if ('kind' in value || !('loot' in value) || !Array.isArray(value.loot)) return true;
        const loot: unknown[] = value.loot;
        return loot.some((item) => {
            if (
                typeof item !== 'object'
                || item === null
                || Array.isArray(item)
                || !('min' in item)
                || typeof item.min !== 'number'
                || !('max' in item)
                || typeof item.max !== 'number'
            ) return true;
            return item.min > item.max;
        });
    });
    if (invalidPickpocket) throw new Error(`${revision}: invalid pickpocket joins`);
    if (revision === 274 || revision === 289) {
        const expected = [
            ['digworkman1', 25, 'scripts/quests/quest_itexam/scripts/digsite_workman.rs2', '[opnpc3,digworkman1] @pickpocket_digworkman1;', 'if (stat(thieving) < 25) {'],
            ['digworkman2', 25, 'scripts/quests/quest_itexam/scripts/digsite_workman.rs2', '[opnpc3,digworkman2] @pickpocket_digworkman1;', 'if (stat(thieving) < 25) {'],
            ['troll_prison_guard1', 30, 'scripts/quests/quest_troll/scripts/troll_stronghold_camp_guard.rs2', '[opnpc3,troll_prison_guard1]', 'if (stat(thieving) < 30) {'],
            ['troll_prison_guard2', 30, 'scripts/quests/quest_troll/scripts/troll_stronghold_camp_guard.rs2', '[opnpc3,troll_prison_guard2]', 'if (stat(thieving) < 30) {'],
        ] as const;
        for (const [alias, level, sourceFile, targetRow, sourceRow] of expected) {
            const fact = pickpocket.find((value) =>
                hasPickpocketNpcs(value) && value.kind === 'level_only' && value.level === level
                && value.source_file === sourceFile && value.target_row === targetRow
                && value.source_row === sourceRow && value.npcs.length === 1
                && value.npcs[0].alias === alias);
            if (!fact) throw new Error(`${revision}: missing selected Thieving level row for ${alias}`);
        }
    }
    const drops = payload.drop_tables ?? [];
    assertEqual(drops.length, 5, `${revision} bounded drop row count`);
    assertEqual(JSON.stringify(drops.map((row: any) => row.name)), JSON.stringify(['Giant', 'Moss giant', 'Fire giant', 'Green dragon', 'Kalphite Queen']), `${revision} bounded drop target order`);
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
    const spellInput: unknown = payload.spells ?? [];
    if (!Array.isArray(spellInput) || !spellInput.every(isPublishedCombatSpell)) throw new Error(`${revision}: malformed combat spells`);
    const spells: PublishedCombatSpell[] = spellInput;
    const selectableSpells = spells.filter(spell => spell.autocast_selectable);
    if (spells.length !== 21 || selectableSpells.length !== 16 || spells[0]?.name !== 'Wind Strike' || spells[15]?.name !== 'Fire Wave' || selectableSpells.some((spell, index) => spell.ssb !== index)) throw new Error(`${revision}: named combat spells and 16-row chooser`);
    const manualSpells = spells.filter(spell => !spell.autocast_selectable);
    if (JSON.stringify(manualSpells.map(spell => spell.source_row)) !== JSON.stringify(['magic_spell_crumble_undead', 'magic_spell_saradomin_strike', 'magic_spell_claws_of_guthix', 'magic_spell_flames_of_zamorak', 'magic_spell_iban_blast']) || manualSpells.some(spell => spell.ssb !== -1 || spell.component_id < 0)) throw new Error(`${revision}: manual spell identities`);
    if (payload.failed_spell_impact !== 85 || spells.some(spell => spell.impact_spotanim === payload.failed_spell_impact)) throw new Error(`${revision}: splash must remain distinct from successful spell impacts`);
    const wind = spells.find(spell => spell.name === 'Wind Strike');
    if (!wind || wind.ssb !== 0 || wind.level !== 1 || JSON.stringify(wind.runes.map(rune => [rune.name, rune.count])) !== JSON.stringify([['Mind rune', 1], ['Air rune', 1]])) throw new Error(`${revision}: Wind Strike runes`);
    const fireWave = spells.find(spell => spell.name === 'Fire Wave');
    if (!fireWave || fireWave.ssb !== 15 || JSON.stringify(fireWave.runes.map(rune => [rune.name, rune.count])) !== JSON.stringify([['Blood rune', 1], ['Fire rune', 7], ['Air rune', 5]])) throw new Error(`${revision}: Fire Wave runes`);
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
    if (payload.gather_methods !== undefined || payload.gather_placements !== undefined || payload.provenance.placement_inputs !== undefined) throw new Error(`${revision}: gathering facts live in the gathering family, not the core asset`);
    const gathering = extractGatheringFamily(pin.contentRoot);
    const familyFile = path.join(root, `crates/api/data/game-data/${revision}/gathering.json`);
    if (!fs.readFileSync(familyFile).equals(Buffer.from(familyBytes(familyInputs(spec, pinnedCommits), gathering, 'gathering')))) throw new Error(`${revision}: gathering family differs from the writer extract`);
    assertEqual(JSON.stringify(manifestRow.families?.gathering), JSON.stringify({ path: `${revision}/gathering.json`, schema: gathering.schema, ...digest(familyFile) }), `${revision} manifest gathering descriptor`);
    assertEqual(JSON.stringify(manifestRow.gathering), JSON.stringify(gathering.summary), `${revision} manifest gathering summary`);
    assertEqual(JSON.stringify(payload.mining_hazards), JSON.stringify(miningHazards(gathering.payload)), `${revision} core mining_hazards is the family's hazard slice`);
    const gatherItemNames = new Map<number, string>();
    for (const item of payload.items) if (item.name !== null && item.name !== '') gatherItemNames.set(item.id, item.name);
    const gatherRows = gatherResources(gathering.payload, gatherItemNames);
    assertEqual(JSON.stringify(payload.gather_resources), JSON.stringify(gatherRows), `${revision} core gather_resources is the family's admitted option slice`);
    const fishingOptions = gatherRows.filter((row) => row.skill === 'fishing');
    const fishingLabels = fishingOptions.map((row) => row.label);
    if (new Set(fishingLabels).size !== fishingLabels.length) throw new Error(`${revision}: fishing gather labels are not unique`);
    for (const row of fishingOptions) {
        if (row.methods.length === 0
            || JSON.stringify(row.methods) !== JSON.stringify([...row.methods].sort(compareCodepoint))
            || row.method !== row.methods[0]
            || JSON.stringify(row.aliases) !== JSON.stringify(row.methods)) {
            throw new Error(`${revision}: fishing group methods/aliases are not stable: ${JSON.stringify(row)}`);
        }
    }
    // Content pins: the family and the writer could agree and both be wrong, so anchor a few facts to the pinned content.
    const gatherFacts = gathering.payload;
    const aliases = new Map(gatherFacts.entities.map((row) => { const [kind, id, alias] = row.split(' '); return [`${kind}:${id}`, alias]; }));
    for (const [field, kind] of [['hazard_npcs', 'npc'], ['incidental_gem_ids', 'obj']] as const) {
        const ids = gatherFacts[field];
        if (ids.length === 0 || ids.some((id, index) => !Number.isSafeInteger(id) || (index > 0 && ids[index - 1] >= id))) {
            throw new Error(`${revision}: ${field} must be a non-empty sorted unique id list`);
        }
        if (ids.some((id) => !aliases.has(`${kind}:${id}`))) throw new Error(`${revision}: ${field} ids must join to ${kind} entities`);
    }
    const gatherMethod = (id: string) => { const found = gatherFacts.methods.find((each) => each.id === id); if (!found) throw new Error(`${revision}: missing gather method ${id}`); return found; };
    const gatherKnown = <T>(cell: { state: string; value?: T }, label: string): T => { if (cell.state === 'unknown' || cell.value === undefined) throw new Error(`${revision}: ${label} is unknown`); return cell.value; };
    assertEqual(JSON.stringify(gathering.summary.methods), JSON.stringify({ woodcutting: 10, mining: 15, fishing: 16 }), `${revision} gather method counts`);
    const rocks = gathering.summary.rocks;
    assertEqual(rocks.population, rocks.resource + rocks.depleted + rocks.hazard + rocks.unclassified, `${revision} every rock accounted`);
    assertEqual(rocks.resource, 29, `${revision} resource rocks`);
    assertEqual(rocks.hazard, 22, `${revision} gas rocks`);
    // M-215: the tutorial Mine handlers provably yield copper/tin, so those rocks are typed resources.
    for (const [method, names] of [['mining.copper', ['copperrock1', 'copperrock2', 'newbiecopperrock']], ['mining.tin', ['tinrock1', 'tinrock2', 'newbietinrock']]] as const) {
        const typed = gatherKnown(gatherMethod(method).targets, `${method} targets`).filter((target) => target.class === 'resource');
        assertEqual(JSON.stringify(typed.map((target) => aliases.get(`loc:${target.id}`))), JSON.stringify(names), `${revision} ${method} resources`);
    }
    const limestone = gatherKnown(gatherMethod('mining.limestone').targets, 'limestone targets').filter((target) => target.class === 'resource');
    assertEqual(JSON.stringify(limestone.map((target) => gatherKnown(target.respawn, 'limestone respawn')?.raw)), JSON.stringify([10, 20, 40]), `${revision} limestone distinct respawn rates`);
    const essence = gatherKnown(gatherMethod('mining.rune stones').targets, 'essence targets')[0];
    assertEqual(JSON.stringify(gatherKnown(essence.respawn, 'essence respawn')), 'null', `${revision} rune essence never respawns`);
    const fly = gatherMethod('fishing.freshfish.op1');
    assertEqual(JSON.stringify(gatherKnown(fly.tools, 'fly tools').map((tool) => aliases.get(`obj:${tool.item}`))), JSON.stringify(['fly_fishing_rod']), `${revision} lure tool`);
    assertEqual(JSON.stringify(gatherKnown(fly.consumes, 'lure bait').map((amount) => [aliases.get(`obj:${amount.item}`), amount.count])), JSON.stringify([['feather', 1]]), `${revision} lure bait`);
    assertEqual(JSON.stringify(gatherKnown(fly.products, 'lure products').map((each) => [aliases.get(`obj:${each.item}`), each.level])), JSON.stringify([['raw_trout', 20], ['raw_salmon', 30]]), `${revision} lure products`);
    const contest = gatherMethod('fishing.0_41_53_sinisterfishspot.op1');
    assertEqual(JSON.stringify(contest.resources), JSON.stringify(['raw_giant_carp']), `${revision} Hemenster carp method`);
    assertEqual(contest.sources.some((source) => source.startsWith('scripts/quests/quest_fishingcompo/scripts/hemenster_fishing.rs2:')), true, `${revision} Hemenster source provenance`);
    assertEqual(JSON.stringify(contest.op), JSON.stringify({ slot: 1, label: 'Fish' }), `${revision} Hemenster spot operation`);
    const target = gatherKnown(contest.targets, 'Hemenster carp target')[0]!;
    assertEqual(JSON.stringify([target.kind, target.id, target.op, target.class, aliases.get(`npc:${target.id}`)]), JSON.stringify(['npc', 234, 1, 'resource', '0_41_53_sinisterfishspot']), `${revision} Hemenster target`);
    assertEqual(JSON.stringify(gatherKnown(contest.tools, 'Hemenster carp tools').map((tool) => [aliases.get(`obj:${tool.item}`), tool.item])), JSON.stringify([['fishing_rod', 307]]), `${revision} Hemenster rod`);
    assertEqual(JSON.stringify(gatherKnown(contest.consumes, 'Hemenster carp bait').map((amount) => [aliases.get(`obj:${amount.item}`), amount.item, amount.count])), JSON.stringify([['red_vine_worm', 25, 1]]), `${revision} Hemenster worm`);
    assertEqual(JSON.stringify(gatherKnown(contest.products, 'Hemenster carp product').map((product) => [aliases.get(`obj:${product.item}`), product.item, product.level])), JSON.stringify([['raw_giant_carp', 338, 10]]), `${revision} Hemenster carp level`);
    assertEqual(JSON.stringify(contest.requirements.state === 'partial' ? contest.requirements.gaps.map((each) => each.code) : []), JSON.stringify(['varp-gate']), `${revision} Hemenster quest-state gate remains explicit`);
    assertEqual(JSON.stringify(gatherKnown(contest.requirements, 'Hemenster carp requirements').map((requirement) => requirement.kind === 'skill' ? [requirement.kind, requirement.skill, requirement.level] : [requirement.kind])), JSON.stringify([['skill', 10, 10]]), `${revision} Hemenster skill requirement`);
    const oak = gatherMethod('woodcutting.oak');
    assertEqual(JSON.stringify(gatherKnown(oak.products, 'oak products').map((each) => [aliases.get(`obj:${each.item}`), each.level])), JSON.stringify([['oak_logs', 15]]), `${revision} oak product`);
    if (gathering.summary.placements.rows === 0 || gatherFacts.placements.some((file) => !/^maps\/m\d+_\d+\.jm2$/.test(file.file))) throw new Error(`${revision}: gathering placements are map rows`);
    for (const banned of ['curated', 'frozen', 'operator_placement', 'RIDDLE_KEY_COORDS', 'HARD_SPECIAL_COORDS', 'invented']) {
        if (JSON.stringify(gatherFacts).includes(banned)) throw new Error(`${revision}: gathering family published ${banned}`);
    }
    const questIdentity = payload.quest_identity;
    if (!questIdentity || Array.isArray(questIdentity) || !Array.isArray(questIdentity.rows)) throw new Error(`${revision}: quest_identity must be one family object, not a bare list`);
    if (payload.quest_prereqs !== undefined) throw new Error(`${revision}: quest_prereqs is not a second field`);
    const extractedQuest = extractQuestIdentityFacts(pin.contentRoot, revision);
    assertEqual(JSON.stringify(questIdentity), JSON.stringify(extractedQuest), `${revision} quest_identity matches the writer extract`);
    if (revision === 289 && payload.quest_starts === undefined) throw new Error(`${revision}: quest_starts family missing`);
    if (payload.quest_starts !== undefined) {
        const questStarts = payload.quest_starts;
        if (Array.isArray(questStarts) || questStarts.schema !== 1 || questStarts.revision !== revision
            || questStarts.content_id !== pin.cache.content_id || !Array.isArray(questStarts.rows)) {
            throw new Error(`${revision}: quest_starts family shape or identity mismatch`);
        }
        const extractedQuestStarts = extractQuestStartFacts(pin.contentRoot, questIdentity.rows, revision, pin.cache.content_id);
        assertEqual(JSON.stringify(questStarts), JSON.stringify(extractedQuestStarts), `${revision} quest_starts matches the writer extract`);
    }
    const expectedQuest = [
        ['cook', "Cook's Assistant", 'cook', 'cookquest', 29, 2, 1],
        ['runemysteries', 'Rune Mysteries Quest', 'runemysteries', 'runemysteries', 63, 6, 1],
        ['murder', 'Murder Mystery', 'murder', 'murderquest', 192, 2, 3],
        ['waterfall', 'Waterfall Quest', 'waterfall', 'waterfall_quest', 65, 10, 1],
        ['death', 'Death Plateau', 'death', 'death_equiproom', 314, 80, 1],
        ['zanaris', 'Lost City', 'zanaris', 'zanaris', 147, 6, 3],
    ];
    if (revision === 289) assertEqual(questIdentity.rows.length, 69, `${revision} quest rows`);
    const byId = Object.fromEntries(questIdentity.rows.map((row: any) => [row.id, row]));
    assertEqual(JSON.stringify(expectedQuest.map((row) => [byId[row[0]].id, byId[row[0]].display, byId[row[0]].component, byId[row[0]].varp, byId[row[0]].varp_id, byId[row[0]].complete, byId[row[0]].quest_points])), JSON.stringify(expectedQuest), `${revision} quest identity seed rows`);
    if (questIdentity.rows.some((row: any) => row.requirements.qualification !== 'partial' || row.requirements.unknown_as_satisfied !== false || row.unknown_sides.length !== 0 || row.enum_index !== undefined || row.engine_id !== undefined || typeof row.complete !== 'number')) throw new Error(`${revision}: quest row promoted, ranged, or aliased`);
    const cookQuest = questIdentity.rows[0];
    assertEqual(JSON.stringify(cookQuest.requirements.items), JSON.stringify([{ alias: 'egg', quantity: 1, kind: 'inv' }, { alias: 'bucket_milk', quantity: 1, kind: 'inv' }, { alias: 'pot_flour', quantity: 1, kind: 'inv' }]), `${revision} cook aliases`);
    if (cookQuest.requirements.empty_must_have !== false) throw new Error(`${revision}: cook mustHave is not empty`);
    for (const id of ['runemysteries', 'murder', 'death']) {
        const row = questIdentity.rows.find((entry: any) => entry.id === id);
        if (!row.requirements.empty_must_have || row.requirements.items.length !== 0 || row.requirements.skills.length !== 0) throw new Error(`${revision}: ${id} empty mustHave`);
    }
    if (revision === 289) {
        if (!payload.npc_names || !Array.isArray(payload.npc_names.rows) || payload.npc_names.rows.length === 0) {
            throw new Error(`${revision}: npc_names family missing`);
        }
        if (!payload.npc_names.rows.some((row: { config: string; id: number }) => row.config === 'khazard_warlord' && row.id === 477)) {
            throw new Error(`${revision}: npc_names missing khazard_warlord`);
        }
        if (!payload.loc_names || !Array.isArray(payload.loc_names.rows) || !payload.loc_names.rows.some((row: { config: string; id: number }) => row.config === 'hopper_lumbridge' && row.id === 2714)) {
            throw new Error(`${revision}: loc_names missing hopper_lumbridge`);
        }
        if (!payload.npc_placements || !Array.isArray(payload.npc_placements.rows) || payload.npc_placements.rows.length === 0) {
            throw new Error(`${revision}: npc_placements family missing`);
        }
    }
    const waterfallQuest = questIdentity.rows.find((row: any) => row.id === 'waterfall');
    assertEqual(JSON.stringify(waterfallQuest.requirements.items), JSON.stringify([{ alias: 'rope', quantity: null, kind: 'use-site' }]), `${revision} waterfall rope`);
    const zanarisQuest = questIdentity.rows.find((row: any) => row.id === 'zanaris');
    assertEqual(JSON.stringify(zanarisQuest.requirements.skills), JSON.stringify([{ skill: 'woodcutting', level: 36 }, { skill: 'crafting', level: 31 }]), `${revision} zanaris gates`);
    if (zanarisQuest.requirements.items.length !== 0) throw new Error(`${revision}: zanaris must not add axe, knife, branch, or spirit`);
    const questBlob = JSON.stringify(questIdentity);
    for (const banned of ['family-unavailable', 'enum_index', 'Lost City Of Zanaris', 'death_climbingboots', 'death_secretwaymap', 'death_spikedboots', 'Egg', 'Pot of flour', 'Bucket of milk']) {
        if (questBlob.includes(banned)) throw new Error(`${revision}: quest identity published ${banned}`);
    }
    if (questIdentity.rows.some((row: { varp_id: number; varp: string }) => row.varp_id === 101 || row.varp_id === 219 || row.varp === 'death')) throw new Error(`${revision}: quest identity published a rejected binding`);
    if (revision === 274) {
        assertEqual(questIdentity.coverage.length, 1, '274 quest coverage');
        assertEqual(JSON.stringify(questIdentity.coverage[0]), JSON.stringify({ class: 'revision-absent', alias: 'routequest', on_revision: 274, other_pin_id: 387, copied: false, reason: '289-only quest, not copied onto 274' }), '274 routequest coverage');
        if (questIdentity.coverage[0].display !== undefined || questIdentity.coverage[0].complete !== undefined || questIdentity.coverage[0].quest_points !== undefined) throw new Error('274 copied routequest facts');
    } else {
        if (questIdentity.coverage.length !== 0) throw new Error('289 quest coverage must be empty');
        assertEqual(questIdentity.rows.length, 69, '289 identity roster');
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
    const rs2b0tRoot = requireEnvPath('RS2B0T', 'the pinned rs2b0t checkout root');
    assertRs2b0tPinned(rs2b0tRoot);
    const bankSource = path.join(rs2b0tRoot, 'src/bot/api/bank/BankLocations.ts');
    const bankCatalog = await extractBankCatalog(pin.engineRoot, fs.readFileSync(bankSource, 'utf8'));
    const bankPlacements = extractBankPlacements(pin.contentRoot, bankCatalog);
    assertEqual(JSON.stringify(payload.bank_placements), JSON.stringify(bankPlacements.facts), `${revision} bank access placements match selected content`);
    assertEqual(JSON.stringify(payload.provenance.bank_inputs), JSON.stringify({
        catalog: { path: 'rs2b0t-00d39a17e0/src/bot/api/bank/BankLocations.ts', ...digest(bankSource) },
        ...bankPlacements.inputs,
    }), `${revision} bank catalog and placement inputs`);
    assertEqual(fs.readFileSync(path.join(root, 'crates/api/data/game-data/bank-catalog.rs'), 'utf8'), bankCatalogRust(bankCatalog), `${revision} compiled bank roster matches frozen AST`);
    // Named sites (§2.3 P-sites): the core table is the family's site slice and the manifest carries its report.
    const siteResult = gatherSites(gathering.payload, gatherRows, pin.contentRoot, bankCatalog);
    assertEqual(JSON.stringify(payload.gather_sites), JSON.stringify(siteResult.rows), `${revision} core gather_sites is the family's named site slice`);
    assertEqual(JSON.stringify(manifestRow.gather_sites), JSON.stringify(siteResult.report), `${revision} manifest gather_sites summary`);
    const expectedSiteReport = revision === 289
        ? { woodcutting: { sites: 246, direct: 0, dropped: 2537, outside_box: 0 }, mining: { sites: 33, direct: 0, dropped: 16, outside_box: 0 }, fishing: { sites: 32, direct: 20, dropped: 9, outside_box: 0 } }
        : { woodcutting: { sites: 243, direct: 0, dropped: 2287, outside_box: 0 }, mining: { sites: 33, direct: 0, dropped: 9, outside_box: 0 }, fishing: { sites: 32, direct: 20, dropped: 7, outside_box: 0 } };
    assertEqual(JSON.stringify(siteResult.report), JSON.stringify(expectedSiteReport), `${revision} gather_sites report`);
    assertEqual((payload.gather_sites as GatherSiteWire[]).length, revision === 289 ? 311 : 308, `${revision} gather_sites row count`);
    const siteRows = payload.gather_sites as GatherSiteWire[];
    const siteRow = (id: string) => {
        const found = siteRows.find((row) => row.id === id);
        if (!found) throw new Error(`${revision}: missing gather site ${id}`);
        return found;
    };
    const assertSite = (id: string, label: string, region: { min_x: number; min_z: number; max_x: number; max_z: number; level: number }) => {
        const found = siteRow(id);
        assertEqual(found.label, label, `${revision} ${id} label`);
        assertEqual(JSON.stringify(found.region), JSON.stringify(region), `${revision} ${id} region`);
    };
    assertSite('fishing.musa_point', 'Musa Point · Bait, Cage, Harpoon, Net', { min_x: 2923, min_z: 3179, max_x: 2926, max_z: 3181, level: 0 });
    assertSite('fishing.barbarian_village', 'Barbarian Village · Bait, Lure', { min_x: 3104, min_z: 3424, max_x: 3110, max_z: 3434, level: 0 });
    assertSite('fishing.catherby', 'Catherby · Harpoon, Net, Bait, Cage', { min_x: 2836, min_z: 3423, max_x: 2859, max_z: 3431, level: 0 });
    assertSite('fishing.baxtorian_falls', 'Baxtorian Falls · Bait, Lure', { min_x: 2527, min_z: 3403, max_x: 2537, max_z: 3412, level: 0 });
    assertSite('fishing.baxtorian_falls.2', 'Baxtorian Falls (2) · Bait, Lure', { min_x: 2508, min_z: 3421, max_x: 2508, max_z: 3421, level: 0 });
    assertSite('fishing.agility_training_area.sw', 'Agility Training Area SW19 · Bait, Lure', revision === 289
        ? { min_x: 2501, min_z: 3498, max_x: 2520, max_z: 3518, level: 0 }
        : { min_x: 2498, min_z: 3498, max_x: 2520, max_z: 3518, level: 0 });
    assertSite('fishing.rimmington', 'Rimmington · Bait, Net', { min_x: 2986, min_z: 3176, max_x: 2986, max_z: 3176, level: 0 });
    assertSite('fishing.rimmington.2', 'Rimmington (2) · Bait, Net', { min_x: 2996, min_z: 3158, max_x: 2996, max_z: 3158, level: 0 });
    assertSite('mining.varrock_east.se', 'Varrock East SE50 · Copper ore 9, Tin ore 6, Iron ore 4', { min_x: 3282, min_z: 3361, max_x: 3290, max_z: 3370, level: 0 });
    assertSite('mining.lumbridge.ne', 'Lumbridge NE54 · Iron ore 9, Silver ore 5, Coal 3 +5', { min_x: 3293, min_z: 3284, max_x: 3305, max_z: 3318, level: 0 });
    assertSite('mining.al_kharid.w', 'Al Kharid W26 · Coal 7, Mithril ore 5, Adamantite ore 2', { min_x: 3233, min_z: 3157, max_x: 3243, max_z: 3167, level: 0 });
    assertSite('mining.river_lum', 'River Lum · Tin ore 8, Clay 3, Iron ore 3 +1', { min_x: 3172, min_z: 3365, max_x: 3183, max_z: 3377, level: 0 });
    assertSite('woodcutting.draynor', 'Draynor · Logs 25, Willow logs 5, Oak logs 4', { min_x: 3082, min_z: 3200, max_x: 3136, max_z: 3256, level: 0 });
    for (const gone of ['fishing.rimmington.sw', 'fishing.cooks_guild.w']) {
        if (siteRows.some((row) => row.id === gone)) throw new Error(`${revision}: round-2 site ${gone} is not a row`);
    }
    // The published wire shape: exactly the TS GatherSiteWire fields, with region keys in writer order.
    for (const row of siteRows) {
        assertEqual(JSON.stringify(Object.keys(row).sort()), JSON.stringify(['id', 'keys', 'label', 'region', 'skill']), `${revision} ${row.id} wire fields`);
        assertEqual(JSON.stringify(Object.keys(row.region).sort()), JSON.stringify(['level', 'max_x', 'max_z', 'min_x', 'min_z']), `${revision} ${row.id} region fields`);
    }
    publishedSiteIds.set(revision, siteRows.map((row) => row.id));
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
    const debug = await verifyDebugArtifact(revision, manifestRow, pin);
    if (debug.status !== 'verified') process.exitCode = 1;
    results.push({ revision, records: payload.items.length, consumption: payload.consumption.length, pickpocket: payload.pickpocket.length, drop_tables: drops.length, spells: spells.length, staves: staves.length, herbs: herbs.length, prayers: prayers.length, pickaxes: nurmof.pickaxes.length, flour_six: 6, gathering: gathering.summary, quest_identity: { rows: questIdentity.rows.length, coverage: questIdentity.coverage.length }, trails: { rows: trailRows.length, clues: trailRows.filter((row: any) => row.role === 'clue').length, caskets: trailRows.filter((row: any) => row.role === 'casket').length, challenge_answers: trails.challenge_answers.length, access_constrained: constrained.length }, talk_key: { talk: talkRows.length, talk_with_spawn: talkRows.filter((row) => row.spawn !== undefined).length, keys: keyRows.length, keys_with_spawn: keyRows.filter((row) => row.spawn !== undefined).length, coverage: talkKey.coverage.length, scripts: extractedTalkKey.inputs.scripts.files, npc_configs: extractedTalkKey.inputs.npc_configs.files }, trio_givers: { rows: giverRows.length, with_spawn: giverRows.filter((row) => row.spawn !== undefined).length, coverage: trioGivers.coverage.length, maps: extractedTrioGivers.inputs.maps.files, handlers: extractedTrioGivers.inputs.handlers.length, npc_configs: extractedTrioGivers.inputs.npc_configs.length, aliases: giverRows.map((row) => row.alias) }, equipment_names: { resolved: resolved.length, absent: absent.length, family_counts: familyCounts }, fire_staff_providers: fireProviders, autocast, duel, special: { energy_varp: special.energy_varp, armed_varp: special.armed_varp, max_energy: special.max_energy, bars: special.bars.length, weapons: special.weapons.length }, teleports: teleports.length, fixed_food_heals: Object.fromEntries(fixed), output_sha256: output.sha256, input_hashes: true, content_hashes: true, source_pins: true, dirty_gate: true, cache_identity: pin.cache });
    results[results.length - 1].debug = debug;
}
const requested = requestedRevisions(process.argv.slice(2));
const refused: { revision: number; reason: string }[] = [];
for (const revision of requested) {
    try {
        await verifyRevision(revision);
    } catch (error) {
        const reason = error instanceof Error ? error.message : String(error);
        refused.push({ revision, reason });
        console.error(`${revision}: REFUSED: ${reason}`);
        if (revision === 289 || requested.length === 1) process.exitCode = 1;
    }
}
const pinnedGivers274 = publishedTrioGivers.get(274); const pinnedGivers289 = publishedTrioGivers.get(289);
const crossPin = pinnedGivers274 && pinnedGivers289 ? 'verified' : 'not checked: both revisions did not verify';
if (pinnedGivers274 && pinnedGivers289 && JSON.stringify(pinnedGivers289) !== JSON.stringify(pinnedGivers274)) throw new Error('trio_givers: the two pins disagree on the selected identity, display name, or unique spawn');
const pinnedSites274 = publishedSiteIds.get(274); const pinnedSites289 = publishedSiteIds.get(289);
if (pinnedSites274 && pinnedSites289) {
    assertEqual(pinnedSites274.filter((id) => pinnedSites289.includes(id)).length, 308, 'gather_sites shared ids across revisions');
    const extra289 = pinnedSites289.filter((id) => !pinnedSites274.includes(id));
    assertEqual(JSON.stringify(extra289.sort()), JSON.stringify(['woodcutting.mort_ton.e', 'woodcutting.mort_ton.e.2', 'woodcutting.troll_stronghold.nw']), 'gather_sites 289-only rows');
    assertEqual(pinnedSites274.filter((id) => !pinnedSites289.includes(id)).length, 0, 'gather_sites 274-only rows');
}
const evidence = { schema_version: 4, generator: 'tools/game-data/generate.ts', verification: 'tools/game-data/verify.ts', revisions: results, refused, cross_pin: crossPin };
const evidencePath = path.join(root, 'docs/compat/evidence/generated-game-data/verification.json');
fs.mkdirSync(path.dirname(evidencePath), { recursive: true }); fs.writeFileSync(evidencePath, `${JSON.stringify(evidence, null, 2)}\n`); console.log(JSON.stringify(evidence));
