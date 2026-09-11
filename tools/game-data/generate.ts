import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath, pathToFileURL } from 'node:url';

type ObjType = { id: number; debugname: string | null; name: string | null; cost: number; stackable: boolean; members: boolean; certlink: number; certtemplate: number; wearpos: number; wearpos2: number; wearpos3: number };
type NpcType = { id: number; debugname?: string | null; name: string | null };
type Revision = { revision: number; engine: string; content: string; expectedEngine: string; expectedContent: string; cacheIdentity: { cache_id: string; nav_sha256: string; flags_sha256: string }; output: string };

const root = path.resolve(import.meta.dirname, '../..');
const envPath = (name: string, fallback: string) => process.env[name] ? path.resolve(process.env[name]!) : fallback;
const revisions: Revision[] = [
    { revision: 274, engine: envPath('GAME_DATA_274_ENGINE', '/Users/acfrazier/experiments/Server/engine'), content: envPath('GAME_DATA_274_CONTENT', '/Users/acfrazier/experiments/Server/content'), expectedEngine: '4c95f87efe00b068cadbd229d94736626907bd1a', expectedContent: '000c19997e07206131bcb3c884265840efce416d', cacheIdentity: { cache_id: '4aac9b63312dcb75d5de8f686772d083ba0808c57985438246edf21ef522be1c', nav_sha256: '05db24743e9f549ced16c1f00b87c30a390d3aaec391815da3f3563130b3bcd4', flags_sha256: '92d5dea05c886ac8720be6b47e47cbc68355a8ff42676c0886f5b7ea8343a4cb' }, output: path.join(root, 'crates/api/data/game-data/274.json') },
    { revision: 289, engine: envPath('GAME_DATA_289_ENGINE', '/Users/acfrazier/experiments/lostcity-289/engine'), content: envPath('GAME_DATA_289_CONTENT', '/Users/acfrazier/experiments/lostcity-289/content'), expectedEngine: 'cc359656b4acd216ca452495874b6beba9a0ac75', expectedContent: '92649430fcbc83538d8c4367ecb96cee1a67a944', cacheIdentity: { cache_id: 'c4d8ab36bcfd2a7907535b4f619e28623b0a22e98d496fd2a9620d544c5b5b09', nav_sha256: '131db92e32eddcb08148909d477589544320fe7e34a422e8004e97e888032924', flags_sha256: '67e4094dff06def5cf8abc172ce751f4ca8679532ba04c1ba15ab6bf668c7a4a' }, output: path.join(root, 'crates/api/data/game-data/289.json') }
];
const decoderSources = [
    'src/cache/config/ObjType.ts', 'src/cache/config/NpcType.ts', 'src/cache/config/ConfigType.ts', 'src/cache/config/ParamHelper.ts', 'src/cache/config/ParamType.ts', 'src/cache/config/ScriptVarType.ts',
    'src/io/BZip2.ts', 'src/io/Jagfile.ts', 'src/io/Packet.ts', 'src/datastruct/DoublyLinkable.ts', 'src/datastruct/LinkList.ts', 'src/datastruct/Linkable.ts', 'src/util/Environment.ts', 'src/util/Logger.ts', 'src/util/TryParse.ts', 'src/util/WorldConfig.ts'
];
const contentFiles = ['scripts/player/configs/consumption/consume.dbtable', 'scripts/player/configs/consumption/consume_normal.dbrow', 'scripts/player/configs/consumption/consume_effects.dbrow', 'scripts/skill_thieving/configs/pickpocking/pickpocket.dbtable', 'scripts/skill_thieving/configs/pickpocking/pickpocket.dbrow', 'scripts/player/scripts/consumption/effects/scripts/consume_effects.rs2', 'scripts/skill_combat/configs/magic/magic_combat_spells.dbrow', 'scripts/skill_magic/configs/magic.dbtable', 'scripts/skill_magic/configs/magic_staff.dbrow'];
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
function integer(value: string, label: string) { const parsed = Number(value); if (!Number.isInteger(parsed)) throw new Error(`${label}: expected integer, got ${value}`); return parsed; }
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
    const pinned = assertPinned(spec); for (const requiredInput of ['data/pack/server/obj.dat', 'data/pack/server/npc.dat', 'data/pack/client/config']) if (!fs.existsSync(path.join(spec.engine, requiredInput))) throw new Error(`${spec.revision}: missing ${requiredInput}`);
    process.chdir(spec.engine); const objModule = (await import(pathToFileURL(path.join(spec.engine, 'src/cache/config/ObjType.ts')).href)) as { default: { load(dir: string): void; configs: ObjType[] } }; objModule.default.load('data/pack');
    const npcModule = (await import(pathToFileURL(path.join(spec.engine, 'src/cache/config/NpcType.ts')).href)) as { default: { load(dir: string): void; configs: NpcType[] } }; npcModule.default.load('data/pack');
    const items = objModule.default.configs.map(row); const aliases = items.filter((item) => item.alias !== null).map((item) => item.alias as string); if (new Set(items.map((item) => item.id)).size !== items.length || new Set(aliases).size !== aliases.length) throw new Error(`${spec.revision}: duplicate ids or aliases`);
    const facts = extractFacts(spec.content, objModule.default.configs, npcModule.default.configs); const magic = extractMagicFacts(spec.content, objModule.default.configs); if (magic.spells.length !== 16 || magic.spells[15].name !== 'Fire Wave' || magic.staves.length !== 14) throw new Error(`${spec.revision}: expected 16 combat spells and 14 staves, got ${magic.spells.length}/${magic.staves.length}`); const inputs = ['data/pack/server/obj.dat', 'data/pack/server/npc.dat', 'data/pack/client/config'].map((file) => sourceFile(spec.engine, file)); const contentInputs = contentFiles.map((file) => sourceFile(spec.content, file)); const sources = decoderSources.map((file) => sourceFile(spec.engine, file));
    const payload = { schema_version: 3, revision: spec.revision, provenance: { engine_commit: pinned.engineCommit, content_commit: pinned.contentCommit, inputs, content_inputs: contentInputs, decoder_sources: sources, cache_identity: spec.cacheIdentity }, items, ...facts, ...magic }; const bytes = `${JSON.stringify(payload, null, 2)}\n`; fs.mkdirSync(path.dirname(spec.output), { recursive: true }); fs.writeFileSync(spec.output, bytes); return { revision: spec.revision, output: path.relative(root, spec.output), records: items.length, consumption: facts.consumption.length, pickpocket: facts.pickpocket.length, spells: magic.spells.length, staves: magic.staves.length, bytes: Buffer.byteLength(bytes), sha256: crypto.createHash('sha256').update(bytes).digest('hex'), engine_commit: pinned.engineCommit, content_commit: pinned.contentCommit, inputs, content_inputs: contentInputs, decoder_sources: sources, cache_identity: spec.cacheIdentity };
}
async function main() { const results = []; for (const spec of revisions) results.push(await generate(spec)); const manifest = { schema_version: 3, generator: 'tools/game-data/generate.ts', revisions: results }; const manifestPath = path.join(root, 'crates/api/data/game-data/manifest.json'); fs.writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`); console.log(JSON.stringify({ manifest: path.relative(root, manifestPath), revisions: results }, null, 2)); }
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main().catch((error) => { console.error(error); process.exitCode = 1; });
