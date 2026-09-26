import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { verifyCacheIdentity } from './cache-identity.ts';

type ObjType = { id: number; debugname: string | null; name: string | null; cost: number; stackable: boolean; members: boolean; certlink: number; certtemplate: number; wearpos: number; wearpos2: number; wearpos3: number; tradeable?: boolean; countobj?: ArrayLike<number> | null; params?: Map<number, number | string> };
type NpcType = { id: number; debugname?: string | null; name: string | null };
type Revision = { revision: number; engine: string; content: string; expectedEngine: string; expectedContent: string; cacheIdentity: { cache_id: string; content_id?: string; nav_sha256: string; flags_sha256: string }; output: string };

const root = path.resolve(import.meta.dirname, '../..');
const envPath = (name: string, fallback: string) => process.env[name] ? path.resolve(process.env[name]!) : fallback;
const revisions: Revision[] = [
    { revision: 274, engine: envPath('GAME_DATA_274_ENGINE', '/Users/acfrazier/experiments/Server/engine'), content: envPath('GAME_DATA_274_CONTENT', '/Users/acfrazier/experiments/Server/content'), expectedEngine: '4c95f87efe00b068cadbd229d94736626907bd1a', expectedContent: '000c19997e07206131bcb3c884265840efce416d', cacheIdentity: { cache_id: '4aac9b63312dcb75d5de8f686772d083ba0808c57985438246edf21ef522be1c', content_id: '0d14c891b5727142379c6d8844bd5bb1ff9874d4d996db1dc5ceea0e9062469c', nav_sha256: '05db24743e9f549ced16c1f00b87c30a390d3aaec391815da3f3563130b3bcd4', flags_sha256: '92d5dea05c886ac8720be6b47e47cbc68355a8ff42676c0886f5b7ea8343a4cb' }, output: path.join(root, 'crates/api/data/game-data/274.json') },
    { revision: 289, engine: envPath('GAME_DATA_289_ENGINE', '/Users/acfrazier/experiments/lostcity-289/engine'), content: envPath('GAME_DATA_289_CONTENT', '/Users/acfrazier/experiments/lostcity-289/content'), expectedEngine: 'cc359656b4acd216ca452495874b6beba9a0ac75', expectedContent: '92649430fcbc83538d8c4367ecb96cee1a67a944', cacheIdentity: { cache_id: 'c4d8ab36bcfd2a7907535b4f619e28623b0a22e98d496fd2a9620d544c5b5b09', content_id: 'cdb2f161c35239f09bf5175648e15e7dbbc4bbf9be4419cea41f7053ccf8b044', nav_sha256: '131db92e32eddcb08148909d477589544320fe7e34a422e8004e97e888032924', flags_sha256: '67e4094dff06def5cf8abc172ce751f4ca8679532ba04c1ba15ab6bf668c7a4a' }, output: path.join(root, 'crates/api/data/game-data/289.json') }
];
const decoderSources = [
    'src/cache/config/ObjType.ts', 'src/cache/config/NpcType.ts', 'src/cache/config/ConfigType.ts', 'src/cache/config/ParamHelper.ts', 'src/cache/config/ParamType.ts', 'src/cache/config/ScriptVarType.ts',
    'src/io/BZip2.ts', 'src/io/Jagfile.ts', 'src/io/Packet.ts', 'src/datastruct/DoublyLinkable.ts', 'src/datastruct/LinkList.ts', 'src/datastruct/Linkable.ts', 'src/util/Environment.ts', 'src/util/Logger.ts', 'src/util/TryParse.ts', 'src/util/WorldConfig.ts'
];
const dropContentFiles = [
    'scripts/_unpack/225/all.npc',
    'scripts/drop tables/scripts/giant.rs2',
    'scripts/drop tables/scripts/moss_giant.rs2',
    'scripts/drop tables/scripts/fire_giant.rs2',
    'scripts/drop tables/scripts/green_dragon.rs2',
    'scripts/drop tables/scripts/shared_droptables.rs2',
];
export const gatherContentFiles = [
    'scripts/skill_mining/configs/mine.dbrow',
    'scripts/skill_mining/configs/rocks.loc',
    'scripts/skill_woodcutting/configs/trees.dbrow',
    'scripts/skill_woodcutting/configs/trees/achey.loc',
    'scripts/skill_woodcutting/configs/trees/burnt.loc',
    'scripts/skill_woodcutting/configs/trees/hollow.loc',
    'scripts/skill_woodcutting/configs/trees/magic.loc',
    'scripts/skill_woodcutting/configs/trees/maple.loc',
    'scripts/skill_woodcutting/configs/trees/normal.loc',
    'scripts/skill_woodcutting/configs/trees/oak.loc',
    'scripts/skill_woodcutting/configs/trees/willow.loc',
    'scripts/skill_woodcutting/configs/trees/yew.loc',
    'scripts/skill_fishing/configs/fishing.npc',
];
const prayerContentFiles = [
    'scripts/skill_prayer/configs/prayers.dbrow',
    'scripts/skill_prayer/configs/prayers.constant',
    'scripts/skill_prayer/interfaces/prayer.if',
];
const nurmofEssenceContentFiles = [
    'scripts/areas/area_falador/configs/dwarven_mine.inv',
    'scripts/areas/area_falador/configs/dwarven_mine.npc',
    'scripts/skill_runecraft/configs/runecraft.constant',
    'maps/m45_75.jm2',
    'pack/npc.pack',
];
const flourSixContentFiles = [
    'scripts/quests/quest_murder/configs/quest_murder.loc',
    'scripts/general/configs/quest.enum',
    'maps/m42_55.jm2',
    'pack/loc.pack',
    'pack/obj.pack',
];
export const questIdentityContentFiles = [
    'scripts/general/scripts/quests.rs2',
    'scripts/general/configs/quest.constant',
    'scripts/player/interfaces/questlist.if',
    'scripts/quests/quest_cook/scripts/quest_cook.rs2',
    'scripts/quests/quest_waterfall/scripts/quest_waterfall.rs2',
    'scripts/quests/quest_zanaris/scripts/quest_zanaris.rs2',
];
export const trailContentFiles = [
    'scripts/minigames/game_trail/configs/trail_easy.enum',
    'scripts/minigames/game_trail/configs/trail_easy.obj',
    'scripts/minigames/game_trail/configs/trail_medium.enum',
    'scripts/minigames/game_trail/configs/trail_medium.obj',
    'scripts/minigames/game_trail/configs/trail_hard.enum',
    'scripts/minigames/game_trail/configs/trail_hard.obj',
    'scripts/minigames/game_trail/configs/trail_casket.obj',
];
const contentFiles = ['scripts/player/configs/consumption/consume.dbtable', 'scripts/player/configs/consumption/consume_normal.dbrow', 'scripts/player/configs/consumption/consume_effects.dbrow', 'scripts/skill_thieving/configs/pickpocking/pickpocket.dbtable', 'scripts/skill_thieving/configs/pickpocking/pickpocket.dbrow', 'scripts/player/scripts/consumption/effects/scripts/consume_effects.rs2', 'scripts/skill_combat/configs/magic/magic_combat_spells.dbrow', 'scripts/skill_magic/configs/magic.dbtable', 'scripts/skill_magic/configs/magic_spells.dbrow', 'scripts/skill_magic/configs/magic_staff.dbrow', 'scripts/skill_combat/configs/combat.constant', 'scripts/skill_herblore/configs/herbs.obj', 'scripts/skill_herblore/configs/identifying/identify.param', 'scripts/skill_herblore/scripts/identifying/identify.rs2', ...prayerContentFiles, ...nurmofEssenceContentFiles, ...flourSixContentFiles, 'pack/interface.pack', 'pack/varp.pack', 'pack/param.pack', ...dropContentFiles, ...gatherContentFiles, ...questIdentityContentFiles, ...trailContentFiles];
function sha256(file: string) { const data = fs.readFileSync(file); return { bytes: data.length, sha256: crypto.createHash('sha256').update(data).digest('hex') }; }
function commit(dir: string) { return execFileSync('git', ['-C', dir, 'rev-parse', 'HEAD'], { encoding: 'utf8' }).trim(); }
function sourceFile(dir: string, relative: string) { return { path: relative, ...sha256(path.join(dir, relative)) }; }
export function assertPinned(spec: Revision) {
    const engineCommit = commit(spec.engine); const contentCommit = commit(spec.content);
    if (engineCommit !== spec.expectedEngine || contentCommit !== spec.expectedContent) throw new Error(`${spec.revision}: expected pinned commits, got ${engineCommit}/${contentCommit}`);
    const engineInputs = [...decoderSources, 'data/pack/server/obj.dat', 'data/pack/server/npc.dat', 'data/pack/client/config'];
    const dirtyEngine = execFileSync('git', ['-C', spec.engine, 'status', '--porcelain', '--untracked-files=all', '--', ...engineInputs], { encoding: 'utf8' }).trim();
    if (dirtyEngine) throw new Error(`${spec.revision}: relevant engine inputs are dirty:\n${dirtyEngine}`);
    const dirtyContent = contentDirt(spec.content);
    if (dirtyContent) throw new Error(`${spec.revision}: relevant content inputs are dirty:\n${dirtyContent}`);
    return { engineCommit, contentCommit };
}
/**
 * Every content path the extractors read beyond `contentFiles`: the placement
 * families read every on-disk map, the bank and cook families every `.loc`
 * (and the bank family every `.npc`) config under `scripts/`, and both packs.
 * A git pathspec `*` spans directories, so the tree specs cover every depth.
 */
export const CONTENT_TREE_PATHSPECS = ['maps', 'scripts/*.loc', 'scripts/*.npc', 'pack/loc.pack', 'pack/npc.pack'];
/** Porcelain status of every selected-content input; empty when clean. */
export function contentDirt(content: string) {
    return execFileSync('git', ['-C', content, 'status', '--porcelain', '--untracked-files=all', '--', ...contentFiles, ...CONTENT_TREE_PATHSPECS], { encoding: 'utf8' }).trim();
}
/**
 * The rs2b0t sources the generator reads, as git blob ids at the pinned
 * commit (`git rev-parse 00d39a17e056df6c5e461f3f2cfd3598ff9720b6:<path>`).
 * The pin is a `git archive` export without history, so identity is checked
 * per file: a different commit or a local edit changes the blob id.
 */
export const RS2B0T_PIN = {
    commit: '00d39a17e056df6c5e461f3f2cfd3598ff9720b6',
    blobs: {
        'src/bot/api/bank/BankLocations.ts': '82171ae05322fa3abfd7db1161eb3efd1a3543e6',
        'src/bot/data/cookLocations.ts': '7c5b1a858662cd06b345228de63c7b5628ae591f',
        'src/bot/data/cookingRanges.ts': '4606747ff64fc31c2b9af20b7853d2c7dd655a48',
        'tools/cooking/gen-cooksurfaces.ts': '83db08d8327eb0356daea08e75fda29f8aaf0b4b',
    } as Record<string, string>,
};
/** Refuse an rs2b0t root whose read sources are not the pinned commit's blobs. */
export function assertRs2b0tPinned(rs2b0tRoot: string, pin = RS2B0T_PIN) {
    for (const [relative, expected] of Object.entries(pin.blobs)) {
        const bytes = fs.readFileSync(path.join(rs2b0tRoot, relative));
        const blob = crypto.createHash('sha1').update(`blob ${bytes.length}\0`).update(bytes).digest('hex');
        if (blob !== expected) throw new Error(`rs2b0t ${relative}: blob ${blob} is not ${pin.commit}'s ${expected} (dirty or not the pinned export)`);
    }
}
/**
 * `tradeable` is the engine's own decode after load: opcode 15 (`tradeable=no`), a nonzero `dummyitem`, or a note of an
 * untradeable item. `stack_variant` marks the objs another obj names as a pile-size model (`countobj`); they repeat the
 * base name and are not separate items.
 */
function row(obj: ObjType, piles: ReadonlySet<number>) { if (typeof obj.tradeable !== 'boolean') throw new Error(`obj ${obj.id}: engine decode has no tradeable flag`); return { alias: obj.debugname, id: obj.id, name: obj.name, cost: obj.cost, stackable: obj.stackable, members: obj.members, certificate_link: obj.certlink, certificate_template: obj.certtemplate, wear_position: obj.wearpos, wear_position_2: obj.wearpos2, wear_position_3: obj.wearpos3, tradeable: obj.tradeable, stack_variant: piles.has(obj.id) }; }
function pileModels(objs: readonly ObjType[]) { const piles = new Set<number>(); for (const obj of objs) for (const pile of Array.from(obj.countobj ?? [])) if (pile > 0) piles.add(pile); return piles; }
export function parseRows(text: string) {
    const rows: { name: string; values: Record<string, string[][]> }[] = []; let current: { name: string; values: Record<string, string[][]> } | null = null;
    for (const raw of text.split(/\r?\n/)) { const line = raw.trim(); if (!line || line.startsWith('//')) continue; if (line.startsWith('[') && line.endsWith(']')) { current = { name: line.slice(1, -1), values: {} }; rows.push(current); continue; } if (!current || !line.startsWith('data=')) continue; const [, rest] = line.split('=', 2); const [key, ...values] = rest.split(','); (current.values[key] ??= []).push(values); }
    return rows;
}
function required(values: Record<string, string[][]>, key: string, rowName: string) { const value = values[key]?.[0]?.[0]; if (value === undefined) throw new Error(`${rowName}: missing ${key}`); return value; }
function namedItem(itemIds: Map<string, { id: number; name: string | null }>, alias: string, label: string) {
    const item = itemIds.get(alias);
    if (!item) throw new Error(`${label}: unknown item ${alias}`);
    if (!item.name) throw new Error(`${label}: ${alias} has no display name`);
    return { alias, id: item.id, name: item.name };
}
function runeCosts(values: Record<string, string[][]>, rowName: string, itemIds: Map<string, { id: number; name: string | null }>) {
    const raw = values.runesrequired?.[0] ?? [];
    const runes: { alias: string; id: number; name: string; count: number }[] = [];
    for (let i = 0; i + 1 < raw.length; i += 2) {
        const alias = raw[i];
        if (!alias || alias === 'null') continue;
        const item = namedItem(itemIds, alias, `${rowName} rune`);
        runes.push({ ...item, count: integer(raw[i + 1], `${rowName} ${alias}`) });
    }
    return runes;
}
export function extractMagicFacts(content: string, items: ObjType[]) {
    const itemIds = new Map(items.filter((item) => item.debugname !== null).map((item) => [item.debugname as string, { id: item.id, name: item.name }]));
    const spells = parseRows(fs.readFileSync(path.join(content, 'scripts/skill_combat/configs/magic/magic_combat_spells.dbrow'), 'utf8'))
        .filter((parsed) => parsed.values.name?.[0]?.[0] && parsed.values.continue_by_autocast?.[0]?.[0] === 'true')
        .map((parsed, ssb) => ({
            name: required(parsed.values, 'name', parsed.name),
            source_row: parsed.name,
            ssb,
            level: integer(required(parsed.values, 'levelrequired', parsed.name), parsed.name),
            continue_by_autocast: true,
            runes: runeCosts(parsed.values, parsed.name, itemIds)
        }));
    if (spells[0]?.name !== 'Wind Strike') {
        throw new Error(`expected Wind Strike first, got ${spells[0]?.name}`);
    }
    const byStaff = new Map<string, { alias: string; id: number; name: string; runes: Map<string, { alias: string; id: number; name: string }> }>();
    for (const parsed of parseRows(fs.readFileSync(path.join(content, 'scripts/skill_magic/configs/magic_staff.dbrow'), 'utf8'))) {
        const rune = namedItem(itemIds, required(parsed.values, 'rune', parsed.name), `${parsed.name} rune`);
        for (const row of parsed.values.staff ?? []) {
            const staff = namedItem(itemIds, row[0], `${parsed.name} staff`);
            const current = byStaff.get(staff.alias) ?? { ...staff, runes: new Map() };
            current.runes.set(rune.alias, rune);
            byStaff.set(staff.alias, current);
        }
    }
    const staves = [...byStaff.values()]
        .map((staff) => ({ alias: staff.alias, id: staff.id, name: staff.name, runes: [...staff.runes.values()] }))
        .sort((a, b) => a.name.localeCompare(b.name));
    return { spells, staves };
}
export function parsePack(text: string) {
    const out = new Map<string, number>();
    for (const raw of text.split(/\r?\n/)) {
        const line = raw.trim();
        if (!line || line.startsWith('//')) continue;
        const eq = line.indexOf('=');
        if (eq <= 0) continue;
        const id = Number(line.slice(0, eq));
        const name = line.slice(eq + 1);
        if (!Number.isInteger(id) || !name) continue;
        out.set(name, id);
    }
    return out;
}

const EQUIPMENT_FAMILY_ORDER = ['bows', 'crossbows', 'darts', 'arrows', 'bolts', 'melee_weapons', 'staffs'] as const;
type EquipmentFamilyId = (typeof EQUIPMENT_FAMILY_ORDER)[number];
const EQUIPMENT_CURATED_RELATIVE = 'tools/game-data/equipment-names.curated.json';
const EQUIPMENT_EVIDENCE_RELATIVE = 'tools/game-data/equipment-names.frozen.ts';
const EQUIPMENT_FROZEN_EXPORTS: { exportName: string; family: EquipmentFamilyId }[] = [
    { exportName: 'BOWS', family: 'bows' },
    { exportName: 'CROSSBOWS', family: 'crossbows' },
    { exportName: 'DARTS', family: 'darts' },
    { exportName: 'ARROWS', family: 'arrows' },
    { exportName: 'BOLTS', family: 'bolts' },
    { exportName: 'MELEE_WEAPONS', family: 'melee_weapons' },
    { exportName: 'STAFFS', family: 'staffs' },
];

export type EquipmentNameEntry = {
    requested_name: string;
    disposition: 'resolved' | 'absent' | 'ambiguous';
    selected_name?: string;
    alias?: string;
    id?: number;
    wear_position?: number;
    disambiguation?: string;
    absent_class?: 'no_exact_selected_match' | 'exact_match_ineligible';
    candidates?: { alias: string; id: number; name: string; wear_position: number; note: string }[];
};

export type EquipmentExactNameJoin = {
    matching: 'exact_display_name_only';
    no_exact_selected_match_means: string;
    bolts: {
        generic_display_name: 'Bolts';
        generic_is_substitute: false;
        note: string;
    };
};

export type EquipmentNamesFacts = {
    curated_input: { path: string; bytes: number; sha256: string };
    equipment_source: { path: string; bytes: number; sha256: string; commit?: string };
    equipment_evidence: { path: string; bytes: number; sha256: string };
    exact_name_join: EquipmentExactNameJoin;
    bows: EquipmentNameEntry[];
    crossbows: EquipmentNameEntry[];
    darts: EquipmentNameEntry[];
    arrows: EquipmentNameEntry[];
    bolts: EquipmentNameEntry[];
    melee_weapons: EquipmentNameEntry[];
    staffs: EquipmentNameEntry[];
};

type EquipmentCurated = {
    schema: string;
    source: { path: string; sha256: string; bytes: number; commit?: string };
    families: Record<EquipmentFamilyId, string[]>;
};

const EQUIPMENT_EXACT_NAME_JOIN: EquipmentExactNameJoin = {
    matching: 'exact_display_name_only',
    no_exact_selected_match_means: 'No exact display name in the selected item table. This is not a claim about all game content or global revision unavailability.',
    bolts: {
        generic_display_name: 'Bolts',
        generic_is_substitute: false,
        note: 'Frozen BOLTS members (Bronze bolts … Bone bolts) are absent under exact join. Generic Bolts / Bolts(p) are not substitutes and are not mapped onto those tier strings.',
    },
};

function isWearableRow(item: ReturnType<typeof row>) {
    return item.wear_position >= 0 || item.wear_position_2 >= 0 || item.wear_position_3 >= 0;
}

function isBankNoteRow(item: ReturnType<typeof row>) {
    return item.certificate_template >= 0;
}

/** Dedicated single-quoted scanner for frozen equipment.ts arrays. Not a generic JS parser. */
export function parseFrozenEquipmentSingleQuoted(text: string, start: number): { value: string; end: number } {
    if (text[start] !== "'") throw new Error(`equipment frozen: expected single quote at ${start}`);
    let index = start + 1;
    let value = '';
    while (index < text.length) {
        const ch = text[index];
        if (ch === '\\') {
            const next = text[index + 1];
            if (next !== "'" && next !== '\\') throw new Error(`equipment frozen: unsupported escape at ${index}`);
            value += next;
            index += 2;
            continue;
        }
        if (ch === "'") return { value, end: index + 1 };
        if (ch === '\n' || ch === '\r') throw new Error('equipment frozen: unterminated string');
        value += ch;
        index += 1;
    }
    throw new Error('equipment frozen: unterminated string');
}

export function parseFrozenEquipmentNameArrays(text: string): Record<EquipmentFamilyId, string[]> {
    const out = {} as Record<EquipmentFamilyId, string[]>;
    for (const { exportName, family } of EQUIPMENT_FROZEN_EXPORTS) {
        const header = `export const ${exportName}: string[] = [`;
        const start = text.indexOf(header);
        if (start < 0) throw new Error(`equipment frozen: missing ${exportName}`);
        if (text.indexOf(header, start + 1) >= 0) throw new Error(`equipment frozen: duplicate ${exportName}`);
        let index = start + header.length;
        const names: string[] = [];
        let closed = false;
        while (index < text.length) {
            const ch = text[index];
            if (ch === ' ' || ch === '\t' || ch === '\n' || ch === '\r' || ch === ',') {
                index += 1;
                continue;
            }
            if (ch === ']') {
                out[family] = names;
                closed = true;
                break;
            }
            if (ch === "'") {
                const parsed = parseFrozenEquipmentSingleQuoted(text, index);
                if (!parsed.value) throw new Error(`equipment frozen: empty name in ${exportName}`);
                names.push(parsed.value);
                index = parsed.end;
                continue;
            }
            throw new Error(`equipment frozen: unexpected token in ${exportName} at ${index}`);
        }
        if (!closed) throw new Error(`equipment frozen: unterminated ${exportName}`);
        if (names.length === 0) throw new Error(`equipment frozen: empty ${exportName}`);
    }
    return out;
}

function assertCuratedMatchesFrozen(curated: EquipmentCurated, frozen: Record<EquipmentFamilyId, string[]>) {
    const extra = Object.keys(curated.families).filter((key) => !EQUIPMENT_FAMILY_ORDER.includes(key as EquipmentFamilyId));
    if (extra.length > 0) throw new Error(`equipment names curated: unknown families ${extra.join(', ')}`);
    for (const family of EQUIPMENT_FAMILY_ORDER) {
        const curatedNames = curated.families[family];
        const frozenNames = frozen[family];
        if (!Array.isArray(curatedNames)) throw new Error(`equipment names curated: missing family ${family}`);
        if (curatedNames.length !== frozenNames.length) {
            throw new Error(`equipment names curated: ${family} length ${curatedNames.length} != frozen ${frozenNames.length}`);
        }
        for (let index = 0; index < frozenNames.length; index += 1) {
            if (curatedNames[index] !== frozenNames[index]) {
                throw new Error(`equipment names curated: ${family}[${index}] ${JSON.stringify(curatedNames[index])} != frozen ${JSON.stringify(frozenNames[index])}`);
            }
        }
    }
}

export function loadEquipmentNamesCurated(repoRoot: string = root): EquipmentCurated {
    const curatedPath = path.join(repoRoot, EQUIPMENT_CURATED_RELATIVE);
    if (!fs.existsSync(curatedPath)) throw new Error(`equipment names curated missing at ${curatedPath}`);
    const curated = JSON.parse(fs.readFileSync(curatedPath, 'utf8')) as EquipmentCurated;
    if (curated.schema !== 'r018-equipment-names-curated-1') {
        throw new Error(`equipment names curated schema mismatch: ${curated.schema}`);
    }
    for (const family of EQUIPMENT_FAMILY_ORDER) {
        const names = curated.families[family];
        if (!Array.isArray(names) || names.length === 0) {
            throw new Error(`equipment names curated: missing family ${family}`);
        }
        const seen = new Set<string>();
        for (const name of names) {
            if (!name || typeof name !== 'string') throw new Error(`equipment names curated: malformed name in ${family}`);
            if (seen.has(name)) throw new Error(`equipment names curated: duplicate ${family} name ${name}`);
            seen.add(name);
        }
    }
    const evidencePath = path.join(repoRoot, EQUIPMENT_EVIDENCE_RELATIVE);
    if (!fs.existsSync(evidencePath)) throw new Error(`equipment evidence missing at ${evidencePath}`);
    const evidenceDigest = sha256(evidencePath);
    if (evidenceDigest.sha256 !== curated.source.sha256 || evidenceDigest.bytes !== curated.source.bytes) {
        throw new Error(`equipment evidence pin mismatch: expected ${curated.source.sha256}/${curated.source.bytes}, got ${evidenceDigest.sha256}/${evidenceDigest.bytes}`);
    }
    assertCuratedMatchesFrozen(curated, parseFrozenEquipmentNameArrays(fs.readFileSync(evidencePath, 'utf8')));
    return curated;
}

function requireObjPack(objPack: Map<string, number> | undefined): Map<string, number> {
    if (!(objPack instanceof Map)) throw new Error('equipment names: missing required obj.pack');
    return objPack;
}

export function joinEquipmentName(
    family: EquipmentFamilyId,
    requestedName: string,
    items: ReturnType<typeof row>[],
    objPack: Map<string, number>,
): EquipmentNameEntry {
    const pack = requireObjPack(objPack);
    const exact = items.filter((item) => item.name === requestedName);
    let pool = exact.filter((item) => !isBankNoteRow(item));
    const notes = exact.filter((item) => isBankNoteRow(item));
    if (family === 'bows' || family === 'crossbows' || family === 'melee_weapons' || family === 'staffs') {
        pool = pool.filter((item) => isWearableRow(item));
        if (family === 'bows') {
            pool = pool.filter((item) => item.alias !== null && !item.alias.startsWith('unstrung_'));
        }
    } else if (family === 'darts' || family === 'arrows' || family === 'bolts') {
        pool = pool.filter((item) => isWearableRow(item));
    }
    let blackDaggerPreference = false;
    if (family === 'melee_weapons' && requestedName === 'Black dagger' && pool.length > 1) {
        const standard = pool.filter((item) => item.alias === 'black_dagger');
        const quest = pool.filter((item) => item.alias === 'deathdagger');
        if (
            standard.length === 1
            && quest.length === 1
            && pack.get('black_dagger') === standard[0]!.id
            && pack.get('deathdagger') === quest[0]!.id
        ) {
            pool = standard;
            blackDaggerPreference = true;
        }
    }
    if (pool.length === 1) {
        const item = pool[0]!;
        if (!item.alias) throw new Error(`equipment names ${family}: ${requestedName} resolved without alias`);
        const packed = pack.get(item.alias);
        if (packed === undefined) throw new Error(`equipment names ${family}: missing obj.pack alias ${item.alias}`);
        if (packed !== item.id) throw new Error(`equipment names ${family}: obj.pack ${item.alias}=${packed} != decoded ${item.id}`);
        return {
            requested_name: requestedName,
            disposition: 'resolved',
            selected_name: item.name ?? requestedName,
            alias: item.alias,
            id: item.id,
            wear_position: item.wear_position,
            disambiguation: blackDaggerPreference
                ? 'prefer_standard_pack_alias_black_dagger'
                : exact.length !== 1
                    ? 'exclude_bank_note_prefer_wearable'
                    : undefined,
        };
    }
    if (pool.length === 0) {
        return {
            requested_name: requestedName,
            disposition: 'absent',
            absent_class: exact.length === 0 ? 'no_exact_selected_match' : 'exact_match_ineligible',
            candidates: exact.length > 0
                ? exact.map((item) => ({
                    alias: item.alias ?? '',
                    id: item.id,
                    name: item.name ?? requestedName,
                    wear_position: item.wear_position,
                    note: isBankNoteRow(item) ? 'bank_note' : 'filtered_out',
                }))
                : notes.map((item) => ({
                    alias: item.alias ?? '',
                    id: item.id,
                    name: item.name ?? requestedName,
                    wear_position: item.wear_position,
                    note: 'bank_note_only',
                })),
        };
    }
    return {
        requested_name: requestedName,
        disposition: 'ambiguous',
        candidates: pool.map((item) => ({
            alias: item.alias ?? '',
            id: item.id,
            name: item.name ?? requestedName,
            wear_position: item.wear_position,
            note: 'wearable_candidate',
        })),
    };
}

export function extractEquipmentNamesFacts(items: ReturnType<typeof row>[], objPack: Map<string, number>, repoRoot: string = root): EquipmentNamesFacts {
    const pack = requireObjPack(objPack);
    const curated = loadEquipmentNamesCurated(repoRoot);
    const curatedPath = path.join(repoRoot, EQUIPMENT_CURATED_RELATIVE);
    const evidencePath = path.join(repoRoot, EQUIPMENT_EVIDENCE_RELATIVE);
    const curatedDigest = sha256(curatedPath);
    const evidenceDigest = sha256(evidencePath);
    const out: EquipmentNamesFacts = {
        curated_input: { path: path.relative(repoRoot, curatedPath).replaceAll('\\', '/'), ...curatedDigest },
        equipment_source: {
            path: curated.source.path,
            bytes: curated.source.bytes,
            sha256: curated.source.sha256,
            commit: curated.source.commit,
        },
        equipment_evidence: { path: EQUIPMENT_EVIDENCE_RELATIVE, ...evidenceDigest },
        exact_name_join: EQUIPMENT_EXACT_NAME_JOIN,
        bows: [],
        crossbows: [],
        darts: [],
        arrows: [],
        bolts: [],
        melee_weapons: [],
        staffs: [],
    };
    for (const family of EQUIPMENT_FAMILY_ORDER) {
        const entries = curated.families[family].map((requestedName) => joinEquipmentName(family, requestedName, items, pack));
        const ambiguous = entries.filter((entry) => entry.disposition === 'ambiguous');
        if (ambiguous.length > 0) {
            throw new Error(`equipment names ${family}: ambiguous joins ${ambiguous.map((row) => row.requested_name).join(', ')}`);
        }
        out[family] = entries;
    }
    return out;
}
export function extractAutocastControls(content: string) {
    const interfaces = parsePack(fs.readFileSync(path.join(content, 'pack/interface.pack'), 'utf8'));
    const varps = parsePack(fs.readFileSync(path.join(content, 'pack/varp.pack'), 'utf8'));
    const required = (table: Map<string, number>, name: string) => {
        const id = table.get(name);
        if (id === undefined) throw new Error(`autocast: missing ${name}`);
        return id;
    };
    return {
        staff_tab_root: required(interfaces, 'combat_staff_2'),
        spell_panel_root: required(interfaces, 'staff_spells'),
        choose_com: required(interfaces, 'combat_staff_2:auto_choose'),
        toggle_com: required(interfaces, 'combat_staff_2:auto_toggle'),
        spell_grid_base: required(interfaces, 'staff_spells:ssb0'),
        magic_varp: required(varps, 'attackstyle_magic'),
        selected_value: 2,
        armed_value: 3,
    };
}
export function parsePrayerConstants(text: string) {
    const names = new Set<string>();
    for (const raw of text.split(/\r?\n/)) {
        const line = raw.trim();
        if (!line.startsWith('^prayer_')) continue;
        const eq = line.indexOf('=');
        if (eq <= 0) continue;
        names.add(line.slice(1, eq).trim());
    }
    if (names.size === 0) throw new Error('prayer: missing prayers.constant entries');
    return names;
}

/** prayer.if section name → transmitted varp alias from pushvar wiring. */
export function parsePrayerInterface(text: string) {
    const out = new Map<string, string>();
    let current: string | null = null;
    for (const raw of text.split(/\r?\n/)) {
        const line = raw.trim();
        if (line.startsWith('[') && line.endsWith(']')) {
            current = line.slice(1, -1);
            continue;
        }
        if (!current || !line.startsWith('script1op1=pushvar,')) continue;
        const alias = line.slice('script1op1=pushvar,'.length);
        if (!alias) throw new Error(`prayer.if: empty pushvar in ${current}`);
        if (out.has(current)) throw new Error(`prayer.if: duplicate section ${current}`);
        out.set(current, alias);
    }
    return out;
}

export function extractPrayerFacts(content: string) {
    const interfaces = parsePack(fs.readFileSync(path.join(content, 'pack/interface.pack'), 'utf8'));
    const varps = parsePack(fs.readFileSync(path.join(content, 'pack/varp.pack'), 'utf8'));
    const constants = parsePrayerConstants(
        fs.readFileSync(path.join(content, 'scripts/skill_prayer/configs/prayers.constant'), 'utf8'),
    );
    const ifVarps = parsePrayerInterface(
        fs.readFileSync(path.join(content, 'scripts/skill_prayer/interfaces/prayer.if'), 'utf8'),
    );
    const prayers: {
        name: string;
        level: number;
        source_row: string;
        prayer_constant: string;
        button_com: number;
        com_alias: string;
        varp: number;
        varp_alias: string;
    }[] = [];
    const seenName = new Set<string>();
    const seenCom = new Set<number>();
    const seenVarp = new Set<number>();
    const seenConstant = new Set<string>();
    for (const parsed of parseRows(
        fs.readFileSync(path.join(content, 'scripts/skill_prayer/configs/prayers.dbrow'), 'utf8'),
    )) {
        const prayerRaw = parsed.values.prayer?.[0]?.[0];
        if (!prayerRaw) continue;
        const constant = prayerRaw.startsWith('^') ? prayerRaw.slice(1) : prayerRaw;
        if (!constants.has(constant)) throw new Error(`${parsed.name}: unknown prayer constant ${constant}`);
        if (seenConstant.has(constant)) throw new Error(`${parsed.name}: duplicate prayer constant ${constant}`);
        seenConstant.add(constant);
        const varpAlias = ifVarps.get(constant);
        if (!varpAlias) throw new Error(`${parsed.name}: missing prayer.if pushvar for ${constant}`);
        const varpId = varps.get(varpAlias);
        if (varpId === undefined) throw new Error(`${parsed.name}: missing varp.pack entry ${varpAlias}`);
        const comAlias = `prayer:${constant}`;
        const buttonCom = interfaces.get(comAlias);
        if (buttonCom === undefined) throw new Error(`${parsed.name}: missing interface.pack entry ${comAlias}`);
        const name = required(parsed.values, 'name', parsed.name);
        const level = integer(required(parsed.values, 'level', parsed.name), parsed.name);
        if (seenName.has(name)) throw new Error(`${parsed.name}: duplicate prayer name ${name}`);
        if (seenCom.has(buttonCom)) throw new Error(`${parsed.name}: duplicate button com ${buttonCom}`);
        if (seenVarp.has(varpId)) throw new Error(`${parsed.name}: duplicate varp ${varpId}`);
        seenName.add(name);
        seenCom.add(buttonCom);
        seenVarp.add(varpId);
        prayers.push({
            name,
            level,
            source_row: parsed.name,
            prayer_constant: constant,
            button_com: buttonCom,
            com_alias: comAlias,
            varp: varpId,
            varp_alias: varpAlias,
        });
    }
    if (prayers.length !== 15) throw new Error(`prayer: expected 15 rows, got ${prayers.length}`);
    prayers.sort((a, b) => a.button_com - b.button_com || a.name.localeCompare(b.name));
    const thick = prayers[0];
    const melee = prayers[14];
    if (
        thick.name !== 'Thick Skin'
        || thick.level !== 1
        || thick.button_com !== 5609
        || thick.varp !== 83
        || thick.varp_alias !== 'prayer0'
    ) {
        throw new Error(`prayer: Thick Skin anchor mismatch ${JSON.stringify(thick)}`);
    }
    if (
        melee.name !== 'Protect from Melee'
        || melee.level !== 43
        || melee.button_com !== 5623
        || melee.varp !== 97
        || melee.varp_alias !== 'prayer14'
    ) {
        throw new Error(`prayer: Protect from Melee anchor mismatch ${JSON.stringify(melee)}`);
    }
    for (let index = 0; index < prayers.length; index += 1) {
        const expectedCom = 5609 + index;
        const expectedVarp = 83 + index;
        const row = prayers[index];
        if (row.button_com !== expectedCom || row.varp !== expectedVarp) {
            throw new Error(`prayer: expected com ${expectedCom}/varp ${expectedVarp}, got ${row.button_com}/${row.varp} for ${row.name}`);
        }
    }
    return { prayers };
}

export function extractDuelControls(content: string) {
    const interfaces = parsePack(fs.readFileSync(path.join(content, 'pack/interface.pack'), 'utf8'));
    const required = (name: string) => {
        const id = interfaces.get(name);
        if (id === undefined) throw new Error(`duel: missing ${name}`);
        return id;
    };
    return {
        select_modal: required('duel_select_type'),
        confirm_modal: required('duel_confirm'),
        win_modal: required('duel_win'),
        select_accept: required('duel_select_type:accept'),
        confirm_accept: required('duel_confirm:accept'),
        select_partner: required('duel_select_type:otherplayer'),
        select_status: required('duel_select_type:status'),
        confirm_status: required('duel_confirm:status'),
    };
}
function parseNamedConstant(text: string, name: string) {
    const prefix = `^${name}`;
    for (const raw of text.split(/\r?\n/)) {
        const line = raw.trim();
        if (!line.startsWith(prefix)) continue;
        const eq = line.indexOf('=');
        if (eq <= 0) continue;
        const parsed = Number(line.slice(eq + 1).trim());
        if (!Number.isInteger(parsed) || parsed <= 0) throw new Error(`special: bad ${name} in ${line}`);
        return parsed;
    }
    throw new Error(`special: missing ${name}`);
}
function paramTruthy(value: number | string | undefined) {
    return value === 1 || value === '1';
}
export function extractSpecialControls(content: string, items: ObjType[]) {
    const interfaces = parsePack(fs.readFileSync(path.join(content, 'pack/interface.pack'), 'utf8'));
    const varps = parsePack(fs.readFileSync(path.join(content, 'pack/varp.pack'), 'utf8'));
    const params = parsePack(fs.readFileSync(path.join(content, 'pack/param.pack'), 'utf8'));
    const required = (table: Map<string, number>, name: string) => {
        const id = table.get(name);
        if (id === undefined) throw new Error(`special: missing ${name}`);
        return id;
    };
    const specwepParam = required(params, 'specwep');
    const saEnergyParam = required(params, 'sa_energy');
    const maxEnergy = parseNamedConstant(fs.readFileSync(path.join(content, 'scripts/skill_combat/configs/combat.constant'), 'utf8'), 'sa_max_energy');
    const bars: { root: string; root_id: number; bar: number }[] = [];
    for (const [name, id] of interfaces) {
        if (name.includes(':') || !name.startsWith('combat_')) continue;
        const bar = interfaces.get(`${name}:specbar`);
        if (bar === undefined) continue;
        bars.push({ root: name, root_id: id, bar });
    }
    bars.sort((a, b) => a.root_id - b.root_id || a.bar - b.bar);
    if (bars.length === 0) throw new Error('special: no combat spec bars');
    const weapons: { alias: string; id: number; name: string; cost: number }[] = [];
    for (const item of items) {
        if (!item.debugname || !item.name) continue;
        const itemParams = item.params;
        if (!itemParams) continue;
        if (!paramTruthy(itemParams.get(specwepParam))) continue;
        const cost = itemParams.get(saEnergyParam);
        // specwep without sa_energy is a self-buff (Dragon battleaxe, Excalibur).
        if (typeof cost !== 'number' || !Number.isInteger(cost) || cost <= 0) continue;
        weapons.push({ alias: item.debugname, id: item.id, name: item.name, cost });
    }
    weapons.sort((a, b) => a.id - b.id || a.alias.localeCompare(b.alias));
    if (weapons.length === 0) throw new Error('special: no sa_energy spec weapons');
    return {
        energy_varp: required(varps, 'sa_energy'),
        armed_varp: required(varps, 'sa_attack'),
        armed_value: 1,
        max_energy: maxEnergy,
        arm_confirm_ticks: 2,
        bars,
        weapons,
    };
}
function parseTeleCoord(value: string, label: string) {
    const parts = value.split('_');
    if (parts.length !== 5) throw new Error(`${label}: bad tele_coord ${value}`);
    const plane = integer(parts[0], `${label} plane`);
    const mx = integer(parts[1], `${label} mx`);
    const mz = integer(parts[2], `${label} mz`);
    const lx = integer(parts[3], `${label} lx`);
    const lz = integer(parts[4], `${label} lz`);
    return { packed: value, plane, x: mx * 64 + lx, z: mz * 64 + lz };
}
function titleDest(spell: string) {
    const key = spell.startsWith('^') ? spell.slice(1) : spell;
    if (!key.endsWith('_teleport')) throw new Error(`teleport: ${spell} is not a *_teleport spell`);
    const dest = key.slice(0, -'_teleport'.length);
    if (!dest) throw new Error(`teleport: empty dest in ${spell}`);
    return { key, name: dest.charAt(0).toUpperCase() + dest.slice(1) };
}
export function extractTeleportSpells(content: string, items: ObjType[]) {
    const itemIds = new Map(items.filter((item) => item.debugname !== null).map((item) => [item.debugname as string, { id: item.id, name: item.name }]));
    const interfaces = parsePack(fs.readFileSync(path.join(content, 'pack/interface.pack'), 'utf8'));
    const teleports = [];
    for (const parsed of parseRows(fs.readFileSync(path.join(content, 'scripts/skill_magic/configs/magic_spells.dbrow'), 'utf8'))) {
        const packed = parsed.values.tele_coord?.[0]?.[0];
        if (!packed) continue;
        const spellConst = required(parsed.values, 'spell', parsed.name);
        const { key, name } = titleDest(spellConst);
        const componentId = interfaces.get(`magic:${key}`);
        if (componentId === undefined) throw new Error(`teleport: missing magic:${key}`);
        const coord = parseTeleCoord(packed, parsed.name);
        const members = required(parsed.values, 'members', parsed.name) === 'true';
        teleports.push({
            name,
            source_row: parsed.name,
            spell: key,
            component_id: componentId,
            members,
            level: integer(required(parsed.values, 'levelrequired', parsed.name), parsed.name),
            runes: runeCosts(parsed.values, parsed.name, itemIds),
            experience: integer(required(parsed.values, 'experience', parsed.name), parsed.name),
            tele_coord: coord.packed,
            x: coord.x,
            z: coord.z,
            plane: coord.plane,
        });
    }
    if (teleports.length === 0) throw new Error('teleport: no tele_coord rows');
    return teleports;
}
function integer(value: string, label: string) { const parsed = Number(value); if (!Number.isInteger(parsed)) throw new Error(`${label}: expected integer, got ${value}`); return parsed; }

type ObjSection = { name?: string; cost?: number; params: Map<string, string> };

export function parseObjSections(text: string) {
    const sections = new Map<string, ObjSection>();
    let current: string | null = null;
    for (const raw of text.split(/\r?\n/)) {
        const line = raw.trim();
        if (!line || line.startsWith('//')) continue;
        if (line.startsWith('[') && line.endsWith(']')) {
            current = line.slice(1, -1);
            sections.set(current, { params: new Map() });
            continue;
        }
        if (!current) continue;
        const entry = sections.get(current)!;
        if (line.startsWith('name=')) entry.name = line.slice('name='.length);
        else if (line.startsWith('cost=')) entry.cost = integer(line.slice('cost='.length), `${current} cost`);
        else if (line.startsWith('param=')) {
            const [, rest] = line.split('=', 2);
            const comma = rest.indexOf(',');
            if (comma <= 0) throw new Error(`${current}: bad param ${line}`);
            entry.params.set(rest.slice(0, comma), rest.slice(comma + 1));
        }
    }
    return sections;
}

export function parseIdentifyHerbPairs(text: string) {
    const pairs: { unidAlias: string; idAlias: string }[] = [];
    const re = /\[opheld1,(unidentified_\w+)\][^\n]*\n~attempt_identify_herb\((\w+),/g;
    for (const match of text.matchAll(re)) {
        pairs.push({ unidAlias: match[1], idAlias: match[2] });
    }
    if (pairs.length === 0) throw new Error('herbs: no identify.rs2 pairs');
    return pairs;
}

type ParamDef = { type?: string; default?: string };

export function parseParamDefinitions(text: string) {
    const defs = new Map<string, ParamDef>();
    let current: string | null = null;
    for (const raw of text.split(/\r?\n/)) {
        const line = raw.trim();
        if (!line || line.startsWith('//')) continue;
        if (line.startsWith('[') && line.endsWith(']')) {
            current = line.slice(1, -1);
            defs.set(current, {});
            continue;
        }
        if (!current) continue;
        const entry = defs.get(current)!;
        if (line.startsWith('type=')) entry.type = line.slice('type='.length);
        else if (line.startsWith('default=')) entry.default = line.slice('default='.length);
    }
    return defs;
}

/** Default Herblore level from pinned `identify.param`, not obj cost. */
export function identifiedHerbLevelDefault(content: string) {
    const defs = parseParamDefinitions(
        fs.readFileSync(path.join(content, 'scripts/skill_herblore/configs/identifying/identify.param'), 'utf8'),
    );
    const level = defs.get('identified_herb_level');
    if (!level?.default) throw new Error('herbs: missing identified_herb_level default in identify.param');
    return integer(level.default, 'identified_herb_level default');
}

/** Script-facing herb key from cleaned display name (reference herbs.ts parity). */
export function herbKeyFromName(name: string) {
    const lower = name.toLowerCase();
    if (lower.endsWith(' leaf')) return lower.slice(0, -' leaf'.length);
    if (lower === 'dwarf weed' || lower === 'snake weed') return lower;
    if (lower.endsWith(' weed')) return lower.slice(0, -' weed'.length);
    return lower;
}

export function extractHerbFacts(content: string, items: ObjType[]) {
    const itemIds = new Map(items.filter((item) => item.debugname !== null).map((item) => [item.debugname as string, { id: item.id, name: item.name }]));
    const levelDefault = identifiedHerbLevelDefault(content);
    const herbsObj = parseObjSections(fs.readFileSync(path.join(content, 'scripts/skill_herblore/configs/herbs.obj'), 'utf8'));
    const identify = fs.readFileSync(path.join(content, 'scripts/skill_herblore/scripts/identifying/identify.rs2'), 'utf8');
    const pairs = parseIdentifyHerbPairs(identify);
    const herbs: { key: string; name: string; id: number; unidId: number; level: number; level_source: string; source_identified: string; source_unidentified: string }[] = [];
    for (const { unidAlias, idAlias } of pairs) {
        const section = herbsObj.get(idAlias);
        if (!section) throw new Error(`herbs: missing herbs.obj section ${idAlias}`);
        const display = section.name;
        if (!display) throw new Error(`herbs: ${idAlias} missing name`);
        const levelRaw = section.params.get('identified_herb_level');
        const level = levelRaw !== undefined
            ? integer(levelRaw, `${idAlias} identified_herb_level`)
            : levelDefault;
        const levelSource = levelRaw !== undefined ? 'identified_herb_level' : 'identified_herb_level_default';
        const cleaned = itemIds.get(idAlias);
        const unid = itemIds.get(unidAlias);
        if (!cleaned) throw new Error(`herbs: unknown identified obj ${idAlias}`);
        if (!unid) throw new Error(`herbs: unknown unidentified obj ${unidAlias}`);
        if (!cleaned.name) throw new Error(`herbs: ${idAlias} missing obj display name`);
        herbs.push({
            key: herbKeyFromName(display),
            name: display,
            id: cleaned.id,
            unidId: unid.id,
            level,
            level_source: levelSource,
            source_identified: idAlias,
            source_unidentified: unidAlias,
        });
    }
    const guam = herbs.find((herb) => herb.key === 'guam');
    if (!guam || guam.level !== levelDefault || guam.level_source !== 'identified_herb_level_default') {
        throw new Error(`herbs: guam must use identify.param default ${levelDefault}, got ${JSON.stringify(guam)}`);
    }
    const snake = herbs.find((herb) => herb.key === 'snake weed');
    if (snake && (snake.level !== levelDefault || snake.level_source !== 'identified_herb_level')) {
        throw new Error(`herbs: snake weed level/source mismatch, got ${JSON.stringify(snake)}`);
    }
    return { herbs, herb_level_default: levelDefault };
}

type DropBlock = { type: string; name: string; body: string };
type DropToken = { value: string; optional: boolean };

function parseDropBlocks(content: string) {
    const blocks = new Map<string, DropBlock>();
    for (const relative of dropContentFiles.filter((file) => file.endsWith('.rs2'))) {
        const file = path.join(content, relative);
        if (!fs.existsSync(file)) throw new Error(`missing content input ${relative}`);
        let current: DropBlock | null = null;
        let lines: string[] = [];
        const flush = () => {
            if (!current) return;
            const key = `${current.type}:${current.name}`;
            if (blocks.has(key)) throw new Error(`duplicate drop block ${key}`);
            current.body = lines.join('\n');
            blocks.set(key, current);
            lines = [];
        };
        for (const raw of fs.readFileSync(file, 'utf8').split(/\r?\n/)) {
            const head = /^\[([a-z0-9_]+)\s*,\s*([a-z0-9_]+)\]/.exec(raw.trim());
            if (head) {
                flush();
                current = { type: head[1], name: head[2], body: '' };
            } else if (current) {
                lines.push(raw);
            }
        }
        flush();
    }
    return blocks;
}

function parseDropNpcs(content: string) {
    const relative = dropContentFiles.find((file) => file.endsWith('.npc'))!;
    const file = path.join(content, relative);
    if (!fs.existsSync(file)) throw new Error(`missing content input ${relative}`);
    const rows = new Map<string, { name?: string; death_drop?: string }>();
    let current: string | null = null;
    for (const raw of fs.readFileSync(file, 'utf8').split(/\r?\n/)) {
        const line = raw.trim();
        const head = /^\[([a-z0-9_]+)\]$/.exec(line);
        if (head) {
            current = head[1];
            if (rows.has(current)) throw new Error(`duplicate npc config ${current}`);
            rows.set(current, {});
        } else if (current && line.startsWith('name=')) {
            rows.get(current)!.name = line.slice('name='.length);
        } else if (current && line.startsWith('param=death_drop,')) {
            rows.get(current)!.death_drop = line.slice('param=death_drop,'.length).split(',')[0].trim();
        }
    }
    return rows;
}

function dropTokens(body: string) {
    const tokens: DropToken[] = [];
    for (const match of body.matchAll(/obj_add\s*\(\s*npc_coord\s*,\s*(~?[a-z0-9_]+)/g)) {
        tokens.push({ value: match[1], optional: false });
    }
    for (const match of body.matchAll(/return\s*\(\s*(~?[a-z0-9_]+)/g)) {
        tokens.push({ value: match[1], optional: false });
    }
    // Some tables stage an item in a local before returning the variable.
    // Non-item assignments are ignored only when they do not join to ObjType.
    for (const match of body.matchAll(/=\s*([a-z][a-z0-9_]*)\s*;/g)) {
        tokens.push({ value: match[1], optional: true });
    }
    return tokens;
}

export function parseInvShopStock(text: string, shopName: string) {
    const stock: { alias: string; baseline_qty: number; restock_delta: number }[] = [];
    let inSection = false;
    for (const raw of text.split(/\r?\n/)) {
        const line = raw.trim();
        if (line === `[${shopName}]`) {
            inSection = true;
            continue;
        }
        if (line.startsWith('[') && line.endsWith(']')) {
            if (inSection) break;
            continue;
        }
        if (!inSection || !line.startsWith('stock')) continue;
        const eq = line.indexOf('=');
        if (eq <= 0) throw new Error(`${shopName}: bad stock line ${line}`);
        const parts = line.slice(eq + 1).split(',');
        if (parts.length < 3) throw new Error(`${shopName}: bad stock line ${line}`);
        stock.push({
            alias: parts[0],
            baseline_qty: integer(parts[1], line),
            restock_delta: integer(parts[2], line),
        });
    }
    if (stock.length === 0) throw new Error(`${shopName}: missing stock rows`);
    return stock;
}

function parseNpcConfigSections(text: string) {
    const sections = new Map<string, Record<string, string>>();
    let current: string | null = null;
    for (const raw of text.split(/\r?\n/)) {
        const line = raw.trim();
        if (line.startsWith('[') && line.endsWith(']')) {
            current = line.slice(1, -1);
            sections.set(current, {});
            continue;
        }
        if (!current || !line.includes('=')) continue;
        const entry = sections.get(current)!;
        if (line.startsWith('param=')) {
            const comma = line.indexOf(',', 'param='.length);
            if (comma <= 0) throw new Error(`${current}: bad param ${line}`);
            entry[line.slice('param='.length, comma)] = line.slice(comma + 1);
            continue;
        }
        const eq = line.indexOf('=');
        entry[line.slice(0, eq)] = line.slice(eq + 1);
    }
    return sections;
}

export function parseNpcSection(text: string, alias: string) {
    const section = parseNpcConfigSections(text).get(alias);
    if (!section) throw new Error(`npc config: missing [${alias}]`);
    return section;
}

export function parseQuestEnumEntry(text: string, questName: string) {
    let inQuestNames = false;
    let found: string | null = null;
    for (const raw of text.split(/\r?\n/)) {
        const line = raw.trim();
        if (!line || line.startsWith('//')) continue;
        if (line.startsWith('[') && line.endsWith(']')) {
            const section = line.slice(1, -1);
            if (inQuestNames && section !== 'quest_names_enum') break;
            inQuestNames = section === 'quest_names_enum';
            continue;
        }
        if (!inQuestNames || !line.startsWith('val=')) continue;
        const eq = line.indexOf('=');
        const comma = line.indexOf(',', eq + 1);
        if (comma <= eq) throw new Error(`quest.enum: malformed val line ${line}`);
        const name = line.slice(comma + 1).trim();
        if (name !== questName) continue;
        if (found) throw new Error(`quest.enum: duplicate val for ${questName}`);
        found = line;
    }
    if (!found) throw new Error(`quest.enum: missing ${questName} in [quest_names_enum]`);
    return found;
}

export function parseMapsquarePath(relative: string) {
    const base = path.basename(relative, '.jm2');
    const match = /^m(\d+)_(\d+)$/.exec(base);
    if (!match) throw new Error(`mapsquare path: expected m<x>_<z>.jm2, got ${relative}`);
    return { mx: integer(match[1], relative), mz: integer(match[2], relative) };
}

function jm2SectionName(line: string) {
    const trimmed = line.trim();
    if (!trimmed.startsWith('==== ') || !trimmed.endsWith(' ====')) return null;
    return trimmed.slice('==== '.length, -' ===='.length);
}

export function parseLocSection(text: string, alias: string) {
    const sections = new Map<string, { name?: string }>();
    let current: string | null = null;
    for (const raw of text.split(/\r?\n/)) {
        const line = raw.trim();
        if (line.startsWith('[') && line.endsWith(']')) {
            current = line.slice(1, -1);
            sections.set(current, {});
            continue;
        }
        if (!current || !line.startsWith('name=')) continue;
        sections.get(current)!.name = line.slice('name='.length);
    }
    const section = sections.get(alias);
    if (!section?.name) throw new Error(`loc config: missing [${alias}] name`);
    return section;
}

/**
 * LOC placements only — mirrors `crates/nav/src/transport.rs` `parse_jm2_locs` gating.
 * One jm2 is parsed once against the whole selected loc-id set; a singleton set is
 * the one-id case. Fail-closed: bad tokens, ranges, and extra tokens throw.
 */
export function parseJm2LocPlacements(text: string, locIds: ReadonlySet<number>) {
    const placements: { plane: number; lx: number; lz: number; loc_id: number; shape: number; angle: number }[] = [];
    let inLoc = false;
    for (const raw of text.split(/\r?\n/)) {
        const line = raw.trim();
        if (!line) continue;
        const section = jm2SectionName(line);
        if (section !== null) {
            inLoc = section === 'LOC';
            continue;
        }
        if (!inLoc) continue;
        const colon = line.indexOf(':');
        if (colon <= 0) throw new Error(`jm2 LOC: malformed row ${line}`);
        const coords = line.slice(0, colon).trim();
        const data = line.slice(colon + 1).trim();
        const coordTokens = coords.split(/\s+/);
        if (coordTokens.length !== 3) throw new Error(`jm2 LOC: bad coords ${line}`);
        const plane = integer(coordTokens[0], line);
        const lx = integer(coordTokens[1], line);
        const lz = integer(coordTokens[2], line);
        if (plane < 0 || plane > 3) throw new Error(`jm2 LOC: plane out of range ${line}`);
        if (lx < 0 || lx > 63 || lz < 0 || lz > 63) throw new Error(`jm2 LOC: local coords out of range ${line}`);
        const dataTokens = data.split(/\s+/).filter((token) => token.length > 0);
        if (dataTokens.length === 0) throw new Error(`jm2 LOC: missing loc id ${line}`);
        if (dataTokens.length > 3) throw new Error(`jm2 LOC: extra tokens ${line}`);
        const id = integer(dataTokens[0], line);
        const shape = dataTokens[1] !== undefined ? integer(dataTokens[1], line) : 0;
        const angle = dataTokens[2] !== undefined ? integer(dataTokens[2], line) : 0;
        if (!locIds.has(id)) continue;
        placements.push({ plane, lx, lz, loc_id: id, shape, angle });
    }
    return placements;
}

function worldFromMapsquare(mx: number, mz: number, lx: number, lz: number, plane: number) {
    return { x: mx * 64 + lx, z: mz * 64 + lz, plane };
}

const PICKAXE_SHOP_ORDER = [
    'bronze_pickaxe',
    'iron_pickaxe',
    'steel_pickaxe',
    'mithril_pickaxe',
    'adamant_pickaxe',
    'rune_pickaxe',
] as const;

const PICKAXE_BASE_COSTS: Record<string, number> = {
    'Bronze pickaxe': 1,
    'Iron pickaxe': 140,
    'Steel pickaxe': 500,
    'Mithril pickaxe': 1300,
    'Adamant pickaxe': 3200,
    'Rune pickaxe': 32000,
};

export function extractNurmofEssenceFacts(content: string, items: ObjType[], npcs: NpcType[]) {
    const itemByAlias = new Map(items.filter((item) => item.debugname !== null).map((item) => [item.debugname as string, item]));
    const invRelative = 'scripts/areas/area_falador/configs/dwarven_mine.inv';
    const stock = parseInvShopStock(fs.readFileSync(path.join(content, invRelative), 'utf8'), 'pickaxeshop');
    if (stock.length !== 6) throw new Error(`pickaxeshop: expected 6 stock rows, got ${stock.length}`);
    const stockAliases = stock.map((row) => row.alias);
    if (JSON.stringify(stockAliases) !== JSON.stringify([...PICKAXE_SHOP_ORDER])) {
        throw new Error(`pickaxeshop: unexpected stock order ${JSON.stringify(stockAliases)}`);
    }
    const pickaxes = stock.map((row) => {
        const item = itemByAlias.get(row.alias);
        if (!item?.name) throw new Error(`pickaxeshop: missing obj ${row.alias}`);
        const expected = PICKAXE_BASE_COSTS[item.name];
        if (expected === undefined) throw new Error(`pickaxeshop: unexpected pickaxe name ${item.name}`);
        if (item.cost !== expected) {
            throw new Error(`pickaxeshop: ${item.name} base cost ${item.cost}, expected ${expected}`);
        }
        return {
            alias: row.alias,
            id: item.id,
            name: item.name,
            base_cost: item.cost,
            shop_baseline_qty: row.baseline_qty,
            cost_source: 'obj.cost',
        };
    });
    const npcConfig = parseNpcSection(
        fs.readFileSync(path.join(content, 'scripts/areas/area_falador/configs/dwarven_mine.npc'), 'utf8'),
        'nurmof',
    );
    if (npcConfig.name !== 'Nurmof') throw new Error(`nurmof: bad display name ${npcConfig.name}`);
    if (npcConfig.owned_shop !== 'pickaxeshop') throw new Error(`nurmof: bad owned_shop ${npcConfig.owned_shop}`);
    const nurmof = npcs.find((npc) => npc.debugname === 'nurmof');
    if (!nurmof?.name) throw new Error('nurmof: missing decoded npc');
    if (nurmof.id !== 594 || nurmof.name !== 'Nurmof') {
        throw new Error(`nurmof: npc join mismatch ${nurmof.id}/${nurmof.name}`);
    }
    const npcPack = parsePack(fs.readFileSync(path.join(content, 'pack/npc.pack'), 'utf8'));
    const packedId = npcPack.get('nurmof');
    if (packedId !== 594) throw new Error(`nurmof: pack id ${packedId}, expected 594`);
    const essenceMap = 'maps/m45_75.jm2';
    if (!fs.existsSync(path.join(content, essenceMap))) throw new Error(`missing ${essenceMap}`);
    const { mx: essence_mapsquare_mx, mz: essence_mapsquare_mz } = parseMapsquarePath(essenceMap);
    const minePortalLocId = 2492;
    const portalPlacements = parseJm2LocPlacements(
        fs.readFileSync(path.join(content, essenceMap), 'utf8'),
        new Set([minePortalLocId]),
    );
    if (portalPlacements.length === 0) {
        throw new Error(`essence mine: no loc ${minePortalLocId} placements in ${essenceMap}`);
    }
    const locPack = parsePack(fs.readFileSync(path.join(content, 'pack/loc.pack'), 'utf8'));
    const portalAlias = [...locPack.entries()].find(([, id]) => id === minePortalLocId)?.[0];
    if (portalAlias !== 'blankrunestone_exit_portal') {
        throw new Error(`essence mine: loc ${minePortalLocId} alias ${portalAlias}, expected blankrunestone_exit_portal`);
    }
    const auburyPacked = npcPack.get('aubury');
    const auburyNpc = npcs.find((npc) => npc.debugname === 'aubury');
    if (auburyPacked !== 553 || !auburyNpc?.name) {
        throw new Error(`aubury: pack/npc join mismatch ${auburyPacked}/${auburyNpc?.name}`);
    }
    const runecraftConstantPath = 'scripts/skill_runecraft/configs/runecraft.constant';
    const runecraftConstant = fs.readFileSync(path.join(content, runecraftConstantPath), 'utf8');
    const returnAnchorMatch = runecraftConstant.match(/^\^essence_mine_to_aubury\s*=\s*(\S+)/m);
    if (!returnAnchorMatch) throw new Error('runecraft.constant: missing ^essence_mine_to_aubury return anchor');
    const example = { x: 2880, z: 4800, plane: 0 };
    return {
        npc_alias: 'nurmof',
        npc_id: nurmof.id,
        npc_name: nurmof.name,
        shop_inv: 'pickaxeshop',
        shop_inv_source: invRelative,
        pickaxes,
        essence_region: {
            mapsquare_mx: essence_mapsquare_mx,
            mapsquare_mz: essence_mapsquare_mz,
            predicate: `(x >> 6) === ${essence_mapsquare_mx} && (z >> 6) === ${essence_mapsquare_mz}`,
            source_map: essenceMap,
            mapsquare_from_filename: true,
            mine_portal_loc_alias: portalAlias,
            mine_portal_loc_id: minePortalLocId,
            mine_portal_loc_placements: portalPlacements.length,
            example_inside: example,
        },
        aubury_travel: {
            note: 'Essence wizard Aubury entry/return hops are already packed in nav transport; not duplicated here.',
            npc_alias: 'aubury',
            npc_id: auburyNpc.id,
            npc_name: auburyNpc.name,
            already_packed: true,
            return_anchor_constant: '^essence_mine_to_aubury',
            return_anchor_coord: returnAnchorMatch[1],
            return_anchor_source: runecraftConstantPath,
            return_anchor_role: 'overworld_exit_after_mine_teleport',
        },
        curated_vendor_tactics: {
            label: 'curated',
            authority: 'reference rs2b0t-00d39a17e0 src/bot/api/acquisition/ToolAcquire.ts NURMOF_VENDOR',
            keeper: 'Nurmof',
            stand: { x: 2997, z: 9844, plane: 0 },
            bank_stand: { x: 3013, z: 3355, plane: 0 },
            hop_from: { x: 3019, z: 3449, plane: 0 },
            hop_loc: 'Trapdoor',
            hop_action: 'Climb-down',
        },
    };
}

export function extractFlourSixFacts(content: string, items: ObjType[]) {
    const itemByAlias = new Map(items.filter((item) => item.debugname !== null).map((item) => [item.debugname as string, item]));
    const questEnumLine = parseQuestEnumEntry(
        fs.readFileSync(path.join(content, 'scripts/general/configs/quest.enum'), 'utf8'),
        'Murder Mystery',
    );
    if (!questEnumLine.includes('Murder Mystery')) throw new Error(`quest.enum: bad line ${questEnumLine}`);
    const locRelative = 'scripts/quests/quest_murder/configs/quest_murder.loc';
    const flourLoc = parseLocSection(fs.readFileSync(path.join(content, locRelative), 'utf8'), 'flourbarrel');
    const locPack = parsePack(fs.readFileSync(path.join(content, 'pack/loc.pack'), 'utf8'));
    const flourLocId = locPack.get('flourbarrel');
    if (flourLocId !== 2662) throw new Error(`flourbarrel: pack id ${flourLocId}, expected 2662`);
    const pot = itemByAlias.get('pot_empty');
    const potFlour = itemByAlias.get('pot_flour');
    if (!pot?.name || pot.id !== 1931) throw new Error(`flour: pot_empty join ${JSON.stringify(pot)}`);
    if (!potFlour?.name || potFlour.id !== 1933) throw new Error(`flour: pot_flour join ${JSON.stringify(potFlour)}`);
    const objPack = parsePack(fs.readFileSync(path.join(content, 'pack/obj.pack'), 'utf8'));
    if (objPack.get('pot_empty') !== 1931 || objPack.get('pot_flour') !== 1933) {
        throw new Error('flour: obj.pack id mismatch for pot items');
    }
    const mapRelative = 'maps/m42_55.jm2';
    const { mx, mz } = parseMapsquarePath(mapRelative);
    const placements = parseJm2LocPlacements(fs.readFileSync(path.join(content, mapRelative), 'utf8'), new Set([2662]));
    if (placements.length !== 1) {
        throw new Error(`flourbarrel: expected one ${mapRelative} LOC placement, got ${placements.length}`);
    }
    const placement = placements[0];
    const derivedBarrel = worldFromMapsquare(mx, mz, placement.lx, placement.lz, placement.plane);
    return {
        quest_name: 'Murder Mystery',
        quest_name_source: 'scripts/general/configs/quest.enum',
        pot: { alias: 'pot_empty', id: pot.id, name: pot.name },
        pot_flour: { alias: 'pot_flour', id: potFlour.id, name: potFlour.name },
        flour_barrel: {
            alias: 'flourbarrel',
            id: flourLocId,
            name: flourLoc.name!,
            loc_config_source: locRelative,
        },
        flour_barrel_object_tile: {
            ...derivedBarrel,
            role: 'loc_placement',
            provenance: 'derived',
            source: mapRelative,
            mapsquare: `m${mx}_${mz}`,
            local: { lx: placement.lx, lz: placement.lz },
            loc_shape: placement.shape,
            loc_angle: placement.angle,
        },
        flour_barrel_approach_tile: {
            x: 2735,
            z: 3581,
            plane: 0,
            role: 'interaction_near',
            provenance: 'curated',
            authority: 'reference rs2b0t-00d39a17e0 src/bot/api/ai/quests/defs/murder/areas.ts MURDER_TILE.FLOUR_BARREL',
            note: 'FlourCollector Reach.locOp near / recovery anchor; not the jm2 loc tile (object at z=3582).',
        },
        bank_tile: {
            x: 2725,
            z: 3491,
            plane: 0,
            provenance: 'curated',
            authority: 'reference rs2b0t-00d39a17e0 src/bot/api/ai/quests/defs/murder/areas.ts MURDER_TILE.BANK',
            note: 'Bank stand tile for Murder Mystery withdraw; not a loc placement row in selected content.',
        },
    };
}

export function extractDropFacts(content: string, items: ObjType[], npcs: NpcType[]) {
    const targets = [
        { alias: 'giant', block: 'ai_queue3:giant' },
        { alias: 'mossgiant', block: 'ai_queue3:mossgiant' },
        { alias: 'firegiant', block: 'ai_queue3:firegiant' },
        { alias: 'green_dragon', block: 'ai_queue3:green_dragon' },
    ];
    const blocks = parseDropBlocks(content);
    const npcConfigs = parseDropNpcs(content);
    const itemIds = new Map(items.filter((item) => item.debugname !== null).map((item) => [item.debugname as string, item]));
    const npcIds = new Map(npcs.filter((npc) => npc.debugname != null).map((npc) => [npc.debugname as string, npc]));

    const resolve = (key: string, deathDrop: string, seen: Set<string>, out: Set<string>) => {
        if (seen.has(key)) return;
        const block = blocks.get(key);
        if (!block) throw new Error(`missing drop block ${key}`);
        seen.add(key);
        for (const token of dropTokens(block.body)) {
            if (token.value.startsWith('~')) {
                resolve(`proc:${token.value.slice(1)}`, deathDrop, seen, out);
                continue;
            }
            const alias = token.value === 'npc_param' ? deathDrop : token.value;
            if (itemIds.has(alias)) {
                out.add(alias);
            } else if (alias.startsWith('cert_') && itemIds.has(alias.slice('cert_'.length))) {
                out.add(alias.slice('cert_'.length));
            } else if (!token.optional) {
                throw new Error(`${key}: unknown drop item ${alias}`);
            }
        }
    };

    return targets.map((target) => {
        const config = npcConfigs.get(target.alias);
        if (!config?.name) throw new Error(`${target.alias}: missing npc display name`);
        if (!config.death_drop) throw new Error(`${target.alias}: missing death_drop`);
        const npc = npcIds.get(target.alias);
        if (!npc) throw new Error(`${target.alias}: missing decoded npc`);
        if (npc.name !== config.name) throw new Error(`${target.alias}: npc display mismatch ${npc.name}/${config.name}`);
        const aliases = new Set<string>();
        resolve(target.block, config.death_drop, new Set(), aliases);
        const rows = [...aliases]
            .map((alias) => {
                const item = itemIds.get(alias)!;
                if (!item.name) throw new Error(`${target.alias}: ${alias} has no display name`);
                return { alias, id: item.id, name: item.name };
            })
            .sort((a, b) => a.name.localeCompare(b.name) || a.alias.localeCompare(b.alias));
        const displayNames = [...new Set(rows.map((item) => item.name))].sort((a, b) => a.localeCompare(b));
        if (rows.length === 0 || displayNames.length === 0) throw new Error(`${target.alias}: empty drop table`);
        return {
            npc_alias: target.alias,
            npc_id: npc.id,
            name: config.name,
            source_block: target.block,
            items: rows,
            display_names: displayNames,
        };
    });
}

type GatherLocRef = { alias: string; id: number };
type GatherOutputRef = { alias: string; id: number };
type GatherConfigSection = { params: Record<string, string>; [key: string]: string | Record<string, string> };

const WOOD_PUBLICATION: Record<string, 'published' | 'conditional' | 'unpublished'> = {
    normal: 'published',
    oak: 'published',
    willow: 'published',
    maple: 'published',
    yew: 'published',
    magic: 'published',
    achey: 'conditional',
    hollow: 'conditional',
    jungle: 'unpublished',
    burnt: 'unpublished',
};
const REVISION_ABSENT_ON_274 = [
    { alias: 'dungeon_tree_closed', other_pin_id: 5083 },
    { alias: 'karam_dungeon_exit', other_pin_id: 5084 },
];

export function parseConfigSections(text: string) {
    const sections = new Map<string, GatherConfigSection>();
    let current: GatherConfigSection | null = null;
    for (const raw of text.split(/\r?\n/)) {
        const line = raw.trim();
        if (!line || line.startsWith('//')) continue;
        if (line.startsWith('[') && line.endsWith(']')) {
            current = { params: {} };
            sections.set(line.slice(1, -1), current);
            continue;
        }
        if (!current) continue;
        const eq = line.indexOf('=');
        if (eq <= 0) continue;
        const key = line.slice(0, eq);
        const value = line.slice(eq + 1);
        if (key === 'param') {
            const comma = value.indexOf(',');
            const paramKey = comma < 0 ? value : value.slice(0, comma);
            const paramValue = comma < 0 ? '' : value.slice(comma + 1);
            if (paramKey) current.params[paramKey] = paramValue;
            continue;
        }
        if (typeof current[key] !== 'string') current[key] = value;
    }
    return sections;
}

function requireGatherText(content: string, relative: string) {
    const file = path.join(content, relative);
    if (!fs.existsSync(file)) throw new Error(`${relative}: required file missing`);
    return fs.readFileSync(file, 'utf8');
}

function joinPack(pack: Map<string, number>, alias: string, label: string, packName: string) {
    const id = pack.get(alias);
    if (id === undefined) throw new Error(`${label}: failed join, ${packName} lacks ${alias}`);
    return { alias, id };
}

function assertSelectedColumn(sections: Map<string, GatherConfigSection>, key: string, label: string) {
    if (![...sections.values()].some((section) => Object.prototype.hasOwnProperty.call(section.params, key))) {
        throw new Error(`${label}: missing selected column ${key}`);
    }
}

function extractLocResources(
    rows: { name: string; values: Record<string, string[][]> }[],
    locPack: Map<string, number>,
    objPack: Map<string, number>,
    sections: Map<string, GatherConfigSection>,
    spec: { aliasKey: string; outputKey: string; levelKey: string; transformKey: string; wood: boolean },
) {
    return rows.map((parsed) => {
        const aliases = (parsed.values[spec.aliasKey] ?? []).map((row) => row[0]).filter((alias): alias is string => Boolean(alias));
        if (aliases.length === 0) throw new Error(`${parsed.name}: no loc aliases`);
        const levelRaw = parsed.values[spec.levelKey]?.[0]?.[0];
        if (levelRaw === undefined) throw new Error(`${parsed.name}: missing ${spec.levelKey}`);
        const locIds = aliases.map((alias) => joinPack(locPack, alias, parsed.name, 'loc.pack'));
        const missingTransform: string[] = [];
        const empty = new Map<string, GatherLocRef>();
        for (const alias of aliases) {
            const section = sections.get(alias);
            const next = section?.params?.[spec.transformKey];
            if (!next) {
                missingTransform.push(alias);
                continue;
            }
            const joined = joinPack(locPack, next, `${parsed.name} transform ${alias}`, 'loc.pack');
            empty.set(joined.alias, joined);
        }
        const outputAlias = parsed.values[spec.outputKey]?.[0]?.[0];
        const output = outputAlias ? joinPack(objPack, outputAlias, `${parsed.name} output`, 'obj.pack') : null;
        const partialSides: string[] = [];
        if (missingTransform.length > 0) partialSides.push('transform');
        if (!output) partialSides.push('output');
        const resourceKey = spec.wood ? woodKey(parsed.name) : parsed.values.ore_name?.[0]?.[0];
        if (!resourceKey) throw new Error(`${parsed.name}: missing resource key`);
        const row: {
            table: string;
            resource_key: string;
            loc_ids: GatherLocRef[];
            empty_ids: GatherLocRef[];
            output: GatherOutputRef | null;
            level: number;
            qualification: 'complete' | 'partial';
            partial_sides: string[];
            missing_transform: string[];
            publication?: 'published' | 'conditional' | 'unpublished';
        } = {
            table: parsed.name,
            resource_key: resourceKey,
            loc_ids: locIds,
            empty_ids: [...empty.values()].sort((a, b) => a.id - b.id || a.alias.localeCompare(b.alias)),
            output,
            level: integer(levelRaw, parsed.name),
            qualification: partialSides.length === 0 ? 'complete' : 'partial',
            partial_sides: partialSides,
            missing_transform: missingTransform,
        };
        if (spec.wood) {
            const publication = WOOD_PUBLICATION[resourceKey];
            if (!publication) throw new Error(`${parsed.name}: unknown wood publication ${resourceKey}`);
            row.publication = publication;
        }
        return row;
    });
}

function woodKey(table: string) {
    const suffix = '_tree_table';
    if (!table.endsWith(suffix) || table.length === suffix.length) throw new Error(`${table}: expected wood table key`);
    return table.slice(0, -suffix.length);
}

export function extractGatherMethodsFacts(content: string, revision: number) {
    const mineText = requireGatherText(content, 'scripts/skill_mining/configs/mine.dbrow');
    const rockText = requireGatherText(content, 'scripts/skill_mining/configs/rocks.loc');
    const treeText = requireGatherText(content, 'scripts/skill_woodcutting/configs/trees.dbrow');
    const treeLocTexts = gatherContentFiles
        .filter((relative) => relative.startsWith('scripts/skill_woodcutting/configs/trees/') && relative.endsWith('.loc'))
        .map((relative) => requireGatherText(content, relative));
    const fishingText = requireGatherText(content, 'scripts/skill_fishing/configs/fishing.npc');
    const locPack = parsePack(requireGatherText(content, 'pack/loc.pack'));
    const objPack = parsePack(requireGatherText(content, 'pack/obj.pack'));
    if (locPack.size === 0) throw new Error('pack/loc.pack: required file missing ids');
    if (objPack.size === 0) throw new Error('pack/obj.pack: required file missing ids');
    const rockSections = parseConfigSections(rockText);
    const treeSections = new Map<string, GatherConfigSection>();
    for (const text of treeLocTexts) {
        for (const [alias, section] of parseConfigSections(text)) treeSections.set(alias, section);
    }
    assertSelectedColumn(rockSections, 'next_loc_stage_mining', 'scripts/skill_mining/configs/rocks.loc');
    assertSelectedColumn(treeSections, 'next_loc_stage', 'scripts/skill_woodcutting/configs/trees');
    const mining = extractLocResources(parseRows(mineText), locPack, objPack, rockSections, {
        aliasKey: 'rock',
        outputKey: 'rock_output',
        levelKey: 'rock_level',
        transformKey: 'next_loc_stage_mining',
        wood: false,
    });
    const woods = extractLocResources(parseRows(treeText), locPack, objPack, treeSections, {
        aliasKey: 'tree',
        outputKey: 'product',
        levelKey: 'levelrequired',
        transformKey: 'next_loc_stage',
        wood: true,
    });
    const fishingSeen = new Set<string>();
    const fishing: {
        category: string;
        primary_op: string;
        pair_op: string | null;
        level: null;
        output: null;
        qualification: 'partial';
        partial_sides: string[];
    }[] = [];
    for (const [name, section] of parseConfigSections(fishingText)) {
        const primary = typeof section.op1 === 'string' ? section.op1 : '';
        if (!primary) throw new Error(`${name}: missing primary op`);
        const category = typeof section.category === 'string' && section.category ? section.category : 'unknown';
        const pair = typeof section.op3 === 'string' ? section.op3 : null;
        const signature = `${category}\0${primary}\0${pair ?? ''}`;
        if (fishingSeen.has(signature)) continue;
        fishingSeen.add(signature);
        const partialSides = category === 'unknown' ? ['category', 'level', 'output'] : ['level', 'output'];
        fishing.push({
            category,
            primary_op: primary,
            pair_op: pair,
            level: null,
            output: null,
            qualification: 'partial',
            partial_sides: partialSides,
        });
    }
    if (mining.length === 0 || woods.length === 0 || fishing.length === 0) {
        throw new Error('gather_methods: no extracted rows');
    }
    const coverage: {
        class: string;
        table?: string;
        resource_key?: string;
        alias?: string;
        on_revision?: number;
        other_pin_id?: number;
        copied?: boolean;
        reason: string;
    }[] = [];
    for (const wood of woods) {
        if (wood.publication === 'conditional') {
            coverage.push({ class: 'conditional', table: wood.table, resource_key: wood.resource_key, reason: 'no supported consumer' });
        } else if (wood.publication === 'unpublished') {
            coverage.push({ class: 'unpublished', table: wood.table, resource_key: wood.resource_key, reason: 'unpublished wood' });
        }
    }
    if (revision === 274) {
        for (const absent of REVISION_ABSENT_ON_274) {
            if (locPack.has(absent.alias)) continue;
            coverage.push({
                class: 'revision-absent',
                alias: absent.alias,
                on_revision: 274,
                other_pin_id: absent.other_pin_id,
                copied: false,
                reason: '289-only loc, not copied onto 274',
            });
        }
    }
    return { woods, mining, fishing, coverage };
}

/** The `gather_methods.woods` row shape this family consumes. */
type GatherPlacementSource = { resource_key: string; publication?: 'published' | 'conditional' | 'unpublished'; loc_ids: { alias: string; id: number }[] };

/** One world LOC placement of a published resource loc id. No local coords, shape, or angle. */
export type GatherPlacement = { loc_id: number; x: number; z: number; plane: number };

/** A gather family with no selected published set. Unknown is not empty rows. */
export type GatherPlacementCoverage = { class: string; family: string; reason: string };

/** Published-resource world placements. `coverage` records what this family does not publish. */
export type GatherPlacementsFacts = { rows: GatherPlacement[]; coverage: GatherPlacementCoverage[] };

/** Per-wood placement count. A zero-hit loc_id variant is allowed; a zero-hit wood is not. */
export type GatherPlacementWood = { resource_key: string; loc_ids: number; placements: number };

/**
 * Provenance identity for this family: every scanned `maps/*.jm2` as one digest over
 * their per-file digests, `pack/loc.pack`, and the published loc-id set. Distinct from
 * `content_inputs` because the maps tree is a directory, not one tracked file per row.
 */
export type GatherPlacementInputs = {
    maps_directory: string;
    maps: { files: number; bytes: number; sha256: string };
    loc_pack: { path: string; bytes: number; sha256: string };
    published_loc_ids: { count: number; sha256: string };
};

export type GatherPlacementExtract = { facts: GatherPlacementsFacts; inputs: GatherPlacementInputs; woods: GatherPlacementWood[] };

const PLACEMENT_MAPS_DIRECTORY = 'maps';

/**
 * Required maps inventory (O-NAVINPUT). The directory must exist and hold at least
 * one map; every on-disk `m*.jm2` must be a readable mapsquare name; and every map
 * the content tree tracks must be on disk, so a truncated tree fails closed instead
 * of silently under-extracting. Filename order.
 */
function placementMapInputs(content: string) {
    const directory = path.join(content, PLACEMENT_MAPS_DIRECTORY);
    if (!fs.existsSync(directory)) throw new Error(`${PLACEMENT_MAPS_DIRECTORY}: required directory missing`);
    const names = fs.readdirSync(directory).filter((name) => name.endsWith('.jm2')).sort();
    if (names.length === 0) throw new Error(`${PLACEMENT_MAPS_DIRECTORY}: required directory has no maps`);
    const inputs = names.map((name) => {
        const relative = `${PLACEMENT_MAPS_DIRECTORY}/${name}`;
        parseMapsquarePath(relative);
        return { path: relative, ...sha256(path.join(content, relative)) };
    });
    const onDisk = new Set(inputs.map((input) => input.path));
    const tracked = execFileSync('git', ['-C', content, 'ls-files', '--', `${PLACEMENT_MAPS_DIRECTORY}/m*.jm2`], { encoding: 'utf8' }).split('\n').map((line) => line.trim()).filter(Boolean);
    if (tracked.length === 0) throw new Error(`${PLACEMENT_MAPS_DIRECTORY}: no tracked maps in the content tree`);
    for (const relative of tracked) if (!onDisk.has(relative)) throw new Error(`${relative}: tracked map missing from the content tree`);
    return inputs;
}

/**
 * World LOC placements of the published woods' loc ids, plus the provenance identity
 * that invalidates them. LOC-only and fail-closed: a malformed row, a missing or
 * truncated `maps/`, a badly named map, a missing map, or a published wood with no
 * hits throws. Stored rows are resource loc_ids only, never empty/stump ids; the one
 * local coordinate pair is converted with `worldFromMapsquare`.
 */
export function extractGatherPlacementsFacts(content: string, woods: GatherPlacementSource[]): GatherPlacementExtract {
    const published = woods.filter((row) => row.publication === 'published');
    if (published.length === 0) throw new Error('gather_placements: no published woods');
    const locIds = new Set(published.flatMap((row) => row.loc_ids.map((loc) => loc.id)));
    const maps = placementMapInputs(content);
    const rows: GatherPlacement[] = [];
    const hits = new Map<number, number>();
    for (const input of maps) {
        const { mx, mz } = parseMapsquarePath(input.path);
        for (const placement of parseJm2LocPlacements(fs.readFileSync(path.join(content, input.path), 'utf8'), locIds)) {
            const world = worldFromMapsquare(mx, mz, placement.lx, placement.lz, placement.plane);
            rows.push({ loc_id: placement.loc_id, x: world.x, z: world.z, plane: world.plane });
            hits.set(placement.loc_id, (hits.get(placement.loc_id) ?? 0) + 1);
        }
    }
    rows.sort((a, b) => a.loc_id - b.loc_id || a.x - b.x || a.z - b.z || a.plane - b.plane);
    const perWood: GatherPlacementWood[] = [];
    for (const wood of published) {
        const placements = wood.loc_ids.reduce((sum, loc) => sum + (hits.get(loc.id) ?? 0), 0);
        if (placements === 0) throw new Error(`gather_placements: published wood ${wood.resource_key} has no LOC placements`);
        perWood.push({ resource_key: wood.resource_key, loc_ids: wood.loc_ids.length, placements });
    }
    const locPack = 'pack/loc.pack';
    const locPackFile = path.join(content, locPack);
    if (!fs.existsSync(locPackFile)) throw new Error(`${locPack}: required file missing`);
    return {
        facts: {
            rows,
            coverage: [{ class: 'unknown', family: 'mining', reason: 'no selected published-ore set' }],
        },
        inputs: {
            maps_directory: PLACEMENT_MAPS_DIRECTORY,
            maps: {
                files: maps.length,
                bytes: maps.reduce((sum, input) => sum + input.bytes, 0),
                sha256: crypto.createHash('sha256').update(maps.map((input) => `${input.sha256}  ${input.path}`).join('\n')).digest('hex'),
            },
            loc_pack: { path: locPack, ...sha256(locPackFile) },
            published_loc_ids: {
                count: locIds.size,
                sha256: crypto.createHash('sha256').update([...new Set(published.flatMap((row) => row.loc_ids.map((loc) => `${row.resource_key}\t${loc.id}`)))].sort().join('\n')).digest('hex'),
            },
        },
        woods: perWood,
    };
}

const QUEST_IDENTITY_SEEDS = [
    { id: 'cook', component: 'cook' },
    { id: 'runemysteries', component: 'runemysteries' },
    { id: 'murder', component: 'murder' },
    { id: 'waterfall', component: 'waterfall' },
    { id: 'death', component: 'death' },
    { id: 'zanaris', component: 'zanaris' },
] as const;

type QuestItemAlias = { alias: string; quantity: number | null; kind: 'inv' | 'use-site' };
type QuestSkillGate = { skill: string; level: number };
type QuestRequirements = {
    qualification: 'partial';
    skills: QuestSkillGate[];
    items: QuestItemAlias[];
    empty_must_have: boolean;
    unknown_as_satisfied: false;
};

function parseQuestColourCalls(text: string) {
    const calls: { component: string; progress: string; complete: string }[] = [];
    for (const raw of text.split(/\r?\n/)) {
        const line = raw.trim();
        if (!line.startsWith('~send_quest_progress_colour(questlist:')) continue;
        const match = /^~send_quest_progress_colour\(questlist:([A-Za-z0-9_]+),\s*([^,]+),\s*(.+)\);$/.exec(line);
        if (!match) throw new Error(`quests.rs2: malformed colour call ${line}`);
        calls.push({ component: match[1], progress: match[2].trim(), complete: match[3].trim() });
    }
    return calls;
}

function parseQuestConstants(text: string) {
    const out = new Map<string, number>();
    for (const raw of text.split(/\r?\n/)) {
        const line = raw.trim();
        if (!line || line.startsWith('//')) continue;
        const match = /^\^([A-Za-z0-9_]+)\s*=\s*(-?\d+)\s*(?:\/\/.*)?$/.exec(line);
        if (!match) continue;
        if (out.has(match[1])) throw new Error(`quest.constant: duplicate ^${match[1]}`);
        out.set(match[1], integer(match[2], `^${match[1]}`));
    }
    if (out.size === 0) throw new Error('quest.constant: no constants');
    return out;
}

function parseQuestListText(text: string) {
    const out = new Map<string, string>();
    let section: string | null = null;
    for (const raw of text.split(/\r?\n/)) {
        const line = raw.trim();
        if (!line || line.startsWith('//')) continue;
        if (line.startsWith('[') && line.endsWith(']')) {
            section = line.slice(1, -1);
            continue;
        }
        if (!section || !line.startsWith('text=')) continue;
        if (out.has(section)) throw new Error(`questlist.if: duplicate text= for ${section}`);
        const display = line.slice('text='.length).trim();
        if (!display) throw new Error(`questlist.if: empty text= for ${section}`);
        out.set(section, display);
    }
    return out;
}

function parseQuestEnumDisplays(text: string) {
    const names = new Set<string>();
    let inQuestNames = false;
    let saw = false;
    for (const raw of text.split(/\r?\n/)) {
        const line = raw.trim();
        if (!line || line.startsWith('//')) continue;
        if (line.startsWith('[') && line.endsWith(']')) {
            const section = line.slice(1, -1);
            if (inQuestNames && section !== 'quest_names_enum') break;
            inQuestNames = section === 'quest_names_enum';
            if (inQuestNames) saw = true;
            continue;
        }
        if (!inQuestNames || !line.startsWith('val=')) continue;
        const comma = line.indexOf(',');
        if (comma < 0) throw new Error(`quest.enum: malformed val line ${line}`);
        const name = line.slice(comma + 1).trim();
        if (!name) throw new Error(`quest.enum: empty display ${line}`);
        if (names.has(name)) throw new Error(`quest.enum: duplicate display ${name}`);
        names.add(name);
    }
    if (!saw || names.size === 0) throw new Error('quest.enum: missing [quest_names_enum]');
    return names;
}

function scriptHas(text: string, pattern: RegExp, label: string) {
    if (!pattern.test(text)) throw new Error(label);
}

function cookRequirements(text: string): QuestRequirements {
    const aliases = ['egg', 'bucket_milk', 'pot_flour'] as const;
    for (const alias of aliases) {
        scriptHas(text, new RegExp(`inv_total\\(inv, ${alias}\\)(?![A-Za-z0-9_])`), `quest_cook.rs2: missing inv_total for ${alias}`);
        scriptHas(text, new RegExp(`inv_del\\(inv, ${alias}, 1\\)(?![A-Za-z0-9_])`), `quest_cook.rs2: missing inv_del quantity 1 for ${alias}`);
    }
    return {
        qualification: 'partial',
        skills: [],
        items: aliases.map((alias) => ({ alias, quantity: 1, kind: 'inv' as const })),
        empty_must_have: false,
        unknown_as_satisfied: false,
    };
}

function waterfallRequirements(text: string): QuestRequirements {
    scriptHas(text, /last_useitem ! rope(?![A-Za-z0-9_])/, 'quest_waterfall.rs2: missing rope use-site check');
    return {
        qualification: 'partial',
        skills: [],
        items: [{ alias: 'rope', quantity: null, kind: 'use-site' }],
        empty_must_have: false,
        unknown_as_satisfied: false,
    };
}

function zanarisRequirements(text: string): QuestRequirements {
    const skills = [
        { skill: 'woodcutting', level: 36 },
        { skill: 'crafting', level: 31 },
    ];
    for (const gate of skills) {
        scriptHas(text, new RegExp(`stat\\(${gate.skill}\\) < ${gate.level}(?!\\d)`), `quest_zanaris.rs2: missing ${gate.skill} gate ${gate.level}`);
    }
    return {
        qualification: 'partial',
        skills,
        items: [],
        empty_must_have: false,
        unknown_as_satisfied: false,
    };
}

function emptyMustHave(): QuestRequirements {
    return {
        qualification: 'partial',
        skills: [],
        items: [],
        empty_must_have: true,
        unknown_as_satisfied: false,
    };
}

function requirementsFor(id: string, content: string): QuestRequirements {
    if (id === 'cook') return cookRequirements(requireGatherText(content, 'scripts/quests/quest_cook/scripts/quest_cook.rs2'));
    if (id === 'waterfall') return waterfallRequirements(requireGatherText(content, 'scripts/quests/quest_waterfall/scripts/quest_waterfall.rs2'));
    if (id === 'zanaris') return zanarisRequirements(requireGatherText(content, 'scripts/quests/quest_zanaris/scripts/quest_zanaris.rs2'));
    if (id === 'runemysteries' || id === 'murder' || id === 'death') return emptyMustHave();
    throw new Error(`quest_identity: unknown seed ${id}`);
}

export function extractQuestIdentityFacts(content: string, revision: number) {
    const quests = requireGatherText(content, 'scripts/general/scripts/quests.rs2');
    const constants = parseQuestConstants(requireGatherText(content, 'scripts/general/configs/quest.constant'));
    const displays = parseQuestListText(requireGatherText(content, 'scripts/player/interfaces/questlist.if'));
    const varpPack = parsePack(requireGatherText(content, 'pack/varp.pack'));
    const enumNames = parseQuestEnumDisplays(requireGatherText(content, 'scripts/general/configs/quest.enum'));
    if (varpPack.size === 0) throw new Error('pack/varp.pack: required file missing ids');
    const calls = parseQuestColourCalls(quests);
    const rows = QUEST_IDENTITY_SEEDS.map((seed) => {
        const found = calls.filter((call) => call.component === seed.component);
        if (found.length === 0) throw new Error(`quest_identity: no extracted rows; quests.rs2 has no colour call for ${seed.id}`);
        if (found.length !== 1) throw new Error(`quests.rs2: dual binding for ${seed.id}`);
        const call = found[0];
        const progress = /^%([A-Za-z0-9_]+)$/.exec(call.progress);
        if (!progress) {
            const why = call.progress.startsWith('~') ? 'proc operand' : 'not a %varp';
            throw new Error(`quests.rs2: ${seed.id} ${why} ${call.progress}`);
        }
        const varp = progress[1];
        const varpId = varpPack.get(varp);
        if (varpId === undefined) throw new Error(`quests.rs2: ${seed.id} colour operand %${varp} is absent from varp.pack`);
        const stem = `^${seed.id}_complete`;
        if (call.complete !== stem) {
            const why = call.complete.includes('(') || call.complete.includes(',') ? 'computed complete' : 'constant stem';
            throw new Error(`quests.rs2: ${seed.id} ${why} ${call.complete}`);
        }
        const complete = constants.get(`${seed.id}_complete`);
        const questPoints = constants.get(`${seed.id}_questpoints`);
        if (complete === undefined) throw new Error(`quest.constant: missing ^${seed.id}_complete`);
        if (questPoints === undefined) throw new Error(`quest.constant: missing ^${seed.id}_questpoints`);
        const display = displays.get(seed.component);
        if (!display) throw new Error(`questlist.if: missing text= for ${seed.component}`);
        if (!enumNames.has(display)) throw new Error(`quest.enum: display mismatch for ${seed.id}: ${display}`);
        return {
            id: seed.id,
            component: seed.component,
            display,
            varp,
            varp_id: varpId,
            complete,
            quest_points: questPoints,
            unknown_sides: [] as string[],
            requirements: requirementsFor(seed.id, content),
        };
    });
    if (rows.length !== QUEST_IDENTITY_SEEDS.length) throw new Error('quest_identity: no extracted rows');
    if (rows.some((row) => row.requirements.qualification !== 'partial' || row.requirements.unknown_as_satisfied)) {
        throw new Error('quest_identity: requirements must stay partial');
    }
    const coverage: {
        class: 'revision-absent';
        alias: string;
        on_revision: number;
        other_pin_id: number;
        copied: false;
        reason: string;
    }[] = [];
    if (revision === 274) {
        if (varpPack.has('routequest')) throw new Error('274: routequest is in varp.pack; do not copy it and do not emit revision-absent');
        coverage.push({
            class: 'revision-absent',
            alias: 'routequest',
            on_revision: 274,
            other_pin_id: 387,
            copied: false,
            reason: '289-only quest, not copied onto 274',
        });
    }
    const facts = { rows, coverage };
    if (JSON.stringify(facts).includes('family-unavailable')) throw new Error('quest_identity: must not emit family-unavailable');
    return facts;
}

/** One selected `param=` line. The value stays the raw string, in file order. */
export type TrailParam = { key: string; value: string };
/** One membership row: an enum alias or a casket those rows name. `access` is present only on packed 3554. */
export type TrailMembershipRow = { alias: string; id: number; role: 'clue' | 'casket'; params: TrailParam[]; access?: 'constrained' };
/** One challenge answer. The raw param string, not a coerced number. */
export type TrailChallengeAnswer = { alias: string; id: number; answer: string };
export type TrailFacts = { rows: TrailMembershipRow[]; challenge_answers: TrailChallengeAnswer[] };

const TRAIL_TIERS = ['easy', 'medium', 'hard'] as const;
const TRAIL_CONFIG_DIR = 'scripts/minigames/game_trail/configs';
const TRAIL_CASKET_RELATIVE = `${TRAIL_CONFIG_DIR}/trail_casket.obj`;
/** The one bounded inclusion. 3554 is an inventory row, access-constrained, not supported. */
const TRAIL_CONSTRAINED = { alias: 'trail_clue_hard_sextant028', id: 3554 };
const TRAIL_CHALLENGE_ANSWERS_EXPECTED = [
    ['trail_clue_medium_anagram001_challenge', 2842, '6859'],
    ['trail_clue_medium_anagram002_challenge', 2844, '9'],
    ['trail_clue_medium_anagram003_challenge', 2846, '40'],
    ['trail_clue_medium_anagram006_challenge', 2850, '5'],
    ['trail_clue_medium_anagram007_challenge', 2852, '48'],
    ['trail_clue_medium_anagram008_challenge', 2854, '5096'],
] as const;

/**
 * Trail obj blocks by alias. Only `param=` lines are selected. Repeated keys stay
 * a list in file order; no last-write-wins. This is not `parseConfigSections`.
 */
export function parseTrailObjBlocks(text: string) {
    const blocks = new Map<string, TrailParam[]>();
    let current: TrailParam[] | null = null;
    for (const raw of text.split(/\r?\n/)) {
        const line = raw.trim();
        if (!line || line.startsWith('//')) continue;
        if (line.startsWith('[') && line.endsWith(']')) {
            const alias = line.slice(1, -1);
            if (blocks.has(alias)) throw new Error(`${alias}: duplicate trail obj block`);
            current = [];
            blocks.set(alias, current);
            continue;
        }
        if (!current) continue;
        const eq = line.indexOf('=');
        if (eq <= 0 || line.slice(0, eq) !== 'param') continue;
        const value = line.slice(eq + 1);
        const comma = value.indexOf(',');
        current.push({ key: comma < 0 ? value : value.slice(0, comma), value: comma < 0 ? '' : value.slice(comma + 1) });
    }
    return blocks;
}

/** `val=<index>,<alias>` rows of one trail enum, in file order. */
export function parseTrailEnumAliases(text: string) {
    const aliases: string[] = [];
    for (const raw of text.split(/\r?\n/)) {
        const line = raw.trim();
        if (!line || line.startsWith('//') || !line.startsWith('val=')) continue;
        const comma = line.indexOf(',');
        if (comma < 0) throw new Error(`${line}: malformed trail enum row`);
        const alias = line.slice(comma + 1).trim();
        if (!alias) throw new Error(`${line}: malformed trail enum row`);
        aliases.push(alias);
    }
    return aliases;
}

function trailJoin(objPack: Map<string, number>, itemIds: Map<string, { id: number; name: string | null }>, alias: string, label: string) {
    const packed = objPack.get(alias);
    if (packed === undefined) throw new Error(`${label}: pack/obj.pack lacks ${alias}`);
    const decoded = itemIds.get(alias);
    if (!decoded) throw new Error(`${label}: ${alias} has no decoded item row`);
    if (decoded.id !== packed) throw new Error(`${label}: pack/obj.pack ${alias}=${packed} disagrees with decoded item id ${decoded.id}`);
    return { alias, id: packed };
}

/**
 * Trail membership on the selected content. Clue rows are the three enum joins
 * (`trail_easy.enum`, `trail_medium.enum`, `trail_hard.enum`) against the matching
 * obj block and `pack/obj.pack`, in enum order. Casket rows are the unique
 * `param=trail_casket` aliases those rows name, in first-named order. A named
 * casket needs no obj block. Every join must equal the decoded item id.
 * Challenge answers are a sibling list, not membership rows.
 */
export function extractTrailFacts(content: string, items: ObjType[]): TrailFacts {
    const itemIds = new Map(items.filter((item) => item.debugname !== null).map((item) => [item.debugname as string, { id: item.id, name: item.name }]));
    const objPack = parsePack(requireGatherText(content, 'pack/obj.pack'));
    if (objPack.size === 0) throw new Error('pack/obj.pack: required file missing ids');
    const blocks = new Map<string, TrailParam[]>();
    for (const relative of [...TRAIL_TIERS.map((tier) => `${TRAIL_CONFIG_DIR}/trail_${tier}.obj`), TRAIL_CASKET_RELATIVE]) {
        for (const [alias, params] of parseTrailObjBlocks(requireGatherText(content, relative))) {
            if (blocks.has(alias)) throw new Error(`${alias}: declared by more than one trail obj file`);
            blocks.set(alias, params);
        }
    }
    const rows: TrailMembershipRow[] = [];
    const namedCaskets: string[] = [];
    for (const tier of TRAIL_TIERS) {
        const enumRelative = `${TRAIL_CONFIG_DIR}/trail_${tier}.enum`;
        for (const alias of parseTrailEnumAliases(requireGatherText(content, enumRelative))) {
            if (!blocks.has(alias)) throw new Error(`${enumRelative}: ${alias} has no obj block`);
            const joined = trailJoin(objPack, itemIds, alias, enumRelative);
            const params = blocks.get(alias) as TrailParam[];
            rows.push({ alias: joined.alias, id: joined.id, role: 'clue', params });
            for (const param of params) {
                if (param.key === 'trail_casket' && !namedCaskets.includes(param.value)) namedCaskets.push(param.value);
            }
        }
    }
    for (const alias of namedCaskets) {
        const joined = trailJoin(objPack, itemIds, alias, 'trail_casket param');
        rows.push({ alias: joined.alias, id: joined.id, role: 'casket', params: blocks.get(alias) ?? [] });
    }
    const constrained = rows.filter((row) => row.alias === TRAIL_CONSTRAINED.alias);
    if (constrained.length !== 1) throw new Error(`trails: ${TRAIL_CONSTRAINED.alias} must be one membership row`);
    if (constrained[0].id !== TRAIL_CONSTRAINED.id) throw new Error(`trails: ${TRAIL_CONSTRAINED.alias} pack id ${constrained[0].id}, expected ${TRAIL_CONSTRAINED.id}`);
    constrained[0].access = 'constrained';
    const challenge_answers: TrailChallengeAnswer[] = [];
    for (const [alias, params] of blocks) {
        const answers = params.filter((param) => param.key === 'trail_challenge_answer');
        if (answers.length === 0) continue;
        if (answers.length !== 1) throw new Error(`${alias}: repeated trail_challenge_answer`);
        const joined = trailJoin(objPack, itemIds, alias, 'trail_challenge_answer');
        challenge_answers.push({ alias: joined.alias, id: joined.id, answer: answers[0].value });
    }
    if (challenge_answers.length === 0) throw new Error('trails: no selected challenge answers');
    return { rows, challenge_answers };
}

/** Fail-closed pins for the trail publication. The counts are never copied from the corpus. */
export function assertTrailPins(trails: TrailFacts, revision: number) {
    const row = (alias: string) => {
        const found = trails.rows.find((entry) => entry.alias === alias);
        if (!found) throw new Error(`${revision}: trails is missing ${alias}`);
        return found;
    };
    const params = (alias: string) => row(alias).params.map((param) => `${param.key}=${param.value}`);
    const riddle004 = row('trail_clue_hard_riddle004_casket');
    if (riddle004.role !== 'casket' || riddle004.id !== 2779 || riddle004.params.length !== 0) throw new Error(`${revision}: trails riddle004_casket must stay a parameterless casket 2779`);
    if (row('trail_clue_hard_sextant016_casket').id !== 3531 || trails.rows.filter((entry) => entry.id === 3531).length !== 1) throw new Error(`${revision}: trails must inventory the named casket 3531 once`);
    if (!params('trail_clue_hard_sextant017').includes('trail_casket=trail_clue_hard_sextant016_casket')) throw new Error(`${revision}: trails sextant017 must name its selected casket alias`);
    if (trails.rows.some((entry) => entry.id === 3533 || entry.alias === 'trail_clue_hard_sextant017_casket')) throw new Error(`${revision}: trails must not emit the unnamed casket 3533`);
    for (const alias of ['trail_clue_medium_map002', 'trail_clue_medium_map002_casket', 'trail_clue_hard_sextant026', 'trail_clue_hard_sextant026_casket']) {
        if (trails.rows.some((entry) => entry.alias === alias)) throw new Error(`${revision}: ${alias} is not a membership row`);
    }
    const constrained = trails.rows.filter((entry) => entry.access !== undefined);
    if (constrained.length !== 1 || constrained[0].alias !== TRAIL_CONSTRAINED.alias || constrained[0].access !== 'constrained') throw new Error(`${revision}: trails access must stay on ${TRAIL_CONSTRAINED.alias} only`);
    if (!params('trail_clue_easy_simple001').includes('trail_loc=^true')) throw new Error(`${revision}: trails simple001 trail_loc must stay the raw ^true`);
    if (!params('trail_clue_hard_sextant028').includes('trail_sextant=yes')) throw new Error(`${revision}: trails sextant028 trail_sextant must stay the raw yes`);
    const anagram001 = params('trail_clue_medium_anagram001');
    if (anagram001.length !== 1 || anagram001[0] !== 'trail_desc=Speak to Hazelmere.') throw new Error(`${revision}: trails anagram001 must keep its own single selected param`);
    if (JSON.stringify(trails.challenge_answers) !== JSON.stringify(TRAIL_CHALLENGE_ANSWERS_EXPECTED.map(([alias, id, answer]) => ({ alias, id, answer })))) throw new Error(`${revision}: trails challenge answers must be the six selected raw strings`);
    const blob = JSON.stringify(trails);
    for (const banned of ['facts-verified-both', 'facts_field_verified_both', 'family-unavailable', 'supported', 'coverage', 'regicide', 'legends', 'curated-unverified', '0_51_54_45_47', '1_42_53_14_17', '1_40_51_14_62']) {
        if (blob.includes(banned)) throw new Error(`${revision}: trails published ${banned}`);
    }
}

/* ------------------------------------------------------------------------- *
 * talk_key: the selected talk steps and key keepers
 *
 * A sibling family of `trails`, never a field on a membership row. Talk steps
 * are the membership clues an `opnpc1` handler checks; key keepers are the
 * `trail_checkmediumdrop` arms. Spawns are the static `==== NPC ====` jm2 rows:
 * one hit publishes a tile, no hit throws for a packed identity, two or more
 * omit the spawn and record honest coverage. Unique spawn and 0-hit throws are
 * the only two outcomes; first-in-file is never taken.
 * ------------------------------------------------------------------------- */

const TALK_KEY_SCRIPTS_DIRECTORY = 'scripts';
const TALK_KEY_NPC_PACK_RELATIVE = 'pack/npc.pack';
const TALK_KEY_MEDIUM_RELATIVE = 'scripts/minigames/game_trail/scripts/medium/trail_clue_medium.rs2';
const TALK_KEY_RS2_SUFFIX = '.rs2';
const TALK_KEY_NPC_SUFFIX = '.npc';

/** One jm2 `==== NPC ====` spawn in world coordinates. Plane, never scene `level`. */
export type TalkKeySpawn = { x: number; z: number; plane: number };

/**
 * Keeper matcher, discriminated on `kind`. `type` is one packed npc id and carries
 * the `.npc` display name; `category` and `name` match many npcs and never carry
 * an alias or an id.
 */
export type TalkKeyKeeper =
    | { kind: 'type'; alias: string; id: number; name: string }
    | { kind: 'category'; category: string }
    | { kind: 'name'; name: string };

/** One opnpc1 talk step: the membership clue that anchors it and the NPC it names. */
export type TalkKeyTalkRow = { alias: string; id: number; npc: { alias: string; id: number; name: string }; spawn?: TalkKeySpawn };

/** One key-keeper step: the membership clue, its key object, and the keeper matcher. */
export type TalkKeyKeyRow = { alias: string; id: number; key_alias: string; key_id: number; keeper: TalkKeyKeeper; spawn?: TalkKeySpawn };

/** Why one step publishes no spawn. A coverage record is a sibling, not a row. */
export type TalkKeyCoverageRow = { class: string; family: string; alias: string; reason: string };

/** Talk steps and key keepers. A step without a unique spawn keeps its row and omits `spawn`. */
export type TalkKeyFacts = { talk: TalkKeyTalkRow[]; keys: TalkKeyKeyRow[]; coverage: TalkKeyCoverageRow[] };

/**
 * Provenance identity for this family: every scanned `scripts` rs2 file as one
 * digest, every scanned npc config as one digest, the maps inventory the spawns
 * come from, `pack/npc.pack`, and the pinned medium clue proc.
 */
export type TalkKeyInputs = {
    maps_directory: string;
    maps: { files: number; bytes: number; sha256: string };
    npc_pack: { path: string; bytes: number; sha256: string };
    scripts: { files: number; bytes: number; sha256: string };
    npc_configs: { files: number; bytes: number; sha256: string };
    trail_clue_medium: { path: string; bytes: number; sha256: string };
};

export type TalkKeyExtract = { facts: TalkKeyFacts; inputs: TalkKeyInputs };

/** One keeper matcher as written in the proc, before the pack and `.npc` joins. */
type TalkKeyKeeperMatch = { kind: 'type'; alias: string } | { kind: 'category'; category: string } | { kind: 'name'; name: string };

/**
 * NPC placements only — the `==== NPC ====` section of one jm2, mirroring the LOC
 * parser's fail-closed gating on a different section. Rows are `plane lx lz: npc_id`
 * with exactly one data token: `==== LOC ====` and `==== OBJ ====` are not placements,
 * and a shape, angle, or any other extra token throws.
 */
export function parseJm2NpcPlacements(text: string) {
    const placements: { plane: number; lx: number; lz: number; npc_id: number }[] = [];
    let inNpc = false;
    for (const raw of text.split(/\r?\n/)) {
        const line = raw.trim();
        if (!line) continue;
        const section = jm2SectionName(line);
        if (section !== null) {
            inNpc = section === 'NPC';
            continue;
        }
        if (!inNpc) continue;
        const colon = line.indexOf(':');
        if (colon <= 0) throw new Error(`jm2 NPC: malformed row ${line}`);
        const coordTokens = line.slice(0, colon).trim().split(/\s+/);
        if (coordTokens.length !== 3) throw new Error(`jm2 NPC: bad coords ${line}`);
        const plane = integer(coordTokens[0], line);
        const lx = integer(coordTokens[1], line);
        const lz = integer(coordTokens[2], line);
        if (plane < 0 || plane > 3) throw new Error(`jm2 NPC: plane out of range ${line}`);
        if (lx < 0 || lx > 63 || lz < 0 || lz > 63) throw new Error(`jm2 NPC: local coords out of range ${line}`);
        const dataTokens = line.slice(colon + 1).trim().split(/\s+/).filter((token) => token.length > 0);
        if (dataTokens.length === 0) throw new Error(`jm2 NPC: missing npc id ${line}`);
        if (dataTokens.length > 1) throw new Error(`jm2 NPC: extra tokens ${line}`);
        placements.push({ plane, lx, lz, npc_id: integer(dataTokens[0], line) });
    }
    return placements;
}

/**
 * `opnpc1` handlers that check one of the three membership enums, in file order.
 * Talk-to only: `opnpc2`, `opnpc3`, `opnpcu`, and `apnpc*` are never read. The
 * identity is the header alias, except on a `_<category>` header, which must
 * declare exactly one inner `npc_type = <alias>` — the category itself is a
 * trigger, not an NPC. Clues outside the membership set (challenge scrolls,
 * puzzle-box extras) are dropped.
 */
export function parseTalkKeyHandlers(text: string, membership: ReadonlySet<string>) {
    const rows: { handler: string; alias: string; clue: string }[] = [];
    let header: string | null = null;
    let body: string[] = [];
    const flush = () => {
        if (header === null) return;
        const parts = header.split(',').map((part) => part.trim());
        const handler = parts[1] ?? '';
        if (parts[0] === 'opnpc1' && handler) {
            const block = body.join('\n');
            const clues = [...new Set([...block.matchAll(/inv_total\(\s*inv\s*,\s*(trail_clue_[A-Za-z0-9_]+)\s*\)/g)].map((match) => match[1]))].filter((clue) => membership.has(clue)).sort();
            if (clues.length > 0) {
                let alias = handler;
                if (handler.startsWith('_')) {
                    const inner = [...new Set([...block.matchAll(/npc_type\s*=\s*([A-Za-z0-9_]+)/g)].map((match) => match[1]))];
                    if (inner.length !== 1) throw new Error(`talk_key: ${header} must declare exactly one inner npc_type, got ${inner.length}`);
                    alias = inner[0];
                }
                for (const clue of clues) rows.push({ handler, alias, clue });
            }
        }
        header = null;
        body = [];
    };
    for (const raw of text.split(/\r?\n/)) {
        const line = raw.trim();
        if (line.startsWith('[') && line.endsWith(']')) {
            flush();
            header = line.slice(1, -1);
            continue;
        }
        if (header !== null) body.push(line);
    }
    flush();
    return rows;
}

/**
 * The `[proc,trail_checkmediumdrop]` arms, in file order. Each arm is one keeper
 * matcher, one membership clue check, and one `obj_add(npc_coord, key, ...)` whose
 * `obj_gettotal` guard names the same key; a loc-search label is not an arm and a
 * malformed arm throws. Call sites of the proc are not a second mapping.
 */
export function parseTalkKeyKeeperArms(text: string) {
    let body: string | null = null;
    let header: string | null = null;
    let lines: string[] = [];
    const flush = () => {
        if (header === 'proc,trail_checkmediumdrop') body = lines.join('\n');
        header = null;
        lines = [];
    };
    for (const raw of text.split(/\r?\n/)) {
        const line = raw.trim();
        if (line.startsWith('[') && line.endsWith(']')) {
            flush();
            header = line.slice(1, -1);
            continue;
        }
        if (header !== null) lines.push(line);
    }
    flush();
    if (body === null) throw new Error('talk_key: required content is missing [proc,trail_checkmediumdrop]');
    const arms: { clue: string; key: string; keeper: TalkKeyKeeperMatch }[] = [];
    for (const arm of body.split(/else\s+if\(/)) {
        const types = [...new Set([...arm.matchAll(/npc_type\s*=\s*([A-Za-z0-9_]+)/g)].map((match) => match[1]))];
        const categories = [...new Set([...arm.matchAll(/npc_category\s*=\s*([A-Za-z0-9_]+)/g)].map((match) => match[1]))];
        const names = [...new Set([...arm.matchAll(/compare\(\s*npc_name\s*,\s*"([^"]+)"\s*\)\s*=\s*0/g)].map((match) => match[1]))];
        const clues = [...new Set([...arm.matchAll(/inv_total\(\s*inv\s*,\s*(trail_clue_[A-Za-z0-9_]+)\s*\)\s*>\s*0/g)].map((match) => match[1]))];
        const added = [...new Set([...arm.matchAll(/obj_add\(\s*npc_coord\s*,\s*([A-Za-z0-9_]+)/g)].map((match) => match[1]))];
        if (clues.length === 0 && types.length === 0 && categories.length === 0 && names.length === 0 && added.length === 0) continue;
        const matches = types.length + categories.length + names.length;
        if (matches !== 1 || clues.length !== 1 || added.length !== 1) throw new Error(`talk_key: malformed trail_checkmediumdrop arm (${matches} keeper matches, ${clues.length} clues, ${added.length} keys)`);
        const guards = [...new Set([...arm.matchAll(/obj_gettotal\(\s*([A-Za-z0-9_]+)\s*\)\s*=\s*0/g)].map((match) => match[1]))];
        if (guards.length !== 1 || guards[0] !== added[0]) throw new Error(`talk_key: trail_checkmediumdrop arm for ${clues[0]} must guard the key it adds`);
        const keeper: TalkKeyKeeperMatch = types.length === 1 ? { kind: 'type', alias: types[0] } : categories.length === 1 ? { kind: 'category', category: categories[0] } : { kind: 'name', name: names[0] };
        arms.push({ clue: clues[0], key: added[0], keeper });
    }
    if (arms.length === 0) throw new Error('talk_key: trail_checkmediumdrop has no keeper arms');
    return arms;
}

/** One digest over every inventoried file's digest, in path order. */
function fileInventoryDigest(entries: { path: string; bytes: number; sha256: string }[]) {
    return {
        files: entries.length,
        bytes: entries.reduce((sum, entry) => sum + entry.bytes, 0),
        sha256: crypto.createHash('sha256').update(entries.map((entry) => `${entry.sha256}  ${entry.path}`).join('\n')).digest('hex'),
    };
}

/**
 * Required scanned content inventory. The directory must exist and hold at least one
 * matching file, and every tracked file the content tree names must be on disk, so a
 * truncated tree fails closed instead of silently under-hashing. Path order.
 */
function contentTreeInputs(content: string, directory: string, suffix: string) {
    const rootDirectory = path.join(content, directory);
    if (!fs.existsSync(rootDirectory)) throw new Error(`${directory}: required directory missing`);
    const relativePaths: string[] = [];
    const walk = (absolute: string) => {
        for (const entry of fs.readdirSync(absolute, { withFileTypes: true })) {
            const child = path.join(absolute, entry.name);
            if (entry.isDirectory()) walk(child);
            else if (entry.name.endsWith(suffix)) relativePaths.push(path.relative(content, child).split(path.sep).join('/'));
        }
    };
    walk(rootDirectory);
    if (relativePaths.length === 0) throw new Error(`${directory}: required directory has no ${suffix} files`);
    relativePaths.sort();
    const tracked = execFileSync('git', ['-C', content, 'ls-files', '--', `${directory}/*${suffix}`], { encoding: 'utf8' }).split('\n').map((line) => line.trim()).filter(Boolean);
    if (tracked.length === 0) throw new Error(`${directory}: no tracked ${suffix} files in the content tree`);
    const onDisk = new Set(relativePaths);
    for (const relative of tracked) if (!onDisk.has(relative)) throw new Error(`${relative}: tracked ${suffix} file missing from the content tree`);
    const entries = relativePaths.map((relative) => ({ path: relative, ...sha256(path.join(content, relative)) }));
    return { files: relativePaths, digest: fileInventoryDigest(entries) };
}

/**
 * The selected talk steps and key keepers, from the `opnpc1` membership anchors and
 * the medium clue proc. `.npc` sections are read with `parseNpcSection`, so a missing
 * section throws; a section without `name=` throws here. Clue and key aliases join
 * `pack/obj.pack` and the decoded item table through `trailJoin`. Identity names come
 * from the `.npc` configs and are corroborated against the decoded `npc.dat` rows by
 * `assertTalkKeyNpcJoins` in the writer.
 */
export function extractTalkKeyFacts(content: string, items: ObjType[]): TalkKeyExtract {
    const itemIds = new Map(items.filter((item) => item.debugname !== null).map((item) => [item.debugname as string, { id: item.id, name: item.name }]));
    const objPack = parsePack(requireGatherText(content, 'pack/obj.pack'));
    if (objPack.size === 0) throw new Error('pack/obj.pack: required file missing ids');
    const npcPack = parsePack(requireGatherText(content, TALK_KEY_NPC_PACK_RELATIVE));
    if (npcPack.size === 0) throw new Error(`${TALK_KEY_NPC_PACK_RELATIVE}: required file missing ids`);
    const membership = new Set(TRAIL_TIERS.flatMap((tier) => parseTrailEnumAliases(requireGatherText(content, `${TRAIL_CONFIG_DIR}/trail_${tier}.enum`))));
    const scripts = contentTreeInputs(content, TALK_KEY_SCRIPTS_DIRECTORY, TALK_KEY_RS2_SUFFIX);
    const npcConfigs = contentTreeInputs(content, TALK_KEY_SCRIPTS_DIRECTORY, TALK_KEY_NPC_SUFFIX);
    const configText = new Map<string, string>();
    const configSection = (relative: string, alias: string, label: string) => {
        const text = configText.get(relative) ?? fs.readFileSync(path.join(content, relative), 'utf8');
        configText.set(relative, text);
        try {
            return parseNpcSection(text, alias);
        } catch (error) {
            throw new Error(`${label}: ${(error as Error).message}`);
        }
    };
    const configOwners = new Map<string, string>();
    for (const relative of npcConfigs.files) {
        const text = fs.readFileSync(path.join(content, relative), 'utf8');
        configText.set(relative, text);
        for (const raw of text.split(/\r?\n/)) {
            const line = raw.trim();
            if (!line.startsWith('[') || !line.endsWith(']')) continue;
            const alias = line.slice(1, -1);
            if (configOwners.has(alias)) throw new Error(`npc config: [${alias}] is declared by more than one .npc file`);
            configOwners.set(alias, relative);
        }
    }
    const identity = (alias: string, label: string) => {
        const id = npcPack.get(alias);
        if (id === undefined) throw new Error(`${label}: pack/npc.pack lacks ${alias}`);
        const relative = configOwners.get(alias);
        if (relative === undefined) throw new Error(`${label}: missing [${alias}] in the scanned npc configs`);
        const name = configSection(relative, alias, label).name;
        if (!name) throw new Error(`${label}: [${alias}] has no name`);
        return { alias, id, name };
    };
    const maps = placementMapInputs(content);
    const spawns = new Map<number, TalkKeySpawn[]>();
    for (const input of maps) {
        const { mx, mz } = parseMapsquarePath(input.path);
        for (const placement of parseJm2NpcPlacements(fs.readFileSync(path.join(content, input.path), 'utf8'))) {
            const world = worldFromMapsquare(mx, mz, placement.lx, placement.lz, placement.plane);
            const found = spawns.get(placement.npc_id);
            if (found) found.push(world);
            else spawns.set(placement.npc_id, [world]);
        }
    }
    const talk: TalkKeyTalkRow[] = [];
    const keys: TalkKeyKeyRow[] = [];
    const coverage: TalkKeyCoverageRow[] = [];
    const uniqueSpawn = (npcId: number, clue: string, family: 'talk' | 'keys') => {
        const found = spawns.get(npcId) ?? [];
        if (found.length === 1) return found[0];
        if (found.length === 0) throw new Error(`talk_key: ${clue} npc ${npcId} has no jm2 NPC spawn`);
        coverage.push({ class: 'unknown', family, alias: clue, reason: 'non-unique jm2 NPC spawn' });
        return null;
    };
    for (const relative of scripts.files) {
        for (const handler of parseTalkKeyHandlers(fs.readFileSync(path.join(content, relative), 'utf8'), membership)) {
            if (talk.some((row) => row.alias === handler.clue)) throw new Error(`talk_key: ${handler.clue} is anchored by more than one opnpc1 handler`);
            const npc = identity(handler.alias, `talk_key ${handler.clue}`);
            const joined = trailJoin(objPack, itemIds, handler.clue, `talk_key ${handler.clue}`);
            const spawn = uniqueSpawn(npc.id, handler.clue, 'talk');
            talk.push({ alias: joined.alias, id: joined.id, npc, ...(spawn ? { spawn } : {}) });
        }
    }
    for (const arm of parseTalkKeyKeeperArms(requireGatherText(content, TALK_KEY_MEDIUM_RELATIVE))) {
        if (!membership.has(arm.clue)) throw new Error(`talk_key: keeper clue ${arm.clue} is not a membership alias`);
        const joined = trailJoin(objPack, itemIds, arm.clue, `talk_key ${arm.clue}`);
        const key = trailJoin(objPack, itemIds, arm.key, `talk_key ${arm.clue} key`);
        let keeper: TalkKeyKeeper;
        let spawn: TalkKeySpawn | null = null;
        if (arm.keeper.kind === 'type') {
            const npc = identity(arm.keeper.alias, `talk_key ${arm.clue} keeper`);
            keeper = { kind: 'type', alias: npc.alias, id: npc.id, name: npc.name };
            spawn = uniqueSpawn(npc.id, arm.clue, 'keys');
        } else if (arm.keeper.kind === 'category') {
            keeper = { kind: 'category', category: arm.keeper.category };
            coverage.push({ class: 'unknown', family: 'keys', alias: arm.clue, reason: 'keeper is a category, not one packed npc id' });
        } else {
            keeper = { kind: 'name', name: arm.keeper.name };
            coverage.push({ class: 'unknown', family: 'keys', alias: arm.clue, reason: 'keeper is a name match, not one packed npc id' });
        }
        keys.push({ alias: joined.alias, id: joined.id, key_alias: key.alias, key_id: key.id, keeper, ...(spawn ? { spawn } : {}) });
    }
    talk.sort((a, b) => a.alias.localeCompare(b.alias));
    keys.sort((a, b) => a.alias.localeCompare(b.alias));
    coverage.sort((a, b) => a.family.localeCompare(b.family) || a.alias.localeCompare(b.alias));
    return {
        facts: { talk, keys, coverage },
        inputs: {
            maps_directory: PLACEMENT_MAPS_DIRECTORY,
            maps: fileInventoryDigest(maps),
            npc_pack: { path: TALK_KEY_NPC_PACK_RELATIVE, ...sha256(path.join(content, TALK_KEY_NPC_PACK_RELATIVE)) },
            scripts: scripts.digest,
            npc_configs: npcConfigs.digest,
            trail_clue_medium: { path: TALK_KEY_MEDIUM_RELATIVE, ...sha256(path.join(content, TALK_KEY_MEDIUM_RELATIVE)) },
        },
    };
}

/**
 * Corroborate every published npc identity against the decoded `npc.dat` rows: the
 * packed alias, the packed id, and the `.npc` display name must all agree with the
 * decoder. A disagreement throws instead of being published under the selected name.
 */
export function assertTalkKeyNpcJoins(facts: TalkKeyFacts, npcs: NpcType[]) {
    const decoded = new Map(npcs.filter((npc) => npc.debugname != null).map((npc) => [npc.debugname as string, npc]));
    const check = (alias: string, id: number, name: string, label: string) => {
        const npc = decoded.get(alias);
        if (!npc) throw new Error(`${label}: npc.dat lacks ${alias}`);
        if (npc.id !== id) throw new Error(`${label}: npc.pack ${alias}=${id} disagrees with decoded npc id ${npc.id}`);
        if (npc.name !== name) throw new Error(`${label}: ${alias} display name ${name} disagrees with decoded npc name ${npc.name}`);
    };
    for (const row of facts.talk) check(row.npc.alias, row.npc.id, row.npc.name, row.alias);
    for (const row of facts.keys) if (row.keeper.kind === 'type') check(row.keeper.alias, row.keeper.id, row.keeper.name, row.alias);
}

/** Fail-closed pins for the talk_key publication. The counts are never copied from the corpus. */
export function assertTalkKeyPins(facts: TalkKeyFacts, revision: number) {
    const talk = (alias: string) => {
        const found = facts.talk.find((row) => row.alias === alias);
        if (!found) throw new Error(`${revision}: talk_key is missing talk ${alias}`);
        return found;
    };
    const key = (alias: string) => {
        const found = facts.keys.find((row) => row.alias === alias);
        if (!found) throw new Error(`${revision}: talk_key is missing key ${alias}`);
        return found;
    };
    const spawned = facts.talk.filter((row) => row.spawn !== undefined);
    if (facts.talk.length !== 47 || spawned.length !== 42) throw new Error(`${revision}: talk_key talk must be 47 membership steps with 42 unique spawns, got ${facts.talk.length}/${spawned.length}`);
    if (facts.keys.length !== 7 || facts.keys.filter((row) => row.spawn !== undefined).length !== 2) throw new Error(`${revision}: talk_key keys must be 7 keepers with 2 unique spawns, got ${facts.keys.length}`);
    if (facts.coverage.length !== 10 || facts.coverage.filter((row) => row.family === 'talk').length !== 5 || facts.coverage.filter((row) => row.family === 'keys').length !== 5) throw new Error(`${revision}: talk_key coverage must be the ten unknown spawns, got ${facts.coverage.length}`);
    if (facts.coverage.some((row) => row.class !== 'unknown' || row.reason.length === 0)) throw new Error(`${revision}: talk_key coverage rows must be named unknown with a reason`);
    const coverage = facts.coverage.map((row) => `${row.family}:${row.alias}`).sort();
    const noSpawn = [
        ...facts.talk.filter((row) => row.spawn === undefined).map((row) => `talk:${row.alias}`),
        ...facts.keys.filter((row) => row.spawn === undefined).map((row) => `keys:${row.alias}`),
    ].sort();
    if (JSON.stringify(coverage) !== JSON.stringify(noSpawn)) throw new Error(`${revision}: talk_key coverage must be exactly the steps without a unique spawn`);
    const hazelmere = talk('trail_clue_medium_anagram001');
    if (hazelmere.npc.alias !== 'grandtree_hazelmere' || hazelmere.npc.id !== 669 || hazelmere.npc.name !== 'Hazelmere' || JSON.stringify(hazelmere.spawn) !== JSON.stringify({ x: 2678, z: 3086, plane: 1 })) throw new Error(`${revision}: talk_key anagram001 must stay Hazelmere 669 at 2678,3086,1`);
    const hans = facts.talk.filter((row) => row.npc.id === 0);
    if (hans.length !== 2 || hans.some((row) => row.npc.name !== 'Hans' || JSON.stringify(row.spawn) !== JSON.stringify({ x: 3207, z: 3233, plane: 0 }))) throw new Error(`${revision}: talk_key Hans must stay two steps on one npc and one spawn`);
    const tobias = talk('trail_clue_easy_vague012');
    if (tobias.npc.alias !== 'captain_tobias' || tobias.npc.id !== 376 || tobias.spawn === undefined) throw new Error(`${revision}: talk_key vague012 must stay the inner captain_tobias identity, not the _sailor category`);
    for (const alias of ['trail_clue_easy_simple008', 'trail_clue_hard_riddle019', 'trail_clue_hard_riddle021', 'trail_clue_hard_riddle026', 'trail_clue_medium_anagram003']) {
        if (talk(alias).spawn !== undefined) throw new Error(`${revision}: talk_key ${alias} is a multi-spawn step and must omit the spawn`);
    }
    const blackHeather = key('trail_clue_medium_riddle001');
    if (blackHeather.keeper.kind !== 'type' || blackHeather.keeper.alias !== 'black_heather' || blackHeather.keeper.id !== 202 || JSON.stringify(blackHeather.spawn) !== JSON.stringify({ x: 3039, z: 3700, plane: 0 })) throw new Error(`${revision}: talk_key riddle001 must stay the black_heather type keeper at 3039,3700,0`);
    const penda = key('trail_clue_medium_riddle008');
    if (penda.keeper.kind !== 'type' || penda.keeper.alias !== 'death_man_indoors2' || penda.keeper.id !== 1087 || JSON.stringify(penda.spawn) !== JSON.stringify({ x: 2910, z: 3539, plane: 0 })) throw new Error(`${revision}: talk_key riddle008 must stay the death_man_indoors2 type keeper at 2910,3539,0`);
    const chicken = key('trail_clue_medium_riddle004');
    if (chicken.keeper.kind !== 'category' || chicken.keeper.category !== 'chicken' || 'alias' in chicken.keeper || 'id' in chicken.keeper) throw new Error(`${revision}: talk_key riddle004 must stay a category keeper with no invented npc id`);
    const man = key('trail_clue_medium_riddle005');
    if (man.keeper.kind !== 'name' || man.keeper.name !== 'Man' || 'alias' in man.keeper || 'id' in man.keeper) throw new Error(`${revision}: talk_key riddle005 must stay a name keeper with no invented npc id`);
    const pirate = key('trail_clue_medium_riddle007');
    if (pirate.keeper.kind !== 'category' || pirate.keeper.category !== 'pirate') throw new Error(`${revision}: talk_key riddle007 must stay a category keeper`);
    const guarddog = key('trail_clue_medium_riddle002');
    const guard = key('trail_clue_medium_riddle003');
    if (guarddog.keeper.kind !== 'type' || guarddog.spawn !== undefined || guard.keeper.kind !== 'type' || guard.spawn !== undefined) throw new Error(`${revision}: talk_key guard keepers are non-unique spawns and must omit the spawn`);
    const blob = JSON.stringify(facts);
    for (const banned of ['TALK_ANCHORS', 'KILL_ANCHORS', 'RIDDLE_KEY_COORDS', 'HARD_SPECIAL_COORDS']) {
        if (blob.includes(banned)) throw new Error(`${revision}: talk_key published ${banned}`);
    }
}

/* ------------------------------------------------------------------------- *
 * trio_givers: the three selected givers behind the coordinate-tool trio
 *
 * A sibling family of `talk_key`, never a field on a membership row. The
 * identity set is closed: one exact handler path per alias, joined to
 * `pack/npc.pack`, the owning `.npc` name, and the unique jm2 `==== NPC ====`
 * world tile. A lookalike alias (`observatory_professor2`, the `murphy_*`
 * variants, the quest_cog labels) is never published: a display name is not an
 * identity. Two or more spawn hits keep the row and record coverage; zero hits
 * throw for a packed identity, because zero is not an unknown tile.
 * ------------------------------------------------------------------------- */

const TRIO_GIVERS_NPC_PACK_RELATIVE = 'pack/npc.pack';
const TRIO_GIVERS_FAMILY = 'trio_givers';

/**
 * The closed giver set: one alias, the exact script that declares its `opnpc1`
 * header, and the `.npc` config that owns its display name. Required content
 * lists the three script names; that listing is inventory evidence, not this join.
 */
const TRIO_GIVERS: { alias: string; handler: string; config: string }[] = [
    { alias: 'observatory_professor', handler: 'scripts/quests/quest_itgronigen/scripts/observatory_professor.rs2', config: 'scripts/quests/quest_itgronigen/configs/quest_itgronigen.npc' },
    { alias: 'murphy', handler: 'scripts/minigames/game_trawler/scripts/murphy.rs2', config: 'scripts/minigames/game_trawler/configs/trawler.npc' },
    { alias: 'brother_kojo', handler: 'scripts/areas/area_ardougne_east/scripts/brother_kojo.rs2', config: 'scripts/areas/area_ardougne_east/configs/ardougne_east.npc' },
];

/** One jm2 `==== NPC ====` spawn in world coordinates. Plane, never scene `level`. */
export type TrioGiverSpawn = { x: number; z: number; plane: number };

/** One selected giver: the packed NPC identity itself, plus its unique world tile. */
export type TrioGiverRow = { alias: string; id: number; name: string; spawn?: TrioGiverSpawn };

/** Why one giver publishes no spawn. A coverage record is a sibling, not a row. */
export type TrioGiverCoverageRow = { class: string; family: string; alias: string; reason: string };

/** The three selected givers. A giver without a unique spawn keeps its row and omits `spawn`. */
export type TrioGiverFacts = { rows: TrioGiverRow[]; coverage: TrioGiverCoverageRow[] };

/**
 * Provenance identity for this family: the closed handler and config set, the
 * pack its ids come from, and the maps inventory its tiles come from.
 */
export type TrioGiverInputs = {
    maps_directory: string;
    maps: { files: number; bytes: number; sha256: string };
    npc_pack: { path: string; bytes: number; sha256: string };
    handlers: { path: string; bytes: number; sha256: string }[];
    npc_configs: { path: string; bytes: number; sha256: string }[];
};

export type TrioGiverExtract = { facts: TrioGiverFacts; inputs: TrioGiverInputs };

/**
 * The `opnpc1` header aliases one exact giver script declares, in file order.
 * `opnpc2`, `opnpc3`, `opnpcu`, and `apnpc*` are never read, and an inner
 * `npc_type` is not an identity: the header alias is. A malformed header throws
 * instead of being skipped.
 */
export function parseTrioGiverHandlers(text: string) {
    const aliases: string[] = [];
    for (const raw of text.split(/\r?\n/)) {
        const line = raw.trim();
        if (!line.startsWith('[') || !line.endsWith(']')) continue;
        const parts = line.slice(1, -1).split(',').map((part) => part.trim());
        if (parts[0] !== 'opnpc1') continue;
        if (parts.length !== 2 || !parts[1]) throw new Error(`trio_givers: malformed opnpc1 header ${line}`);
        aliases.push(parts[1]);
    }
    return aliases;
}

/**
 * The three selected givers, from the closed handler set. Each exact script must
 * declare its `[opnpc1,<alias>]` header, and the alias joins `pack/npc.pack` and
 * the section `parseNpcSection` reads out of the owning `.npc` config, so a
 * missing section or a section without `name=` throws. The spawn is the unique
 * jm2 `==== NPC ====` world tile.
 */
export function extractTrioGiversFacts(content: string): TrioGiverExtract {
    const npcPack = parsePack(requireGatherText(content, TRIO_GIVERS_NPC_PACK_RELATIVE));
    if (npcPack.size === 0) throw new Error(`${TRIO_GIVERS_NPC_PACK_RELATIVE}: required file missing ids`);
    const maps = placementMapInputs(content);
    const spawns = new Map<number, TrioGiverSpawn[]>();
    for (const input of maps) {
        const { mx, mz } = parseMapsquarePath(input.path);
        for (const placement of parseJm2NpcPlacements(fs.readFileSync(path.join(content, input.path), 'utf8'))) {
            const world = worldFromMapsquare(mx, mz, placement.lx, placement.lz, placement.plane);
            const found = spawns.get(placement.npc_id);
            if (found) found.push(world);
            else spawns.set(placement.npc_id, [world]);
        }
    }
    const rows: TrioGiverRow[] = [];
    const coverage: TrioGiverCoverageRow[] = [];
    const handlers: { path: string; bytes: number; sha256: string }[] = [];
    const npcConfigs: { path: string; bytes: number; sha256: string }[] = [];
    for (const giver of TRIO_GIVERS) {
        const aliases = parseTrioGiverHandlers(requireGatherText(content, giver.handler));
        if (!aliases.includes(giver.alias)) throw new Error(`trio_givers: ${giver.handler} does not declare [opnpc1,${giver.alias}]`);
        const crossed = aliases.find((alias) => alias !== giver.alias && TRIO_GIVERS.some((entry) => entry.alias === alias));
        if (crossed) throw new Error(`trio_givers: ${giver.handler} also declares [opnpc1,${crossed}]`);
        const id = npcPack.get(giver.alias);
        if (id === undefined) throw new Error(`trio_givers: ${TRIO_GIVERS_NPC_PACK_RELATIVE} lacks ${giver.alias}`);
        let name: string | undefined;
        try {
            name = parseNpcSection(requireGatherText(content, giver.config), giver.alias).name;
        } catch (error) {
            throw new Error(`trio_givers ${giver.alias}: ${(error as Error).message}`);
        }
        if (!name) throw new Error(`trio_givers: [${giver.alias}] has no name`);
        const found = spawns.get(id) ?? [];
        if (found.length === 0) throw new Error(`trio_givers: ${giver.alias} npc ${id} has no jm2 NPC spawn`);
        if (found.length > 1) coverage.push({ class: 'unknown', family: TRIO_GIVERS_FAMILY, alias: giver.alias, reason: 'non-unique jm2 NPC spawn' });
        rows.push({ alias: giver.alias, id, name, ...(found.length === 1 ? { spawn: found[0] } : {}) });
        handlers.push({ path: giver.handler, ...sha256(path.join(content, giver.handler)) });
        npcConfigs.push({ path: giver.config, ...sha256(path.join(content, giver.config)) });
    }
    return {
        facts: { rows, coverage },
        inputs: {
            maps_directory: PLACEMENT_MAPS_DIRECTORY,
            maps: fileInventoryDigest(maps),
            npc_pack: { path: TRIO_GIVERS_NPC_PACK_RELATIVE, ...sha256(path.join(content, TRIO_GIVERS_NPC_PACK_RELATIVE)) },
            handlers,
            npc_configs: npcConfigs,
        },
    };
}

/**
 * Corroborate every published giver against the decoded `npc.dat` rows: the
 * packed alias, the packed id, and the `.npc` display name must all agree with
 * the decoder. A disagreement throws instead of being published under the
 * selected name.
 */
export function assertTrioGiverNpcJoins(facts: TrioGiverFacts, npcs: NpcType[]) {
    const decoded = new Map(npcs.filter((npc) => npc.debugname != null).map((npc) => [npc.debugname as string, npc]));
    for (const row of facts.rows) {
        const npc = decoded.get(row.alias);
        if (!npc) throw new Error(`trio_givers: npc.dat lacks ${row.alias}`);
        if (npc.id !== row.id) throw new Error(`trio_givers: ${TRIO_GIVERS_NPC_PACK_RELATIVE} ${row.alias}=${row.id} disagrees with decoded npc id ${npc.id}`);
        if (npc.name !== row.name) throw new Error(`trio_givers: ${row.alias} display name ${row.name} disagrees with decoded npc name ${npc.name}`);
    }
}

/** Fail-closed pins for the trio_givers publication. The rows are never copied from the corpus. */
export function assertTrioGiverPins(facts: TrioGiverFacts, revision: number) {
    const row = (alias: string) => {
        const found = facts.rows.find((entry) => entry.alias === alias);
        if (!found) throw new Error(`${revision}: trio_givers is missing ${alias}`);
        return found;
    };
    const identity = [
        ['observatory_professor', 488, 'Observatory professor'],
        ['murphy', 463, 'Murphy'],
        ['brother_kojo', 223, 'Brother Kojo'],
    ] as const;
    const published = facts.rows.map((entry) => [entry.alias, entry.id, entry.name]);
    if (JSON.stringify(published) !== JSON.stringify(identity.map((entry) => [...entry]))) throw new Error(`${revision}: trio_givers must stay the three closed givers, got ${JSON.stringify(published)}`);
    const spawned = facts.rows.filter((entry) => entry.spawn !== undefined);
    if (spawned.length !== 3 || facts.coverage.length !== 0) throw new Error(`${revision}: trio_givers must publish three unique jm2 spawns, got ${spawned.length} spawns and ${facts.coverage.length} coverage rows`);
    const jm2 = [
        ['observatory_professor', { x: 2438, z: 3186, plane: 0 }],
        ['murphy', { x: 2668, z: 3162, plane: 0 }],
        ['brother_kojo', { x: 2569, z: 3249, plane: 0 }],
    ] as const;
    for (const [alias, spawn] of jm2) {
        if (JSON.stringify(row(alias).spawn) !== JSON.stringify(spawn)) throw new Error(`${revision}: trio_givers ${alias} must stay the selected jm2 tile ${JSON.stringify(spawn)}, got ${JSON.stringify(row(alias).spawn)}`);
    }
    const blob = JSON.stringify(facts);
    for (const banned of ['TALK_ANCHORS', 'KILL_ANCHORS', 'RIDDLE_KEY_COORDS', 'HARD_SPECIAL_COORDS', 'frozen', 'invented', 'family-unavailable']) {
        if (blob.includes(banned)) throw new Error(`${revision}: trio_givers published ${banned}`);
    }
}

export function extractFacts(content: string, items: ObjType[], npcs: NpcType[]) {
    const itemIds = new Map(items.filter((item) => item.debugname !== null).map((item) => [item.debugname as string, { id: item.id, name: item.name }]));
    const npcIds = new Map(npcs.filter((npc) => npc.debugname != null).map((npc) => [npc.debugname as string, { id: npc.id, name: npc.name }]));
    const consumption: any[] = [];
    for (const file of ['consume_normal.dbrow', 'consume_effects.dbrow']) for (const parsed of parseRows(fs.readFileSync(path.join(content, 'scripts/player/configs/consumption', file), 'utf8'))) {
        const changes = parsed.values.stat_change ?? []; const heals = parsed.values.stat_heal ?? []; const energy = parsed.values.healenergy ?? [];
        for (const alias of parsed.values.consumable ?? []) { const item = itemIds.get(alias[0]); if (!item) throw new Error(`consumption ${file}/${parsed.name}: unknown item ${alias[0]}`); const statChange = changes.map((v) => ({ stat: v[0], base: integer(v[1], parsed.name), percent: integer(v[2], parsed.name) })); const statHeal = heals.map((v) => ({ stat: v[0], base: integer(v[1], parsed.name), percent: integer(v[2], parsed.name) })); const fixedHp = file === 'consume_normal.dbrow' && statChange.length === 0 && statHeal.length === 1 && statHeal[0].stat === 'hitpoints' && statHeal[0].percent === 0 && statHeal[0].base > 0 && energy.length === 0; consumption.push({ action: 'consume', item: { alias: alias[0], id: item.id, name: item.name }, source_row: parsed.name, source_file: file, stat_change: statChange, stat_heal: statHeal, heal_energy: energy.map((v) => integer(v[0], parsed.name)), qualification: fixedHp ? 'fixed_hp_heal' : 'not_fixed_hp_heal' }); }
    }
    const pickpocket: any[] = [];
    for (const parsed of parseRows(fs.readFileSync(path.join(content, 'scripts/skill_thieving/configs/pickpocking/pickpocket.dbrow'), 'utf8'))) { const npc = (parsed.values.npc ?? []).map((v) => { const found = npcIds.get(v[0]); if (!found) throw new Error(`pickpocket ${parsed.name}: unknown NPC ${v[0]}`); return { alias: v[0], id: found.id, name: found.name }; }); const chance = parsed.values.success_chance?.[0]; if (!chance || chance.length !== 2) throw new Error(`${parsed.name}: malformed success chance`); const loot = (parsed.values.loot ?? []).map((v) => { const item = itemIds.get(v[0]); if (!item) throw new Error(`pickpocket ${parsed.name}: unknown loot ${v[0]}`); return { item: { alias: v[0], id: item.id, name: item.name }, min: integer(v[1], parsed.name), max: integer(v[2], parsed.name), weight: integer(v[3], parsed.name) }; }); pickpocket.push({ group: parsed.name, npcs: npc, level: integer(required(parsed.values, 'level', parsed.name), parsed.name), experience: integer(required(parsed.values, 'experience', parsed.name), parsed.name), stun_ticks: integer(required(parsed.values, 'stun_ticks', parsed.name), parsed.name), stun_damage: integer(required(parsed.values, 'stun_damage', parsed.name), parsed.name), success_chance: { numerator: integer(chance[0], parsed.name), denominator: integer(chance[1], parsed.name) }, loot, pocket: required(parsed.values, 'pocket', parsed.name) }); }
    return { consumption, pickpocket };
}
type CatalogBank = {
    name: string; tile: { x: number; z: number; level: number };
    approach?: { x: number; z: number; level: number };
    requires?: { skill?: { name: string; level: number }; quest?: string; setting?: string };
    access?: { name: string; op: string; openFirst?: { name: string; op: string } };
    npcAccess?: { name: string; op: string; choose?: string };
};

// The runtime-selected engine owns TypeScript. Only fields guarded by its
// is* predicates are read; no foreign module is executed.
type BankAstNode = {
    text: string; name: BankAstNode; initializer: BankAstNode;
    elements: BankAstNode[]; properties: BankAstNode[]; arguments: BankAstNode[];
    expression: BankAstNode; getText(source: unknown): string;
};
type BankLiteral = string | number | BankLiteral[] | { [key: string]: BankLiteral };

function catalogBank(value: BankLiteral): CatalogBank {
    const record = (input: BankLiteral): { [key: string]: BankLiteral } => {
        if (input === null || typeof input !== 'object' || Array.isArray(input)) throw new Error('bank catalog: expected object');
        return input;
    };
    const string = (input: BankLiteral) => {
        if (typeof input !== 'string') throw new Error('bank catalog: expected string');
        return input;
    };
    const integer = (input: BankLiteral) => {
        if (typeof input !== 'number' || !Number.isInteger(input)) throw new Error('bank catalog: expected integer');
        return input;
    };
    const tile = (input: BankLiteral) => {
        const row = record(input);
        return { x: integer(row.x), z: integer(row.z), level: integer(row.level) };
    };
    const operation = (input: BankLiteral) => {
        const row = record(input);
        return { name: string(row.name), op: string(row.op) };
    };
    const row = record(value);
    const bank: CatalogBank = { name: string(row.name), tile: tile(row.tile) };
    if (row.approach !== undefined) bank.approach = tile(row.approach);
    if (row.requires !== undefined) {
        const req = record(row.requires);
        bank.requires = {};
        if (req.quest !== undefined) bank.requires.quest = string(req.quest);
        if (req.setting !== undefined) bank.requires.setting = string(req.setting);
        if (req.skill !== undefined) {
            const skill = record(req.skill);
            bank.requires.skill = { name: string(skill.name), level: integer(skill.level) };
        }
    }
    if (row.access !== undefined) {
        const access = record(row.access);
        bank.access = operation(row.access);
        if (access.openFirst !== undefined) bank.access.openFirst = operation(access.openFirst);
    }
    if (row.npcAccess !== undefined) {
        const npc = record(row.npcAccess);
        bank.npcAccess = operation(row.npcAccess);
        if (npc.choose !== undefined) bank.npcAccess.choose = string(npc.choose);
    }
    return bank;
}

/**
 * Named top-level literals of one pinned rs2b0t source, read from its AST:
 * strings, numbers, arrays, object literals, `new Tile(x, z, level)` and
 * references to other top-level constants. Nothing is evaluated.
 */
async function pinnedLiterals(engine: string, fileName: string, source: string, label: string) {
    // Engine path is selected per revision; a static import would bind the wrong installation.
    const ts = await import(pathToFileURL(path.join(engine, 'node_modules/typescript/lib/typescript.js')).href);
    const ast = ts.createSourceFile(fileName, source, ts.ScriptTarget.Latest, true);
    const constants = new Map<string, BankAstNode>();
    for (const statement of ast.statements) {
        if (ts.isVariableStatement(statement)) {
            for (const declaration of statement.declarationList.declarations) {
                if (ts.isIdentifier(declaration.name)) constants.set(declaration.name.text, declaration.initializer);
            }
        }
    }
    function literal(node: BankAstNode | undefined): BankLiteral {
        if (!node) throw new Error(`${label}: missing literal`);
        if (ts.isStringLiteral(node)) return node.text;
        if (ts.isNumericLiteral(node)) return Number(node.text);
        if (ts.isIdentifier(node) && constants.has(node.text)) return literal(constants.get(node.text));
        if (ts.isArrayLiteralExpression(node)) return node.elements.map(literal);
        if (ts.isObjectLiteralExpression(node)) {
            return Object.fromEntries(node.properties.map((property: BankAstNode) => {
                if (!ts.isPropertyAssignment(property)) throw new Error(`${label}: nonliteral property`);
                return [property.name.text, literal(property.initializer)];
            }));
        }
        if (ts.isNewExpression(node) && node.expression.getText(ast) === 'Tile') {
            const [x, z, level] = node.arguments.map(literal);
            if (![x, z, level].every(value => typeof value === 'number' && Number.isInteger(value))) throw new Error(`${label}: invalid tile`);
            return { x, z, level };
        }
        throw new Error(`${label}: unsupported AST ${node.getText(ast)}`);
    }
    return (name: string) => literal(constants.get(name));
}

/** Read the pinned literal AST, not a second curated copy or evaluated foreign module. */
export async function extractBankCatalog(engine: string, source: string) {
    const literal = await pinnedLiterals(engine, 'BankLocations.ts', source, 'bank catalog');
    const value = literal('BANK_LOCATIONS');
    if (!Array.isArray(value)) throw new Error('bank catalog: expected array');
    const rows = value.map(catalogBank);
    if (!rows.length || new Set(rows.map(row => row.name)).size !== rows.length) throw new Error('bank catalog: empty or duplicate names');
    return rows;
}

type CookTile = { x: number; z: number; level: number };
type CookKind = 'oven' | 'fire';
/** A frozen curated bank cook surface (`curatedPlan`, `data/cookLocations.ts:98-116`). */
type CookCamp = { bank: string; stand: CookTile; approach?: CookTile; loc: CookTile; locName: string; kind: CookKind; label: string };
export type CookCatalog = { camps: CookCamp[]; kinds: Record<string, CookKind>; maxSurfaceCheb: number; derivedArriveRadius: number; obstacles: string[] };

/**
 * The frozen cook pairing inputs, read from the pinned AST: the curated bank
 * surfaces (`CURATED_CAMP` → `FISH_CAMP_COOK_PLANS[camp].bank ?? .pier`,
 * `data/cookLocations.ts:92-116`, `data/cookingRanges.ts:98-181`), the
 * surface loc kinds (`tools/cooking/gen-cooksurfaces.ts` `COOK_SURFACE_KINDS`)
 * and the pairing constants (`MAX_SURFACE_CHEB`, `DERIVED_ARRIVE_RADIUS`,
 * `DEFAULT_OBSTACLES`).
 */
export async function extractCookCatalog(engine: string, sources: { cookLocations: string; cookingRanges: string; genCookSurfaces: string }): Promise<CookCatalog> {
    const label = 'cook catalog';
    const record = (input: BankLiteral): { [key: string]: BankLiteral } => {
        if (input === null || typeof input !== 'object' || Array.isArray(input)) throw new Error(`${label}: expected object`);
        return input;
    };
    const string = (input: BankLiteral) => {
        if (typeof input !== 'string') throw new Error(`${label}: expected string`);
        return input;
    };
    const integer = (input: BankLiteral) => {
        if (typeof input !== 'number' || !Number.isInteger(input)) throw new Error(`${label}: expected integer`);
        return input;
    };
    const tile = (input: BankLiteral): CookTile => {
        const row = record(input);
        return { x: integer(row.x), z: integer(row.z), level: integer(row.level) };
    };
    const locations = await pinnedLiterals(engine, 'cookLocations.ts', sources.cookLocations, label);
    const ranges = await pinnedLiterals(engine, 'cookingRanges.ts', sources.cookingRanges, label);
    const generator = await pinnedLiterals(engine, 'gen-cooksurfaces.ts', sources.genCookSurfaces, label);
    const plans = record(ranges('FISH_CAMP_COOK_PLANS'));
    const camps = Object.entries(record(locations('CURATED_CAMP'))).map(([bank, campName]) => {
        // `cookSurfaceForFishCamp(camp, 'bank')`: the bank surface, else the pier one (`cookingRanges.ts:169-181`).
        const plan = record(plans[string(campName)] ?? {});
        const chosen = plan.bank ?? plan.pier;
        if (chosen === undefined) throw new Error(`${label}: no plan for ${bank}`);
        const surface = record(chosen);
        const stand = tile(surface.stand);
        const locName = string(surface.locName);
        const camp: CookCamp = {
            bank,
            stand,
            loc: surface.loc === undefined ? stand : tile(surface.loc),
            locName,
            // `curatedPlan`: a curated `range` is an oven, anything else a fire (`cookLocations.ts:111`).
            kind: string(surface.kind) === 'range' ? 'oven' : 'fire',
            label: surface.label === undefined ? locName : string(surface.label),
        };
        if (surface.approach !== undefined) camp.approach = tile(surface.approach);
        return camp;
    });
    const kinds = Object.fromEntries(Object.entries(record(generator('COOK_SURFACE_KINDS'))).map(([debugname, kind]) => {
        if (kind !== 'oven' && kind !== 'fire') throw new Error(`${label}: unknown kind ${String(kind)}`);
        return [debugname, kind as CookKind];
    }));
    const obstacles = locations('DEFAULT_OBSTACLES');
    if (!Array.isArray(obstacles)) throw new Error(`${label}: expected obstacle array`);
    return {
        camps,
        kinds,
        maxSurfaceCheb: integer(locations('MAX_SURFACE_CHEB')),
        derivedArriveRadius: integer(locations('DERIVED_ARRIVE_RADIUS')),
        obstacles: obstacles.map(string),
    };
}

export function cookCatalogRust(catalog: CookCatalog) {
    const str = (value: string) => JSON.stringify(value);
    const tile = (value: CookTile) => `WorldTile { x: ${value.x}, z: ${value.z}, level: ${value.level} }`;
    const kind = (value: CookKind) => (value === 'oven' ? 'CookSurfaceKind::Oven' : 'CookSurfaceKind::Fire');
    return '// Generated by tools/game-data/generate.ts from pinned rs2b0t cook sources; do not curate.\n'
        + `pub const MAX_SURFACE_CHEB: i32 = ${catalog.maxSurfaceCheb};\n`
        + `pub const DERIVED_ARRIVE_RADIUS: i32 = ${catalog.derivedArriveRadius};\n`
        + `pub const DEFAULT_OBSTACLES: &[&str] = &[${catalog.obstacles.map(str).join(', ')}];\n`
        + 'pub const COOK_CAMPS: &[CookCamp] = &[\n'
        + catalog.camps.map(camp => `    CookCamp { bank: ${str(camp.bank)}, stand: ${tile(camp.stand)}, approach: ${camp.approach ? `Some(${tile(camp.approach)})` : 'None'}, `
            + `loc: ${tile(camp.loc)}, loc_name: ${str(camp.locName)}, kind: ${kind(camp.kind)}, label: ${str(camp.label)} },`).join('\n') + '\n];\n';
}

/** The level-1 MAP tiles whose flags carry LINK_BELOW (0x2), as `lx,lz`. */
export function parseJm2LinkBelow(text: string) {
    const tiles = new Set<string>();
    let inMap = false;
    for (const raw of text.split(/\r?\n/)) {
        const line = raw.trim();
        if (!line) continue;
        const section = jm2SectionName(line);
        if (section !== null) {
            inMap = section === 'MAP';
            continue;
        }
        if (!inMap) continue;
        const colon = line.indexOf(':');
        if (colon <= 0) throw new Error(`jm2 MAP: malformed row ${line}`);
        const [plane, lx, lz] = line.slice(0, colon).trim().split(/\s+/).map((token) => integer(token, line));
        if (plane !== 1) continue;
        const flags = line.slice(colon + 1).trim().split(/\s+/).find((token) => token.startsWith('f'));
        if (flags !== undefined && (integer(flags.slice(1), line) & 0x2) !== 0) tiles.add(`${lx},${lz}`);
    }
    return tiles;
}

/**
 * Every cook surface placed in the selected map pack: LOC placements whose
 * type is one of the frozen surface kinds, named by their loc config, in the
 * frozen generator's order (level, x, z). Each curated surface must be one of
 * them, so a moved Range fails generation instead of pointing at nothing.
 */
export function extractCookSurfaces(content: string, catalog: CookCatalog) {
    const locTree = contentTreeInputs(content, 'scripts', '.loc');
    const maps = placementMapInputs(content);
    const packed = parsePack(fs.readFileSync(path.join(content, 'pack/loc.pack'), 'utf8'));
    const names = new Map<string, string>();
    for (const file of locTree.files) {
        for (const [alias, config] of parseNpcConfigSections(fs.readFileSync(path.join(content, file), 'utf8'))) {
            if (catalog.kinds[alias] !== undefined && config.name !== undefined) names.set(alias, config.name);
        }
    }
    const wanted = new Map<number, { debugname: string; name: string; kind: CookKind }>();
    for (const [alias, id] of packed) {
        const kind = catalog.kinds[alias];
        if (kind !== undefined) wanted.set(id, { debugname: alias, name: names.get(alias) ?? alias, kind });
    }
    if (wanted.size === 0) throw new Error('cook surfaces: no surface loc types in loc.pack');
    const rows: { x: number; z: number; level: number; name: string; debugname: string; kind: CookKind }[] = [];
    const ids = new Set(wanted.keys());
    for (const input of maps) {
        const { mx, mz } = parseMapsquarePath(input.path);
        const text = fs.readFileSync(path.join(content, input.path), 'utf8');
        const linkBelow = parseJm2LinkBelow(text);
        for (const placement of parseJm2LocPlacements(text, ids)) {
            // Frozen `bridgedLevel` (tools/nav/lib.ts:377-380): a LINK_BELOW
            // level-1 tile moves the loc down a plane; the client drops one
            // that would fall below level 0.
            const level = linkBelow.has(`${placement.lx},${placement.lz}`) ? placement.plane - 1 : placement.plane;
            if (level < 0) continue;
            const tile = worldFromMapsquare(mx, mz, placement.lx, placement.lz, level);
            const type = wanted.get(placement.loc_id)!;
            rows.push({ x: tile.x, z: tile.z, level: tile.plane, name: type.name, debugname: type.debugname, kind: type.kind });
        }
    }
    rows.sort((a, b) => a.level - b.level || a.x - b.x || a.z - b.z);
    for (const camp of catalog.camps) {
        if (!rows.some(row => row.x === camp.loc.x && row.z === camp.loc.z && row.level === camp.loc.level && row.name === camp.locName)) {
            throw new Error(`cook surfaces: curated ${camp.bank} ${camp.locName} at ${camp.loc.x},${camp.loc.z},${camp.loc.level} is not in the selected content`);
        }
    }
    return { facts: { rows }, inputs: { loc_configs: locTree.digest, maps: fileInventoryDigest(maps), loc_pack: sourceFile(content, 'pack/loc.pack') } };
}

export function bankCatalogRust(rows: CatalogBank[]) {
    const str = (value: string) => JSON.stringify(value);
    const opt = <T>(value: T | undefined, emit: (value: T) => string) => value === undefined ? 'None' : `Some(${emit(value)})`;
    const tile = (value: CatalogBank['tile']) => `WorldTile { x: ${value.x}, z: ${value.z}, level: ${value.level} }`;
    const object = (value: { name: string; op: string }) => `BankOperation { name: ${str(value.name)}, op: ${str(value.op)} }`;
    return '// Generated by tools/game-data/generate.ts from pinned BankLocations.ts; do not curate.\n'
        + 'pub const BANK_CATALOG: &[BankDefinition] = &[\n'
        + rows.map(row => {
            const skill = row.requires?.skill;
            // The frozen catalog currently has one skill gate; refuse silently assigning another id.
            if (skill && skill.name !== 'fishing') throw new Error(`bank catalog: unknown skill ${skill.name}`);
            return `    BankDefinition { name: ${str(row.name)}, tile: ${tile(row.tile)}, approach: ${opt(row.approach, tile)}, `
                + `skill: ${skill ? `Some((10, ${skill.level}))` : 'None'}, quest: ${opt(row.requires?.quest, str)}, setting: ${opt(row.requires?.setting, str)}, `
                + `object: ${opt(row.access, object)}, open_first: ${opt(row.access?.openFirst, object)}, `
                + `npc: ${opt(row.npcAccess, object)}, choose: ${opt(row.npcAccess?.choose, str)} },`;
        }).join('\n') + '\n];\n';
}

/** Selected content supplies the access identity, footprint, and placement; collision resolves the stand at bind time. */
export function extractBankPlacements(content: string, catalog: CatalogBank[]) {
    const locTree = contentTreeInputs(content, 'scripts', '.loc');
    const npcTree = contentTreeInputs(content, 'scripts', '.npc');
    const files = [...locTree.files, ...npcTree.files];
    const maps = placementMapInputs(content);
    const packedLocs = new Map([...parsePack(fs.readFileSync(path.join(content, 'pack/loc.pack'), 'utf8'))].map(([alias, id]) => [id, alias]));
    const packedNpcs = new Map([...parsePack(fs.readFileSync(path.join(content, 'pack/npc.pack'), 'utf8'))].map(([alias, id]) => [id, alias]));
    const configs = new Map<string, Record<string, string>>();
    for (const file of files) {
        const text = fs.readFileSync(path.join(content, file), 'utf8');
        for (const [alias, config] of parseNpcConfigSections(text)) {
            const key = `${path.extname(file)}:${alias}`;
            if (configs.has(key)) throw new Error(`bank placements: duplicate ${key}`);
            configs.set(key, config);
        }
    }
    const rows: { name: string; kind: string; id: number; x: number; z: number; level: number; width: number; length: number }[] = [];
    const ids = new Set(packedLocs.keys());
    const near = (bank: CatalogBank, tile: { x: number; z: number; plane: number }) =>
        bank.tile.level === tile.plane && Math.max(Math.abs(bank.tile.x - tile.x), Math.abs(bank.tile.z - tile.z)) <= 14;
    const matches = (config: Record<string, string> | undefined, access: { name: string; op: string }) =>
        config?.name === access.name && [1, 2, 3, 4, 5].some(index => config[`op${index}`] === access.op);
    for (const input of maps) {
        const { mx, mz } = parseMapsquarePath(input.path);
        const text = fs.readFileSync(path.join(content, input.path), 'utf8');
        for (const placement of parseJm2LocPlacements(text, ids)) {
            const tile = worldFromMapsquare(mx, mz, placement.lx, placement.lz, placement.plane);
            const config = configs.get(`.loc:${packedLocs.get(placement.loc_id)}`);
            for (const bank of catalog) {
                if (bank.npcAccess || !near(bank, tile)) continue;
                const access = bank.access ?? { name: 'Bank booth', op: 'Use-quickly' };
                if (!matches(config, access) && !(access.openFirst && matches(config, access.openFirst))) continue;
                let width = Number(config?.width ?? 1), length = Number(config?.length ?? 1);
                if (placement.angle & 1) [width, length] = [length, width];
                if (![width, length].every(value => Number.isInteger(value) && value > 0)) throw new Error(`bank placements: invalid footprint ${bank.name}`);
                rows.push({ name: bank.name, kind: 'object', id: placement.loc_id, x: tile.x, z: tile.z, level: tile.plane, width, length });
            }
        }
        for (const placement of parseJm2NpcPlacements(text)) {
            const tile = worldFromMapsquare(mx, mz, placement.lx, placement.lz, placement.plane);
            const config = configs.get(`.npc:${packedNpcs.get(placement.npc_id)}`);
            for (const bank of catalog) {
                if (bank.npcAccess && near(bank, tile) && matches(config, bank.npcAccess)) {
                    const size = Number(config?.size ?? 1);
                    if (!Number.isInteger(size) || size < 1) throw new Error(`bank placements: invalid NPC size ${bank.name}`);
                    rows.push({ name: bank.name, kind: 'npc', id: placement.npc_id, x: tile.x, z: tile.z, level: tile.plane, width: size, length: size });
                }
            }
        }
    }
    rows.sort((a, b) => catalog.findIndex(bank => bank.name === a.name) - catalog.findIndex(bank => bank.name === b.name) || a.x - b.x || a.z - b.z || a.id - b.id);
    const missing = catalog.filter(bank => !rows.some(row => row.name === bank.name)).map(bank => bank.name);
    return { facts: { rows, missing }, inputs: { loc_configs: locTree.digest, npc_configs: npcTree.digest, maps: fileInventoryDigest(maps), loc_pack: sourceFile(content, 'pack/loc.pack'), npc_pack: sourceFile(content, 'pack/npc.pack') } };
}

async function generate(spec: Revision) {
    const pinned = assertPinned(spec);
    verifyCacheIdentity(spec.revision, spec.engine, spec.cacheIdentity);
    for (const requiredInput of ['data/pack/server/obj.dat', 'data/pack/server/npc.dat', 'data/pack/client/config']) if (!fs.existsSync(path.join(spec.engine, requiredInput))) throw new Error(`${spec.revision}: missing ${requiredInput}`);
    process.chdir(spec.engine); const environment = (await import(pathToFileURL(path.join(spec.engine, 'src/util/Environment.ts')).href)) as { default: { node: { members: boolean } } };
    // an F2P world config clears `tradeable` on every members obj at load; the published flag is the content's, so refuse that decode
    if (environment.default.node.members !== true) throw new Error(`${spec.revision}: generate with a members world config (NODE_MEMBERS) so tradeable is the content's own flag`);
    const objModule = (await import(pathToFileURL(path.join(spec.engine, 'src/cache/config/ObjType.ts')).href)) as { default: { load(dir: string): void; configs: ObjType[] } }; objModule.default.load('data/pack');
    const npcModule = (await import(pathToFileURL(path.join(spec.engine, 'src/cache/config/NpcType.ts')).href)) as { default: { load(dir: string): void; configs: NpcType[] } }; npcModule.default.load('data/pack');
    const piles = pileModels(objModule.default.configs); const items = objModule.default.configs.map((obj) => row(obj, piles)); const aliases = items.filter((item) => item.alias !== null).map((item) => item.alias as string); if (new Set(items.map((item) => item.id)).size !== items.length || new Set(aliases).size !== aliases.length) throw new Error(`${spec.revision}: duplicate ids or aliases`);
    const facts = extractFacts(spec.content, objModule.default.configs, npcModule.default.configs); const drops = extractDropFacts(spec.content, objModule.default.configs, npcModule.default.configs); if (drops.length !== 4) throw new Error(`${spec.revision}: expected four combat drop tables, got ${drops.length}`); const magic = extractMagicFacts(spec.content, objModule.default.configs); if (magic.spells.length !== 16 || magic.spells[15].name !== 'Fire Wave' || magic.staves.length !== 14) throw new Error(`${spec.revision}: expected 16 combat spells and 14 staves, got ${magic.spells.length}/${magic.staves.length}`); const herbs = extractHerbFacts(spec.content, objModule.default.configs); if (herbs.herbs.length < 14) throw new Error(`${spec.revision}: expected a full herb identify table, got ${herbs.herbs.length}`); if (herbs.herb_level_default !== 3) throw new Error(`${spec.revision}: expected identify.param default 3, got ${herbs.herb_level_default}`); const autocast = extractAutocastControls(spec.content); const duel = extractDuelControls(spec.content); const special = extractSpecialControls(spec.content, objModule.default.configs);     const teleports = extractTeleportSpells(spec.content, objModule.default.configs); if (teleports.length !== 7 || teleports[0].name !== 'Varrock' || teleports[6].name !== 'Trollheim' || teleports[0].component_id !== 1164 || teleports[6].component_id !== 7455) throw new Error(`${spec.revision}: expected 7 standard teleports, got ${teleports.map((row) => row.name).join(',')}`);     const prayer = extractPrayerFacts(spec.content); if (prayer.prayers.length !== 15) throw new Error(`${spec.revision}: expected 15 prayers, got ${prayer.prayers.length}`); const nurmofEssence = extractNurmofEssenceFacts(spec.content, objModule.default.configs, npcModule.default.configs); if (nurmofEssence.pickaxes.length !== 6) throw new Error(`${spec.revision}: expected six pickaxes, got ${nurmofEssence.pickaxes.length}`);     const flourSix = extractFlourSixFacts(spec.content, objModule.default.configs); if (flourSix.pot.id !== 1931 || flourSix.flour_barrel.id !== 2662) throw new Error(`${spec.revision}: flour six join mismatch`); const objPackPath = path.join(spec.content, 'pack/obj.pack'); if (!fs.existsSync(objPackPath)) throw new Error(`${spec.revision}: missing pack/obj.pack`); const objPack = parsePack(fs.readFileSync(objPackPath, 'utf8')); if (objPack.size === 0) throw new Error(`${spec.revision}: empty pack/obj.pack`); const equipmentNames = extractEquipmentNamesFacts(items, objPack); const gatherMethods = extractGatherMethodsFacts(spec.content, spec.revision); if (gatherMethods.mining.length !== 17 || gatherMethods.woods.length !== 10 || gatherMethods.fishing.length !== 9) throw new Error(`${spec.revision}: expected 17 mine, 10 wood, and 9 fishing rows, got ${gatherMethods.mining.length}/${gatherMethods.woods.length}/${gatherMethods.fishing.length}`); const gatherPlacements = extractGatherPlacementsFacts(spec.content, gatherMethods.woods); if (gatherPlacements.woods.length !== 6) throw new Error(`${spec.revision}: expected six published woods, got ${gatherPlacements.woods.length}`); if (gatherPlacements.facts.coverage.length !== 1 || gatherPlacements.facts.coverage[0].class !== 'unknown' || gatherPlacements.facts.coverage[0].family !== 'mining') throw new Error(`${spec.revision}: gather placements must record mining as unknown coverage`); const questIdentity = extractQuestIdentityFacts(spec.content, spec.revision); if (questIdentity.rows.length !== 6 || questIdentity.rows[4].id !== 'death' || questIdentity.rows[4].varp !== 'death_equiproom' || questIdentity.rows[4].varp_id !== 314 || questIdentity.rows[4].complete !== 80 || questIdentity.rows.some((row) => row.requirements.qualification !== 'partial')) throw new Error(`${spec.revision}: quest identity join mismatch`); if (spec.revision === 274 && (questIdentity.coverage.length !== 1 || questIdentity.coverage[0].alias !== 'routequest' || questIdentity.coverage[0].other_pin_id !== 387 || questIdentity.coverage[0].copied !== false)) throw new Error(`${spec.revision}: quest coverage mismatch`); if (spec.revision !== 274 && questIdentity.coverage.length !== 0) throw new Error(`${spec.revision}: quest coverage must be empty`); const trails = extractTrailFacts(spec.content, objModule.default.configs); assertTrailPins(trails, spec.revision); const talkKey = extractTalkKeyFacts(spec.content, objModule.default.configs); assertTalkKeyPins(talkKey.facts, spec.revision); assertTalkKeyNpcJoins(talkKey.facts, npcModule.default.configs); const trioGivers = extractTrioGiversFacts(spec.content); assertTrioGiverPins(trioGivers.facts, spec.revision); assertTrioGiverNpcJoins(trioGivers.facts, npcModule.default.configs); const inputs = ['data/pack/server/obj.dat', 'data/pack/server/npc.dat', 'data/pack/client/config'].map((file) => sourceFile(spec.engine, file)); const contentInputs = contentFiles.map((file) => sourceFile(spec.content, file)); const sources = decoderSources.map((file) => sourceFile(spec.engine, file));
    const rs2b0tRoot = envPath('RS2B0T', path.join(root, '.superpowers/release-0.1.9/reference/rs2b0t-00d39a17e0'));
    assertRs2b0tPinned(rs2b0tRoot);
    const bankSource = path.join(rs2b0tRoot, 'src/bot/api/bank/BankLocations.ts');
    const bankCatalog = await extractBankCatalog(spec.engine, fs.readFileSync(bankSource, 'utf8'));
    const bankPlacements = extractBankPlacements(spec.content, bankCatalog);
    const bankInputs = { catalog: { path: 'rs2b0t-00d39a17e0/src/bot/api/bank/BankLocations.ts', ...sha256(bankSource) }, ...bankPlacements.inputs };
    fs.writeFileSync(path.join(root, 'crates/api/data/game-data/bank-catalog.rs'), bankCatalogRust(bankCatalog));
    const cookFiles = { cookLocations: 'src/bot/data/cookLocations.ts', cookingRanges: 'src/bot/data/cookingRanges.ts', genCookSurfaces: 'tools/cooking/gen-cooksurfaces.ts' };
    const cookCatalog = await extractCookCatalog(spec.engine, {
        cookLocations: fs.readFileSync(path.join(rs2b0tRoot, cookFiles.cookLocations), 'utf8'),
        cookingRanges: fs.readFileSync(path.join(rs2b0tRoot, cookFiles.cookingRanges), 'utf8'),
        genCookSurfaces: fs.readFileSync(path.join(rs2b0tRoot, cookFiles.genCookSurfaces), 'utf8'),
    });
    const cookSurfaces = extractCookSurfaces(spec.content, cookCatalog);
    const cookInputs = { catalog: Object.values(cookFiles).map((file) => ({ path: `rs2b0t-00d39a17e0/${file}`, ...sha256(path.join(rs2b0tRoot, file)) })), ...cookSurfaces.inputs };
    fs.writeFileSync(path.join(root, 'crates/api/data/game-data/cook-catalog.rs'), cookCatalogRust(cookCatalog));
    const payload = { schema_version: 4, revision: spec.revision, provenance: { engine_commit: pinned.engineCommit, content_commit: pinned.contentCommit, inputs, content_inputs: contentInputs, placement_inputs: gatherPlacements.inputs, talk_key_inputs: talkKey.inputs, trio_givers_inputs: trioGivers.inputs, decoder_sources: sources, cache_identity: spec.cacheIdentity, bank_inputs: bankInputs, cook_inputs: cookInputs }, items, ...facts, drop_tables: drops, ...magic, ...herbs, ...prayer, nurmof_essence: nurmofEssence, flour_six: flourSix, equipment_names: equipmentNames, gather_methods: gatherMethods, gather_placements: gatherPlacements.facts, quest_identity: questIdentity, trails, talk_key: talkKey.facts, trio_givers: trioGivers.facts, autocast, duel, special, teleports, bank_placements: bankPlacements.facts, cook_surfaces: cookSurfaces.facts }; const bytes = `${JSON.stringify(payload, null, 2)}\n`; fs.mkdirSync(path.dirname(spec.output), { recursive: true }); fs.writeFileSync(spec.output, bytes); return { revision: spec.revision, output: path.relative(root, spec.output), records: items.length, consumption: facts.consumption.length, pickpocket: facts.pickpocket.length, drop_tables: drops.length, spells: magic.spells.length, staves: magic.staves.length, herbs: herbs.herbs.length, prayers: prayer.prayers.length, pickaxes: nurmofEssence.pickaxes.length, flour_six: 6, gather_methods: { mining: gatherMethods.mining.length, woods: gatherMethods.woods.length, fishing: gatherMethods.fishing.length }, gather_placements: { rows: gatherPlacements.facts.rows.length, maps: gatherPlacements.inputs.maps.files, published_loc_ids: gatherPlacements.inputs.published_loc_ids.count, coverage: gatherPlacements.facts.coverage.length, woods: gatherPlacements.woods }, equipment_names: { bows: equipmentNames.bows.length, crossbows: equipmentNames.crossbows.length, darts: equipmentNames.darts.length, arrows: equipmentNames.arrows.length, bolts: equipmentNames.bolts.length, melee_weapons: equipmentNames.melee_weapons.length, staffs: equipmentNames.staffs.length, resolved: EQUIPMENT_FAMILY_ORDER.reduce((sum, family) => sum + equipmentNames[family].filter((row) => row.disposition === 'resolved').length, 0), absent: EQUIPMENT_FAMILY_ORDER.reduce((sum, family) => sum + equipmentNames[family].filter((row) => row.disposition === 'absent').length, 0) }, autocast, duel, special: { energy_varp: special.energy_varp, armed_varp: special.armed_varp, max_energy: special.max_energy, bars: special.bars.length, weapons: special.weapons.length }, teleports: teleports.length, quest_identity: { rows: questIdentity.rows.length, coverage: questIdentity.coverage.length }, trails: { rows: trails.rows.length, clues: trails.rows.filter((row) => row.role === 'clue').length, caskets: trails.rows.filter((row) => row.role === 'casket').length, challenge_answers: trails.challenge_answers.length, access_constrained: trails.rows.filter((row) => row.access !== undefined).length }, talk_key: { talk: talkKey.facts.talk.length, talk_with_spawn: talkKey.facts.talk.filter((row) => row.spawn !== undefined).length, keys: talkKey.facts.keys.length, keys_with_spawn: talkKey.facts.keys.filter((row) => row.spawn !== undefined).length, coverage: talkKey.facts.coverage.length, maps: talkKey.inputs.maps.files, scripts: talkKey.inputs.scripts.files, npc_configs: talkKey.inputs.npc_configs.files, digest: crypto.createHash('sha256').update(JSON.stringify(talkKey.facts)).digest('hex') }, trio_givers: { rows: trioGivers.facts.rows.length, with_spawn: trioGivers.facts.rows.filter((row) => row.spawn !== undefined).length, coverage: trioGivers.facts.coverage.length, maps: trioGivers.inputs.maps.files, handlers: trioGivers.inputs.handlers.length, npc_configs: trioGivers.inputs.npc_configs.length, digest: crypto.createHash('sha256').update(JSON.stringify(trioGivers.facts)).digest('hex') }, bytes: Buffer.byteLength(bytes), sha256: crypto.createHash('sha256').update(bytes).digest('hex'), engine_commit: pinned.engineCommit, content_commit: pinned.contentCommit, inputs, content_inputs: contentInputs, decoder_sources: sources, cache_identity: spec.cacheIdentity, bank_inputs: bankInputs, bank_placements: { rows: bankPlacements.facts.rows.length, missing: bankPlacements.facts.missing }, cook_inputs: cookInputs, cook_surfaces: cookSurfaces.facts.rows.length };
}

/**
 * 289 is the selected content and 274 is corroboration: both extracts are parsed
 * independently from their own tree, and the two must agree on every selected
 * identity, name, and unique spawn. Disagreement refuses to publish either.
 */
async function main() {
    const results = [];
    for (const spec of revisions) results.push(await generate(spec));
    const digests = new Map(results.map((result) => [result.revision, result.talk_key.digest]));
    if (new Set(digests.values()).size !== 1) {
        throw new Error(`talk_key: the two pins disagree on the selected identity (${[...digests].map(([revision, digest]) => `${revision}:${digest}`).join(', ')})`);
    }
    const giverDigests = new Map(results.map((result) => [result.revision, result.trio_givers.digest]));
    if (new Set(giverDigests.values()).size !== 1) {
        throw new Error(`trio_givers: the two pins disagree on the selected identity, display name, or unique spawn (${[...giverDigests].map(([revision, digest]) => `${revision}:${digest}`).join(', ')})`);
    }
    const manifest = { schema_version: 4, generator: 'tools/game-data/generate.ts', revisions: results };
    const manifestPath = path.join(root, 'crates/api/data/game-data/manifest.json');
    fs.writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);
    console.log(JSON.stringify({ manifest: path.relative(root, manifestPath), revisions: results }, null, 2));
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main().catch((error) => { console.error(error); process.exitCode = 1; });
