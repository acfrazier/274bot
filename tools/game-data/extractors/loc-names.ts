import fs from 'node:fs';
import path from 'node:path';
import { parsePack, requireGatherText, walkContentFiles } from './common.ts';

export const locNamesContentFiles = ['pack/loc.pack'];

type LocBlock = { name: string; display: string | null; ops: string[] };

function parseLocFiles(content: string) {
    const blocks = new Map<string, LocBlock>();
    const files: string[] = [];
    for (const file of walkContentFiles(path.join(content, 'scripts'), '.loc')) {
        files.push(path.relative(content, file));
        let current: LocBlock | null = null;
        for (const raw of fs.readFileSync(file, 'utf8').split(/\r?\n/)) {
            const line = raw.trim();
            if (!line || line.startsWith('//')) continue;
            if (line.startsWith('[') && line.endsWith(']')) {
                const name = line.slice(1, -1);
                if (blocks.has(name)) throw new Error(`loc_names: duplicate config [${name}]`);
                current = { name, display: null, ops: [] };
                blocks.set(name, current);
                continue;
            }
            if (!current) continue;
            const eq = line.indexOf('=');
            if (eq <= 0) continue;
            const key = line.slice(0, eq);
            const value = line.slice(eq + 1);
            if (key === 'name') current.display = value;
            else if (/^op[1-5]$/.test(key) && value) current.ops.push(value);
        }
    }
    return { blocks, files };
}

export function extractLocNamesFacts(content: string) {
    const pack = parsePack(requireGatherText(content, 'pack/loc.pack'));
    if (pack.size === 0) throw new Error('loc_names: empty pack/loc.pack');
    const { blocks, files } = parseLocFiles(content);
    const rows = [...pack.entries()]
        .sort((a, b) => a[1] - b[1])
        .map(([config, id]) => {
            const block = blocks.get(config);
            return {
                id,
                config,
                display: block?.display ?? null,
                ops: block?.ops ?? [],
            };
        });
    if (rows.length === 0) throw new Error('loc_names: no extracted rows');
    if (!rows.some((row) => row.config === 'hopper_lumbridge' && row.id === 2714)) {
        throw new Error('loc_names: missing hopper_lumbridge pack join');
    }
    return { rows, files };
}
