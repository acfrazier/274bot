import { stripComment, type Rs2Block } from './gathering-content.ts';
import { parseBody, walkStatements, type Node, type PathCond, type Stmt } from './gathering-rs2.ts';
import type { RangeWire } from './prereq-steps.ts';
import { createProgressValueResolver, type Guard, type ProgressValueResolver } from './progress-values.ts';

type Statement = { text: string; line: number; guards: Guard[] };
const selectorResolvers = new WeakMap<readonly Rs2Block[], WeakMap<ReadonlyMap<string, number>, ProgressValueResolver>>();
function selectorResolver(blocks: readonly Rs2Block[], constants: ReadonlyMap<string, number>): ProgressValueResolver {
    let byConstants = selectorResolvers.get(blocks);
    if (!byConstants) {
        byConstants = new WeakMap();
        selectorResolvers.set(blocks, byConstants);
    }
    let resolver = byConstants.get(constants);
    if (resolver) return resolver;
    const statements = new Map<Rs2Block, Statement[]>();
    resolver = createProgressValueResolver(blocks, constants, (block) => {
        let cached = statements.get(block);
        if (!cached) {
            cached = parseProgressStatements(block, '', constants);
            statements.set(block, cached);
        }
        return cached;
    }, argumentsOf, stripStrings);
    byConstants.set(constants, resolver);
    return resolver;
}
type BlockRef = { id: string; block: Rs2Block; statements: Statement[]; entry: boolean };
type CallEdge = { caller: string; guards: Guard[] };
type ScriptGraph = {
    byId: Map<string, BlockRef>;
    reverse: Map<string, CallEdge[]>;
    queueTargetsComplete: boolean;
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
function negatedOperator(operator: string) {
    return ({ '<': '>=', '<=': '>', '>': '<=', '>=': '<', '=': '!=', '==': '!=', '!=': '=' } as Record<string, string>)[operator];
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

/** Partition a bounded domain without enumerating its values. Unknown atoms refuse the proof. */
export function stageConditionCases(
    conditions: readonly string[],
    variable: string,
    range: RangeWire,
    constants: ReadonlyMap<string, number>,
): { values: RangeWire; outcomes: ReadonlyMap<string, boolean> }[] | null {
    if (!Number.isSafeInteger(range.min) || !Number.isSafeInteger(range.max)
        || range.min > range.max || range.max >= Number.MAX_SAFE_INTEGER) return null;
    const unique = [...new Set(conditions)];
    const expressions = unique.map((text) => ({ text, expression: parseBool(text) }));
    const known = (expression: BoolExpr): boolean => {
        if (expression.kind === 'atom') {
            const comparison = comparisonAt(expression.text, variable, constants);
            return comparison !== null && Number.isSafeInteger(comparison.value);
        }
        if (expression.kind === 'not') return known(expression.child);
        return known(expression.left) && known(expression.right);
    };
    if (!expressions.every(({ expression }) => known(expression))) return null;
    const boundaries = new Set([range.min, range.max + 1]);
    for (const value of criticalValues(unique.map((expression) => ({ expression, polarity: true })), variable, range, constants)) {
        boundaries.add(value);
        boundaries.add(value + 1);
    }
    const ordered = [...boundaries].sort((left, right) => left - right);
    const cases: { values: RangeWire; outcomes: ReadonlyMap<string, boolean> }[] = [];
    for (let index = 0; index + 1 < ordered.length; index += 1) {
        const values = { min: ordered[index], max: ordered[index + 1] - 1 };
        const outcomes = new Map<string, boolean>();
        for (const { text, expression } of expressions) {
            const possible = possibleAt(expression, variable, values.min, constants);
            if (possible.size !== 1) return null;
            const value = possible.values().next().value;
            if (typeof value !== 'boolean') return null;
            outcomes.set(text, value);
        }
        cases.push({ values, outcomes });
    }
    return cases;
}

function negate(expression: BoolExpr): BoolExpr {
    if (expression.kind === 'not') return expression.child;
    if (expression.kind === 'and' || expression.kind === 'or') return {
        kind: expression.kind === 'and' ? 'or' : 'and',
        left: negate(expression.left),
        right: negate(expression.right),
    };
    return { kind: 'not', child: expression };
}

function dnf(expression: BoolExpr): Literal[][] {
    if (expression.kind === 'atom') return [[{ text: expression.text, negated: false }]];
    if (expression.kind === 'not' && expression.child.kind === 'atom') return [[{ text: expression.child.text, negated: true }]];
    if (expression.kind === 'not') return dnf(negate(expression.child));
    const left = dnf(expression.left);
    const right = dnf(expression.right);
    if (expression.kind === 'or') return [...left, ...right];
    return left.flatMap((a) => right.map((b) => [...a, ...b]));
}
type VariableComparison = { variable: string; operator: string; value: number };

function comparisonForAnyVariable(text: string, constants: ReadonlyMap<string, number>): VariableComparison | null {
    const value = outerParens(text);
    const match = /^(.+?)\s*(>=|<=|!=|==|=|>|<)\s*(.+)$/.exec(value);
    if (!match) return null;
    const left = outerParens(match[1]);
    const right = outerParens(match[3]);
    const leftVariable = /^%([A-Za-z0-9_]+)$/.exec(left);
    const rightVariable = /^%([A-Za-z0-9_]+)$/.exec(right);
    if (leftVariable) {
        const operand = constantValue(right, constants);
        return operand !== undefined && Number.isSafeInteger(operand)
            ? { variable: leftVariable[1], operator: match[2], value: operand }
            : null;
    }
    if (rightVariable) {
        const operand = constantValue(left, constants);
        return operand !== undefined && Number.isSafeInteger(operand)
            ? { variable: rightVariable[1], operator: reversedOperator(match[2]), value: operand }
            : null;
    }
    return null;
}

function supportedBooleanExpression(expression: BoolExpr, constants: ReadonlyMap<string, number>): boolean {
    if (expression.kind === 'atom') return comparisonForAnyVariable(expression.text, constants) !== null;
    if (expression.kind === 'not') return supportedBooleanExpression(expression.child, constants);
    return supportedBooleanExpression(expression.left, constants)
        && supportedBooleanExpression(expression.right, constants);
}

function constrainedClauseCanHold(
    clause: readonly Literal[],
    variable: string,
    range: RangeWire,
    constants: ReadonlyMap<string, number>,
    domainForVariable?: (name: string) => RangeWire | null,
) {
    if (!Number.isSafeInteger(range.min) || !Number.isSafeInteger(range.max) || range.min > range.max) return true;
    type IntegerDomain = { min: bigint | null; max: bigint | null; excluded: Set<bigint> };
    const domains = new Map<string, IntegerDomain>();
    const domainFor = (name: string) => {
        let domain = domains.get(name);
        if (!domain) {
            const bounds = name === variable ? range : domainForVariable?.(name) ?? null;
            if (bounds === null || !Number.isSafeInteger(bounds.min) || !Number.isSafeInteger(bounds.max) || bounds.min > bounds.max) {
                domain = { min: null, max: null, excluded: new Set() };
            } else {
                domain = { min: BigInt(bounds.min), max: BigInt(bounds.max), excluded: new Set() };
            }
            domains.set(name, domain);
        }
        return domain;
    };
    for (const literal of clause) {
        const comparison = comparisonForAnyVariable(literal.text, constants);
        // Unknown guards stay possible unless supported constraints contradict the path.
        if (!comparison) continue;
        const operator = literal.negated ? negatedOperator(comparison.operator) : comparison.operator;
        const domain = domainFor(comparison.variable);
        const value = BigInt(comparison.value);
        if (operator === '=' || operator === '==') {
            domain.min = domain.min === null || value > domain.min ? value : domain.min;
            domain.max = domain.max === null || value < domain.max ? value : domain.max;
        } else if (operator === '!=') {
            domain.excluded.add(value);
        } else if (operator === '>') {
            const min = value + 1n;
            domain.min = domain.min === null || min > domain.min ? min : domain.min;
        } else if (operator === '>=') {
            domain.min = domain.min === null || value > domain.min ? value : domain.min;
        } else if (operator === '<') {
            const max = value - 1n;
            domain.max = domain.max === null || max < domain.max ? max : domain.max;
        } else if (operator === '<=') {
            domain.max = domain.max === null || value < domain.max ? value : domain.max;
        } else {
            return true;
        }
        if (domain.min !== null && domain.max !== null && domain.min > domain.max) return false;
    }
    for (const domain of domains.values()) {
        if (domain.min === null || domain.max === null) continue;
        const size = domain.max - domain.min + 1n;
        const excluded = [...domain.excluded].filter((value) => value >= domain.min! && value <= domain.max!).length;
        if (size <= BigInt(excluded)) return false;
    }
    return true;
}

function guardsCanHoldWithPrerequisite(
    guards: readonly Guard[],
    variable: string,
    range: RangeWire,
    constants: ReadonlyMap<string, number>,
    prerequisite: BoolExpr,
    domainForVariable?: (name: string) => RangeWire | null,
) {
    const expressions = guards.map(({ expression, polarity }) => {
        const parsed = parseBool(expression);
        return polarity ? parsed : negate(parsed);
    });
    expressions.push(prerequisite);
    if (!supportedBooleanExpression(prerequisite, constants)) return true;
    const combined = expressions.slice(1).reduce<BoolExpr>((left, right) => ({ kind: 'and', left, right }), expressions[0]);
    return dnf(combined).some((clause) => constrainedClauseCanHold(clause, variable, range, constants, domainForVariable));
}

/** Read a quest's exact boolean requirement proc as a content-derived completion constraint. */
export function questCompletionPrerequisites(
    blocks: readonly Rs2Block[],
    quest: string,
    constants: ReadonlyMap<string, number>,
): string | null {
    const directory = `scripts/quests/quest_${quest}/`;
    const procedures = blocks.filter((block) =>
        block.kind === 'proc'
        && block.name === `has_${quest}_quest_requirements`
        && block.span.file.replace(/\\/g, '/').startsWith(directory));
    if (procedures.length !== 1) return null;
    let nodes: Node[];
    try {
        nodes = parseBody(procedures[0].body);
    } catch {
        return null;
    }
    if (nodes.length !== 2 || nodes[0].kind !== 'if' || nodes[0].els.length !== 0 || nodes[0].then.length !== 1) return null;
    const failed = nodes[0].then[0];
    const success = nodes[1];
    if (failed.kind !== 'stmt' || !/^return\s*(?:\(\s*false\s*\)|false)\s*;?$/.test(failed.text)
        || success.kind !== 'stmt' || !/^return\s*(?:\(\s*true\s*\)|true)\s*;?$/.test(success.text)) return null;
    const condition = parseBool(nodes[0].cond);
    if (!supportedBooleanExpression(condition, constants)) return null;
    return `!(${nodes[0].cond})`;
}

function literalComparison(literal: Literal, variable: string, constants: ReadonlyMap<string, number>) {
    let comparison = comparisonAt(literal.text, variable, constants);
    if (!comparison) return null;
    if (literal.negated) comparison = { ...comparison, operator: negatedOperator(comparison.operator) };
    return comparison;
}

function feasibleClause(clause: readonly Literal[], variable: string, value: number, constants: ReadonlyMap<string, number>) {
    return clause.every((literal) => {
        const expression = parseBool(literal.text);
        const possible = possibleAt(expression, variable, value, constants);
        return possible.has(!literal.negated);
    });
}

function upperBound(guards: readonly Guard[], variable: string, range: RangeWire, constants: ReadonlyMap<string, number>) {
    const expression: BoolExpr = guards.map((guard) => guard.polarity ? parseBool(guard.expression) : negate(parseBool(guard.expression)))
        .reduce<BoolExpr>((left, right) => ({ kind: 'and', left, right }), { kind: 'atom', text: 'true' });
    let maximum: number | null = null;
    for (const clause of dnf(expression)) {
        if (!criticalValues(guards, variable, range, constants).some((value) => feasibleClause(clause, variable, value, constants))) continue;
        let clauseUpper: number | null = null;
        for (const literal of clause) {
            const comparison = literalComparison(literal, variable, constants);
            if (!comparison) continue;
            let limit: number | null = null;
            if (comparison.operator === '<') limit = comparison.value - 1;
            else if (comparison.operator === '<=') limit = comparison.value;
            else if (comparison.operator === '=' || comparison.operator === '==') limit = comparison.value;
            if (limit !== null) clauseUpper = clauseUpper === null ? limit : Math.min(clauseUpper, limit);
        }
        if (clauseUpper === null) return null;
        maximum = maximum === null ? clauseUpper : Math.max(maximum, clauseUpper);
    }
    return maximum;
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
    if (inner.trim().length === 0) return [];
    const args: string[] = [];
    let depth = 0;
    let start = 0;
    let inString = false;
    for (let index = 0; index < inner.length; index += 1) {
        const char = inner[index];
        if (char === '"') { inString = !inString; continue; }
        if (inString) continue;
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
        const expression = new RegExp(`\\b${name}\\s*\\*?\\s*\\(`, 'g');
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

type SwitchRange = { variable: string | null; selector: string; start: number; end: number };
function switchRanges(block: Rs2Block): SwitchRange[] {
    const ranges: SwitchRange[] = [];
    for (let index = 0; index < block.body.length; index += 1) {
        const code = stripComment(block.body[index].text).trim();
        const start = /\bswitch_(?:int|component)\s*\(/.exec(code);
        if (!start) continue;
        const open = code.indexOf('(', start.index);
        const close = matchingParen(code, open);
        if (close < 0 || !/^\s*\{/.test(code.slice(close + 1))) continue;
        const operand = code.slice(open + 1, close).trim();
        const variableMatch = /^%([A-Za-z0-9_]+)$/.exec(operand);
        // Computed selectors stay unconstrained, but their calls and every arm are scanned.
        const variable = variableMatch?.[1] ?? null;
        let depth = 0;
        let end = index;
        for (let current = index; current < block.body.length; current += 1) {
            depth += braceDelta(stripComment(block.body[current].text));
            end = current;
            if (depth === 0) break;
        }
        if (depth !== 0) throw new Error(`${block.span.file}:${block.body[index].line}: unclosed switch`);
        ranges.push({ variable, selector: operand, start: index, end });
    }
    return ranges;
}

function switchStatements(block: Rs2Block, variable: string, constants: ReadonlyMap<string, number>, ranges: readonly SwitchRange[], parentNodes: Node[], resolveSelector?: ProgressValueResolver): Statement[] {
    const out: Statement[] = [];
    for (const range of ranges) {
        const switchLine = block.body[range.start].line;
        const outerGuards: Guard[] = [];
        walkStatements(parentNodes, (statement, path) => {
            if (statement.line === switchLine) {
                outerGuards.push(...path.map((part) => ({ expression: part.cond, polarity: part.polarity })));
            }
        });
        for (const previous of priorReturns(parentNodes)) {
            if (previous.through < switchLine && switchLine <= previous.within) {
                outerGuards.push({ expression: previous.condition, polarity: previous.polarity });
            }
        }
        out.push({ text: range.selector, line: switchLine, guards: outerGuards });
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
                const header = /^\s*(case\s+((?:[A-Za-z_][A-Za-z0-9_]*:[A-Za-z_][A-Za-z0-9_]*|[^:])+?)\s*:|default\s*:)(.*)$/.exec(code);
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
            if (range.variable !== null && match) {
                const values = match[1].split(',').map((part) => resolveCaseValue(part, constants));
                if (values.length > 0 && values.every((value): value is number => value !== undefined)) {
                    guard = { expression: values.map((value) => `%${range.variable} = ${value}`).join(' | '), polarity: true };
                }
            } else if (range.variable !== null && caseValues.length > 0) {
                guard = { expression: caseValues.map((value) => `%${range.variable} != ${value}`).join(' & '), polarity: true };
            } else if (resolveSelector) {
                const values = match ? match[1].split(',').map((part) => resolveCaseValue(part, constants)) : caseValues;
                if (values.every((value): value is number => value !== undefined)) {
                    const branches = resolveSelector(block, switchLine, range.selector, values, !match);
                    if (branches !== null && !branches.some((branch) => branch.length === 0)) {
                        const terms = branches.map((branch) => branch.map(({ expression, polarity }) =>
                            polarity ? `(${expression})` : `!(${expression})`).join(' & '));
                        guard = {
                            expression: terms.length > 0 ? terms.map((term) => `(${term})`).join(' | ')
                                : `%${variable} < 0 & %${variable} >= 0`,
                            polarity: true,
                        };
                    }
                }
            }
            if (entry.body.length === 0) continue;
            const nodes = parseBody(entry.body);
            const prior = priorReturns(nodes);
            walkStatements(nodes, (statement, path) => {
                if (nestedSwitchLines.has(statement.line)) return;
                const guards = [...outerGuards, ...(guard ? [guard] : []), ...path.map((part: PathCond) => ({ expression: part.cond, polarity: part.polarity }))];
                for (const previous of prior) if (previous.through < statement.line && statement.line <= previous.within) guards.push({ expression: previous.condition, polarity: previous.polarity });
                out.push({ text: statement.text, line: statement.line, guards });
            });
        }
    }
    return out;
}

/** Share the switch-aware statement walk with the exhaustive progress-writer census. */
export function parseProgressStatements(
    block: Rs2Block,
    variable: string,
    constants: ReadonlyMap<string, number>,
    nodes: Node[] = parseBody(block.body),
    resolveSelector?: ProgressValueResolver,
) {
    const ranges = switchRanges(block);
    const switchLines = new Set(ranges.flatMap((range) => block.body.slice(range.start, range.end + 1).map((line) => line.line)));
    const switches = switchStatements(block, variable, constants, ranges, nodes, resolveSelector);
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

function controlGraph(sourceBlocks: readonly Rs2Block[], variable: string, constants: ReadonlyMap<string, number>, resolveSelector?: ProgressValueResolver) {
    const blocks = sourceBlocks.map((block, index): BlockRef => ({
        id: `${block.span.file}#${index}`,
        block,
        statements: parseProgressStatements(block, variable, constants, undefined, resolveSelector),
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
    let queueTargetsComplete = true;
    for (const block of blocks) for (const statement of block.statements) {
        const clean = stripStrings(statement.text);
        for (const call of clean.matchAll(/\b(?:queue|weakqueue|strongqueue|longqueue)\s*\*?\s*\(/g)) {
            const open = clean.indexOf('(', call.index!);
            const target = argumentsOf(clean, open)[0];
            if (!target || !kinds.has(`queue\u0000${target}`)) queueTargetsComplete = false;
        }
        for (const target of callTargets(statement.text, labels, procs, kinds)) {
            (reverse.get(target.id) ?? reverse.set(target.id, []).get(target.id)!).push({ caller: block.id, guards: statement.guards });
        }
    }
    return { byId: new Map(blocks.map((block) => [block.id, block])), reverse, queueTargetsComplete };
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

function feasiblePaths(
    writer: { block: BlockRef; guards: Guard[] },
    graph: ScriptGraph,
    variable: string,
    range: RangeWire,
    constants: ReadonlyMap<string, number>,
    prerequisite: BoolExpr | null = null,
    domainForVariable?: (name: string) => RangeWire | null,
) {
    const out: Guard[][] = [];
    const canHold = (guards: readonly Guard[]) => prerequisite
        ? guardsCanHoldWithPrerequisite(guards, variable, range, constants, prerequisite, domainForVariable)
        : guardsCanHold(guards, variable, range, constants);
    const visit = (blockId: string, guards: Guard[], seen: Set<string>) => {
        const block = graph.byId.get(blockId)!;
        if (block.entry) {
            if (canHold(guards)) out.push(guards);
            return;
        }
        if (seen.has(blockId)) return;
        const nextSeen = new Set(seen).add(blockId);
        for (const edge of graph.reverse.get(blockId) ?? []) {
            const next = [...guards, ...edge.guards];
            if (!canHold(next)) continue;
            visit(edge.caller, next, nextSeen);
        }
    };
    visit(writer.block.id, writer.guards, new Set());
    return out;
}

/** Fresh-login mode gates only statically bound future queues by their scheduling paths. */
export function postCompletionProgressWriters(
    blocks: readonly Rs2Block[],
    variable: string,
    completed: RangeWire,
    constants: ReadonlyMap<string, number>,
    candidates: readonly { block: Rs2Block; line: number; right: string }[],
    completionPrerequisite?: string | null,
    domainForVariable?: (name: string) => RangeWire | null,
    afterFreshLogin = false,
) {
    const parsedPrerequisite = completionPrerequisite ? parseBool(completionPrerequisite) : null;
    const prerequisite = parsedPrerequisite && supportedBooleanExpression(parsedPrerequisite, constants)
        ? parsedPrerequisite : null;
    const graph = controlGraph(blocks, variable, constants, selectorResolver(blocks, constants));
    if (afterFreshLogin && graph.queueTargetsComplete) {
        for (const block of graph.byId.values()) {
            if (block.block.kind === 'queue') block.entry = false;
        }
    }
    // Default keeps queues as possible pending writes because they may predate completion.
    // Fresh-login mode constrains them only when every queue target is bound.
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
        if (feasiblePaths(writer, graph, variable, completed, constants, prerequisite, domainForVariable).length > 0) {
            possible.push(site);
        }
    }
    return possible;
}
