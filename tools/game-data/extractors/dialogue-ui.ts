import { requireGatherText } from './common.ts';
import { parseRs2Blocks, stripComment, type Rs2Block } from './gathering-content.ts';
import { parseBody, walkStatements } from './gathering-rs2.ts';

export type DialogueUiIds = {
    scroll_root: number;
    quest_scroll_root: number;
    book_root: number;
    book_forward: number;
    book_close: number;
    book_forward_marker: number;
};

export const dialogueUiContentFiles = [
    'pack/interface.pack',
    'scripts/general/scripts/book.rs2',
    'scripts/general/scripts/scroll.rs2',
    'scripts/general/scripts/quests.rs2',
] as const;

function body(block: Rs2Block): string {
    return block.body.map(line => stripComment(line.text)).join('\n');
}

function hasStatement(block: Rs2Block, expected: string): boolean {
    let found = false;
    walkStatements(parseBody(block.body), statement => {
        if (statement.text === expected) found = true;
    });
    return found;
}

function hasQuestScrollCompletionProof(block: Rs2Block): boolean {
    const switchBody = /\bswitch_component\s*\(\s*\$component\s*\)\s*\{([\s\S]*?)\n\}/.exec(body(block))?.[1];
    if (!switchBody) return false;
    const related = /(?:^|\n)\s*case\s+questlist:seaslug\s*,\s*questlist:itwatchtower\s*:\s*([\s\S]*?)(?=\n\s*case\b|$)/.exec(switchBody)?.[1];
    const fallback = /(?:^|\n)\s*case\s+default\s*:\s*([\s\S]*?)(?=\n\s*case\b|$)/.exec(switchBody)?.[1];
    const opensQuestScroll = (branch: string | undefined) =>
        branch !== undefined && /^\s*if_openmain\(questscroll\)\s*;/m.test(branch);
    return opensQuestScroll(related) && opensQuestScroll(fallback);
}


function oneBlock(blocks: Rs2Block[], kind: string, name: string): Rs2Block {
    const found = blocks.filter(block => block.kind === kind && block.name === name);
    if (found.length !== 1) throw new Error(`dialogue_ui: expected one [${kind},${name}] block`);
    return found[0];
}

export type InterfaceAliasIdentity = { id: number; line: number };

/** Resolve selected interface.pack aliases without losing duplicate rows. */
export function resolveInterfaceAliases(content: string, aliases: readonly string[], family: string): Record<string, InterfaceAliasIdentity> {
    const wanted = new Set(aliases);
    if (wanted.size !== aliases.length) throw new Error(`${family}: requested interface aliases are duplicated`);

    const rows = new Map<string, InterfaceAliasIdentity[]>();
    const pack = requireGatherText(content, 'pack/interface.pack');
    for (const [index, raw] of pack.split(/\r?\n/).entries()) {
        const line = raw.trim();
        if (!line || line.startsWith('//')) continue;
        const equals = line.indexOf('=');
        if (equals <= 0) continue;
        const alias = line.slice(equals + 1);
        if (!wanted.has(alias)) continue;
        const matches = rows.get(alias) ?? [];
        matches.push({ id: Number(line.slice(0, equals)), line: index + 1 });
        rows.set(alias, matches);
    }

    const resolved: Record<string, InterfaceAliasIdentity> = {};
    const ids = new Map<number, string>();
    for (const alias of aliases) {
        const found = rows.get(alias) ?? [];
        if (found.length === 0 || (found.length === 1 && (!Number.isInteger(found[0].id) || found[0].id <= 0))) {
            throw new Error(`${family}: missing or invalid interface alias ${alias}`);
        }
        if (found.length !== 1) throw new Error(`${family}: ambiguous interface alias ${alias}`);
        const previous = ids.get(found[0].id);
        if (previous !== undefined) throw new Error(`${family}: controls have ambiguous packed identities (${previous}, ${alias})`);
        ids.set(found[0].id, alias);
        resolved[alias] = found[0];
    }
    return resolved;
}


/** Resolve only source-proven generic/quest-completion scrolls and book controls. */
export function extractDialogueUiFacts(content: string): DialogueUiIds {
    const book = parseRs2Blocks(dialogueUiContentFiles[1], requireGatherText(content, dialogueUiContentFiles[1]));
    const scroll = parseRs2Blocks(dialogueUiContentFiles[2], requireGatherText(content, dialogueUiContentFiles[2]));
    if (!hasStatement(oneBlock(scroll, 'label', 'scroll_pirate_message'), 'if_openmain(scroll)')) {
        throw new Error('dialogue_ui: pirate scroll does not open the main scroll root');
    }

    const questComplete = oneBlock(
        parseRs2Blocks(dialogueUiContentFiles[3], requireGatherText(content, dialogueUiContentFiles[3])),
        'proc',
        'send_quest_complete',
    );
    if (!hasQuestScrollCompletionProof(questComplete)) {
        throw new Error('dialogue_ui: quest completion does not open the authored quest scroll in both branches');
    }
    if (!book.some(block => hasStatement(block, 'if_openmain(book)'))) {
        throw new Error('dialogue_ui: book does not open its main root');
    }
    const forward = book.filter(block => block.kind === 'if_button'
        && /^book:[a-zA-Z0-9_]+$/.test(block.name)
        && /^\s*case\s+the_shield_of_arrav\s*:\s*@book_flip_page\s*\(\s*\^book_direction_forward\s*,/m.test(body(block)));
    if (forward.length !== 1) throw new Error('dialogue_ui: expected one source-proven book forward handler');
    const close = parseBody(oneBlock(book, 'if_button', 'book:close').body);
    if (close.length !== 1 || close[0].kind !== 'stmt' || close[0].text !== 'if_close()') {
        throw new Error('dialogue_ui: book close is not the source close handler');
    }
    const maxPage = parseBody(oneBlock(book, 'label', 'book_flip_page').body)
        .filter(node => node.kind === 'if' && node.cond === '%book_page = $max_page');
    if (maxPage.length !== 1 || maxPage[0].kind !== 'if') {
        throw new Error('dialogue_ui: expected one max-page forward visibility branch');
    }
    const marker = maxPage[0];
    const hide = marker.then.length === 1 && marker.then[0].kind === 'stmt'
        ? /^if_sethide\((book:[a-zA-Z0-9_]+), true\)$/.exec(marker.then[0].text) : null;
    if (!hide || marker.els.length !== 1 || marker.els[0].kind !== 'stmt'
        || marker.els[0].text !== `if_sethide(${hide[1]}, false)`) {
        throw new Error('dialogue_ui: unsupported max-page forward visibility marker');
    }

    const resolved = resolveInterfaceAliases(content, [
        'scroll',
        'questscroll',
        'book',
        forward[0].name,
        'book:close',
        hide[1],
    ], 'dialogue_ui');
    return {
        scroll_root: resolved.scroll.id,
        quest_scroll_root: resolved.questscroll.id,
        book_root: resolved.book.id,
        book_forward: resolved[forward[0].name].id,
        book_close: resolved['book:close'].id,
        book_forward_marker: resolved[hide[1]].id,
    };
}
