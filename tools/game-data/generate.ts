import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { pathToFileURL } from 'node:url';

type ObjType = { id: number; debugname: string | null; name: string | null; cost: number; stackable: boolean; members: boolean; certlink: number; certtemplate: number; wearpos: number; wearpos2: number; wearpos3: number };
type Revision = { revision: number; engine: string; content: string; expectedEngine: string; expectedContent: string; cacheIdentity: { cache_id: string; nav_sha256: string; flags_sha256: string }; output: string };

const root = path.resolve(import.meta.dirname, '../..');
const envPath = (name: string, fallback: string) => process.env[name] ? path.resolve(process.env[name]!) : fallback;
const revisions: Revision[] = [
    { revision: 274, engine: envPath('GAME_DATA_274_ENGINE', '/Users/acfrazier/experiments/Server/engine'), content: envPath('GAME_DATA_274_CONTENT', '/Users/acfrazier/experiments/Server/content'), expectedEngine: '4c95f87efe00b068cadbd229d94736626907bd1a', expectedContent: '000c19997e07206131bcb3c884265840efce416d', cacheIdentity: { cache_id: '4aac9b63312dcb75d5de8f686772d083ba0808c57985438246edf21ef522be1c', nav_sha256: '05db24743e9f549ced16c1f00b87c30a390d3aaec391815da3f3563130b3bcd4', flags_sha256: '92d5dea05c886ac8720be6b47e47cbc68355a8ff42676c0886f5b7ea8343a4cb' }, output: path.join(root, 'crates/api/data/game-data/274.json') },
    { revision: 289, engine: envPath('GAME_DATA_289_ENGINE', '/Users/acfrazier/experiments/lostcity-289/engine'), content: envPath('GAME_DATA_289_CONTENT', '/Users/acfrazier/experiments/lostcity-289/content'), expectedEngine: 'cc359656b4acd216ca452495874b6beba9a0ac75', expectedContent: '92649430fcbc83538d8c4367ecb96cee1a67a944', cacheIdentity: { cache_id: 'c4d8ab36bcfd2a7907535b4f619e28623b0a22e98d496fd2a9620d544c5b5b09', nav_sha256: '131db92e32eddcb08148909d477589544320fe7e34a422e8004e97e888032924', flags_sha256: '67e4094dff06def5cf8abc172ce751f4ca8679532ba04c1ba15ab6bf668c7a4a' }, output: path.join(root, 'crates/api/data/game-data/289.json') }
];
const decoderSources = ['src/cache/config/ObjType.ts', 'src/cache/config/ConfigType.ts', 'src/cache/config/ParamHelper.ts', 'src/cache/config/ParamType.ts', 'src/io/Jagfile.ts', 'src/io/Packet.ts', 'src/util/Environment.ts', 'src/util/Logger.ts'];

function sha256(file: string) { const data = fs.readFileSync(file); return { bytes: data.length, sha256: crypto.createHash('sha256').update(data).digest('hex') }; }
function commit(dir: string) { return execFileSync('git', ['-C', dir, 'rev-parse', 'HEAD'], { encoding: 'utf8' }).trim(); }
function sourceFile(engine: string, relative: string) { return { path: relative, ...sha256(path.join(engine, relative)) }; }
function assertPinned(spec: Revision) {
    const engineCommit = commit(spec.engine); const contentCommit = commit(spec.content);
    if (engineCommit !== spec.expectedEngine || contentCommit !== spec.expectedContent) throw new Error(`${spec.revision}: expected pinned engine/content commits, got ${engineCommit}/${contentCommit}`);
    const relevant = [...decoderSources, 'data/pack/server/obj.dat', 'data/pack/client/config'];
    const dirty = execFileSync('git', ['-C', spec.engine, 'status', '--porcelain', '--untracked-files=all', '--', ...relevant], { encoding: 'utf8' }).trim();
    if (dirty) throw new Error(`${spec.revision}: relevant engine inputs are dirty:\n${dirty}`);
    return { engineCommit, contentCommit };
}
function row(obj: ObjType) { return { alias: obj.debugname, id: obj.id, name: obj.name, cost: obj.cost, stackable: obj.stackable, members: obj.members, certificate_link: obj.certlink, certificate_template: obj.certtemplate, wear_position: obj.wearpos, wear_position_2: obj.wearpos2, wear_position_3: obj.wearpos3 }; }

async function generate(spec: Revision) {
    const pinned = assertPinned(spec);
    for (const required of ['data/pack/server/obj.dat', 'data/pack/client/config']) if (!fs.existsSync(path.join(spec.engine, required))) throw new Error(`${spec.revision}: missing ${required}`);
    process.chdir(spec.engine);
    const moduleUrl = pathToFileURL(path.join(spec.engine, 'src/cache/config/ObjType.ts')).href;
    const module = (await import(moduleUrl)) as { default: { load(dir: string): void; configs: ObjType[] } };
    module.default.load('data/pack');
    const items = module.default.configs.map(row);
    const aliases = items.filter((item) => item.alias !== null).map((item) => item.alias as string);
    if (new Set(items.map((item) => item.id)).size !== items.length || new Set(aliases).size !== aliases.length) throw new Error(`${spec.revision}: duplicate ids or aliases`);
    const inputs = ['data/pack/server/obj.dat', 'data/pack/client/config'].map((file) => sourceFile(spec.engine, file));
    const sources = decoderSources.map((file) => sourceFile(spec.engine, file));
    const payload = { schema_version: 2, revision: spec.revision, provenance: { engine_commit: pinned.engineCommit, content_commit: pinned.contentCommit, inputs, decoder_sources: sources, cache_identity: spec.cacheIdentity }, items };
    const bytes = `${JSON.stringify(payload, null, 2)}\n`; fs.mkdirSync(path.dirname(spec.output), { recursive: true }); fs.writeFileSync(spec.output, bytes);
    return { revision: spec.revision, output: path.relative(root, spec.output), records: items.length, bytes: Buffer.byteLength(bytes), sha256: crypto.createHash('sha256').update(bytes).digest('hex'), engine_commit: pinned.engineCommit, content_commit: pinned.contentCommit, inputs, decoder_sources: sources, cache_identity: spec.cacheIdentity };
}
async function main() { const results = []; for (const spec of revisions) results.push(await generate(spec)); const manifest = { schema_version: 2, generator: 'tools/game-data/generate.ts', revisions: results }; const manifestPath = path.join(root, 'crates/api/data/game-data/manifest.json'); fs.writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`); console.log(JSON.stringify({ manifest: path.relative(root, manifestPath), revisions: results }, null, 2)); }
main().catch((error) => { console.error(error); process.exitCode = 1; });
