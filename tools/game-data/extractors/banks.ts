import fs from 'node:fs';
import path from 'node:path';
import { parsePack, requireGatherText } from './common.ts';
import { parseRs2Blocks, parseSections, stripComments, walkContent, type Rs2Block, type Span } from './gathering-content.ts';

export type NpcBankAccess = {
    id: number;
    alias: string;
    name: string;
    op: number;
    action: string;
    choose: string | null;
    sources: Span[];
};

export type NpcBankAccessRefusal = {
    id: number;
    alias: string;
    name: string;
    op: number;
    action: string;
    reason: 'conditional-control-flow' | 'early-return' | 'unsupported-choice';
    sources: Span[];
};

export type NpcBankAccessExtraction = {
    rows: NpcBankAccess[];
    refusals: NpcBankAccessRefusal[];
};

const code = (block: Rs2Block) => stripComments(block.body.map(line => line.text).join('\n'));
const withoutStrings = (text: string) => text.replace(/"(?:\\.|[^"\\])*"/g, string => string.replace(/[^\n]/g, ' '));

/** One optional choice fits the existing packed NPC access. Ordinary Bank options need none. */
function bankChoice(blocks: Rs2Block[]): string | null | undefined {
    const choices = blocks.flatMap(block => [...code(block).matchAll(/def_int\s+(\$\w+)\s*=\s*~p_choice2\(\s*("(?:\\.|[^"\\])*")\s*,\s*(\d+)\s*,\s*("(?:\\.|[^"\\])*")\s*,\s*(\d+)\s*\)\s*;/g)].map(match => ({ block, match })));
    const choiceCalls = blocks.reduce((count, block) => count + [...withoutStrings(code(block)).matchAll(/~p_choice\d+\s*\(/g)].length, 0);
    if (choiceCalls === 0) return null;
    if (choiceCalls !== 1 || choices.length !== 1) return undefined;
    const { block, match } = choices[0];
    const variable = match[1].replace('$', '\\$');
    const refusal = new RegExp(`if\\s*\\(\\s*${variable}\\s*=\\s*(\\d+)\\s*\\)\\s*\\{[^{}]*\\breturn\\s*;\\s*\\}`, 'g');
    const text = withoutStrings(code(block));
    const refused = [...text.matchAll(refusal)];
    if (refused.length !== 1) return undefined;
    const next = blocks[blocks.indexOf(block) + 1];
    if (!next) return undefined;
    const call = new RegExp(`${next.kind === 'label' ? '@' : '~'}${next.name}\\b`).exec(text);
    const start = refused[0].index!;
    const end = start + refused[0][0].length;
    if (!call || call.index < end) return undefined;
    if (refused[0][1] === match[3]) return JSON.parse(match[4]);
    if (refused[0][1] === match[5]) return JSON.parse(match[2]);
    return undefined;
}

function choiceRefusal(block: Rs2Block) {
    const source = code(block);
    const declarations = [...source.matchAll(/def_int\s+(\$\w+)\s*=\s*~p_choice2\(\s*("(?:\\.|[^"\\])*")\s*,\s*(\d+)\s*,\s*("(?:\\.|[^"\\])*")\s*,\s*(\d+)\s*\)\s*;/g)];
    const choiceCalls = [...withoutStrings(source).matchAll(/~p_choice\d+\s*\(/g)];
    if (declarations.length !== 1 || choiceCalls.length !== 1) return null;
    const variable = declarations[0][1].replace('$', '\\$');
    const refusal = new RegExp(`if\\s*\\(\\s*${variable}\\s*=\\s*\\d+\\s*\\)\\s*\\{[^{}]*\\breturn\\s*;\\s*\\}`, 'g');
    const text = withoutStrings(source);
    const matches = [...text.matchAll(refusal)];
    if (matches.length !== 1) return null;
    return { start: matches[0].index!, end: matches[0].index! + matches[0][0].length };
}

type BankOpen = { prefix: string; firstCall: number };

function bankOpen(block: Rs2Block): BankOpen | null {
    const text = withoutStrings(code(block));
    const inventory = /\binv_transmit\s*\(\s*bank\s*,\s*(\w+):\w+\s*\)/.exec(text);
    if (!inventory) return null;
    const side = new RegExp(`\\bif_openmain_side\\s*\\(\\s*${inventory[1]}\\s*,`);
    const open = side.exec(text.slice(inventory.index + inventory[0].length));
    if (!open) return null;
    const end = inventory.index + inventory[0].length + open.index + open[0].length;
    return { prefix: text.slice(0, end), firstCall: inventory.index };
}

function bankGateReason(block: Rs2Block, entry: Rs2Block): NpcBankAccessRefusal['reason'] | null {
    const open = bankOpen(block);
    let text = open?.prefix ?? withoutStrings(code(block));
    const boundary = open?.firstCall ?? text.length;
    const choice = choiceRefusal(block);
    if (choice && choice.end <= boundary) {
        text = `${text.slice(0, choice.start)}${text.slice(choice.start, choice.end).replace(/[^\n]/g, ' ')}${text.slice(choice.end)}`;
    }
    if (block === entry && /^apnpc[1-5]$/i.test(entry.kind)) {
        const approach = /^\s*if\s*\(\s*npc_range\s*\(\s*coord\s*\)\s*>\s*(\d+)\s*\)\s*\{\s*p_aprange\s*\(\s*\1\s*\)\s*;\s*return\s*;\s*\}\s*/.exec(text);
        if (approach && approach.index + approach[0].length <= boundary) {
            text = `${text.slice(0, approach.index)}${text.slice(approach.index, approach.index + approach[0].length).replace(/[^\n]/g, ' ')}${text.slice(approach.index + approach[0].length)}`;
        }
    }
    if (/\b(?:if|else|switch(?:_int)?|case|while|for|do)\b/.test(text)) return 'conditional-control-flow';
    if (/\breturn\s*;/.test(text)) return 'early-return';
    return null;
}

type BankPath =
    | { route: Rs2Block[]; status: 'accepted'; choose: string | null }
    | { route: Rs2Block[]; status: 'refused'; reason: NpcBankAccessRefusal['reason'] };

/** Resolve an authored NPC operation to unguarded bank calls and retain rejected guarded candidates. */
export function extractNpcBankAccesses(content: string): NpcBankAccessExtraction {
    const scripts = new Map<string, Rs2Block[]>();
    for (const file of walkContent(content, 'scripts', ['.rs2'])) {
        for (const block of parseRs2Blocks(file, stripComments(fs.readFileSync(path.join(content, file), 'utf8')))) {
            const key = `${block.kind},${block.name.trim()}`;
            (scripts.get(key) ?? scripts.set(key, []).get(key)!).push(block);
        }
    }
    const unique = (key: string) => {
        const blocks = scripts.get(key) ?? [];
        if (blocks.length > 1) throw new Error(`NPC bank access: ambiguous script ${key}`);
        return blocks[0];
    };
    const bankPath = (entry: Rs2Block): BankPath | null => {
        const pending: { route: Rs2Block[]; refusal: NpcBankAccessRefusal['reason'] | null }[] = [{ route: [entry], refusal: null }];
        const visited = new Set<string>();
        let refused: Extract<BankPath, { status: 'refused' }> | null = null;
        for (let index = 0; index < pending.length; index++) {
            const { route, refusal: inheritedRefusal } = pending[index];
            const block = route[route.length - 1];
            const choices = route.flatMap(item => {
                const count = [...withoutStrings(code(item)).matchAll(/~p_choice\d+\s*\(/g)].length;
                return count ? [`${item.span.file}:${item.span.first}:${count}`] : [];
            }).join('|');
            const state = `${block.span.file}:${block.span.first}:${inheritedRefusal ?? 'clear'}:${choices}`;
            if (visited.has(state)) continue;
            visited.add(state);
            const ownRefusal = bankGateReason(block, entry);
            const pathRefusal = inheritedRefusal ?? ownRefusal;
            if (bankOpen(block) !== null) {
                const choice = bankChoice(route);
                if (choice === undefined) {
                    refused ??= { route, status: 'refused', reason: 'unsupported-choice' };
                    continue;
                }
                if (pathRefusal) {
                    refused ??= { route, status: 'refused', reason: pathRefusal };
                    continue;
                }
                return { route, status: 'accepted', choose: choice };
            }
            const text = withoutStrings(code(block));
            // Menu callbacks are separate operations, not an automatic bank-opening call path.
            if (/@multi\d+\s*\(/.test(text)) continue;
            for (const call of text.matchAll(/([@~])([A-Za-z_]\w*)\b/g)) {
                const target = unique(`${call[1] === '@' ? 'label' : 'proc'},${call[2]}`);
                if (target && !route.includes(target)) pending.push({ route: [...route, target], refusal: pathRefusal });
            }
        }
        return refused;
    };
    const ids = parsePack(requireGatherText(content, 'pack/npc.pack'));
    const rows: NpcBankAccess[] = [];
    const refusals: NpcBankAccessRefusal[] = [];
    const seen = new Set<string>();
    for (const file of walkContent(content, 'scripts', ['.npc'])) {
        for (const config of parseSections(file, stripComments(fs.readFileSync(path.join(content, file), 'utf8')))) {
            const id = ids.get(config.name);
            if (id === undefined) continue;
            if (seen.has(config.name)) throw new Error(`NPC bank access: duplicate config ${config.name}`);
            seen.add(config.name);
            const category = config.props.get('category')?.[0];
            const candidates: { row: NpcBankAccess; depth: number }[] = [];
            for (let op = 1; op <= 5; op++) {
                const action = config.props.get(`op${op}`)?.[0]?.trim();
                if (!action || action.toLowerCase() === 'hidden') continue;
                const resolve = (trigger: string) => unique(`${trigger},${config.name}`)
                    ?? (category ? unique(`${trigger},_${category}`) : undefined)
                    ?? unique(`${trigger},_`);
                const handler = resolve(`opnpc${op}`) ?? resolve(`apnpc${op}`);
                if (!handler) continue;
                const path = bankPath(handler);
                if (!path) continue;
                const sources = [config.span, ...path.route.map(block => block.span)];
                if (path.status === 'refused') {
                    refusals.push({
                        id, alias: config.name, name: config.props.get('name')?.[0] ?? config.name,
                        op, action, reason: path.reason, sources,
                    });
                    continue;
                }
                candidates.push({ row: {
                    id, alias: config.name, name: config.props.get('name')?.[0] ?? config.name,
                    op, action, choose: path.choose, sources,
                }, depth: path.route.length });
            }
            candidates.sort((a, b) => Number(a.row.choose !== null) - Number(b.row.choose !== null) || a.depth - b.depth || a.row.op - b.row.op);
            if (candidates.length) rows.push(candidates[0].row);
        }
    }
    return {
        rows: rows.sort((a, b) => a.id - b.id),
        refusals: refusals.sort((a, b) => a.id - b.id || a.op - b.op),
    };
}


