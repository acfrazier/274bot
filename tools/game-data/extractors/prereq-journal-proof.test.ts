import assert from 'node:assert/strict';
import { parseBody, type Node } from './gathering-rs2.ts';
import { boundedJournalRange } from './prereq-journal-proof.ts';

const constants = new Map<string, number>();
const needles = ['priest', 'drezel'];

function parsed(lines: string[]): Node[] {
    return parseBody(lines.map((text, index) => ({ line: index + 1, text })));
}

function cited(lines: string[]): Set<number> {
    return new Set(lines.map((_, index) => index + 1));
}

const ladder = [
    'if (%progress <= 8) {',
    '    def_string $page = "A different page";',
    '    ~quest_journal("Priest", $page);',
    '} else if (%progress >= 10 & %progress <= 59) {',
    '    def_string $page = "Another different page";',
    '    ~quest_journal("Priest", $page);',
    '} else {',
    '    def_string $page = "@red@Priest|Drezel needs advice";',
    '    ~quest_journal("Priest", $page);',
    '}',
];

for (const domain of [{ min: 0, max: 61 }, { min: 5, max: 61 }, { min: 8, max: 61 }]) {
    assert.deepEqual(boundedJournalRange(parsed(ladder), 'progress', constants, domain, cited(ladder), needles), { min: 9, max: 61 });
}
assert.notDeepEqual(
    boundedJournalRange(parsed(ladder), 'progress', constants, { min: 0, max: 61 }, cited(ladder), needles),
    { min: 61, max: 61 },
);

assert.equal(boundedJournalRange(parsed(ladder), 'progress', constants, null, cited(ladder), needles), null);

const missingGuardCitation = cited(ladder);
missingGuardCitation.delete(1);
assert.equal(boundedJournalRange(parsed(ladder), 'progress', constants, { min: 0, max: 61 }, missingGuardCitation, needles), null);
const missingStatementCitation = cited(ladder);
missingStatementCitation.delete(8);
assert.equal(boundedJournalRange(parsed(ladder), 'progress', constants, { min: 0, max: 61 }, missingStatementCitation, needles), null);

const unknownGuard = parsed([
    'if (%progress = ^missing) {',
    '    def_string $page = "Priest Drezel";',
    '    ~quest_journal("Priest", $page);',
    '} else {',
    '    def_string $page = "Other";',
    '    ~quest_journal("Priest", $page);',
    '}',
]);
assert.equal(boundedJournalRange(unknownGuard, 'progress', constants, { min: 0, max: 61 }, cited([
    'if (%progress = ^missing) {',
    '    def_string $page = "Priest Drezel";',
    '    ~quest_journal("Priest", $page);',
    '} else {',
    '    def_string $page = "Other";',
    '    ~quest_journal("Priest", $page);',
    '}',
]), needles), null);

const opaqueBranch = [
    'if (%progress = 0) {',
    '    def_string $page = "Priest Drezel";',
    '    ~quest_journal("Priest", $page);',
    '} else {',
    '    $page = %dynamic;',
    '    ~quest_journal("Priest", $page);',
    '}',
];
assert.equal(boundedJournalRange(parsed(opaqueBranch), 'progress', constants, { min: 0, max: 61 }, cited(opaqueBranch), needles), null);

const loop = [
    'while (%progress < 1) {',
    '    def_string $page = "Priest Drezel";',
    '    ~quest_journal("Priest", $page);',
    '}',
];
assert.equal(boundedJournalRange(parsed(loop), 'progress', constants, { min: 0, max: 61 }, cited(loop), needles), null);

const varpWrite = ['%progress = 1;'];
assert.equal(boundedJournalRange(parsed(varpWrite), 'progress', constants, { min: 0, max: 61 }, cited(varpWrite), needles), null);

const splitNeedles = [
    'if (%progress = 0) {',
    '    def_string $page = "Priest";',
    '    ~quest_journal("Priest", $page);',
    '} else {',
    '    def_string $page = "Drezel";',
    '    ~quest_journal("Priest", $page);',
    '}',
];
assert.equal(boundedJournalRange(parsed(splitNeedles), 'progress', constants, { min: 0, max: 61 }, cited(splitNeedles), needles), null);

const interpolatedLiteralPrograms = [
    [
        'if (%progress = 0) {',
        '    def_string $page = "<displayname>";',
        '    ~quest_journal("Priest", $page);',
        '} else {',
        '    def_string $page = "Priest Drezel";',
        '    ~quest_journal("Priest", $page);',
        '}',
    ],
    [
        'def_string $page = "safe";',
        'if (%progress = 0) {',
        '    $page = "<displayname>";',
        '    ~quest_journal("Priest", $page);',
        '} else {',
        '    $page = "Priest Drezel";',
        '    ~quest_journal("Priest", $page);',
        '}',
    ],
    [
        'def_string $page = "safe";',
        'if (%progress = 0) {',
        '    $page = append($page, "<displayname>");',
        '    ~quest_journal("Priest", $page);',
        '} else {',
        '    $page = "Priest Drezel";',
        '    ~quest_journal("Priest", $page);',
        '}',
    ],
    [
        'def_string $page = "Priest Drezel";',
        'if (%progress = 0) {',
        '    ~quest_journal("<displayname>", $page);',
        '} else {',
        '    ~quest_journal("Priest", $page);',
        '}',
    ],
];
for (const lines of interpolatedLiteralPrograms) {
    assert.equal(
        boundedJournalRange(parsed(lines), 'progress', constants, { min: 1, max: 61 }, cited(lines), needles),
        null,
    );
}
