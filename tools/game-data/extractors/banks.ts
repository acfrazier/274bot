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

const code = (block: Rs2Block) => stripComments(block.body.map(line => line.text).join('\n'));
const withoutStrings = (text: string) => text.replace(/"(?:\\.|[^"\\])*"/g, '""');

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
    if (!call || call.index < refused[0].index! + refused[0][0].length) return undefined;
    if (refused[0][1] === match[3]) return JSON.parse(match[4]);
    if (refused[0][1] === match[5]) return JSON.parse(match[2]);
    return undefined;
}

/** Resolve an authored NPC operation to the content's bank-inventory opening call, not its menu label. */
export function extractNpcBankAccesses(content: string): NpcBankAccess[] {
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
    const opensBank = (block: Rs2Block) => {
        const text = withoutStrings(code(block));
        const inventory = /\binv_transmit\s*\(\s*bank\s*,\s*(\w+):\w+\s*\)/.exec(text);
        return inventory !== null && new RegExp(`\\bif_openmain_side\\s*\\(\\s*${inventory[1]}\\s*,`).test(text);
    };
    const bankPath = (entry: Rs2Block): Rs2Block[] | null => {
        const pending = [[entry]];
        const visited = new Set<Rs2Block>();
        for (let index = 0; index < pending.length; index++) {
            const route = pending[index];
            const block = route[route.length - 1];
            if (visited.has(block)) continue;
            visited.add(block);
            if (opensBank(block)) return route;
            const text = withoutStrings(code(block));
            // Menu callbacks are separate operations, not an automatic bank-opening call path.
            if (/@multi\d+\s*\(/.test(text)) continue;
            for (const call of text.matchAll(/([@~])([A-Za-z_]\w*)\b/g)) {
                const target = unique(`${call[1] === '@' ? 'label' : 'proc'},${call[2]}`);
                if (target) pending.push([...route, target]);
            }
        }
        return null;
    };
    const ids = parsePack(requireGatherText(content, 'pack/npc.pack'));
    const rows: NpcBankAccess[] = [];
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
                const route = bankPath(handler);
                if (!route) continue;
                const choose = bankChoice(route);
                if (choose === undefined) continue;
                candidates.push({ row: {
                    id, alias: config.name, name: config.props.get('name')?.[0] ?? config.name,
                    op, action, choose, sources: [config.span, ...route.map(block => block.span)],
                }, depth: route.length });
            }
            candidates.sort((a, b) => Number(a.row.choose !== null) - Number(b.row.choose !== null) || a.depth - b.depth || a.row.op - b.row.op);
            if (candidates.length) rows.push(candidates[0].row);
        }
    }
    return rows.sort((a, b) => a.id - b.id);
}
