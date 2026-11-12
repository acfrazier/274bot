import type { Rs2Block } from './gathering-content.ts';

export type Guard = { expression: string; polarity: boolean };
export type ProgressValueResolver = (block: Rs2Block, line: number, selector: string, caseValues: readonly number[], defaultCase: boolean) => readonly (readonly Guard[])[] | null;
type Statement = { text: string; line: number; guards: readonly Guard[] };
type Alternative = { value: number | null; guards: readonly Guard[] };
type Bindings = ReadonlyMap<string, readonly Alternative[]>;
type Path = readonly Guard[];
type Context = { block: Rs2Block; line: number; beforeIndex: number | null; bindings: Bindings; visiting: ReadonlySet<number> };
type BlockInfo = { block: Rs2Block; index: number; statements: readonly Statement[] };
type Reference = { caller: BlockInfo; statement: Statement; statementIndex: number; args: string[] | null; bound: boolean };
type Literal = Guard;
type Formula = Literal[][];

const normalizeSpace = (text: string) => text.trim().replace(/\s+/g, ' ');

function stripComment(text: string, stripStrings: (text: string) => string) {
    const clean = stripStrings(text);
    const comment = clean.indexOf('//');
    return comment < 0 ? clean : `${clean.slice(0, comment)}${' '.repeat(clean.length - comment)}`;
}

function closingParen(text: string, open: number, stripStrings: (text: string) => string) {
    const clean = stripStrings(text);
    let depth = 0;
    for (let index = open; index < clean.length; index += 1) {
        if (clean[index] === '(') depth += 1;
        else if (clean[index] === ')') {
            depth -= 1;
            if (depth === 0) return index;
        }
    }
    return -1;
}

function unwrap(text: string, stripStrings: (text: string) => string) {
    let value = text.trim();
    while (value.startsWith('(')) {
        const open = value.indexOf('(');
        if (closingParen(value, open, stripStrings) !== value.length - 1) break;
        value = value.slice(1, -1).trim();
    }
    return value;
}

function callArguments(text: string, marker: RegExp, argumentsOf: (text: string, open: number) => string[], stripStrings: (text: string) => string) {
    const clean = stripComment(text, stripStrings);
    const match = marker.exec(clean);
    if (!match) return null;
    const open = clean.indexOf('(', match.index + match[0].length - 1);
    if (open < 0) return null;
    const close = closingParen(clean, open, stripStrings);
    if (close < 0) return null;
    if (clean.slice(close + 1).trim().replace(/;$/, '').trim()) return null;
    const args = argumentsOf(text, open);
    return args.length === 1 && args[0].trim() === '' ? [] : args;
}

function comparisonAtom(text: string): { expression: string; polarity: boolean } | null {
    const match = /^(.+?)\s*(==|!=|<=|>=|=|<|>)\s*(.+)$/.exec(normalizeSpace(text));
    if (!match) return null;
    const left = normalizeSpace(match[1]);
    const right = normalizeSpace(match[3]);
    const operator = match[2] === '==' ? '=' : match[2];
    if (operator === '!=') return { expression: `${left} = ${right}`, polarity: false };
    if (operator === '>=') return { expression: `${left} < ${right}`, polarity: false };
    if (operator === '>') return { expression: `${left} <= ${right}`, polarity: false };
    return { expression: `${left} ${operator} ${right}`, polarity: true };
}

function splitBoolean(text: string, operator: '&' | '|', stripStrings: (text: string) => string) {
    const clean = stripStrings(text);
    let depth = 0;
    const parts: string[] = [];
    let start = 0;
    for (let index = 0; index < clean.length; index += 1) {
        if (clean[index] === '(') depth += 1;
        else if (clean[index] === ')') depth -= 1;
        else if (depth === 0 && clean[index] === operator) {
            parts.push(text.slice(start, index).trim());
            start = index + 1;
        }
    }
    if (parts.length === 0) return null;
    parts.push(text.slice(start).trim());
    return parts;
}

function simplifyClause(clause: readonly Literal[]) {
    const byExpression = new Map<string, boolean>();
    for (const literal of clause) {
        const key = normalizeSpace(literal.expression);
        const previous = byExpression.get(key);
        if (previous !== undefined && previous !== literal.polarity) return null;
        byExpression.set(key, literal.polarity);
    }
    return [...byExpression].map(([expression, polarity]) => ({ expression, polarity })).sort((left, right) =>
        left.expression.localeCompare(right.expression) || Number(left.polarity) - Number(right.polarity));
}

function clauseKey(clause: readonly Literal[]) {
    return clause.map(({ expression, polarity }) => `${polarity ? '+' : '-'}${normalizeSpace(expression)}`).join('\u0001');
}

function simplifyFormula(formula: readonly (readonly Literal[])[]) {
    const byKey = new Map<string, Literal[]>();
    for (const raw of formula) {
        const clause = simplifyClause(raw);
        if (!clause) continue;
        byKey.set(clauseKey(clause), clause);
    }
    const clauses = [...byKey.values()];
    const sets = clauses.map((clause) => new Set(clause.map((literal) => `${literal.polarity ? '+' : '-'}${normalizeSpace(literal.expression)}`)));
    return clauses.filter((clause, index) => !sets.some((candidate, candidateIndex) =>
        candidateIndex !== index && candidate.size <= sets[index].size && [...candidate].every((literal) => sets[index].has(literal))));
}

function combineFormulas(left: readonly (readonly Literal[])[], right: readonly (readonly Literal[])[]) {
    if (left.length === 0 || right.length === 0) return [];
    const combined: Literal[][] = [];
    for (const leftClause of left) for (const rightClause of right) combined.push([...leftClause, ...rightClause]);
    return simplifyFormula(combined);
}

function formulaForGuard(guard: Guard, stripStrings: (text: string) => string): Formula {
    const parse = (raw: string, negated: boolean): Formula => {
        let expression = unwrap(raw.trim().replace(/;$/, ''), stripStrings);
        if (!expression) return negated ? [[]] : [];
        const leadingNot = /^(?:!|\bnot\b\s*)/.exec(expression);
        if (leadingNot) return parse(expression.slice(leadingNot[0].length), !negated);

        const ors = splitBoolean(expression, '|', stripStrings);
        if (ors) {
            if (negated) {
                return ors.reduce<Formula>((result, part) => combineFormulas(result, parse(part, true)), [[]]);
            }
            return simplifyFormula(ors.flatMap((part) => parse(part, false)));
        }
        const ands = splitBoolean(expression, '&', stripStrings);
        if (ands) {
            if (negated) return simplifyFormula(ands.flatMap((part) => parse(part, true)));
            return ands.reduce<Formula>((result, part) => combineFormulas(result, parse(part, false)), [[]]);
        }

        const atomText = normalizeSpace(stripStrings(expression));
        if (/^true$/i.test(atomText)) return negated ? [] : [[]];
        if (/^false$/i.test(atomText)) return negated ? [[]] : [];
        const comparison = comparisonAtom(atomText);
        const literal = comparison ?? { expression: atomText, polarity: true };
        return [[{ expression: literal.expression, polarity: literal.polarity !== negated }]];
    };
    return parse(guard.expression, !guard.polarity);
}

function formulaForPath(path: Path, stripStrings: (text: string) => string): Formula {
    return path.reduce<Formula>((formula, guard) => combineFormulas(formula, formulaForGuard(guard, stripStrings)), [[]]);
}



function pathPossible(path: Path, stripStrings: (text: string) => string) {
    return formulaForPath(path, stripStrings).length > 0;
}

function normalizePath(path: Path, stripStrings: (text: string) => string): Guard[] | null {
    const byExpression = new Map<string, Guard>();
    for (const guard of path) {
        const expression = normalizeSpace(guard.expression);
        const prior = byExpression.get(expression);
        if (prior && prior.polarity !== guard.polarity) return null;
        byExpression.set(expression, { expression: guard.expression.trim(), polarity: guard.polarity });
    }
    const normalized = [...byExpression.values()];
    return pathPossible(normalized, stripStrings) ? normalized : null;
}

function pathKey(path: Path) {
    return path.map(({ expression, polarity }) => `${polarity ? '+' : '-'}${normalizeSpace(expression)}`).sort().join('\u0001');
}

function dedupePaths(paths: readonly Path[], stripStrings: (text: string) => string) {
    const byKey = new Map<string, Guard[]>();
    for (const path of paths) {
        const normalized = normalizePath(path, stripStrings);
        if (normalized) byKey.set(pathKey(normalized), normalized);
    }
    return [...byKey.values()];
}

function subtractPath(paths: readonly Path[], blocked: Path, stripStrings: (text: string) => string) {
    if (!pathPossible(blocked, stripStrings)) return dedupePaths(paths, stripStrings);
    if (blocked.length === 0) return [];
    const complements = blocked.map((guard) => [{ expression: guard.expression, polarity: !guard.polarity }]);
    const out: Guard[][] = [];
    for (const path of paths) for (const complement of complements) out.push([...path, ...complement]);
    return dedupePaths(out, stripStrings);
}

function uncoveredPaths(paths: readonly Path[], stripStrings: (text: string) => string) {
    let uncovered: Path[] = [[]];
    for (const path of paths) {
        uncovered = subtractPath(uncovered, path, stripStrings);
        if (uncovered.length === 0) break;
    }
    return uncovered;
}

function assignmentFor(statement: Statement, stripStrings: (text: string) => string) {
    const clean = stripComment(statement.text, stripStrings).trim();
    const match = /^(?:def_int\s+)?\$([A-Za-z0-9_]+)\s*=\s*(.*?)\s*;?$/.exec(clean);
    return match ? { name: match[1], expression: match[2].trim() } : null;
}

function parameterList(block: Rs2Block, argumentsOf: (text: string, open: number) => string[]) {
    const open = block.params.indexOf('(');
    if (open < 0) return [];
    return argumentsOf(block.params, open)
        .map((parameter) => {
            const match = /^\s*([A-Za-z_][A-Za-z0-9_]*)\s+\$([A-Za-z0-9_]+)\s*$/.exec(parameter);
            return match ? { type: match[1], name: match[2] } : null;
        })
        .filter((parameter): parameter is { type: string; name: string } => parameter !== null);
}

function useIndex(statements: readonly Statement[], line: number, selector: string, stripStrings: (text: string) => string) {
    const wanted = normalizeSpace(stripComment(selector, stripStrings));
    for (let index = 0; index < statements.length; index += 1) {
        const statement = statements[index];
        if (statement.line === line && normalizeSpace(stripComment(statement.text, stripStrings)) === wanted) return index;
        if (statement.line === line) {
            const clean = stripComment(statement.text, stripStrings);
            const match = /\bswitch_(?:int|component)\s*\(/.exec(clean);
            if (match) {
                const open = clean.indexOf('(', match.index);
                const close = closingParen(clean, open, stripStrings);
                if (close >= 0 && normalizeSpace(clean.slice(open + 1, close)) === wanted) return index;
            }
        }
    }
    return null;
}

function statementBefore(statement: Statement, statementIndex: number, line: number, beforeIndex: number | null) {
    return statement.line < line || (statement.line === line && beforeIndex !== null && statementIndex < beforeIndex);
}

function hasUnsupportedLoop(block: Rs2Block, line: number, stripStrings: (text: string) => string) {
    return block.body.some((row) => row.line <= line && /\b(?:while|for|do)\b/.test(stripComment(row.text, stripStrings)));
}

export function createProgressValueResolver(
    blocks: readonly Rs2Block[],
    constants: ReadonlyMap<string, number>,
    statementsFor: (block: Rs2Block) => readonly { text: string; line: number; guards: readonly Guard[] }[],
    argumentsOf: (text: string, open: number) => string[],
    stripStrings: (text: string) => string,
): ProgressValueResolver {
    const infos: BlockInfo[] = blocks.map((block, index) => ({ block, index, statements: statementsFor(block) }));
    const infoByBlock = new Map(infos.map((info) => [info.block, info] as const));
    const procedures = new Map<string, BlockInfo[]>();
    const labels = new Map<string, BlockInfo[]>();
    for (const info of infos) {
        if (info.block.kind === 'proc') (procedures.get(info.block.name) ?? procedures.set(info.block.name, []).get(info.block.name)!).push(info);
        if (info.block.kind === 'label') (labels.get(info.block.name) ?? labels.set(info.block.name, []).get(info.block.name)!).push(info);
    }

    const incomingCache = new Map<string, Reference[]>();
    const entryBindingsCache = new Map<Rs2Block, Map<string, Alternative[]>>();
    const entryBindingsVisiting = new Set<Rs2Block>();
    const procedureCache = new Map<string, Alternative[]>();

    const sourceReferences = (name: string, marker: '@' | '~', index: Map<string, BlockInfo[]>) => {
        const key = `${marker}${name}`;
        const cached = incomingCache.get(key);
        if (cached) return cached;
        const references: Reference[] = [];
        const directPattern = new RegExp(`${marker}([A-Za-z0-9_]+)`, 'g');
        for (const caller of infos) for (let statementIndex = 0; statementIndex < caller.statements.length; statementIndex += 1) {
            const statement = caller.statements[statementIndex];
            const clean = stripComment(statement.text, stripStrings);
            for (const match of clean.matchAll(directPattern)) {
                if (match[1] !== name) continue;
                const open = clean.indexOf('(', match.index! + match[0].length);
                const hasArguments = open >= 0 && clean.slice(match.index! + match[0].length, open).trim() === '';
                const close = hasArguments ? closingParen(clean, open, stripStrings) : -1;
                const args = close >= 0 ? argumentsOf(statement.text, open) : null;
                references.push({
                    caller,
                    statement,
                    statementIndex,
                    args: args && args.length === 1 && args[0].trim() === '' ? [] : args,
                    bound: close >= 0,
                });
            }
            const tokenPattern = /[A-Za-z_][A-Za-z0-9_]*/g;
            for (const match of clean.matchAll(tokenPattern)) {
                if (match[0] !== name) continue;
                if (previous === '@' || previous === '~' || previous === '$' || previous === '%' || previous === '^') continue;
                references.push({ caller, statement, statementIndex, args: null, bound: false });
            }
        }
        const unique = [...new Map(references.map((reference) => [
            `${reference.caller.index}:${reference.statementIndex}:${reference.args?.join('\u0000') ?? '<unbound>'}:${reference.bound}`,
            reference,
        ] as const)).values()];
        // Only exact declarations of the requested source kind are candidates.
        if (!index.has(name)) unique.length = 0;
        incomingCache.set(key, unique);
        return unique;
    };

    const entryBindings = (info: BlockInfo): Map<string, Alternative[]> => {
        const cached = entryBindingsCache.get(info.block);
        if (cached) return cached;
        const parameters = parameterList(info.block, argumentsOf);
        const bindings = new Map<string, Alternative[]>();
        if (parameters.length === 0) {
            entryBindingsCache.set(info.block, bindings);
            return bindings;
        }
        if (entryBindingsVisiting.has(info.block)) {
            for (const parameter of parameters) bindings.set(parameter.name, [{ value: null, guards: [] }]);
            return bindings;
        }
        entryBindingsVisiting.add(info.block);
        const marker = info.block.kind === 'label' ? '@' : '~';
        const index = info.block.kind === 'label' ? labels : procedures;
        const references = sourceReferences(info.block.name, marker, index);
        for (const parameter of parameters) bindings.set(parameter.name, []);
        for (const reference of references) {
            const callerBindings = entryBindings(reference.caller);
            const guards = reference.statement.guards;
            const arityMatches = reference.bound && reference.args !== null && reference.args.length === parameters.length;
            for (let position = 0; position < parameters.length; position += 1) {
                const parameter = parameters[position];
                const alternatives = bindings.get(parameter.name)!;
                if (!arityMatches || parameter.type !== 'int') {
                    alternatives.push({ value: null, guards });
                    continue;
                }
                const actuals = resolveExpression(reference.caller, reference.statement.line, reference.statementIndex, reference.args![position], callerBindings, new Set());
                if (actuals === null) alternatives.push({ value: null, guards });
                else for (const actual of actuals) alternatives.push({ value: actual.value, guards: [...guards, ...actual.guards] });
            }
        }
        for (const parameter of parameters) {
            const alternatives = bindings.get(parameter.name)!;
            if (alternatives.length === 0) alternatives.push({ value: null, guards: [] });
            bindings.set(parameter.name, dedupeAlternatives(alternatives));
        }
        entryBindingsVisiting.delete(info.block);
        entryBindingsCache.set(info.block, bindings);
        return bindings;
    };

    const rootBindings = (block: Rs2Block) => {
        const info = infoByBlock.get(block);
        if (!info || (block.kind !== 'label' && block.kind !== 'proc')) return new Map<string, Alternative[]>();
        return entryBindings(info);
    };

    function dedupeAlternatives(alternatives: readonly Alternative[]) {
        const byKey = new Map<string, Alternative>();
        for (const alternative of alternatives) {
            const normalized = normalizePath(alternative.guards, stripStrings);
            if (!normalized) continue;
            const key = `${alternative.value ?? 'unknown'}\u0002${pathKey(normalized)}`;
            byKey.set(key, { value: alternative.value, guards: normalized });
        }
        return [...byKey.values()];
    }

    function expressionsForBlock(block: Rs2Block) {
        return infoByBlock.get(block)?.statements ?? statementsFor(block);
    }

    function assignmentAlternatives(context: Context, name: string): Alternative[] | null {
        if (hasUnsupportedLoop(context.block, context.line, stripStrings)) return [{ value: null, guards: [] }];
        const statements = expressionsForBlock(context.block);
        const writes = statements.flatMap((statement, statementIndex) => {
            if (!statementBefore(statement, statementIndex, context.line, context.beforeIndex)) return [];
            const assignment = assignmentFor(statement, stripStrings);
            return assignment?.name === name ? [{ statement, statementIndex, expression: assignment.expression }] : [];
        });
        const initial = context.bindings.get(name);
        if (writes.length === 0) return initial ? [...initial] : null;

        let noWritePaths: Path[] = [[]];
        for (const write of writes) noWritePaths = subtractPath(noWritePaths, write.statement.guards, stripStrings);
        const results: Alternative[] = [];
        if (initial) {
            for (const value of initial) for (const path of noWritePaths) {
                results.push({ value: value.value, guards: [...value.guards, ...path] });
            }
        } else {
            for (const path of noWritePaths) results.push({ value: null, guards: path });
        }

        for (let index = 0; index < writes.length; index += 1) {
            const write = writes[index];
            let lastWritePaths: Path[] = [write.statement.guards];
            for (let later = index + 1; later < writes.length; later += 1) {
                lastWritePaths = lastWritePaths.flatMap((path) => subtractPath([path], writes[later].statement.guards, stripStrings));
                if (lastWritePaths.length === 0) break;
            }
            const expression = resolveExpression(context.block, write.statement.line, write.statementIndex, write.expression, context.bindings, context.visiting);
            if (expression === null) {
                for (const path of lastWritePaths) results.push({ value: null, guards: path });
            } else {
                for (const path of lastWritePaths) for (const value of expression) {
                    results.push({ value: value.value, guards: [...path, ...value.guards] });
                }
            }
        }
        return dedupeAlternatives(results);
    }

    function resolveExpression(
        block: Rs2Block,
        line: number,
        beforeIndex: number | null,
        rawExpression: string,
        bindings: Bindings,
        visiting: ReadonlySet<number>,
    ): Alternative[] | null {
        const expression = unwrap(rawExpression.trim().replace(/;$/, ''), stripStrings);
        if (!expression) return null;
        const clean = stripComment(expression, stripStrings).trim();
        if (/^-?\d+$/.test(clean)) return [{ value: Number(clean), guards: [] }];
        const constant = /^\^([A-Za-z0-9_]+)$/.exec(clean);
        if (constant) {
            const value = constants.get(constant[1]);
            return value === undefined ? null : [{ value, guards: [] }];
        }
        const local = /^\$([A-Za-z0-9_]+)$/.exec(clean);
        if (local) return assignmentAlternatives({ block, line, beforeIndex, bindings, visiting }, local[1]);

        const call = /^~([A-Za-z0-9_]+)(?=\s*(?:\(|$))/.exec(clean);
        if (!call) return null;
        const suffix = clean.slice(call[0].length).trim();
        if (suffix && !suffix.startsWith('(')) return null;
        const args = suffix ? callArguments(expression, new RegExp(`^~${call[1]}\\s*\\(`), argumentsOf, stripStrings) : [];
        if (args === null) return null;
        const targets = procedures.get(call[1]);
        if (!targets || targets.length === 0) return null;
        const matching = targets.filter((target) => parameterList(target.block, argumentsOf).length === args.length);
        if (matching.length === 0) return null;

        const out: Alternative[] = [];
        for (const target of matching) {
            const parameters = parameterList(target.block, argumentsOf);
            const argumentChoices: Alternative[][] = [];
            for (let position = 0; position < parameters.length; position += 1) {
                if (parameters[position].type !== 'int') {
                    argumentChoices.push([{ value: null, guards: [] }]);
                    continue;
                }
                const actual = resolveExpression(block, line, beforeIndex, args[position], bindings, visiting);
                argumentChoices.push(actual === null ? [{ value: null, guards: [] }] : [...actual]);
            }
            let combinations: { values: Alternative[]; guards: Guard[] }[] = [{ values: [], guards: [] }];
            for (const choices of argumentChoices) {
                const next: { values: Alternative[]; guards: Guard[] }[] = [];
                for (const combination of combinations) for (const choice of choices) {
                    const guards = [...combination.guards, ...choice.guards];
                    if (!pathPossible(guards, stripStrings)) continue;
                    next.push({ values: [...combination.values, choice], guards });
                }
                combinations = next;
            }
            for (const combination of combinations) {
                const callBindings = new Map<string, Alternative[]>();
                for (let position = 0; position < parameters.length; position += 1) {
                    const value = combination.values[position];
                    callBindings.set(parameters[position].name, [{ value: value.value, guards: [] }]);
                }
                const returned = procedureReturns(target, callBindings, visiting);
                if (returned === null) out.push({ value: null, guards: combination.guards });
                else for (const value of returned) out.push({ value: value.value, guards: [...combination.guards, ...value.guards] });
            }
        }
        return dedupeAlternatives(out);
    }

    function procedureReturns(info: BlockInfo, bindings: Bindings, visiting: ReadonlySet<number>): Alternative[] | null {
        const key = `${info.index}\u0002${JSON.stringify([...bindings].map(([name, alternatives]) => [name, alternatives.map(({ value }) => value)]).sort())}`;
        const cached = procedureCache.get(key);
        if (cached) return [...cached];
        if (visiting.has(info.index)) return null;
        const nextVisiting = new Set(visiting).add(info.index);
        const statements = info.statements;
        const returns = statements.flatMap((statement, statementIndex) => {
            const clean = stripComment(statement.text, stripStrings).trim();
            const match = /^return\s*\(/.exec(clean);
            if (!match) return [];
            const open = clean.indexOf('(', match.index);
            const close = closingParen(clean, open, stripStrings);
            if (close < 0) return [{ statement, statementIndex, expression: null as string | null }];
            const raw = statement.text;
            const rawOpen = raw.indexOf('(', match.index);
            const rawClose = closingParen(raw, rawOpen, stripStrings);
            const expression = rawClose < 0 ? null : raw.slice(rawOpen + 1, rawClose).trim();
            return [{ statement, statementIndex, expression }];
        });
        if (returns.length === 0) return null;

        const out: Alternative[] = [];
        for (const returned of returns) {
            const values = returned.expression === null
                ? null
                : resolveExpression(info.block, returned.statement.line, returned.statementIndex, returned.expression, bindings, nextVisiting);
            if (values === null) out.push({ value: null, guards: returned.statement.guards });
            else for (const value of values) out.push({ value: value.value, guards: [...returned.statement.guards, ...value.guards] });
        }
        const uncovered = uncoveredPaths(returns.map(({ statement }) => statement.guards), stripStrings);
        for (const path of uncovered) out.push({ value: null, guards: path });
        if (hasUnsupportedLoop(info.block, Number.POSITIVE_INFINITY, stripStrings)) out.push({ value: null, guards: [] });
        const result = dedupeAlternatives(out);
        procedureCache.set(key, result);
        return [...result];
    }

    return (block, line, selector, caseValues, defaultCase) => {
        const statements = expressionsForBlock(block);
        const beforeIndex = useIndex(statements, line, selector, stripStrings);
        const alternatives = resolveExpression(block, line, beforeIndex, selector, rootBindings(block), new Set());
        if (alternatives === null) return null;
        const selected = new Set(caseValues);
        const paths: Path[] = [];
        for (const alternative of alternatives) {
            const matches = alternative.value === null
                || (defaultCase ? !selected.has(alternative.value) : selected.has(alternative.value));
            if (matches) paths.push(alternative.guards);
        }
        return dedupePaths(paths, stripStrings);
    };
}
