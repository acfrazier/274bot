// Structural reader for the finite script constructs the gathering handlers use. It is not a RuneScript
// interpreter: it splits a block into `if`/`else`/statement nodes and lets the family extractor match each node
// against an explicit allow-list. Anything it cannot match is reported to the caller, never guessed.
import { stripComment } from './gathering-content.ts';

export type Stmt = { kind: 'stmt'; text: string; line: number };
export type Node =
    | Stmt
    | { kind: 'if'; cond: string; then: Node[]; els: Node[]; line: number }
    | { kind: 'while'; cond: string; body: Node[]; line: number };

/** A condition on the path to a statement; `polarity` false means the statement sits in the `else` branch. */
export type PathCond = { cond: string; polarity: boolean; line: number };

const squash = (text: string) => text.replace(/\s+/g, ' ').trim();

/** Parse comment-stripped block lines into nodes. Brace and parenthesis structure is required to balance. */
export function parseBody(body: { line: number; text: string }[]): Node[] {
    let src = '';
    const lineAt: number[] = [];
    for (const { line, text } of body) {
        for (const ch of stripComment(text)) {
            src += ch;
            lineAt.push(line);
        }
        src += ' ';
        lineAt.push(line);
    }
    let i = 0;
    const skipSpace = () => {
        while (i < src.length && /\s/.test(src[i])) i += 1;
    };
    const keyword = (name: string) => new RegExp(`^${name}\\s*\\(`).test(src.slice(i, i + name.length + 40));
    /** Consume a balanced group starting at `open`; returns the inner range. */
    const group = (open: string, close: string): [number, number] => {
        if (src[i] !== open) throw new Error(`rs2 body line ${lineAt[i]}: expected ${open}`);
        let depth = 0;
        let inString = false;
        const started = i;
        for (; i < src.length; i += 1) {
            const ch = src[i];
            if (ch === '"') inString = !inString;
            if (inString) continue;
            if (ch === open) depth += 1;
            else if (ch === close) {
                depth -= 1;
                if (depth === 0) {
                    i += 1;
                    return [started + 1, i - 1];
                }
            }
        }
        throw new Error(`rs2 body line ${lineAt[started]}: unbalanced ${open}`);
    };
    /** `{ ... }` or one brace-less statement. */
    const branch = (end: number): Node[] => {
        skipSpace();
        if (src[i] === '{') {
            const [start, stop] = group('{', '}');
            return parseRange(start, stop);
        }
        const node = parseOne(end);
        return node ? [node] : [];
    };
    const parseIf = (end: number): Node => {
        const line = lineAt[i];
        i += 2;
        skipSpace();
        const [condStart, condEnd] = group('(', ')');
        const cond = squash(src.slice(condStart, condEnd));
        const then = branch(end);
        skipSpace();
        let els: Node[] = [];
        if (/^else(?![A-Za-z0-9_$])/.test(src.slice(i, i + 5))) {
            i += 4;
            els = branch(end);
        }
        return { kind: 'if', cond, then, els, line };
    };
    const parseOne = (end: number): Node | null => {
        skipSpace();
        if (i >= end) return null;
        if (keyword('if')) return parseIf(end);
        if (keyword('while')) {
            const line = lineAt[i];
            i += 5;
            skipSpace();
            const [condStart, condEnd] = group('(', ')');
            const cond = squash(src.slice(condStart, condEnd));
            return { kind: 'while', cond, body: branch(end), line };
        }
        const line = lineAt[i];
        let depth = 0;
        let inString = false;
        const from = i;
        for (; i < end; i += 1) {
            const ch = src[i];
            if (ch === '"') inString = !inString;
            if (inString) continue;
            if (ch === '(' || ch === '{') depth += 1;
            else if (ch === ')' || ch === '}') depth -= 1;
            else if (ch === ';' && depth === 0) break;
        }
        const text = squash(src.slice(from, i));
        i += 1;
        return text ? { kind: 'stmt', text, line } : parseOne(end);
    };
    const parseRange = (start: number, end: number): Node[] => {
        const resume = i;
        i = start;
        const nodes: Node[] = [];
        for (let node = parseOne(end); node; node = parseOne(end)) nodes.push(node);
        i = resume;
        return nodes;
    };
    return parseRange(0, src.length);
}

/**
 * Visit every statement, with the conditions on its path. Subtrees whose `if` condition `skip` accepts are not
 * entered (random events and other ignored regions).
 */
export function walkStatements(
    nodes: Node[],
    visit: (stmt: Stmt, path: PathCond[]) => void,
    skip: (cond: string) => boolean = () => false,
    path: PathCond[] = [],
) {
    for (const node of nodes) {
        if (node.kind === 'stmt') visit(node, path);
        else if (node.kind === 'while') walkStatements(node.body, visit, skip, [...path, { cond: node.cond, polarity: true, line: node.line }]);
        else if (!skip(node.cond)) {
            walkStatements(node.then, visit, skip, [...path, { cond: node.cond, polarity: true, line: node.line }]);
            walkStatements(node.els, visit, skip, [...path, { cond: node.cond, polarity: false, line: node.line }]);
        }
    }
}

/** True when a branch (top-level statement list) ends the handler with `return;` or `return(...)`. */
export const returns = (nodes: Node[]) => nodes.some((node) => node.kind === 'stmt' && /^return\b/.test(node.text));
