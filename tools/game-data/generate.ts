import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { verifyCacheIdentity } from './cache-identity.ts';

type ObjType = { id: number; debugname: string | null; name: string | null; cost: number; stackable: boolean; members: boolean; certlink: number; certtemplate: number; wearpos: number; wearpos2: number; wearpos3: number; params?: Map<number, number | string> };
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
const contentFiles = ['scripts/player/configs/consumption/consume.dbtable', 'scripts/player/configs/consumption/consume_normal.dbrow', 'scripts/player/configs/consumption/consume_effects.dbrow', 'scripts/skill_thieving/configs/pickpocking/pickpocket.dbtable', 'scripts/skill_thieving/configs/pickpocking/pickpocket.dbrow', 'scripts/player/scripts/consumption/effects/scripts/consume_effects.rs2', 'scripts/skill_combat/configs/magic/magic_combat_spells.dbrow', 'scripts/skill_magic/configs/magic.dbtable', 'scripts/skill_magic/configs/magic_spells.dbrow', 'scripts/skill_magic/configs/magic_staff.dbrow', 'scripts/skill_combat/configs/combat.constant', 'scripts/skill_herblore/configs/herbs.obj', 'scripts/skill_herblore/configs/identifying/identify.param', 'scripts/skill_herblore/scripts/identifying/identify.rs2', ...prayerContentFiles, ...nurmofEssenceContentFiles, ...flourSixContentFiles, 'pack/interface.pack', 'pack/varp.pack', 'pack/param.pack', ...dropContentFiles, ...gatherContentFiles, ...questIdentityContentFiles];
function sha256(file: string) { const data = fs.readFileSync(file); return { bytes: data.length, sha256: crypto.createHash('sha256').update(data).digest('hex') }; }
function commit(dir: string) { return execFileSync('git', ['-C', dir, 'rev-parse', 'HEAD'], { encoding: 'utf8' }).trim(); }
function sourceFile(dir: string, relative: string) { return { path: relative, ...sha256(path.join(dir, relative)) }; }
function assertPinned(spec: Revision) {
    const engineCommit = commit(spec.engine); const contentCommit = commit(spec.content);
    if (engineCommit !== spec.expectedEngine || contentCommit !== spec.expectedContent) throw new Error(`${spec.revision}: expected pinned commits, got ${engineCommit}/${contentCommit}`);
    const engineInputs = [...decoderSources, 'data/pack/server/obj.dat', 'data/pack/server/npc.dat', 'data/pack/client/config'];
    const dirtyEngine = execFileSync('git', ['-C', spec.engine, 'status', '--porcelain', '--untracked-files=all', '--', ...engineInputs], { encoding: 'utf8' }).trim();
    if (dirtyEngine) throw new Error(`${spec.revision}: relevant engine inputs are dirty:\n${dirtyEngine}`);
    const dirtyContent = execFileSync('git', ['-C', spec.content, 'status', '--porcelain', '--untracked-files=all', '--', ...contentFiles], { encoding: 'utf8' }).trim();
    if (dirtyContent) throw new Error(`${spec.revision}: relevant content inputs are dirty:\n${dirtyContent}`);
    return { engineCommit, contentCommit };
}
function row(obj: ObjType) { return { alias: obj.debugname, id: obj.id, name: obj.name, cost: obj.cost, stackable: obj.stackable, members: obj.members, certificate_link: obj.certlink, certificate_template: obj.certtemplate, wear_position: obj.wearpos, wear_position_2: obj.wearpos2, wear_position_3: obj.wearpos3 }; }
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

export function parseNpcSection(text: string, alias: string) {
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
    const section = sections.get(alias);
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

/** LOC placements only — mirrors `crates/nav/src/transport.rs` `parse_jm2_locs` gating. */
export function parseJm2LocPlacements(text: string, locId: number) {
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
        if (id !== locId) continue;
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
        minePortalLocId,
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
            authority: 'reference rs2b0t-beecd9126b src/bot/api/acquisition/ToolAcquire.ts NURMOF_VENDOR',
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
    const placements = parseJm2LocPlacements(fs.readFileSync(path.join(content, mapRelative), 'utf8'), 2662);
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
            authority: 'reference rs2b0t-beecd9126b src/bot/api/ai/quests/defs/murder/areas.ts MURDER_TILE.FLOUR_BARREL',
            note: 'FlourCollector Reach.locOp near / recovery anchor; not the jm2 loc tile (object at z=3582).',
        },
        bank_tile: {
            x: 2725,
            z: 3491,
            plane: 0,
            provenance: 'curated',
            authority: 'reference rs2b0t-beecd9126b src/bot/api/ai/quests/defs/murder/areas.ts MURDER_TILE.BANK',
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
async function generate(spec: Revision) {
    const pinned = assertPinned(spec);
    verifyCacheIdentity(spec.revision, spec.engine, spec.cacheIdentity);
    for (const requiredInput of ['data/pack/server/obj.dat', 'data/pack/server/npc.dat', 'data/pack/client/config']) if (!fs.existsSync(path.join(spec.engine, requiredInput))) throw new Error(`${spec.revision}: missing ${requiredInput}`);
    process.chdir(spec.engine); const objModule = (await import(pathToFileURL(path.join(spec.engine, 'src/cache/config/ObjType.ts')).href)) as { default: { load(dir: string): void; configs: ObjType[] } }; objModule.default.load('data/pack');
    const npcModule = (await import(pathToFileURL(path.join(spec.engine, 'src/cache/config/NpcType.ts')).href)) as { default: { load(dir: string): void; configs: NpcType[] } }; npcModule.default.load('data/pack');
    const items = objModule.default.configs.map(row); const aliases = items.filter((item) => item.alias !== null).map((item) => item.alias as string); if (new Set(items.map((item) => item.id)).size !== items.length || new Set(aliases).size !== aliases.length) throw new Error(`${spec.revision}: duplicate ids or aliases`);
    const facts = extractFacts(spec.content, objModule.default.configs, npcModule.default.configs); const drops = extractDropFacts(spec.content, objModule.default.configs, npcModule.default.configs); if (drops.length !== 4) throw new Error(`${spec.revision}: expected four combat drop tables, got ${drops.length}`); const magic = extractMagicFacts(spec.content, objModule.default.configs); if (magic.spells.length !== 16 || magic.spells[15].name !== 'Fire Wave' || magic.staves.length !== 14) throw new Error(`${spec.revision}: expected 16 combat spells and 14 staves, got ${magic.spells.length}/${magic.staves.length}`); const herbs = extractHerbFacts(spec.content, objModule.default.configs); if (herbs.herbs.length < 14) throw new Error(`${spec.revision}: expected a full herb identify table, got ${herbs.herbs.length}`); if (herbs.herb_level_default !== 3) throw new Error(`${spec.revision}: expected identify.param default 3, got ${herbs.herb_level_default}`); const autocast = extractAutocastControls(spec.content); const duel = extractDuelControls(spec.content); const special = extractSpecialControls(spec.content, objModule.default.configs);     const teleports = extractTeleportSpells(spec.content, objModule.default.configs); if (teleports.length !== 7 || teleports[0].name !== 'Varrock' || teleports[6].name !== 'Trollheim' || teleports[0].component_id !== 1164 || teleports[6].component_id !== 7455) throw new Error(`${spec.revision}: expected 7 standard teleports, got ${teleports.map((row) => row.name).join(',')}`);     const prayer = extractPrayerFacts(spec.content); if (prayer.prayers.length !== 15) throw new Error(`${spec.revision}: expected 15 prayers, got ${prayer.prayers.length}`); const nurmofEssence = extractNurmofEssenceFacts(spec.content, objModule.default.configs, npcModule.default.configs); if (nurmofEssence.pickaxes.length !== 6) throw new Error(`${spec.revision}: expected six pickaxes, got ${nurmofEssence.pickaxes.length}`);     const flourSix = extractFlourSixFacts(spec.content, objModule.default.configs); if (flourSix.pot.id !== 1931 || flourSix.flour_barrel.id !== 2662) throw new Error(`${spec.revision}: flour six join mismatch`); const objPackPath = path.join(spec.content, 'pack/obj.pack'); if (!fs.existsSync(objPackPath)) throw new Error(`${spec.revision}: missing pack/obj.pack`); const objPack = parsePack(fs.readFileSync(objPackPath, 'utf8')); if (objPack.size === 0) throw new Error(`${spec.revision}: empty pack/obj.pack`); const equipmentNames = extractEquipmentNamesFacts(items, objPack); const gatherMethods = extractGatherMethodsFacts(spec.content, spec.revision); if (gatherMethods.mining.length !== 17 || gatherMethods.woods.length !== 10 || gatherMethods.fishing.length !== 9) throw new Error(`${spec.revision}: expected 17 mine, 10 wood, and 9 fishing rows, got ${gatherMethods.mining.length}/${gatherMethods.woods.length}/${gatherMethods.fishing.length}`); const questIdentity = extractQuestIdentityFacts(spec.content, spec.revision); if (questIdentity.rows.length !== 6 || questIdentity.rows[4].id !== 'death' || questIdentity.rows[4].varp !== 'death_equiproom' || questIdentity.rows[4].varp_id !== 314 || questIdentity.rows[4].complete !== 80 || questIdentity.rows.some((row) => row.requirements.qualification !== 'partial')) throw new Error(`${spec.revision}: quest identity join mismatch`); if (spec.revision === 274 && (questIdentity.coverage.length !== 1 || questIdentity.coverage[0].alias !== 'routequest' || questIdentity.coverage[0].other_pin_id !== 387 || questIdentity.coverage[0].copied !== false)) throw new Error(`${spec.revision}: quest coverage mismatch`); if (spec.revision !== 274 && questIdentity.coverage.length !== 0) throw new Error(`${spec.revision}: quest coverage must be empty`); const inputs = ['data/pack/server/obj.dat', 'data/pack/server/npc.dat', 'data/pack/client/config'].map((file) => sourceFile(spec.engine, file)); const contentInputs = contentFiles.map((file) => sourceFile(spec.content, file)); const sources = decoderSources.map((file) => sourceFile(spec.engine, file));
    const payload = { schema_version: 4, revision: spec.revision, provenance: { engine_commit: pinned.engineCommit, content_commit: pinned.contentCommit, inputs, content_inputs: contentInputs, decoder_sources: sources, cache_identity: spec.cacheIdentity }, items, ...facts, drop_tables: drops, ...magic, ...herbs, ...prayer, nurmof_essence: nurmofEssence, flour_six: flourSix, equipment_names: equipmentNames, gather_methods: gatherMethods, quest_identity: questIdentity, autocast, duel, special, teleports }; const bytes = `${JSON.stringify(payload, null, 2)}\n`; fs.mkdirSync(path.dirname(spec.output), { recursive: true }); fs.writeFileSync(spec.output, bytes); return { revision: spec.revision, output: path.relative(root, spec.output), records: items.length, consumption: facts.consumption.length, pickpocket: facts.pickpocket.length, drop_tables: drops.length, spells: magic.spells.length, staves: magic.staves.length, herbs: herbs.herbs.length, prayers: prayer.prayers.length, pickaxes: nurmofEssence.pickaxes.length, flour_six: 6, gather_methods: { mining: gatherMethods.mining.length, woods: gatherMethods.woods.length, fishing: gatherMethods.fishing.length }, equipment_names: { bows: equipmentNames.bows.length, crossbows: equipmentNames.crossbows.length, darts: equipmentNames.darts.length, arrows: equipmentNames.arrows.length, bolts: equipmentNames.bolts.length, melee_weapons: equipmentNames.melee_weapons.length, staffs: equipmentNames.staffs.length, resolved: EQUIPMENT_FAMILY_ORDER.reduce((sum, family) => sum + equipmentNames[family].filter((row) => row.disposition === 'resolved').length, 0), absent: EQUIPMENT_FAMILY_ORDER.reduce((sum, family) => sum + equipmentNames[family].filter((row) => row.disposition === 'absent').length, 0) }, autocast, duel, special: { energy_varp: special.energy_varp, armed_varp: special.armed_varp, max_energy: special.max_energy, bars: special.bars.length, weapons: special.weapons.length }, teleports: teleports.length, quest_identity: { rows: questIdentity.rows.length, coverage: questIdentity.coverage.length }, bytes: Buffer.byteLength(bytes), sha256: crypto.createHash('sha256').update(bytes).digest('hex'), engine_commit: pinned.engineCommit, content_commit: pinned.contentCommit, inputs, content_inputs: contentInputs, decoder_sources: sources, cache_identity: spec.cacheIdentity };
}
async function main() { const results = []; for (const spec of revisions) results.push(await generate(spec)); const manifest = { schema_version: 4, generator: 'tools/game-data/generate.ts', revisions: results }; const manifestPath = path.join(root, 'crates/api/data/game-data/manifest.json'); fs.writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`); console.log(JSON.stringify({ manifest: path.relative(root, manifestPath), revisions: results }, null, 2)); }
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main().catch((error) => { console.error(error); process.exitCode = 1; });
