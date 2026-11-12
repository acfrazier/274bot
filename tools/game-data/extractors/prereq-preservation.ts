import { stripComment, type Rs2Block } from './gathering-content.ts';
import { parseBody, walkStatements, type Node, type PathCond, type Stmt } from './gathering-rs2.ts';
import type { RangeWire } from './prereq-steps.ts';

type Guard = { expression: string; polarity: boolean };
type Statement = { text: string; line: number; guards: Guard[] };
type BlockRef = { id: string; block: Rs2Block; statements: Statement[]; entry: boolean };
type CallEdge = { caller: string; guards: Guard[] };
type ScriptGraph = {
    byId: Map<string, BlockRef>;
    reverse: Map<string, CallEdge[]>;
};
type Compare = { variableOnLeft: boolean; operator: string; value: number };
type BoolExpr = { kind: 'atom'; text: string } | { kind: 'and' | 'or'; left: BoolExpr; right: BoolExpr } | { kind: 'not'; child: BoolExpr };

function splitOperator(text: string, operator: '|' | '&') {
    const parts: string[] = [];
    let depth = 0;
    let inString = false;
    let start = 0;
    for (let index = 0; index < text.length; index += 1) {
        const char = text[index];
        if (char === '"') inString = !inString;
        if (inString) continue;
        if (char === '(') depth += 1;
        else if (char === ')') depth -= 1;
        else if (char === operator && depth === 0) {
            parts.push(text.slice(start, index));
            start = index + 1;
            if (text[index + 1] === operator) index += 1;
        }
    }
    if (parts.length === 0) return [text];
    parts.push(text.slice(start));
    return parts;
}

function outerParens(text: string) {
    let value = text.trim();
    while (value.startsWith('(') && value.endsWith(')')) {
        let depth = 0;
        let inString = false;
        let wraps = true;
        for (let index = 0; index < value.length; index += 1) {
            const char = value[index];
            if (char === '"') inString = !inString;
            if (inString) continue;
            if (char === '(') depth += 1;
            else if (char === ')') {
                depth -= 1;
                if (depth === 0 && index !== value.length - 1) { wraps = false; break; }
            }
        }
        if (!wraps) break;
        value = value.slice(1, -1).trim();
    }
    return value;
}

function parseBool(text: string): BoolExpr {
    const value = outerParens(text);
    const ors = splitOperator(value, '|');
    if (ors.length > 1) return ors.map((part) => parseBool(part)).reduce<BoolExpr>((left, right) => ({ kind: 'or', left, right }));
    const ands = splitOperator(value, '&');
    if (ands.length > 1) return ands.map((part) => parseBool(part)).reduce<BoolExpr>((left, right) => ({ kind: 'and', left, right }));
    if (value.startsWith('!') && !value.startsWith('!=')) return { kind: 'not', child: parseBool(value.slice(1)) };
    return { kind: 'atom', text: value };
}

function constantValue(text: string, constants: ReadonlyMap<string, number>) {
    const value = text.trim();
    if (/^-?\d+$/.test(value)) return Number(value);
    const match = /^\^([A-Za-z0-9_]+)$/.exec(value);
    return match ? constants.get(match[1]) : undefined;
}

function compareFor(text: string, variable: string, constants: ReadonlyMap<string, number>): Compare | null {
    const value = outerParens(text);
    const match = /^(.+?)\s*(>=|<=|!=|==|=|>|<)\s*(.+)$/.exec(value);
    if (!match) return null;
    const left = outerParens(match[1]);
    const right = outerParens(match[3]);
    const target = `%${variable}`;
    if (left === target) {
        const operand = constantValue(right, constants);
        return operand === undefined ? null : { variableOnLeft: true, operator: match[2], value: operand };
    }
    if (right === target) {
        const operand = constantValue(left, constants);
        return operand === undefined ? null : { variableOnLeft: false, operator: match[2], value: operand };
    }
    return null;
}

function reversedOperator(operator: string) {
    return ({ '<': '>', '<=': '>=', '>': '<', '>=': '<=', '=': '=', '==': '==', '!=': '!=' } as Record<string, string>)[operator];
}

function comparisonAt(text: string, variable: string, constants: ReadonlyMap<string, number>) {
    const found = compareFor(text, variable, constants);
    if (!found) return null;
    return found.variableOnLeft ? found : { ...found, operator: reversedOperator(found.operator) };
}

function invertPossible(values: ReadonlySet<boolean>) {
    return new Set([...values].map((value) => !value));
}
function combinePossible(left: ReadonlySet<boolean>, right: ReadonlySet<boolean>, operator: 'and' | 'or') {
    const out = new Set<boolean>();
    for (const a of left) for (const b of right) out.add(operator === 'and' ? a && b : a || b);
    return out;
}

function atomAt(text: string, variable: string, value: number, constants: ReadonlyMap<string, number>): ReadonlySet<boolean> {
    const comparison = comparisonAt(text, variable, constants);
    if (!comparison) return new Set([true, false]);
    let result: boolean;
    switch (comparison.operator) {
        case '=':
        case '==': result = value === comparison.value; break;
        case '!=': result = value !== comparison.value; break;
        case '<': result = value < comparison.value; break;
        case '<=': result = value <= comparison.value; break;
        case '>': result = value > comparison.value; break;
        case '>=': result = value >= comparison.value; break;
        default: return new Set([true, false]);
    }
    return new Set([result]);
}

function possibleAt(expression: BoolExpr, variable: string, value: number, constants: ReadonlyMap<string, number>): ReadonlySet<boolean> {
    if (expression.kind === 'atom') return atomAt(expression.text, variable, value, constants);
    if (expression.kind === 'not') return invertPossible(possibleAt(expression.child, variable, value, constants));
    return combinePossible(
        possibleAt(expression.left, variable, value, constants),
        possibleAt(expression.right, variable, value, constants),
        expression.kind,
    );
}

function criticalValues(guards: readonly Guard[], variable: string, range: RangeWire, constants: ReadonlyMap<string, number>) {
    const min = range.min;
    const max = range.max;
    const values = new Set([min, max]);
    const addBounds = (expression: BoolExpr) => {
        if (expression.kind === 'atom') {
            const comparison = comparisonAt(expression.text, variable, constants);
            if (comparison) for (const candidate of [comparison.value - 1, comparison.value, comparison.value + 1]) {
                if (candidate >= min && candidate <= max) values.add(candidate);
            }
        } else if (expression.kind === 'not') addBounds(expression.child);
        else { addBounds(expression.left); addBounds(expression.right); }
    };
    for (const guard of guards) addBounds(parseBool(guard.expression));
    return [...values].sort((left, right) => left - right);
}

function guardsCanHold(guards: readonly Guard[], variable: string, range: RangeWire, constants: ReadonlyMap<string, number>) {
    const expressions = guards.map((guard) => ({ expression: parseBool(guard.expression), polarity: guard.polarity }));
    return criticalValues(guards, variable, range, constants).some((value) => expressions.every(({ expression, polarity }) => {
        const possibilities = possibleAt(expression, variable, value, constants);
        return possibilities.has(polarity);
    }));
}

function stripStrings(text: string) {
    let inString = false;
    let out = '';
    for (const char of text) {
        if (char === '"') { inString = !inString; out += ' '; }
        else out += inString ? ' ' : char;
    }
    return out;
}

function matchingParen(text: string, open: number) {
    let depth = 0;
    let inString = false;
    for (let index = open; index < text.length; index += 1) {
        const char = text[index];
        if (char === '"') inString = !inString;
        if (inString) continue;
        if (char === '(') depth += 1;
        else if (char === ')') {
            depth -= 1;
            if (depth === 0) return index;
        }
    }
    return -1;
}

function argumentsOf(text: string, open: number) {
    const close = matchingParen(text, open);
    if (close < 0) return [];
    const inner = text.slice(open + 1, close);
    const args: string[] = [];
    let depth = 0;
    let start = 0;
    for (let index = 0; index < inner.length; index += 1) {
        const char = inner[index];
        if (char === '(') depth += 1;
        else if (char === ')') depth -= 1;
        else if (char === ',' && depth === 0) { args.push(inner.slice(start, index).trim()); start = index + 1; }
    }
    args.push(inner.slice(start).trim());
    return args;
}

function callTargets(text: string, labels: ReadonlyMap<string, BlockRef[]>, procs: ReadonlyMap<string, BlockRef[]>, kinds: ReadonlyMap<string, BlockRef[]>) {
    const clean = stripStrings(text);
    const out: BlockRef[] = [];
    const add = (matches: readonly BlockRef[] | undefined) => { if (matches) out.push(...matches); };
    for (const match of clean.matchAll(/@([A-Za-z0-9_]+)/g)) {
        const direct = labels.get(match[1]);
        if (direct) add(direct);
        // Dialogue helpers receive labels as arguments even when the helper
        // itself resolves. Those labels inherit the caller's guarded path.
        const suffix = clean.slice(match.index! + match[0].length);
        if (!suffix.trimStart().startsWith('(')) continue;
        const open = clean.indexOf('(', match.index! + match[0].length);
        const close = matchingParen(clean, open);
        if (close < 0) continue;
        for (const name of clean.slice(open + 1, close).matchAll(/[A-Za-z_][A-Za-z0-9_]*/g)) add(labels.get(name[0]));
    }
    for (const match of clean.matchAll(/~([A-Za-z0-9_]+)/g)) add(procs.get(match[1]));
    for (const name of ['gosub', 'queue', 'weakqueue', 'strongqueue', 'longqueue', 'settimer', 'softtimer']) {
        const expression = new RegExp(`\\b${name}\\s*\\(`, 'g');
        for (const match of clean.matchAll(expression)) {
            const open = clean.indexOf('(', match.index!);
            const first = argumentsOf(clean, open)[0];
            if (!first) continue;
            const kind = name === 'gosub' ? 'label' : name === 'settimer' ? 'timer' : name === 'softtimer' ? 'softtimer' : 'queue';
            add(kinds.get(`${kind}\u0000${first}`));
        }
    }
    if (/\bnpc_queue\s*\(/.test(clean)) add(kinds.get('all_ai_queues\u0000'));
    return [...new Map(out.map((block) => [block.id, block])).values()];
}

function lastLine(nodes: readonly Node[]): number {
    return nodes.reduce((last, node) => Math.max(last, node.kind === 'stmt' ? node.line : node.kind === 'while' ? lastLine(node.body) : Math.max(lastLine(node.then), lastLine(node.els))), 0);
}
function directReturn(nodes: readonly Node[]) {
    const last = nodes.at(-1);
    return last?.kind === 'stmt' && /^return\b/.test(last.text);
}
function priorReturns(nodes: readonly Node[], within = Number.POSITIVE_INFINITY) {
    const out: { through: number; within: number; condition: string; polarity: boolean }[] = [];
    for (const node of nodes) {
        if (node.kind === 'if') {
            const through = lastLine([node]);
            if (directReturn(node.then)) out.push({ through, within, condition: node.cond, polarity: false });
            if (directReturn(node.els)) out.push({ through, within, condition: node.cond, polarity: true });
            out.push(...priorReturns(node.then, lastLine(node.then)));
            out.push(...priorReturns(node.els, lastLine(node.els)));
        } else if (node.kind === 'while') {
            out.push(...priorReturns(node.body, lastLine(node.body)));
        }
    }
    return out;
}

function braceDelta(text: string) {
    const clean = stripStrings(text);
    let delta = 0;
    for (const char of clean) if (char === '{') delta += 1; else if (char === '}') delta -= 1;
    return delta;
}

function resolveCaseValue(text: string, constants: ReadonlyMap<string, number>) {
    return constantValue(text.trim(), constants);
}

type SwitchRange = { variable: string; start: number; end: number };
function switchRanges(block: Rs2Block): SwitchRange[] {
    const ranges: SwitchRange[] = [];
    for (let index = 0; index < block.body.length; index += 1) {
        const code = stripComment(block.body[index].text).trim();
        const start = /\bswitch_int\s*\(\s*%([A-Za-z0-9_]+)\s*\)\s*\{/.exec(code);
        if (!start) continue;
        let depth = 0;
        let end = index;
        for (let current = index; current < block.body.length; current += 1) {
            depth += braceDelta(stripComment(block.body[current].text));
            end = current;
            if (depth === 0) break;
        }
        if (depth !== 0) throw new Error(`${block.span.file}:${block.body[index].line}: unclosed switch_int`);
        ranges.push({ variable: start[1], start: index, end });
    }
    return ranges;
}

function switchStatements(block: Rs2Block, variable: string, constants: ReadonlyMap<string, number>, ranges: readonly SwitchRange[]): Statement[] {
    const out: Statement[] = [];
    for (const range of ranges) {
        const nestedSwitchLines = new Set(ranges
            .filter((nested) => nested.start > range.start && nested.end < range.end)
            .flatMap((nested) => block.body.slice(nested.start, nested.end + 1).map((line) => line.line)));
        let depth = 0;
        const cases: { header: string; body: { line: number; text: string }[] }[] = [];
        let active: { header: string; body: { line: number; text: string }[] } | null = null;
        for (let current = range.start; current <= range.end; current += 1) {
            const raw = block.body[current];
            const code = stripComment(raw.text);
            if (current === range.start) {
                depth = braceDelta(code);
                continue;
            }
            const nextDepth = depth + braceDelta(code);
            if (depth === 1) {
                const header = /^\s*(case\s+(.+?)\s*:|default\s*:)(.*)$/.exec(code);
                if (header) {
                    if (active) cases.push(active);
                    active = { header: header[1], body: [] };
                    const remainder = header[3].trim();
                    if (remainder) active.body.push({ line: raw.line, text: remainder });
                } else if (active && nextDepth !== 0) {
                    active.body.push(raw);
                }
            } else if (active && nextDepth !== 0) {
                active.body.push(raw);
            }
            depth = nextDepth;
        }
        if (active) cases.push(active);
        const caseValues = cases.flatMap((entry) => {
            const match = /^case\s+(.+?)\s*:$/.exec(entry.header);
            return match ? match[1].split(',').map((part) => resolveCaseValue(part, constants)).filter((value): value is number => value !== undefined) : [];
        });
        for (const entry of cases) {
            const match = /^case\s+(.+?)\s*:$/.exec(entry.header);
            let guard: Guard | null = null;
            if (range.variable === variable && match) {
                const values = match[1].split(',').map((part) => resolveCaseValue(part, constants));
                if (values.length > 0 && values.every((value): value is number => value !== undefined)) {
                    guard = { expression: values.map((value) => `%${variable} = ${value}`).join(' | '), polarity: true };
                }
            } else if (range.variable === variable && caseValues.length > 0) {
                guard = { expression: caseValues.map((value) => `%${variable} != ${value}`).join(' & '), polarity: true };
            }
            if (entry.body.length === 0) continue;
            const nodes = parseBody(entry.body);
            const prior = priorReturns(nodes);
            walkStatements(nodes, (statement, path) => {
                if (nestedSwitchLines.has(statement.line)) return;
                const guards = [...(guard ? [guard] : []), ...path.map((part: PathCond) => ({ expression: part.cond, polarity: part.polarity }))];
                for (const previous of prior) if (previous.through < statement.line && statement.line <= previous.within) guards.push({ expression: previous.condition, polarity: previous.polarity });
                out.push({ text: statement.text, line: statement.line, guards });
            });
        }
    }
    return out;
}

function parseStatements(block: Rs2Block, variable: string, constants: ReadonlyMap<string, number>) {
    const nodes = parseBody(block.body);
    const ranges = switchRanges(block);
    const switchLines = new Set(ranges.flatMap((range) => block.body.slice(range.start, range.end + 1).map((line) => line.line)));
    const switches = switchStatements(block, variable, constants, ranges);
    const prior = priorReturns(nodes);
    const statements: Statement[] = [];
    walkStatements(nodes, (statement: Stmt, path: PathCond[]) => {
        if (switchLines.has(statement.line)) return;
        const guards = path.map((part) => ({ expression: part.cond, polarity: part.polarity }));
        for (const previous of prior) if (previous.through < statement.line && statement.line <= previous.within) guards.push({ expression: previous.condition, polarity: previous.polarity });
        statements.push({ text: statement.text, line: statement.line, guards });
    });
    statements.push(...switches);
    return statements;
}

function controlGraph(sourceBlocks: readonly Rs2Block[], variable: string, constants: ReadonlyMap<string, number>) {
    const blocks = sourceBlocks.map((block, index): BlockRef => ({
        id: `${block.span.file}#${index}`,
        block,
        statements: parseStatements(block, variable, constants),
        entry: block.kind !== 'label' && block.kind !== 'proc',
    }));
    const labels = new Map<string, BlockRef[]>();
    const procs = new Map<string, BlockRef[]>();
    const kinds = new Map<string, BlockRef[]>();
    for (const block of blocks) {
        const byName = block.block.kind === 'label' ? labels : block.block.kind === 'proc' ? procs : null;
        if (byName) (byName.get(block.block.name) ?? byName.set(block.block.name, []).get(block.block.name)!).push(block);
        const key = `${block.block.kind}\u0000${block.block.name}`;
        (kinds.get(key) ?? kinds.set(key, []).get(key)!).push(block);
        if (block.block.kind.startsWith('ai_queue')) (kinds.get('all_ai_queues\u0000') ?? kinds.set('all_ai_queues\u0000', []).get('all_ai_queues\u0000')!).push(block);
    }
    const reverse = new Map<string, CallEdge[]>();
    for (const block of blocks) for (const statement of block.statements) {
        for (const target of callTargets(statement.text, labels, procs, kinds)) {
            (reverse.get(target.id) ?? reverse.set(target.id, []).get(target.id)!).push({ caller: block.id, guards: statement.guards });
        }
    }
    return { byId: new Map(blocks.map((block) => [block.id, block])), reverse };
}

function hasEntryPath(writer: { block: BlockRef }, graph: ScriptGraph) {
    const visit = (blockId: string, seen: Set<string>): boolean => {
        const block = graph.byId.get(blockId)!;
        if (block.entry) return true;
        if (seen.has(blockId)) return false;
        const nextSeen = new Set(seen).add(blockId);
        return (graph.reverse.get(blockId) ?? []).some((edge) => visit(edge.caller, nextSeen));
    };
    return visit(writer.block.id, new Set());
}

function feasiblePaths(writer: { block: BlockRef; guards: Guard[] }, graph: ScriptGraph, variable: string, range: RangeWire, constants: ReadonlyMap<string, number>) {
    const out: Guard[][] = [];
    const visit = (blockId: string, guards: Guard[], seen: Set<string>) => {
        const block = graph.byId.get(blockId)!;
        if (block.entry) {
            if (guardsCanHold(guards, variable, range, constants)) out.push(guards);
            return;
        }
        if (seen.has(blockId)) return;
        const nextSeen = new Set(seen).add(blockId);
        for (const edge of graph.reverse.get(blockId) ?? []) {
            const next = [...guards, ...edge.guards];
            if (!guardsCanHold(next, variable, range, constants)) continue;
            visit(edge.caller, next, nextSeen);
        }
    };
    visit(writer.block.id, writer.guards, new Set());
    return out;
}

/** Possible non-idempotent writes from completion; unresolved callers stay possible. */
export function postCompletionProgressWriters(
    blocks: readonly Rs2Block[],
    variable: string,
    completed: RangeWire,
    constants: ReadonlyMap<string, number>,
    candidates: readonly { block: Rs2Block; line: number; right: string }[],
) {
    const graph: ScriptGraph = controlGraph(blocks, variable, constants);
    const byBlock = new Map([...graph.byId.values()].map((block) => [block.block, block]));
    const assignment = new RegExp(`^%${variable}\\s*=\\s*(.+?)\\s*;?$`);
    const possible: { file: string; line: number; right: string }[] = [];
    for (const candidate of candidates) {
        if (completed.min === completed.max && constantValue(candidate.right, constants) === completed.min) continue;
        const site = { file: candidate.block.span.file, line: candidate.line, right: candidate.right };
        const block = byBlock.get(candidate.block);
        const statement = block?.statements.find((statement) => {
            const match = assignment.exec(statement.text);
            return statement.line === candidate.line && match?.[1].trim() === candidate.right;
        });
        if (!block || !statement) {
            possible.push(site);
            continue;
        }
        const writer = { block, guards: statement.guards };
        // Expression calls may not have an edge. Missing callers cannot prove
        // that a writer is dead, just as for journal preservation.
        if (!hasEntryPath(writer, graph)) block.entry = true;
        if (feasiblePaths(writer, graph, variable, completed, constants).length > 0) {
            possible.push(site);
        }
    }
    return possible;
}
