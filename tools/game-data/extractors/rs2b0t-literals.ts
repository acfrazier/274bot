import path from 'node:path';
import { pathToFileURL } from 'node:url';

/** Every external source root is explicit: an unset variable fails closed and names itself. */
export function requireEnvPath(name: string, label: string): string {
    const value = process.env[name];
    if (!value) throw new Error(`set ${name} to ${label}`);
    return path.resolve(value);
}

// The runtime-selected engine owns TypeScript. Only fields guarded by its
// is* predicates are read; no foreign module is executed.
export type AstNode = {
    text: string; name: AstNode; initializer: AstNode; body: AstNode; type: AstNode;
    elements: AstNode[]; properties: AstNode[]; arguments: AstNode[]; parameters: AstNode[]; statements: AstNode[];
    expression: AstNode; operand: AstNode; kind: number; operator: number;
    declarationList: { declarations: AstNode[] };
    getText(source: unknown): string;
};
/** A `new Set([...])` literal: kept apart from arrays so the Rust table knows what upstream built. */
export class LiteralSet {
    constructor(readonly values: Literal[]) {}
}
/** `<array>.map(helper)` with a named local helper: the consumer applies the helper, nothing here evaluates it. */
export class LiteralMapped {
    constructor(readonly helper: string, readonly rows: Literal[]) {}
}
export type Literal = undefined | null | boolean | string | number | Literal[] | LiteralSet | LiteralMapped | { [key: string]: Literal };

function rowName(row: Literal): string {
    if (row !== null && typeof row === 'object' && !Array.isArray(row) && !(row instanceof LiteralSet) && !(row instanceof LiteralMapped) && typeof row.name === 'string') return row.name;
    throw new Error('sorted row has no string name');
}
const SORT_BY_NAME = '(a, b) => a.name.localeCompare(b.name)';
const JUNK_BODY = 'return items.map(([id, name]) => ({ id, name, quest }));';

/**
 * Named top-level literals of one pinned rs2b0t source, read from its AST:
 * strings, numbers, booleans, `undefined`, arrays (with spreads), object
 * literals, `new Tile(x, z, level)`, `new Set([...])`, property reads of other
 * top-level constants (`BANK.draynor`), references to other constants, and
 * calls of local helper functions (`camp(...)`, `mine(...)`, `tool(...)`),
 * which are expanded by binding the call's arguments and defaults into the
 * helper's single returned expression. Two library calls are recognised by
 * their exact source text only: `.sort((a, b) => a.name.localeCompare(b.name))`
 * (ordered with the en-US collation of `localeCompare`) and
 * `.map(<local helper>)` (kept as a [`LiteralMapped`] for the consumer).
 * Nothing is executed. Each constant is evaluated once, so a name referenced
 * twice yields the identical object, which is how shared upstream objects
 * (the `BANK` tiles, `CATHERBY_RANGE`) stay visible to the consumer.
 */
export async function pinnedLiterals(engine: string, fileName: string, source: string, label: string) {
    // Engine path is selected per revision; a static import would bind the wrong installation.
    const ts = await import(pathToFileURL(path.join(engine, 'node_modules/typescript/lib/typescript.js')).href);
    const ast = ts.createSourceFile(fileName, source, ts.ScriptTarget.Latest, true);
    const initializers = new Map<string, AstNode>();
    const evaluated = new Map<string, Literal>();
    const functions = new Map<string, AstNode>();
    for (const statement of ast.statements as AstNode[]) {
        if (ts.isVariableStatement(statement)) {
            for (const declaration of statement.declarationList.declarations) {
                if (!ts.isIdentifier(declaration.name) || !declaration.initializer) continue;
                if (ts.isArrowFunction(declaration.initializer)) functions.set(declaration.name.text, declaration.initializer);
                else initializers.set(declaration.name.text, declaration.initializer);
            }
        } else if (ts.isFunctionDeclaration(statement) && statement.name) {
            functions.set(statement.name.text, statement);
        }
    }
    const collator = new Intl.Collator('en-US');
    const normalized = (node: AstNode) => node.getText(ast).replace(/\s+/g, ' ').trim();
    const unwrap = (node: AstNode): AstNode => {
        let current = node;
        while (ts.isParenthesizedExpression(current) || ts.isAsExpression(current) || ts.isSatisfiesExpression(current) || ts.isNonNullExpression(current)) current = current.expression;
        return current;
    };
    function helperExpression(name: string): AstNode {
        const helper = functions.get(name)!;
        const body = helper.body;
        if (ts.isBlock(body)) {
            if (body.statements.length !== 1 || !ts.isReturnStatement(body.statements[0])) throw new Error(`${label}: helper ${name} is not a single return`);
            return body.statements[0].expression;
        }
        return body;
    }
    function call(node: AstNode, env: Map<string, Literal> | undefined): Literal {
        const callee = unwrap(node.expression);
        if (ts.isIdentifier(callee)) {
            const name = callee.text;
            const helper = functions.get(name);
            if (!helper) throw new Error(`${label}: call of unknown function ${name}`);
            if (name === 'junk') {
                // `junk(quest, [[id, name], ...])`: the only helper whose body maps rows; read its exact text.
                const text = ts.isBlock(helper.body) ? normalized(helper.body).replace(/^\{ | \}$/g, '') : '';
                if (text !== JUNK_BODY) throw new Error(`${label}: helper junk changed: ${text}`);
                const quest = literal(node.arguments[0], env);
                const rows = literal(node.arguments[1], env);
                if (typeof quest !== 'string' || !Array.isArray(rows)) throw new Error(`${label}: junk arguments`);
                return rows.map((row) => {
                    if (!Array.isArray(row) || typeof row[0] !== 'number' || typeof row[1] !== 'string') throw new Error(`${label}: junk row`);
                    return { id: row[0], name: row[1], quest };
                });
            }
            const bound = new Map<string, Literal>();
            helper.parameters.forEach((parameter, index) => {
                const argument = node.arguments[index];
                bound.set(parameter.name.text, argument ? literal(argument, env) : parameter.initializer ? literal(parameter.initializer, bound) : undefined);
            });
            return literal(helperExpression(name), bound);
        }
        if (ts.isPropertyAccessExpression(callee)) {
            const method = callee.name.text;
            const target = literal(callee.expression, env);
            if (method === 'sort' && node.arguments.length === 1 && normalized(node.arguments[0]) === SORT_BY_NAME && Array.isArray(target)) {
                return [...target].sort((a, b) => collator.compare(rowName(a), rowName(b)));
            }
            if (method === 'map' && node.arguments.length === 1 && ts.isIdentifier(node.arguments[0]) && functions.has(node.arguments[0].text) && Array.isArray(target)) {
                return new LiteralMapped(node.arguments[0].text, target);
            }
        }
        throw new Error(`${label}: unsupported call ${node.getText(ast)}`);
    }
    function literal(node: AstNode | undefined, env?: Map<string, Literal>): Literal {
        if (!node) throw new Error(`${label}: missing literal`);
        node = unwrap(node);
        if (ts.isStringLiteral(node) || ts.isNoSubstitutionTemplateLiteral(node)) return node.text;
        if (ts.isNumericLiteral(node)) return Number(node.text);
        if (ts.isPrefixUnaryExpression(node) && node.operator === ts.SyntaxKind.MinusToken && ts.isNumericLiteral(node.operand)) return -Number(node.operand.text);
        if (node.kind === ts.SyntaxKind.TrueKeyword) return true;
        if (node.kind === ts.SyntaxKind.FalseKeyword) return false;
        if (node.kind === ts.SyntaxKind.NullKeyword) return null;
        if (ts.isIdentifier(node)) {
            if (env?.has(node.text)) return env.get(node.text);
            if (node.text === 'undefined') return undefined;
            if (initializers.has(node.text)) {
                if (!evaluated.has(node.text)) evaluated.set(node.text, literal(initializers.get(node.text)));
                return evaluated.get(node.text);
            }
            throw new Error(`${label}: unknown identifier ${node.text}`);
        }
        if (ts.isPropertyAccessExpression(node)) {
            const target = literal(node.expression, env);
            if (target === null || typeof target !== 'object' || Array.isArray(target) || target instanceof LiteralSet || target instanceof LiteralMapped || !(node.name.text in target)) {
                throw new Error(`${label}: unsupported property read ${node.getText(ast)}`);
            }
            return target[node.name.text];
        }
        if (ts.isArrayLiteralExpression(node)) {
            const out: Literal[] = [];
            for (const element of node.elements) {
                if (ts.isSpreadElement(element)) {
                    const spread = literal(element.expression, env);
                    if (!Array.isArray(spread)) throw new Error(`${label}: spread of a non-array ${element.getText(ast)}`);
                    out.push(...spread);
                } else out.push(literal(element, env));
            }
            return out;
        }
        if (ts.isObjectLiteralExpression(node)) {
            const out: { [key: string]: Literal } = {};
            for (const property of node.properties) {
                if (ts.isPropertyAssignment(property)) out[property.name.text] = literal(property.initializer, env);
                else if (ts.isShorthandPropertyAssignment(property)) out[property.name.text] = literal(property.name, env);
                else throw new Error(`${label}: nonliteral property`);
            }
            return out;
        }
        if (ts.isNewExpression(node)) {
            const constructor = node.expression.getText(ast);
            if (constructor === 'Tile') {
                const [x, z, level] = node.arguments.map((argument) => literal(argument, env));
                if (![x, z, level].every(value => typeof value === 'number' && Number.isInteger(value))) throw new Error(`${label}: invalid tile`);
                return { x, z, level } as Literal;
            }
            if (constructor === 'Set' && node.arguments.length === 1) {
                const values = literal(node.arguments[0], env);
                if (!Array.isArray(values)) throw new Error(`${label}: Set argument is not an array`);
                return new LiteralSet(values);
            }
        }
        if (ts.isCallExpression(node)) return call(node, env);
        throw new Error(`${label}: unsupported AST ${node.getText(ast)}`);
    }
    return (name: string) => literal(initializers.get(name) ?? (() => { throw new Error(`${label}: no constant ${name}`); })());
}

/** Checked reads of a pinned literal, shared by the extractors that turn one into a Rust table. */
type LiteralRecord = { [key: string]: Literal };
export function record(input: Literal, label: string): LiteralRecord {
    if (input === null || typeof input !== 'object' || Array.isArray(input) || input instanceof LiteralSet || input instanceof LiteralMapped) throw new Error(`${label}: expected object`);
    return input;
}
export function str(input: Literal, label: string): string {
    if (typeof input !== 'string') throw new Error(`${label}: expected string`);
    return input;
}
export function int(input: Literal, label: string): number {
    if (typeof input !== 'number' || !Number.isInteger(input)) throw new Error(`${label}: expected integer`);
    return input;
}
export function bool(input: Literal, label: string): boolean {
    if (typeof input !== 'boolean') throw new Error(`${label}: expected boolean`);
    return input;
}
export function list(input: Literal, label: string): Literal[] {
    if (!Array.isArray(input)) throw new Error(`${label}: expected array`);
    return input;
}
export function onlyKeys(row: LiteralRecord, allowed: readonly string[], label: string) {
    for (const key of Object.keys(row)) if (!allowed.includes(key)) throw new Error(`${label}: unexpected key ${key}`);
}

