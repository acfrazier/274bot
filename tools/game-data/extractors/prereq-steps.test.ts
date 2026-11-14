import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { extractQuestIdentityFacts } from './quests.ts';
import { assertPrereqStepFacts, extractPrereqStepsFacts, loadPrereqCrossingProjection, PREREQ_STEPS_SCHEMA } from './prereq-steps.ts';
import { revisions } from '../generate.ts';
import { postCompletionProgressWriters, questCompletionPrerequisites } from './prereq-preservation.ts';
import { parseRs2Blocks } from './gathering-content.ts';

assert.equal(PREREQ_STEPS_SCHEMA, 3);
assert.deepEqual(extractPrereqStepsFacts(274), { rows: [], journals: [] });

const content = revisions.find((spec) => spec.revision === 289)!.content;
const facts = extractPrereqStepsFacts(289);
const quests = extractQuestIdentityFacts(content, 289);
assertPrereqStepFacts(facts);

assert.deepEqual(facts.rows.map((row) => row.id), ['priestperil:drezel-advice', 'druidspirit:drezel-start']);
assert.deepEqual(facts.journals.map((journal) => journal.id), [
    'blackarmgang:phoenixgang-stage',
    'hero:phoenixgang-stage',
    'priestperil:mausoleum-progress',
]);
assert.deepEqual(facts.journals[0].establishes.values, { min: 10, max: 10 });
assert.equal(facts.journals[0].establishes.quest, 'blackarmgang');
assert.deepEqual(facts.journals[0].rules.map((rule) => rule.proves), [
    { min: 10, max: 10 },
    { min: 0, max: 8 },
]);
assert.deepEqual(facts.journals[1].establishes.values, { min: 3, max: 15 });
assert.deepEqual(facts.journals[1].rules[0].proves, { min: 3, max: 13 });
assert.deepEqual(quests.rows.find((row) => row.id === 'priestperil')!.stage_domain, { min: 0, max: 61 });
assert.deepEqual(facts.journals[2].establishes.values, { min: 9, max: 61 });
assert.deepEqual(facts.journals[2].rules[0].proves, { min: 9, max: 61 });
assert.deepEqual(facts.journals[2].rules.map((rule) => rule.proves), [
    { min: 9, max: 61 },
    { min: 5, max: 5 },
    { min: 6, max: 6 },
    { min: 7, max: 7 },
    { min: 8, max: 8 },
    { min: 10, max: 59 },
]);
assert.deepEqual(facts.journals[2].after, [
    { proves: { min: 9, max: 61 }, after: { min: 9, max: 61 } },
    { proves: { min: 5, max: 5 }, after: { min: 5, max: 61 } },
    { proves: { min: 6, max: 6 }, after: { min: 6, max: 61 } },
    { proves: { min: 7, max: 7 }, after: { min: 7, max: 61 } },
    { proves: { min: 8, max: 8 }, after: { min: 8, max: 61 } },
    { proves: { min: 10, max: 59 }, after: { min: 10, max: 61 } },
]);
assert.equal(quests.rows.find((row) => row.id === 'priestperil')!.transmission, 'not_transmitted');
assert.deepEqual(quests.rows.find((row) => row.id === 'druidspirit')!.stage_domain, { min: 0, max: 110 });
assert.equal(quests.rows.find((row) => row.id === 'druidspirit')!.transmission, 'not_transmitted');
// Completion is 60, but scripts/areas/area_mausoleum/scripts/drezel.rs2:147
// writes priestperil_access_holy_barrier=61 after completion. Colour cannot
// select between those stages; the explicit observation windows below still can.
assert.equal(quests.rows.find((row) => row.id === 'priestperil')!.colour_authoritative, false);
// Keep the positive control: quest_druidspirit/scripts/quest_druidspirit.rs2:422
// writes completion=110; its earlier-stage writers remain guarded.
assert.equal(quests.rows.find((row) => row.id === 'druidspirit')!.colour_authoritative, true);
assert.deepEqual(facts.rows[0].after, [
    { proves: { min: 61, max: 61 }, after: { min: 61, max: 61 } },
    { proves: { min: 60, max: 61 }, after: { min: 60, max: 61 } },
    { proves: { min: 60, max: 60 }, after: { min: 60, max: 61 } },
    { proves: { min: 0, max: 59 }, after: { min: 0, max: 61 } },
]);
assert.deepEqual(facts.rows[1].after, [
    { proves: { min: 5, max: 5 }, after: { min: 5, max: 110 } },
    { proves: { min: 1, max: 110 }, after: { min: 1, max: 110 } },
    { proves: { min: 5, max: 10 }, after: { min: 5, max: 110 } },
    { proves: { min: 0, max: 0 }, after: { min: 0, max: 110 } },
]);

// Every generated coordinate derives from the CLI shared pack projection: the
// test asserts pack agreement here, never authored constants. at stays the loc
// anchor while the required takeoff carries the exact legal stand.
const projection = loadPrereqCrossingProjection(289);
assert.ok(projection && projection.entries.length > 0, '289 requires a nonempty pack projection');
const legalIdentities = new Set(projection.entries
    .filter((entry) => entry.standable && entry.admissible
        && [3443, 3506, 3507].includes(entry.crossing.loc_id)
        && entry.crossing.option === 1 && entry.crossing.kind === 'door')
    .map((entry) => JSON.stringify(entry.crossing)));
const factIdentities = facts.rows.flatMap((row) => row.crossings.map((entry) => JSON.stringify(entry.crossing)));
assert.deepEqual(new Set(factIdentities), legalIdentities);
for (const row of facts.rows) for (const entry of row.crossings) {
    assert.deepEqual(entry.pass.landing, entry.crossing.to);
    assert.equal(typeof entry.crossing.takeoff.x, 'number');
}
const barrier = facts.rows[0];
assert.ok(barrier.crossings.length >= 1, 'A1 emits every distinct legal takeoff from the pack');
assert.ok(barrier.crossings.every((entry) =>
    entry.crossing.loc_id === 3443 && entry.crossing.option === 1 && entry.crossing.kind === 'door'));
assert.equal(new Set(barrier.crossings.map((entry) => JSON.stringify(entry.crossing.takeoff))).size, barrier.crossings.length);
const doors = facts.rows[1];
assert.equal(doors.crossings.length, 4);
assert.deepEqual(doors.crossings.map((entry) => entry.crossing.loc_id), [3507, 3506, 3507, 3506]);
assert.deepEqual(doors.crossings.map((entry) => entry.delay.ticks), [2, 2, 0, 0]);

const overstatedHeroEstablishment = structuredClone(facts.journals[1]);
overstatedHeroEstablishment.establishes.values.max = 14;
assert.throws(() => assertPrereqStepFacts({ rows: [], journals: [overstatedHeroEstablishment] }), /establishment window has no matching after rule/);

// Synthetic-projection checks below exercise the fail-closed projection joins
// without content: templates bind to pack geometry, never to edited constants.
const tile = (x: number, z: number, level = 0) => ({ x, z, level });
function packEntry(loc: number, at: { x: number; z: number; level: number }, takeoff: { x: number; z: number; level: number }, to: { x: number; z: number; level: number }, standable = true, admissible = true, quest_gate_alternatives: unknown[][] = [[]]) {
    return {
        crossing: { loc_id: loc, option: 1, at, takeoff, to, kind: 'door' },
        standable,
        admissible,
        quest_gate_alternatives,
    };
}
function baseEntries() {
    return [
        packEntry(3443, tile(10, 20), tile(10, 21), tile(30, 40)),
        packEntry(3443, tile(10, 20), tile(11, 20), tile(30, 40)),
        packEntry(3507, tile(50, 60), tile(50, 60), tile(50, 59)),
        packEntry(3507, tile(50, 60), tile(50, 59), tile(50, 60)),
        packEntry(3506, tile(51, 60), tile(51, 60), tile(51, 59)),
        packEntry(3506, tile(51, 60), tile(51, 59), tile(51, 60)),
    ];
}
function writeProjection(entries: unknown[], identical_edge_groups: unknown = []) {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'prereq-projection-'));
    const file = path.join(dir, 'crossings.json');
    fs.writeFileSync(file, JSON.stringify({ schema: 3, crossings: entries, identical_edge_groups }));
    return { dir, file };
}
function withProjection(entries: unknown[], run: (overrides: { projectionPath: string }) => void, identical_edge_groups: unknown = []) {
    const { dir, file } = writeProjection(entries, identical_edge_groups);
    try {
        run({ projectionPath: file });
    } finally {
        fs.rmSync(dir, { recursive: true, force: true });
    }
}
function withContent(files: Record<string, string>, run: (content: string) => void) {
    const content = fs.mkdtempSync(path.join(os.tmpdir(), 'prereq-preservation-'));
    try {
        for (const [relative, text] of Object.entries(files)) {
            const filename = path.join(content, relative);
            fs.mkdirSync(path.dirname(filename), { recursive: true });
            fs.writeFileSync(filename, text);
        }
        run(content);
    } finally {
        fs.rmSync(content, { recursive: true, force: true });
    }
}

const constants = 'scripts/test.constant';

withProjection(baseEntries(), (overrides) => {
    const joined = extractPrereqStepsFacts(289, overrides);
    assert.equal(joined.rows[0].crossings.length, 2);
    assert.deepEqual(joined.rows[0].crossings.map((entry) => entry.crossing.takeoff), [tile(10, 21), tile(11, 20)]);
    assert.equal(joined.rows[1].crossings.length, 4);
    assert.ok(joined.rows.flatMap((row) => row.crossings).every((entry) => JSON.stringify(entry.pass.landing) === JSON.stringify(entry.crossing.to)));
});
const identicalEdges = [['graph.edges[1]', 'graph.edges[2]']];
withProjection(baseEntries(), (overrides) => {
    assert.deepEqual(loadPrereqCrossingProjection(289, overrides)!.identical_edge_groups, identicalEdges);
}, identicalEdges);
withProjection(baseEntries(), (overrides) => {
    assert.throws(() => loadPrereqCrossingProjection(289, overrides), /identical_edge_groups/);
}, [[1, 'graph.edges[2]']]);
{
    const { dir, file } = writeProjection(baseEntries());
    try {
        const older = JSON.parse(fs.readFileSync(file, 'utf8'));
        older.schema = 2;
        fs.writeFileSync(file, JSON.stringify(older));
        assert.throws(() => loadPrereqCrossingProjection(289, { projectionPath: file }), /schema:3/);
    } finally {
        fs.rmSync(dir, { recursive: true, force: true });
    }
}
withProjection([], (overrides) => {
    assert.throws(() => extractPrereqStepsFacts(289, overrides), /no legal pack crossing/);
});
withProjection([baseEntries()[0]], (overrides) => {
    assert.throws(() => extractPrereqStepsFacts(289, overrides), /matches 0 legal pack crossings/);
});
withProjection([...baseEntries(), baseEntries()[2]], (overrides) => {
    assert.throws(() => extractPrereqStepsFacts(289, overrides), /duplicate crossing identity/);
});
withProjection(baseEntries().map((entry) => entry.crossing.loc_id === 3443 ? { ...entry, standable: false } : entry), (overrides) => {
    assert.throws(() => extractPrereqStepsFacts(289, overrides), /no legal pack crossing/);
});
withProjection([...baseEntries(), packEntry(3507, tile(50, 60), tile(50, 60), tile(51, 60))], (overrides) => {
    assert.throws(() => extractPrereqStepsFacts(289, overrides), /bound by no template/);
});
{
    const savedCrossings = process.env.GAME_DATA_289_PREREQ_CROSSINGS;
    delete process.env.GAME_DATA_289_PREREQ_CROSSINGS;
    try {
        assert.throws(() => extractPrereqStepsFacts(289), /set GAME_DATA_289_PREREQ_CROSSINGS/);
    } finally {
        if (savedCrossings !== undefined) process.env.GAME_DATA_289_PREREQ_CROSSINGS = savedCrossings;
    }
}

// Schema-3 condition groups: two stage-window alternatives for one physical
// crossing share that crossing's identity and keep both condition arrays —
// the loader never unions them into one conjunction, and the join still
// yields one catalog identity per physical crossing.
const stageWindow = (stage: number) => ({
    Window: { quest: 'priestperil', signal: 'priestperil_stage', values: { min: stage, max: stage } },
});
{
    const gated = packEntry(3443, tile(10, 20), tile(10, 21), tile(30, 40), true, true,
        [[stageWindow(7)], [stageWindow(8)]]);
    const entries = [gated, ...baseEntries().slice(1)];
    withProjection(entries, (overrides) => {
        const projection = loadPrereqCrossingProjection(289, overrides)!;
        const match = projection.entries.filter((entry) =>
            entry.crossing.loc_id === 3443
            && JSON.stringify(entry.crossing.takeoff) === JSON.stringify(tile(10, 21)));
        assert.equal(match.length, 1);
        assert.deepEqual(match[0].quest_gate_alternatives, [[stageWindow(7)], [stageWindow(8)]]);
        const joined = extractPrereqStepsFacts(289, overrides);
        const barrierTakeoffs = joined.rows[0].crossings.map((entry) => JSON.stringify(entry.crossing.takeoff));
        assert.equal(new Set(barrierTakeoffs).size, barrierTakeoffs.length);
        assert.ok(barrierTakeoffs.includes(JSON.stringify(tile(10, 21))));
        assert.equal(joined.rows[0].crossings.length, 2);
    });
}
{
    const malformed = baseEntries().map((entry, index) => index === 0
        ? { ...entry, quest_gate_alternatives: [{ Complete: 'priestperil' }] }
        : entry);
    withProjection(malformed, (overrides) => {
        assert.throws(() => extractPrereqStepsFacts(289, overrides), /must be a list of QuestGate/);
    });
}

withContent({
    [constants]: '^left_complete = 12\n^right_complete = 34\n',
    'scripts/quests/quest_fixture/scripts/requirements.rs2': `[proc,has_fixture_quest_requirements]()(boolean)
if (%leftgang < ^left_complete & %rightgang < ^right_complete) {
    return(false);
}
return(true);
`,
    'scripts/guards.rs2': `[oploc1,blocked]
if (%leftgang < ^left_complete & %rightgang < ^right_complete) {
    switch_int(~p_choice1("Proceed?", 1)) {
        case 1:
            %progress = 1;
    }
}
[oploc1,reachable]
if (%rightgang < ^right_complete) {
    %progress = 2;
}
[oploc1,unknown]
if (npc_find(coord, goblin, 10, 0) = true) {
    %progress = 3;
}
`,
}, (content) => {
    const requirementFile = 'scripts/quests/quest_fixture/scripts/requirements.rs2';
    const writerFile = 'scripts/guards.rs2';
    const blocks = [
        ...parseRs2Blocks(requirementFile, fs.readFileSync(path.join(content, requirementFile), 'utf8')),
        ...parseRs2Blocks(writerFile, fs.readFileSync(path.join(content, writerFile), 'utf8')),
    ];
    const constants = new Map([['left_complete', 12], ['right_complete', 34]]);
    const prerequisite = questCompletionPrerequisites(blocks, 'fixture', constants);
    assert.notEqual(prerequisite, null);
    const writerBlocks = blocks.filter((block) => block.span.file === writerFile);
    const candidates = writerBlocks.map((block) => {
        const assignment = block.body.find(({ text }) => /%progress\s*=/.test(text));
        assert.ok(assignment, `${block.name} has a progress writer`);
        return {
            block,
            line: assignment.line,
            right: assignment.text.trim().replace(/^%progress\s*=\s*(.+?)\s*;?$/, '$1').trim(),
        };
    });
    const possible = postCompletionProgressWriters(
        blocks,
        'progress',
        { min: 5, max: 5 },
        constants,
        candidates,
        prerequisite,
    );
    assert.deepEqual(possible.map(({ right }) => right), ['2', '3']);
});

withContent({
    [constants]: '^left_complete = 12\n',
    'scripts/quests/quest_fixture/scripts/requirements.rs2': `[proc,has_fixture_quest_requirements]()(boolean)
if (%leftgang < ^left_complete & inv_total(inv, coin) > 0) {
    return(false);
}
return(true);
`,
}, (content) => {
    const file = 'scripts/quests/quest_fixture/scripts/requirements.rs2';
    const blocks = parseRs2Blocks(file, fs.readFileSync(path.join(content, file), 'utf8'));
    assert.equal(questCompletionPrerequisites(blocks, 'fixture', new Map([['left_complete', 12]])), null);
});

for (const fixture of [
    { call: 'queue(delayed, 30, 0);', extra: '', expected: ['2'], freshExpected: [] },
    { call: 'queue*(delayed, 30, 0);', extra: '', expected: ['2'], freshExpected: [] },
    { call: 'weakqueue(delayed, 30, 0);', extra: '', expected: ['2'], freshExpected: [] },
    { call: 'strongqueue(delayed, 30, 0);', extra: '', expected: ['2'], freshExpected: [] },
    { call: 'longqueue(delayed, 30, 0);', extra: '', expected: ['2'], freshExpected: [] },
    { call: 'queue(delayed, 30, 0);', extra: 'queue($unknown_target, 30, 0);', expected: ['2'], freshExpected: ['2'] },
    { call: 'queue(delayed, 30, 0);', extra: 'queue(unbound_target, 30, 0);', expected: ['2'], freshExpected: ['2'] },
    { call: '', extra: '', expected: ['2'], freshExpected: ['2'] },
]) {
    const blocks = parseRs2Blocks('scripts/queued-progress.rs2', `[opnpc1,root]
if (%progress < 5) {
    ${fixture.call}
}
${fixture.extra}
[queue,delayed]
%progress = 2;
`);
    const queued = blocks.find((block) => block.kind === 'queue')!;
    const assignment = queued.body.find(({ text }) => text.includes('%progress ='))!;
    const candidates = [{ block: queued, line: assignment.line, right: '2' }];
    const possible = postCompletionProgressWriters(
        blocks, 'progress', { min: 5, max: 5 }, new Map(), candidates,
    );
    assert.deepEqual(possible.map(({ right }) => right), fixture.expected);
    const afterLogin = postCompletionProgressWriters(
        blocks, 'progress', { min: 5, max: 5 }, new Map(), candidates, undefined, undefined, true,
    );
    assert.deepEqual(afterLogin.map(({ right }) => right), fixture.freshExpected);
}

{
    const blocks = parseRs2Blocks('scripts/post-login-queue.rs2', `[opnpc1,root]
if (%progress >= 5) {
    queue(delayed, 30, 0);
}
[queue,delayed]
%progress = 2;
`);
    const queued = blocks.find((block) => block.kind === 'queue')!;
    const assignment = queued.body.find(({ text }) => text.includes('%progress ='))!;
    const possible = postCompletionProgressWriters(
        blocks,
        'progress',
        { min: 5, max: 5 },
        new Map(),
        [{ block: queued, line: assignment.line, right: '2' }],
        undefined,
        undefined,
        true,
    );
    assert.deepEqual(possible.map(({ right }) => right), ['2'], 'a queue scheduled after completion remains possible');
}

{
    const blocks = parseRs2Blocks('scripts/direct-progress.rs2', `[opnpc1,root]
if (%progress >= 5) {
    %progress = 2;
}
`);
    const entry = blocks.find((block) => block.kind === 'opnpc1')!;
    const assignment = entry.body.find(({ text }) => text.includes('%progress ='))!;
    const possible = postCompletionProgressWriters(
        blocks,
        'progress',
        { min: 5, max: 5 },
        new Map(),
        [{ block: entry, line: assignment.line, right: '2' }],
        '%progress < 5',
        undefined,
        true,
    );
    assert.deepEqual(possible, [], 'completion prerequisites exclude a direct contradictory writer');
}

{
    const blocks = parseRs2Blocks('scripts/menu-progress.rs2', `[opnpc1,local_menu]
def_int $option;
if (%progress = 1) {
    $option = ~three_options("First", 1, "Second", 2, "Special", 3);
} else {
    $option = ~two_options("First, please", 1, "Second", 2);
}
switch_int($option) {
    case 3:
        %progress = 2;
}
[opnpc1,procedure_menu]
switch_int(~which_option) {
    case 4:
        %progress = 3;
}
[proc,which_option]()(int)
if (%progress >= 4) {
    return(~two_options("First", 1, "Second", 2));
}
return(~three_options("First", 1, "Second", 2, "Special", 4));
[opnpc1,label_menu]
if (%progress = 1) {
    @write_option(8);
} else {
    @write_option(1);
}
[label,write_option](int $option)
switch_int($option) {
    case 8:
        %progress = 4;
}
[opnpc1,unknown_menu]
switch_int(~unknown_option) {
    case 7:
        %progress = 1;
}
[proc,unknown_option]()(int)
return(stat(attack));
[opnpc1,partial_assignment]
def_int $option;
if (%progress = 1) {
    $option = 1;
}
switch_int($option) {
    case 8:
        %progress = 0;
}
[proc,two_options](string $first, int $one, string $second, int $two)(int)
switch_component(last_com) {
    case menu:first: return($one);
    case default: return($two);
}
[proc,three_options](string $first, int $one, string $second, int $two, string $third, int $three)(int)
switch_component(last_com) {
    case menu:first: return($one);
    case menu:second: return($two);
    case default: return($three);
}
`);
    const candidates = blocks.flatMap((block) => block.body.flatMap(({ line, text }) => {
        const assignment = /^\s*%progress\s*=\s*(.+?)\s*;?$/.exec(text);
        return assignment ? [{ block, line, right: assignment[1] }] : [];
    }));
    const possible = postCompletionProgressWriters(
        blocks, 'progress', { min: 5, max: 5 }, new Map(), candidates,
    );
    assert.deepEqual(possible.map(({ right }) => right), ['1', '0']);
}
