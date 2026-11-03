import { stageConditionCases } from './prereq-preservation.ts';
import type { Node } from './gathering-rs2.ts';
import type { RangeWire } from './prereq-steps.ts';

type Domain = { min: number; max: number } | null;
type Guard = { condition: string; polarity: boolean };
type PathState = { values: Map<string, string | null>; sinks: string[] };

function literal(value: string): string | null {
    if (!/^"(?:[^"\\]|\\.)*"$/.test(value)) return null;
    try {
        const parsed: unknown = JSON.parse(value);
        return typeof parsed === 'string' ? parsed : null;
    } catch {
        return null;
    }
}

function normalizeJournalText(text: string) {
    return text.replace(/@[A-Za-z0-9_]+@/g, '').replace(/\|/g, ' ').replace(/\s+/g, ' ').trim().toLowerCase();
}

function validate(nodes: readonly Node[], citedLines: ReadonlySet<number>): string[] | null {
    const conditions: string[] = [];
    const visit = (list: readonly Node[]): boolean => {
        for (const node of list) {
            if (node.kind === 'while') return false;
            if (node.kind === 'if') {
                if (!citedLines.has(node.line)) return false;
                conditions.push(node.cond);
                if (!visit(node.then) || !visit(node.els)) return false;
                continue;
            }
            if (!citedLines.has(node.line)) return false;
            const text = node.text;
            if (/^def_string\s+\$[A-Za-z_][A-Za-z0-9_]*$/.test(text)) continue;
            if (/^def_string\s+\$[A-Za-z_][A-Za-z0-9_]*\s*=\s*"(?:[^"\\]|\\.)*"$/.test(text)) continue;
            if (/^\$[A-Za-z_][A-Za-z0-9_]*\s*=\s*"(?:[^"\\]|\\.)*"$/.test(text)) continue;
            if (/^\$[A-Za-z_][A-Za-z0-9_]*\s*=\s*append\(\$[A-Za-z_][A-Za-z0-9_]*,\s*"(?:[^"\\]|\\.)*"\)$/.test(text)) continue;
            if (/^~quest_journal\("(?:[^"\\]|\\.)*",\s*\$[A-Za-z_][A-Za-z0-9_]*\)$/.test(text)) continue;
            return false;
        }
        return true;
    };
    return visit(nodes) ? conditions : null;
}

function applyStatement(text: string, state: PathState): boolean {
    let match = /^def_string\s+(\$[A-Za-z_][A-Za-z0-9_]*)(?:\s*=\s*("(?:[^"\\]|\\.)*"))?$/.exec(text);
    if (match) {
        if (state.values.has(match[1])) return false;
        const value = match[2] === undefined ? null : literal(match[2]);
        if (match[2] !== undefined && value === null) return false;
        state.values.set(match[1], value);
        return true;
    }
    match = /^(\$[A-Za-z_][A-Za-z0-9_]*)\s*=\s*("(?:[^"\\]|\\.)*")$/.exec(text);
    if (match) {
        if (!state.values.has(match[1])) return false;
        const value = literal(match[2]);
        if (value === null) return false;
        state.values.set(match[1], value);
        return true;
    }
    match = /^(\$[A-Za-z_][A-Za-z0-9_]*)\s*=\s*append\((\$[A-Za-z_][A-Za-z0-9_]*),\s*("(?:[^"\\]|\\.)*")\)$/.exec(text);
    if (match) {
        if (match[1] !== match[2] || !state.values.has(match[1])) return false;
        const current = state.values.get(match[1]);
        const suffix = literal(match[3]);
        if (current === null || current === undefined || suffix === null) return false;
        state.values.set(match[1], current + suffix);
        return true;
    }
    match = /^~quest_journal\(("(?:[^"\\]|\\.)*"),\s*(\$[A-Za-z_][A-Za-z0-9_]*)\)$/.exec(text);
    if (match) {
        const title = literal(match[1]);
        const page = state.values.get(match[2]);
        if (title === null || page === null || page === undefined) return false;
        state.sinks.push(page);
        return true;
    }
    return false;
}

function execute(nodes: readonly Node[], states: PathState[], outcomes: ReadonlyMap<string, boolean>): PathState[] | null {
    let active = states;
    for (const node of nodes) {
        if (node.kind === 'while') return null;
        const next: PathState[] = [];
        if (node.kind === 'stmt') {
            for (const state of active) {
                const copy = { values: new Map(state.values), sinks: [...state.sinks] };
                if (!applyStatement(node.text, copy)) return null;
                next.push(copy);
            }
        } else {
            const outcome = outcomes.get(node.cond);
            if (outcome === undefined) return null;
            for (const state of active) {
                const branch = outcome ? node.then : node.els;
                const copy = { values: new Map(state.values), sinks: [...state.sinks] };
                const expanded = execute(branch, [copy], outcomes);
                if (expanded === null) return null;
                next.push(...expanded);
            }
        }
        active = next;
    }
    return active;
}

/** Prove a conservative range where every matching journal page contains all requested needles. */
export function boundedJournalRange(
    nodes: readonly Node[],
    signal: string,
    constants: ReadonlyMap<string, number>,
    domain: Domain,
    citedLines: ReadonlySet<number>,
    needles: readonly string[],
): RangeWire | null {
    if (domain === null || needles.length === 0 || needles.some((needle) => needle.length === 0)) return null;
    const conditions = validate(nodes, citedLines);
    if (conditions === null) return null;
    const cases = stageConditionCases(conditions, signal, domain, constants);
    if (cases === null) return null;

    let min = Number.POSITIVE_INFINITY;
    let max = Number.NEGATIVE_INFINITY;
    for (const conditionCase of cases) {
        const paths = execute(nodes, [{ values: new Map(), sinks: [] }], conditionCase.outcomes);
        if (paths === null || paths.length === 0 || paths.some((path) => path.sinks.length !== 1)) return null;
        if (paths.some((path) => path.sinks.some((page) => needles.every((needle) => normalizeJournalText(page).includes(needle))))) {
            min = Math.min(min, conditionCase.values.min);
            max = Math.max(max, conditionCase.values.max);
        }
    }
    return Number.isFinite(min) ? { min, max } : null;
}
