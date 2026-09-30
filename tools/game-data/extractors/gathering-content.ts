// Content indexes for the gathering family. Reads only the pinned content tree (`scripts`, `pack`, `maps`);
// never the engine, never the network. Every fact carries a content-relative source span.
import fs from 'node:fs';
import path from 'node:path';
import { parseMapsquarePath, parsePack, placementMapInputs, requireGatherText } from './common.ts';

/** One physical line range in a content file (1-based, inclusive). */
export type Span = { file: string; first: number; last: number };
export const spanRef = (span: Span) => `${span.file}:${span.first}-${span.last}`;

/** A `[name]` config section: plain `key=value` properties plus `param=key,value` rows. Values keep file order. */
export type Section = { name: string; span: Span; props: Map<string, string[]>; params: Map<string, string[]> };

/** Recursive sorted walk below `dir`, content-relative POSIX paths of the files with one of `extensions`. */
export function walkContent(content: string, dir: string, extensions: readonly string[]): string[] {
    const out: string[] = [];
    const visit = (relative: string) => {
        const entries = fs.readdirSync(path.join(content, relative), { withFileTypes: true }).sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0));
        for (const entry of entries) {
            const child = `${relative}/${entry.name}`;
            if (entry.isDirectory()) visit(child);
            else if (entry.isFile() && extensions.some((extension) => entry.name.endsWith(extension))) out.push(child);
        }
    };
    if (!fs.existsSync(path.join(content, dir))) throw new Error(`${dir}: required directory missing`);
    visit(dir);
    return out;
}

const lines = (text: string) => text.split(/\r?\n/);

/**
 * `[name]` sections with `key=value` rows. Whole-line `//` comments are skipped. `param=key,value` keeps the
 * value after the first comma. A section's span ends at its last non-blank, non-comment row.
 */
export function parseSections(file: string, text: string): Section[] {
    const sections: Section[] = [];
    let current: Section | null = null;
    lines(text).forEach((raw, index) => {
        const line = raw.trim();
        if (!line || line.startsWith('//')) return;
        const number = index + 1;
        if (line.startsWith('[') && line.endsWith(']')) {
            current = { name: line.slice(1, -1), span: { file, first: number, last: number }, props: new Map(), params: new Map() };
            sections.push(current);
            return;
        }
        if (!current) return;
        current.span.last = number;
        const eq = line.indexOf('=');
        if (eq <= 0) return;
        const key = line.slice(0, eq);
        const value = line.slice(eq + 1);
        if (key === 'param') {
            const comma = value.indexOf(',');
            const paramKey = comma < 0 ? value : value.slice(0, comma);
            if (paramKey) (current.params.get(paramKey) ?? current.params.set(paramKey, []).get(paramKey)!).push(comma < 0 ? '' : value.slice(comma + 1));
            return;
        }
        (current.props.get(key) ?? current.props.set(key, []).get(key)!).push(value);
    });
    return sections;
}

export type DbData = { values: string[]; line: number };
export type DbRow = { name: string; span: Span; table: string | null; data: Map<string, DbData[]> };

/** dbrow sections: `table=` plus every `data=key,v1,v2,...` row with its own line number. */
export function parseDbRows(file: string, text: string): DbRow[] {
    const rows: DbRow[] = [];
    let current: DbRow | null = null;
    lines(text).forEach((raw, index) => {
        const line = raw.trim();
        if (!line || line.startsWith('//')) return;
        const number = index + 1;
        if (line.startsWith('[') && line.endsWith(']')) {
            current = { name: line.slice(1, -1), span: { file, first: number, last: number }, table: null, data: new Map() };
            rows.push(current);
            return;
        }
        if (!current) return;
        current.span.last = number;
        if (line.startsWith('table=')) current.table = line.slice('table='.length);
        if (!line.startsWith('data=')) return;
        const [key, ...values] = line.slice('data='.length).split(',');
        (current.data.get(key) ?? current.data.set(key, []).get(key)!).push({ values, line: number });
    });
    return rows;
}

/** `^name = value` constants (only whole-line definitions). */
export function parseConstants(file: string, text: string, into: Map<string, { value: string; span: Span }>) {
    lines(text).forEach((raw, index) => {
        const match = /^\^([A-Za-z0-9_]+)\s*=\s*(.+?)\s*$/.exec(raw.trim());
        if (match) into.set(match[1], { value: match[2], span: { file, first: index + 1, last: index + 1 } });
    });
}

/** `<plane>_<mx>_<mz>_<lx>_<lz>` content coordinate to world tile. */
export function parseCoord(text: string, label: string) {
    const match = /^(\d+)_(\d+)_(\d+)_(\d+)_(\d+)$/.exec(text.trim());
    if (!match) throw new Error(`${label}: expected plane_mx_mz_lx_lz coordinate, got ${text}`);
    const [plane, mx, mz, lx, lz] = match.slice(1).map(Number);
    if (plane > 3 || lx > 63 || lz > 63) throw new Error(`${label}: coordinate out of range ${text}`);
    return { plane, x: mx * 64 + lx, z: mz * 64 + lz };
}

/** A content script block header `[type,name]` (with `(params)` when declared). */
export type Rs2Header = { kind: string; name: string; span: Span };
export type Rs2Block = { kind: string; name: string; params: string; span: Span; body: { line: number; text: string }[] };

const HEADER = /^\[([A-Za-z0-9_]+),([^\]]+)\](.*)$/;

/** A bare global loc-op header (`[oploc1]` with no name): the engine's third dispatch tier. */
const GLOBAL_LOC_OP = /^\[(oploc[1-5])\s*\](.*)$/;
/**
 * Split a script file into `[kind,name]` blocks. One-line handlers (`[opnpc1,_x] @label;`) keep their trailing
 * statement as the first body line. The span ends at the block's last non-blank, non-comment line.
 */
export function parseRs2Blocks(file: string, text: string): Rs2Block[] {
    const blocks: Rs2Block[] = [];
    let current: Rs2Block | null = null;
    lines(text).forEach((raw, index) => {
        const number = index + 1;
        const match = HEADER.exec(raw);
        if (match) {
            const rest = match[3].trim();
            const params = rest.startsWith('(') ? rest : '';
            current = { kind: match[1], name: match[2], params, span: { file, first: number, last: number }, body: [] };
            blocks.push(current);
            if (rest && !params) {
                current.body.push({ line: number, text: rest });
            }
            return;
        }
        if (!current) return;
        current.body.push({ line: number, text: raw });
        const code = stripComment(raw).trim();
        if (code) current.span.last = number;
    });
    return blocks;
}

/** Remove a `//` comment, except inside a double-quoted string. */
export function stripComment(line: string) {
    let inString = false;
    for (let i = 0; i < line.length; i += 1) {
        const ch = line[i];
        if (ch === '"') inString = !inString;
        else if (!inString && ch === '/' && line[i + 1] === '/') return line.slice(0, i);
    }
    return line;
}

export type SectionIndex = { get(name: string): Section | undefined; all(): Section[]; duplicates(): string[] };

function indexSections(sections: Section[]): SectionIndex {
    const byName = new Map<string, Section[]>();
    for (const section of sections) (byName.get(section.name) ?? byName.set(section.name, []).get(section.name)!).push(section);
    return {
        get(name) {
            const found = byName.get(name);
            if (!found) return undefined;
            if (found.length > 1) throw new Error(`${name}: defined more than once (${found.map((section) => spanRef(section.span)).join(', ')})`);
            return found[0];
        },
        all: () => sections,
        duplicates: () => [...byName].filter(([, found]) => found.length > 1).map(([name]) => name),
    };
}

/** One pass over the content tree: packs, config sections, constants, enums and script headers. */
export type ContentIndex = {
    root: string;
    packs: { loc: Map<string, number>; npc: Map<string, number>; obj: Map<string, number> };
    loc: SectionIndex;
    npc: SectionIndex;
    obj: SectionIndex;
    enums: SectionIndex;
    structs: SectionIndex;
    constants: Map<string, { value: string; span: Span }>;
    /** Every script header, keyed `kind,name`, in file order. */
    headers: Map<string, Rs2Header[]>;
    /** Deep-parse one script file (cached). */
    blocks(file: string): Rs2Block[];
    text(file: string): string;
    /** Map-square files in filename order. */
    maps(): { path: string; mx: number; mz: number }[];
};

export function indexContent(content: string): ContentIndex {
    const packs = {
        loc: parsePack(requireGatherText(content, 'pack/loc.pack')),
        npc: parsePack(requireGatherText(content, 'pack/npc.pack')),
        obj: parsePack(requireGatherText(content, 'pack/obj.pack')),
    };
    for (const [name, pack] of Object.entries(packs)) if (pack.size === 0) throw new Error(`pack/${name}.pack: required file missing ids`);
    const texts = new Map<string, string>();
    const text = (file: string) => texts.get(file) ?? texts.set(file, requireGatherText(content, file)).get(file)!;
    const sections = { loc: [] as Section[], npc: [] as Section[], obj: [] as Section[], enum: [] as Section[], struct: [] as Section[] };
    const constants = new Map<string, { value: string; span: Span }>();
    const headers = new Map<string, Rs2Header[]>();
    for (const file of walkContent(content, 'scripts', ['.loc', '.npc', '.obj', '.enum', '.struct', '.constant', '.rs2'])) {
        const body = fs.readFileSync(path.join(content, file), 'utf8');
        if (file.endsWith('.constant')) parseConstants(file, body, constants);
        else if (file.endsWith('.rs2')) {
            lines(body).forEach((raw, index) => {
                const global = GLOBAL_LOC_OP.exec(raw);
                if (global) throw new Error(`${file}:${index + 1}: global [${global[1]}] handler found, but the mining no-handler exclusion rule doesn't model global handlers`);
                const match = HEADER.exec(raw);
                if (!match) return;
                const key = `${match[1]},${match[2]}`;
                (headers.get(key) ?? headers.set(key, []).get(key)!).push({ kind: match[1], name: match[2], span: { file, first: index + 1, last: index + 1 } });
            });
        } else sections[file.slice(file.lastIndexOf('.') + 1) as keyof typeof sections].push(...parseSections(file, body));
    }
    const blockCache = new Map<string, Rs2Block[]>();
    let mapList: { path: string; mx: number; mz: number }[] | null = null;
    return {
        root: content,
        packs,
        loc: indexSections(sections.loc),
        npc: indexSections(sections.npc),
        obj: indexSections(sections.obj),
        enums: indexSections(sections.enum),
        structs: indexSections(sections.struct),
        constants,
        headers,
        blocks: (file) => blockCache.get(file) ?? blockCache.set(file, parseRs2Blocks(file, text(file))).get(file)!,
        text,
        maps: () => (mapList ??= placementMapInputs(content).map((input) => ({ path: input.path, ...parseMapsquarePath(input.path) }))),
    };
}

/** One `==== LOC ====` or `==== NPC ====` row: 1-based line, plane, local x/z, entity id, angle (LOC only). */
export type MapRow = { line: number; plane: number; lx: number; lz: number; id: number; angle: number };

/**
 * Rows of one map section whose id is in `ids`. Fail-closed on a malformed row: bad coordinates, range, or
 * token count. LOC rows are `plane lx lz: id [shape [angle]]`; NPC rows are `plane lx lz: id`.
 */
export function scanMapSection(file: string, text: string, section: 'LOC' | 'NPC', ids: ReadonlySet<number>): MapRow[] {
    const out: MapRow[] = [];
    let inSection = false;
    lines(text).forEach((raw, index) => {
        const line = raw.trim();
        if (!line) return;
        if (line.startsWith('==== ') && line.endsWith(' ====')) {
            inSection = line.slice('==== '.length, -' ===='.length) === section;
            return;
        }
        if (!inSection) return;
        const colon = line.indexOf(':');
        if (colon <= 0) throw new Error(`${file}:${index + 1}: malformed ${section} row ${line}`);
        const coords = line.slice(0, colon).trim().split(/\s+/);
        const data = line.slice(colon + 1).trim().split(/\s+/).filter(Boolean);
        if (coords.length !== 3) throw new Error(`${file}:${index + 1}: bad ${section} coords ${line}`);
        const [plane, lx, lz] = coords.map(Number);
        if (![plane, lx, lz].every(Number.isInteger) || plane < 0 || plane > 3 || lx < 0 || lx > 63 || lz < 0 || lz > 63) throw new Error(`${file}:${index + 1}: ${section} coordinates out of range ${line}`);
        const max = section === 'LOC' ? 3 : 1;
        if (data.length === 0 || data.length > max) throw new Error(`${file}:${index + 1}: ${section} row has ${data.length} tokens ${line}`);
        const [id, shape, angle] = data.map(Number);
        if (![id, ...(section === 'LOC' ? [shape ?? 0, angle ?? 0] : [])].every(Number.isInteger)) throw new Error(`${file}:${index + 1}: non-integer ${section} token ${line}`);
        if (!ids.has(id)) return;
        out.push({ line: index + 1, plane, lx, lz, id, angle: section === 'LOC' ? angle ?? 0 : 0 });
    });
    return out;
}

