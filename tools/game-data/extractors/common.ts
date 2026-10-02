// Shared content parsing only; no runtime crawling or artifact finalization.
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
export function sha256(file: string) { const data = fs.readFileSync(file); return { bytes: data.length, sha256: crypto.createHash('sha256').update(data).digest('hex') }; }
export function sourceFile(dir: string, relative: string) { return { path: relative, ...sha256(path.join(dir, relative)) }; }
export function parseRows(text: string) {
    const rows: { name: string; values: Record<string, string[][]> }[] = []; let current: { name: string; values: Record<string, string[][]> } | null = null;
    for (const raw of text.split(/\r?\n/)) { const line = raw.trim(); if (!line || line.startsWith('//')) continue; if (line.startsWith('[') && line.endsWith(']')) { current = { name: line.slice(1, -1), values: {} }; rows.push(current); continue; } if (!current || !line.startsWith('data=')) continue; const [, rest] = line.split('=', 2); const [key, ...values] = rest.split(','); (current.values[key] ??= []).push(values); }
    return rows;
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
export function integer(value: string, label: string) { const parsed = Number(value); if (!Number.isInteger(parsed)) throw new Error(`${label}: expected integer, got ${value}`); return parsed; }
export function parseMapsquarePath(relative: string) {
    const base = path.basename(relative, '.jm2');
    const match = /^m(\d+)_(\d+)$/.exec(base);
    if (!match) throw new Error(`mapsquare path: expected m<x>_<z>.jm2, got ${relative}`);
    return { mx: integer(match[1], relative), mz: integer(match[2], relative) };
}

export function jm2SectionName(line: string) {
    const trimmed = line.trim();
    if (!trimmed.startsWith('==== ') || !trimmed.endsWith(' ====')) return null;
    return trimmed.slice('==== '.length, -' ===='.length);
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

/**
 * NPC placements only — the `==== NPC ====` section of one jm2. Rows are
 * `plane lx lz: npc_id` with exactly one data token.
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

export function walkContentFiles(root: string, ext: string): string[] {
    const out: string[] = [];
    const visit = (dir: string) => {
        for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
            const full = path.join(dir, entry.name);
            if (entry.isDirectory()) visit(full);
            else if (entry.isFile() && entry.name.endsWith(ext)) out.push(full);
        }
    };
    visit(root);
    out.sort();
    return out;
}

export function parseConfigBlocks(text: string): { name: string; values: Record<string, string> }[] {
    const rows: { name: string; values: Record<string, string> }[] = [];
    let current: { name: string; values: Record<string, string> } | null = null;
    for (const raw of text.split(/\r?\n/)) {
        const line = raw.trim();
        if (!line || line.startsWith('//')) continue;
        if (line.startsWith('[') && line.endsWith(']')) {
            current = { name: line.slice(1, -1), values: {} };
            rows.push(current);
            continue;
        }
        if (!current) continue;
        const eq = line.indexOf('=');
        if (eq <= 0) continue;
        const key = line.slice(0, eq);
        const value = line.slice(eq + 1);
        current.values[key] = value;
    }
    return rows;
}

export function worldFromMapsquare(mx: number, mz: number, lx: number, lz: number, plane: number) {
    return { x: mx * 64 + lx, z: mz * 64 + lz, plane };
}
export function requireGatherText(content: string, relative: string) {
    const file = path.join(content, relative);
    if (!fs.existsSync(file)) throw new Error(`${relative}: required file missing`);
    return fs.readFileSync(file, 'utf8');
}
export const PLACEMENT_MAPS_DIRECTORY = 'maps';

/**
 * Required maps inventory (O-NAVINPUT). The directory must exist and hold at least
 * one map; every on-disk `m*.jm2` must be a readable mapsquare name; and every map
 * the content tree tracks must be on disk, so a truncated tree fails closed instead
 * of silently under-extracting. Filename order.
 */
export function placementMapInputs(content: string) {
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
