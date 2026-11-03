import { stageConditionCases } from './prereq-preservation.ts';
import type { Node } from './gathering-rs2.ts';
import type { RangeWire } from './prereq-steps.ts';

type Domain = { min: number; max: number } | null;
type PathState = { values: Map<string, string | null>; sinks: string[] };
type Operation =
    | { kind: 'declare'; name: string; value: string | null }
    | { kind: 'set'; name: string; value: string }
    | { kind: 'append'; name: string; source: string; suffix: string }
    | { kind: 'journal'; name: string };
type ProgramNode =
    | { kind: 'stmt'; operation: Operation }
    | { kind: 'if'; cond: string; then: ProgramNode[]; els: ProgramNode[] };

function literal(value: string): string | null {
    if (value.includes('\\') || !/^"[^"\\]*"$/.test(value)) return null;
    try {
        const parsed: unknown = JSON.parse(value);
        return typeof parsed === 'string' && !/<[^>]*>/.test(parsed) ? parsed : null;
    } catch {
        return null;
    }
}

function normalizeJournalText(text: string) {
    return text.replace(/@[A-Za-z0-9_]+@/g, '').replace(/\|/g, ' ').replace(/\s+/g, ' ').trim().toLowerCase();
}

function parseStatement(text: string): Operation | null {
    let match = /^def_string\s+(\$[A-Za-z_][A-Za-z0-9_]*)(?:\s*=\s*("(?:[^"\\]|\\.)*"))?$/.exec(text);
    if (match) {
        const value = match[2] === undefined ? null : literal(match[2]);
        if (match[2] !== undefined && value === null) return null;
        return { kind: 'declare', name: match[1], value };
    }
    match = /^(\$[A-Za-z_][A-Za-z0-9_]*)\s*=\s*("(?:[^"\\]|\\.)*")$/.exec(text);
    if (match) {
        const value = literal(match[2]);
        return value === null ? null : { kind: 'set', name: match[1], value };
    }
    match = /^(\$[A-Za-z_][A-Za-z0-9_]*)\s*=\s*append\((\$[A-Za-z_][A-Za-z0-9_]*),\s*("(?:[^"\\]|\\.)*")\)$/.exec(text);
    if (match) {
        const suffix = literal(match[3]);
        return suffix === null || match[1] !== match[2]
            ? null
            : { kind: 'append', name: match[1], source: match[2], suffix };
    }
    match = /^~quest_journal\(("(?:[^"\\]|\\.)*"),\s*(\$[A-Za-z_][A-Za-z0-9_]*)\)$/.exec(text);
    if (match) {
        const title = literal(match[1]);
        return title === null ? null : { kind: 'journal', name: match[2] };
    }
    return null;
}

function validate(nodes: readonly Node[], citedLines: ReadonlySet<number>): { program: ProgramNode[]; conditions: string[] } | null {
    const conditions: string[] = [];
    const visit = (list: readonly Node[]): ProgramNode[] | null => {
        const program: ProgramNode[] = [];
        for (const node of list) {
            if (node.kind === 'while') return null;
            if (node.kind === 'if') {
                if (!citedLines.has(node.line)) return null;
                const then = visit(node.then);
                const els = visit(node.els);
                if (then === null || els === null) return null;
                conditions.push(node.cond);
                program.push({ kind: 'if', cond: node.cond, then, els });
                continue;
            }
            if (!citedLines.has(node.line)) return null;
            const operation = parseStatement(node.text);
            if (operation === null) return null;
            program.push({ kind: 'stmt', operation });
        }
        return program;
    };
    const program = visit(nodes);
    return program === null ? null : { program, conditions };
}

function applyStatement(operation: Operation, state: PathState): boolean {
    if (operation.kind === 'declare') {
        if (state.values.has(operation.name)) return false;
        state.values.set(operation.name, operation.value);
    } else if (operation.kind === 'set') {
        if (!state.values.has(operation.name)) return false;
        state.values.set(operation.name, operation.value);
    } else if (operation.kind === 'append') {
        const current = state.values.get(operation.source);
        if (operation.name !== operation.source || current === null || current === undefined) return false;
        state.values.set(operation.name, current + operation.suffix);
    } else {
        const page = state.values.get(operation.name);
        if (page === null || page === undefined) return false;
        state.sinks.push(page);
    }
    return true;
}

function execute(nodes: readonly ProgramNode[], state: PathState, outcomes: ReadonlyMap<string, boolean>): boolean {
    for (const node of nodes) {
        if (node.kind === 'stmt') {
            if (!applyStatement(node.operation, state)) return false;
        } else {
            const outcome = outcomes.get(node.cond);
            if (outcome === undefined || !execute(outcome ? node.then : node.els, state, outcomes)) return false;
        }
    }
    return true;
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
    const validated = validate(nodes, citedLines);
    if (validated === null) return null;
    const cases = stageConditionCases(validated.conditions, signal, domain, constants);
    if (cases === null) return null;

    let min = Number.POSITIVE_INFINITY;
    let max = Number.NEGATIVE_INFINITY;
    for (const conditionCase of cases) {
        const state: PathState = { values: new Map(), sinks: [] };
        if (!execute(validated.program, state, conditionCase.outcomes) || state.sinks.length !== 1) return null;
        const page = normalizeJournalText(state.sinks[0]);
        if (needles.every((needle) => page.includes(needle))) {
            min = Math.min(min, conditionCase.values.min);
            max = Math.max(max, conditionCase.values.max);
        }
    }
    return Number.isFinite(min) ? { min, max } : null;
}
