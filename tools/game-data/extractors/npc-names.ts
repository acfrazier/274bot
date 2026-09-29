import fs from 'node:fs';
import path from 'node:path';
import { integer, parsePack, requireGatherText, walkContentFiles } from './common.ts';

export const npcNamesContentFiles = ['pack/npc.pack'];

type NpcBlock = {
    name: string;
    display: string | null;
    ops: string[];
    size: number;
    wanderrange: number;
    maxrange: number;
    attackrange: number;
    huntrange: number;
    vislevel: number;
    hitpoints: number;
    damagetype: string | null;
};

function parseIntField(raw: string | undefined, _key: string, fallback: number) {
    if (raw === undefined || raw === '') return fallback;
    const parsed = Number(raw);
    if (!Number.isInteger(parsed)) return fallback;
    return parsed;
}

function parseNpcFiles(content: string) {
    const blocks = new Map<string, NpcBlock>();
    const files: string[] = [];
    for (const file of walkContentFiles(path.join(content, 'scripts'), '.npc')) {
        files.push(path.relative(content, file));
        let current: NpcBlock | null = null;
        for (const raw of fs.readFileSync(file, 'utf8').split(/\r?\n/)) {
            const line = raw.trim();
            if (!line || line.startsWith('//')) continue;
            if (line.startsWith('[') && line.endsWith(']')) {
                const name = line.slice(1, -1);
                if (blocks.has(name)) throw new Error(`npc_names: duplicate config [${name}]`);
                current = {
                    name,
                    display: null,
                    ops: [],
                    size: 1,
                    wanderrange: 0,
                    maxrange: 0,
                    attackrange: 0,
                    huntrange: 0,
                    vislevel: 0,
                    hitpoints: 0,
                    damagetype: null,
                };
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
            else if (key === 'size') current.size = parseIntField(value, key, 1);
            else if (key === 'wanderrange') current.wanderrange = parseIntField(value, key, 0);
            else if (key === 'maxrange') current.maxrange = parseIntField(value, key, 0);
            else if (key === 'attackrange') current.attackrange = parseIntField(value, key, 0);
            else if (key === 'huntrange') current.huntrange = parseIntField(value, key, 0);
            else if (key === 'vislevel') current.vislevel = value === 'hide' ? 0 : parseIntField(value, key, 0);
            else if (key === 'hitpoints') current.hitpoints = parseIntField(value, key, 0);
            else if (key === 'param' && value.startsWith('damagetype,')) {
                current.damagetype = value.slice('damagetype,'.length);
            }
        }
    }
    return { blocks, files };
}

function dragonfireNpcs(content: string, known: Set<string>) {
    const flagged = new Set<string>();
    for (const file of walkContentFiles(path.join(content, 'scripts'), '.rs2')) {
        const text = fs.readFileSync(file, 'utf8');
        if (!text.includes('%dragonresist')) continue;
        for (const match of text.matchAll(/\[(?:ai_[^\],]+),([A-Za-z0-9_]+)\]/g)) {
            if (known.has(match[1])) flagged.add(match[1]);
        }
    }
    return flagged;
}

export function extractNpcNamesFacts(content: string) {
    const pack = parsePack(requireGatherText(content, 'pack/npc.pack'));
    if (pack.size === 0) throw new Error('npc_names: empty pack/npc.pack');
    const known = new Set(pack.keys());
    const { blocks, files } = parseNpcFiles(content);
    const dragonfire = dragonfireNpcs(content, known);
    const rows = [...pack.entries()]
        .sort((a, b) => a[1] - b[1])
        .map(([config, id]) => {
            const block = blocks.get(config);
            return {
                id,
                config,
                display: block?.display ?? null,
                ops: block?.ops ?? [],
                size: block?.size ?? 1,
                wanderrange: block?.wanderrange ?? 0,
                maxrange: block?.maxrange ?? 0,
                attackrange: block?.attackrange ?? 0,
                huntrange: block?.huntrange ?? 0,
                vislevel: block?.vislevel ?? 0,
                hitpoints: block?.hitpoints ?? 0,
                damagetype: block?.damagetype ?? null,
                dragonfire: dragonfire.has(config),
            };
        });
    if (rows.length === 0) throw new Error('npc_names: no extracted rows');
    if (!rows.some((row) => row.config === 'khazard_warlord' && row.id === 477)) {
        throw new Error('npc_names: missing khazard_warlord pack join');
    }
    return { rows, files };
}
