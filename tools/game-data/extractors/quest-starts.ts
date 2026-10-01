import fs from 'node:fs';
import path from 'node:path';
import { parsePack, walkContentFiles } from './common.ts';

type QuestIdentityRow = {
    id: string;
    varp: string;
};

type Section = {
    kind: 'npc' | 'loc' | 'label';
    op: number;
    alias: string;
    body: string;
};

type ExpandedLine =
    | { kind: 'source'; text: string }
    | { kind: 'label-start'; guardLine?: string }
    | { kind: 'label-end' };

type Scope = {
    kind: 'block' | 'switch' | 'label';
    guard: boolean;
    terminated: boolean;
};

export type QuestStartTarget = {
    kind: 'npc' | 'loc';
    id: number;
};

export type QuestStartRow = {
    quest: string;
    target: QuestStartTarget;
    op: number;
};

export type QuestStartFacts = {
    schema: 1;
    revision: number;
    content_id: string;
    rows: QuestStartRow[];
};

const HANDLER = /^op(npc|loc)([1-5])$/;
const HEADER = /^\[([^,\]]+),([^\]]+)\]\s*$/;
const ASSIGNMENT = /^\s*%(\w+)\s*=\s*(?:\^([A-Za-z0-9_]+)|(-?\d+))\s*;?\s*$/;
const DIRECT_LABEL = /@([A-Za-z_][A-Za-z0-9_]*)/g;
const MULTI_CALL = /@multi\d+(?:_header)?\(([^)]*)\)/g;
const TOKEN = /(?:^|,)\s*([A-Za-z_][A-Za-z0-9_]*)\s*(?=,|$)/g;

function sections(text: string): Section[] {
    const out: Section[] = [];
    let current: { kind: Section['kind']; op: number; alias: string; lines: string[] } | null = null;
    const flush = () => {
        if (!current) return;
        out.push({ ...current, body: current.lines.join('\n') });
        current = null;
    };
    for (const raw of text.split(/\r?\n/)) {
        const line = raw.trim();
        const match = HEADER.exec(line);
        if (match) {
            flush();
            const handler = HANDLER.exec(match[1]);
            if (handler) {
                current = {
                    kind: handler[1] as 'npc' | 'loc',
                    op: Number(handler[2]),
                    alias: match[2].trim(),
                    lines: [],
                };
            } else if (match[1] === 'label') {
                current = {
                    kind: 'label',
                    op: 0,
                    alias: match[2].trim(),
                    lines: [],
                };
            }
            continue;
        }
        if (current) current.lines.push(raw);
    }
    flush();
    return out;
}

function labelReferences(body: string, names: ReadonlySet<string>): string[] {
    const refs: string[] = [];
    for (const match of body.matchAll(DIRECT_LABEL)) {
        if (names.has(match[1])) refs.push(match[1]);
    }
    for (const match of body.matchAll(MULTI_CALL)) {
        for (const token of match[1].matchAll(TOKEN)) {
            if (names.has(token[1])) refs.push(token[1]);
        }
    }
    return [...new Set(refs)];
}

function expandedBody(handler: Section, labels: ReadonlyMap<string, Section>): ExpandedLine[] {
    const names = new Set(labels.keys());
    const expand = (body: string, depth: number, active: ReadonlySet<string>): ExpandedLine[] => {
        const out: ExpandedLine[] = [];
        for (const raw of body.split(/\r?\n/)) {
            out.push({ kind: 'source', text: raw });
            const refs = labelReferences(raw, names);
            const inlineGuard = refs.length === 1
                && /^\s*(?:else\s+)?if\b/.test(raw)
                && !raw.includes('{')
                && !/\belse\b/.test(raw)
                ? raw
                : undefined;
            for (const name of refs) {
                if (active.has(name)) continue;
                const label = labels.get(name);
                if (!label || depth >= 8) continue;
                out.push({ kind: 'label-start', guardLine: inlineGuard });
                const nextActive = new Set(active);
                nextActive.add(name);
                out.push(...expand(label.body, depth + 1, nextActive));
                out.push({ kind: 'label-end' });
            }
        }
        return out;
    };
    return expand(handler.body, 0, new Set());
}

export function questStartContentFiles(content: string): string[] {
    return walkContentFiles(path.join(content, 'scripts'), '.rs2')
        .filter((file) => !file.includes('/journal') && !file.includes('/_test/') && !file.includes('/_unpack/'))
        .map((file) => path.relative(content, file).split(path.sep).join('/'));
}

function firstStartAssignment(lines: readonly ExpandedLine[], questId: string, varp: string): boolean {
    const varpStem = varp.replace(/(?:_quest|quest|_bits)$/, '');
    const guardForLine = (line: string) => {
        const names = [...line.matchAll(/\^([A-Za-z0-9_]+_not_started)\b/g)].map((match) => match[1]);
        const isNotStarted = names.some((name) => {
            const stem = name.slice(0, -'_not_started'.length);
            return stem === questId || stem === varp || stem === varpStem;
        });
        const isIf = /^\s*(?:else\s+)?if\b/.test(line);
        // A numeric zero is only a branch-local guard; a bare assignment or early return cannot latch it.
        const zeroGuard = isIf && new RegExp(`%${varp}\\s*=\\s*0\\b`).test(line);
        return isIf && (isNotStarted || zeroGuard);
    };
    const scopes: Scope[] = [];
    for (const expanded of lines) {
        if (expanded.kind === 'label-start') {
            scopes.push({ kind: 'label', guard: expanded.guardLine !== undefined && guardForLine(expanded.guardLine), terminated: false });
            continue;
        }
        if (expanded.kind === 'label-end') {
            if (scopes.at(-1)?.kind === 'label') scopes.pop();
            continue;
        }
        const line = expanded.text.trim();
        if (!line || line.startsWith('//')) continue;
        if (/^case\b/.test(line)) {
            const switchScope = [...scopes].reverse().find((scope) => scope.kind === 'switch');
            if (switchScope) {
                const names = [...line.matchAll(/\^([A-Za-z0-9_]+_not_started)\b/g)].map((match) => match[1]);
                switchScope.guard = names.some((name) => {
                    const stem = name.slice(0, -'_not_started'.length);
                    return stem === questId || stem === varp || stem === varpStem;
                });
                switchScope.terminated = false;
            }
        }
        const assignment = ASSIGNMENT.exec(line);
        if (assignment && assignment[1] === varp) {
            const value = assignment[2] ?? assignment[3];
            if (value !== undefined && value !== '0' && !value.endsWith('_not_started')
                && scopes.some((scope) => scope.guard && !scope.terminated)) {
                return true;
            }
        }
        if (/^return\b/.test(line)) {
            for (let index = scopes.length - 1; index >= 0; index -= 1) {
                if (!scopes[index].guard) continue;
                scopes[index].terminated = true;
                break;
            }
        }
        const code = line.replace(/"[^"]*"/g, '');
        let firstOpen = true;
        for (const token of code.matchAll(/[{}]/g)) {
            if (token[0] === '}') {
                scopes.pop();
                continue;
            }
            const prefix = code.slice(0, token.index).replace(/^\s*}\s*/, '').trim();
            const switchBlock = firstOpen && /^switch(?:_int)?\b/.test(prefix);
            scopes.push({
                kind: switchBlock ? 'switch' : 'block',
                guard: !switchBlock && firstOpen && guardForLine(line),
                terminated: false,
            });
            firstOpen = false;
        }
    }
    return false;
}

function packs(content: string) {
    return {
        npc: parsePack(fs.readFileSync(path.join(content, 'pack/npc.pack'), 'utf8')),
        loc: parsePack(fs.readFileSync(path.join(content, 'pack/loc.pack'), 'utf8')),
    };
}

/**
 * Find only the simple, content-visible starter relation: an NPC/LOC op handler
 * whose assignment to that quest's progress var stays in the same not-started
 * switch arm or conditional branch. This deliberately leaves ambiguous or
 * indirect handlers out of the facts so map labels remain generic.
 */
export function extractQuestStartFacts(
    content: string,
    quests: readonly QuestIdentityRow[],
    revision: number,
    contentId: string,
): QuestStartFacts {
    const files = questStartContentFiles(content).map((file) => path.join(content, file));
    const handlers: Section[] = [];
    const labels = new Map<string, Section>();
    const duplicateLabels = new Set<string>();
    for (const file of files) {
        for (const section of sections(fs.readFileSync(file, 'utf8'))) {
            if (section.kind === 'label') {
                if (labels.has(section.alias)) duplicateLabels.add(section.alias);
                else labels.set(section.alias, section);
            } else handlers.push(section);
        }
    }
    for (const name of duplicateLabels) labels.delete(name);
    const pack = packs(content);
    const rows: QuestStartRow[] = [];
    for (const quest of quests) {
        if (!/^\w+$/.test(quest.varp)) continue;
        for (const handler of handlers) {
            if (!firstStartAssignment(expandedBody(handler, labels), quest.id, quest.varp)) continue;
            const ids = pack[handler.kind].get(handler.alias);
            if (ids === undefined) continue;
            rows.push({ quest: quest.id, target: { kind: handler.kind, id: ids }, op: handler.op });
        }
    }
    const unique = new Map<string, QuestStartRow>();
    for (const row of rows) unique.set(JSON.stringify([row.quest, row.target.kind, row.target.id, row.op]), row);
    rows.splice(0, rows.length, ...[...unique.values()].sort((a, b) =>
        a.quest.localeCompare(b.quest)
        || a.target.kind.localeCompare(b.target.kind)
        || a.target.id - b.target.id
        || a.op - b.op));
    return { schema: 1, revision, content_id: contentId, rows };
}
