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
const contentFiles = ['scripts/player/configs/consumption/consume.dbtable', 'scripts/player/configs/consumption/consume_normal.dbrow', 'scripts/player/configs/consumption/consume_effects.dbrow', 'scripts/skill_thieving/configs/pickpocking/pickpocket.dbtable', 'scripts/skill_thieving/configs/pickpocking/pickpocket.dbrow', 'scripts/player/scripts/consumption/effects/scripts/consume_effects.rs2', 'scripts/skill_combat/configs/magic/magic_combat_spells.dbrow', 'scripts/skill_magic/configs/magic.dbtable', 'scripts/skill_magic/configs/magic_spells.dbrow', 'scripts/skill_magic/configs/magic_staff.dbrow', 'scripts/skill_combat/configs/combat.constant', 'scripts/skill_herblore/configs/herbs.obj', 'scripts/skill_herblore/configs/identifying/identify.param', 'scripts/skill_herblore/scripts/identifying/identify.rs2', 'pack/interface.pack', 'pack/varp.pack', 'pack/param.pack', ...dropContentFiles];
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
    const facts = extractFacts(spec.content, objModule.default.configs, npcModule.default.configs); const drops = extractDropFacts(spec.content, objModule.default.configs, npcModule.default.configs); if (drops.length !== 4) throw new Error(`${spec.revision}: expected four combat drop tables, got ${drops.length}`); const magic = extractMagicFacts(spec.content, objModule.default.configs); if (magic.spells.length !== 16 || magic.spells[15].name !== 'Fire Wave' || magic.staves.length !== 14) throw new Error(`${spec.revision}: expected 16 combat spells and 14 staves, got ${magic.spells.length}/${magic.staves.length}`); const herbs = extractHerbFacts(spec.content, objModule.default.configs); if (herbs.herbs.length < 14) throw new Error(`${spec.revision}: expected a full herb identify table, got ${herbs.herbs.length}`); if (herbs.herb_level_default !== 3) throw new Error(`${spec.revision}: expected identify.param default 3, got ${herbs.herb_level_default}`); const autocast = extractAutocastControls(spec.content); const duel = extractDuelControls(spec.content); const special = extractSpecialControls(spec.content, objModule.default.configs); const teleports = extractTeleportSpells(spec.content, objModule.default.configs); if (teleports.length !== 7 || teleports[0].name !== 'Varrock' || teleports[6].name !== 'Trollheim' || teleports[0].component_id !== 1164 || teleports[6].component_id !== 7455) throw new Error(`${spec.revision}: expected 7 standard teleports, got ${teleports.map((row) => row.name).join(',')}`); const inputs = ['data/pack/server/obj.dat', 'data/pack/server/npc.dat', 'data/pack/client/config'].map((file) => sourceFile(spec.engine, file)); const contentInputs = contentFiles.map((file) => sourceFile(spec.content, file)); const sources = decoderSources.map((file) => sourceFile(spec.engine, file));
    const payload = { schema_version: 4, revision: spec.revision, provenance: { engine_commit: pinned.engineCommit, content_commit: pinned.contentCommit, inputs, content_inputs: contentInputs, decoder_sources: sources, cache_identity: spec.cacheIdentity }, items, ...facts, drop_tables: drops, ...magic, ...herbs, autocast, duel, special, teleports }; const bytes = `${JSON.stringify(payload, null, 2)}\n`; fs.mkdirSync(path.dirname(spec.output), { recursive: true }); fs.writeFileSync(spec.output, bytes); return { revision: spec.revision, output: path.relative(root, spec.output), records: items.length, consumption: facts.consumption.length, pickpocket: facts.pickpocket.length, drop_tables: drops.length, spells: magic.spells.length, staves: magic.staves.length, herbs: herbs.herbs.length, autocast, duel, special: { energy_varp: special.energy_varp, armed_varp: special.armed_varp, max_energy: special.max_energy, bars: special.bars.length, weapons: special.weapons.length }, teleports: teleports.length, bytes: Buffer.byteLength(bytes), sha256: crypto.createHash('sha256').update(bytes).digest('hex'), engine_commit: pinned.engineCommit, content_commit: pinned.contentCommit, inputs, content_inputs: contentInputs, decoder_sources: sources, cache_identity: spec.cacheIdentity };
}
async function main() { const results = []; for (const spec of revisions) results.push(await generate(spec)); const manifest = { schema_version: 4, generator: 'tools/game-data/generate.ts', revisions: results }; const manifestPath = path.join(root, 'crates/api/data/game-data/manifest.json'); fs.writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`); console.log(JSON.stringify({ manifest: path.relative(root, manifestPath), revisions: results }, null, 2)); }
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main().catch((error) => { console.error(error); process.exitCode = 1; });
