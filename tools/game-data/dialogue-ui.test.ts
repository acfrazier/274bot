import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { dialogueUiContentFiles, extractDialogueUiFacts } from './extractors/dialogue-ui.ts';

function fixture(marker = 'book:com_87') {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'dialogue-ui-'));
    const files = new Map<string, string>([
        [dialogueUiContentFiles[0], `1136=scroll\n297=questscroll\n837=book\n841=book:com_86\n10162=book:close\n842=${marker}\n843=book:misleading_forward\n`],
        [dialogueUiContentFiles[1], `[if_button,book:close]\nif_close();\n[if_button,book:com_86]\nswitch_obj (%open_book) {\ncase the_shield_of_arrav : @book_flip_page(^book_direction_forward, 0, 2, shieldofarrav_book);\n}\np_delay(0);\n[proc,book_page]\nif_openmain(book);\n[label,book_flip_page]\n%book_page = add(%book_page, $page_direction);\nif (%book_page = $max_page) {\nif_sethide(${marker}, true);\n} else {\nif_sethide(${marker}, false);\n}\n`],
        [dialogueUiContentFiles[2], '[label,scroll_pirate_message]\nif_openmain(scroll);\n'],
        [dialogueUiContentFiles[3], '[proc,send_quest_complete]\nswitch_component ($component) {\ncase questlist:seaslug, questlist:itwatchtower :\nif_openmain(questscroll);\ncase default :\nif_openmain(questscroll);\n}\n'],
    ]);
    const save = () => {
        for (const relative of dialogueUiContentFiles) {
            if (!files.has(relative)) fs.rmSync(path.join(root, relative), { force: true });
        }
        for (const [relative, text] of files) {
            const file = path.join(root, relative);
            fs.mkdirSync(path.dirname(file), { recursive: true });
            fs.writeFileSync(file, text);
        }
    };
    save();
    return { root, files, save };
}

for (const marker of ['book:com_87', 'book:page_right_button']) {
    const source = fixture(marker);
    try {
        assert.deepEqual(extractDialogueUiFacts(source.root), {
            scroll_root: 1136, quest_scroll_root: 297, book_root: 837, book_forward: 841,
            book_close: 10162, book_forward_marker: 842,
        }, 'quest completion and book controls are source-proven, not semantic-looking aliases');
    } finally {
        fs.rmSync(source.root, { recursive: true, force: true });
    }
}

for (const [mutate, reason] of [
    [(files: Map<string, string>) => files.set(dialogueUiContentFiles[0], files.get(dialogueUiContentFiles[0])!.replace('841=book:com_86\n', '')), /missing or invalid interface alias/],
    [(files: Map<string, string>) => files.set(dialogueUiContentFiles[0], files.get(dialogueUiContentFiles[0])!.replace('297=questscroll\n', '')), /missing or invalid interface alias/],
    [(files: Map<string, string>) => files.set(dialogueUiContentFiles[0], files.get(dialogueUiContentFiles[0])!.replace('297=questscroll', '0=questscroll')), /missing or invalid interface alias/],
    [(files: Map<string, string>) => files.set(dialogueUiContentFiles[0], files.get(dialogueUiContentFiles[0])!.replace('297=questscroll', '1136=questscroll')), /ambiguous packed identities/],
    [(files: Map<string, string>) => files.set(dialogueUiContentFiles[1], files.get(dialogueUiContentFiles[1])! + '\n[if_button,book:misleading_forward]\ncase the_shield_of_arrav : @book_flip_page(^book_direction_forward, 0, 2, shieldofarrav_book);\n'), /one source-proven book forward handler/],
    [(files: Map<string, string>) => files.set(dialogueUiContentFiles[1], files.get(dialogueUiContentFiles[1])!.replace('@book_flip_page(^book_direction_forward, 0, 2, shieldofarrav_book);', '~mesbox("@book_flip_page(^book_direction_forward, 0, 2, shieldofarrav_book);");')), /one source-proven book forward handler/],
    [(files: Map<string, string>) => files.set(dialogueUiContentFiles[1], files.get(dialogueUiContentFiles[1])!.replace('%book_page = $max_page', '%book_page = $min_page')), /max-page forward visibility branch/],
    [(files: Map<string, string>) => files.set(dialogueUiContentFiles[1], files.get(dialogueUiContentFiles[1])!.replace('if_sethide(book:com_87, false)', 'if_sethide(book:misleading_forward, false)')), /unsupported max-page forward visibility marker/],
    [(files: Map<string, string>) => files.set(dialogueUiContentFiles[1], files.get(dialogueUiContentFiles[1])!.replace('if_close();', 'if_openmain(book);')), /not the source close handler/],
    [(files: Map<string, string>) => files.set(dialogueUiContentFiles[2], '[label,scroll_pirate_message]\n~mesbox("if_openmain(scroll);");\n'), /does not open the main scroll root/],
    [(files: Map<string, string>) => files.set(dialogueUiContentFiles[2], '[label,scroll_pirate_message]\nif_openchat(scroll);\n'), /does not open the main scroll root/],
    [(files: Map<string, string>) => files.delete(dialogueUiContentFiles[3]), /required file missing/],
    [(files: Map<string, string>) => files.set(dialogueUiContentFiles[3], files.get(dialogueUiContentFiles[3])!.replaceAll('if_openmain(questscroll)', 'if_openmain(scroll)')), /quest completion does not open the authored quest scroll/],
] as const) {
    const source = fixture();
    try {
        mutate(source.files);
        source.save();
        assert.throws(() => extractDialogueUiFacts(source.root), reason);
    } finally {
        fs.rmSync(source.root, { recursive: true, force: true });
    }
}

console.log('dialogue_ui source identity regressions passed');
