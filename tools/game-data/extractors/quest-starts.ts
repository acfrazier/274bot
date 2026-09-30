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

function expandedBody(handler: Section, labels: ReadonlyMap<string, Section>): string {
    const names = new Set(labels.keys());
    const seen = new Set<string>();
    const pieces = [handler.body];
    const visit = (body: string, depth: number) => {
        if (depth >= 8) return;
        for (const name of labelReferences(body, names)) {
            if (!seen.add(name)) continue;
            const label = labels.get(name);
            if (!label) continue;
            pieces.push(label.body);
            visit(label.body, depth + 1);
        }
    };
    visit(handler.body, 0);
    return pieces.join('\n');
}

function notStartedNames(content: string): Set<string> {
    const constants = fs.readFileSync(path.join(content, 'scripts/general/configs/quest.constant'), 'utf8');
    const names = new Set<string>();
    for (const raw of constants.split(/\r?\n/)) {
        const match = /^\^([A-Za-z0-9_]+_not_started)\s*=/.exec(raw.trim());
        if (match) names.add(match[1]);
    }
    return names;
}

function firstStartAssignment(body: string, varp: string, constants: ReadonlySet<string>): boolean {
    const varpStem = varp.replace(/(?:_quest|quest|_bits)$/, '');
    const lines = body.split(/\r?\n/);
    let guard = false;
    for (const raw of lines) {
        const line = raw.trim();
        if (!line || line.startsWith('//')) continue;
        const names = [...line.matchAll(/\^([A-Za-z0-9_]+_not_started)\b/g)].map((match) => match[1]);
        const directGuard = new RegExp(`%${varp}\\s*=\\s*\\^\\w+_not_started\\b`).test(line);
        const caseGuard = /^\s*case\b/.test(line);
        if (names.some((name) => {
            const stem = name.slice(0, -'_not_started'.length);
            return (directGuard || caseGuard) && (constants.has(name) || stem === varp || stem === varpStem || stem.startsWith(varpStem) || varpStem.startsWith(stem));
        })) guard = true;
        if (new RegExp(`%${varp}\\s*=\\s*0\\b`).test(line)) guard = true;
        const assignment = ASSIGNMENT.exec(line);
        if (!assignment || assignment[1] !== varp) continue;
        const value = assignment[2] ?? assignment[3];
        if (value === undefined || value === '0' || value.endsWith('_not_started') || constants.has(value)) continue;
        if (guard) return true;
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
 * whose first assignment to that quest's progress var leaves a not-started guard.
 * This deliberately does not interpret RuneScript control flow; uncertain or
 * indirect handlers stay out of the facts and therefore keep map labels generic.
 */
export function extractQuestStartFacts(
    content: string,
    quests: readonly QuestIdentityRow[],
    revision: number,
    contentId: string,
): QuestStartFacts {
    const files = walkContentFiles(path.join(content, 'scripts'), '.rs2')
        .filter((file) => !file.includes('/journal') && !file.includes('/_test/') && !file.includes('/_unpack/'));
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
    const constants = notStartedNames(content);
    const pack = packs(content);
    const rows: QuestStartRow[] = [];
    for (const quest of quests) {
        if (!/^\w+$/.test(quest.varp)) continue;
        for (const handler of handlers) {
            if (!firstStartAssignment(expandedBody(handler, labels), quest.varp, constants)) continue;
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
