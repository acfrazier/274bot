import { parsePack, requireGatherText } from './common.ts';
import { parseRs2Blocks, stripComment, type Rs2Block } from './gathering-content.ts';
import { parseBody, walkStatements } from './gathering-rs2.ts';

export type DialogueUiIds = {
    scroll_root: number;
    book_root: number;
    book_forward: number;
    book_close: number;
    book_forward_marker: number;
};

export const dialogueUiContentFiles = [
    'pack/interface.pack',
    'scripts/general/scripts/book.rs2',
    'scripts/general/scripts/scroll.rs2',
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

function oneBlock(blocks: Rs2Block[], kind: string, name: string): Rs2Block {
    const found = blocks.filter(block => block.kind === kind && block.name === name);
    if (found.length !== 1) throw new Error(`dialogue_ui: expected one [${kind},${name}] block`);
    return found[0];
}

/** Resolve only source-proven main scroll/book controls, never debug-name guesses. */
export function extractDialogueUiFacts(content: string): DialogueUiIds {
    const pack = parsePack(requireGatherText(content, dialogueUiContentFiles[0]));
    const book = parseRs2Blocks(dialogueUiContentFiles[1], requireGatherText(content, dialogueUiContentFiles[1]));
    const scroll = parseRs2Blocks(dialogueUiContentFiles[2], requireGatherText(content, dialogueUiContentFiles[2]));
    const resolve = (alias: string): number => {
        const id = pack.get(alias);
        if (id === undefined || !Number.isInteger(id) || id <= 0) {
            throw new Error(`dialogue_ui: missing or invalid interface alias ${alias}`);
        }
        return id;
    };

    if (!hasStatement(oneBlock(scroll, 'label', 'scroll_pirate_message'), 'if_openmain(scroll)')) {
        throw new Error('dialogue_ui: pirate scroll does not open the main scroll root');
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

    const facts = {
        scroll_root: resolve('scroll'),
        book_root: resolve('book'),
        book_forward: resolve(forward[0].name),
        book_close: resolve('book:close'),
        book_forward_marker: resolve(hide[1]),
    };
    if (new Set(Object.values(facts)).size !== 5) throw new Error('dialogue_ui: controls have ambiguous packed identities');
    return facts;
}
