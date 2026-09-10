import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { pathToFileURL } from 'node:url';

type ObjType = {
    id: number;
    debugname: string | null;
    name: string | null;
    cost: number;
    stackable: boolean;
    members: boolean;
    certlink: number;
    certtemplate: number;
    wearpos: number;
    wearpos2: number;
    wearpos3: number;
};

type Revision = {
    revision: number;
    engine: string;
    content: string;
    output: string;
};

const root = path.resolve(import.meta.dirname, '../..');
const revisions: Revision[] = [
    {
        revision: 274,
        engine: '/Users/acfrazier/experiments/Server/engine',
        content: '/Users/acfrazier/experiments/Server/content',
        output: path.join(root, 'crates/api/data/game-data/274.json')
    },
    {
        revision: 289,
        engine: '/Users/acfrazier/experiments/lostcity-289/engine',
        content: '/Users/acfrazier/experiments/lostcity-289/content',
        output: path.join(root, 'crates/api/data/game-data/289.json')
    }
];

function sha256(file: string) {
    const data = fs.readFileSync(file);
    return { bytes: data.length, sha256: crypto.createHash('sha256').update(data).digest('hex') };
}

function commit(dir: string) {
    return execFileSync('git', ['-C', dir, 'rev-parse', 'HEAD'], { encoding: 'utf8' }).trim();
}

function sourceFile(engine: string, relative: string) {
    const file = path.join(engine, relative);
    return { path: relative, ...sha256(file) };
}

function row(obj: ObjType) {
    return {
        alias: obj.debugname,
        id: obj.id,
        name: obj.name,
        cost: obj.cost,
        stackable: obj.stackable,
        members: obj.members,
        certificate_link: obj.certlink,
        certificate_template: obj.certtemplate,
        wear_position: obj.wearpos,
        wear_position_2: obj.wearpos2,
        wear_position_3: obj.wearpos3
    };
}

async function generate(spec: Revision) {
    for (const required of ['data/pack/server/obj.dat', 'data/pack/client/config']) {
        if (!fs.existsSync(path.join(spec.engine, required))) {
            throw new Error(`${spec.revision}: missing ${required}`);
        }
    }

    process.chdir(spec.engine);
    const moduleUrl = pathToFileURL(path.join(spec.engine, 'src/cache/config/ObjType.ts')).href;
    const module = (await import(moduleUrl)) as { default: { load(dir: string): void; configs: ObjType[] } };
    module.default.load('data/pack');
    const items = module.default.configs.map(row);
    const aliases = items.filter((item) => item.alias !== null).map((item) => item.alias as string);
    if (new Set(items.map((item) => item.id)).size !== items.length) throw new Error(`${spec.revision}: duplicate ids`);
    if (new Set(aliases).size !== aliases.length) throw new Error(`${spec.revision}: duplicate aliases`);
    if (items.some((item) => item.alias === 'rune_platebody' && item.name !== 'Rune platebody')) {
        throw new Error(`${spec.revision}: rune_platebody name mismatch`);
    }

    const objServer = sourceFile(spec.engine, 'data/pack/server/obj.dat');
    const objClient = sourceFile(spec.engine, 'data/pack/client/config');
    const payload = {
        schema_version: 1,
        revision: spec.revision,
        provenance: {
            engine_commit: commit(spec.engine),
            content_commit: commit(spec.content),
            inputs: [objServer, objClient]
        },
        items
    };
    const bytes = `${JSON.stringify(payload, null, 2)}\n`;
    fs.mkdirSync(path.dirname(spec.output), { recursive: true });
    fs.writeFileSync(spec.output, bytes);
    return {
        revision: spec.revision,
        output: path.relative(root, spec.output),
        records: items.length,
        bytes: Buffer.byteLength(bytes),
        sha256: crypto.createHash('sha256').update(bytes).digest('hex'),
        engine_commit: commit(spec.engine),
        content_commit: commit(spec.content),
        inputs: [objServer, objClient]
    };
}

async function main() {
    const results = [];
    for (const spec of revisions) results.push(await generate(spec));
    const manifest = {
        schema_version: 1,
        generator: 'tools/game-data/generate.ts',
        revisions: results
    };
    const manifestPath = path.join(root, 'crates/api/data/game-data/manifest.json');
    fs.writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);
    console.log(JSON.stringify({ manifest: path.relative(root, manifestPath), revisions: results }, null, 2));
}

main().catch((error) => {
    console.error(error);
    process.exitCode = 1;
});
