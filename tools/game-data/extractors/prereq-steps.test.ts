import assert from 'node:assert/strict';
import crypto from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { captureProgressWriterDomains, extractQuestIdentityFacts } from './quests.ts';
import { assertPrereqJournalPins, assertPrereqStepFacts, extractPrereqStepsFacts, loadPrereqCrossingProjection, prereqStepInputs, PREREQ_STEPS_SCHEMA } from './prereq-steps.ts';
import { revisions } from '../generate.ts';
import { parseBody } from './gathering-rs2.ts';
import { boundedJournalRange } from './prereq-journal-proof.ts';

assert.equal(PREREQ_STEPS_SCHEMA, 3);
assert.deepEqual(extractPrereqStepsFacts(274), { rows: [], journals: [] });

const content = revisions.find((spec) => spec.revision === 289)!.content;
const facts = extractPrereqStepsFacts(289);
const quests = extractQuestIdentityFacts(content, 289);
assertPrereqStepFacts(content, facts, quests.rows);

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
assert.deepEqual(facts.journals[2].after, [
    { proves: { min: 9, max: 61 }, after: { min: 9, max: 61 } },
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

// The generator refuses a changed source line before it can publish stale
// prerequisite facts. Only the first cited file is needed for this failure.
{
    const mutatedContent = fs.mkdtempSync(path.join(os.tmpdir(), 'prereq-content-line-'));
    const relative = 'scripts/areas/area_mausoleum/scripts/holy_barrier.rs2';
    const original = fs.readFileSync(path.join(content, relative), 'utf8');
    const pass = barrier.crossings[0].pass.message;
    assert.ok(original.includes(pass));
    try {
        const target = path.join(mutatedContent, relative);
        fs.mkdirSync(path.dirname(target), { recursive: true });
        fs.writeFileSync(target, original.replace(pass, 'The content pass message changed.'));
        assert.throws(() => assertPrereqStepFacts(mutatedContent, facts, quests.rows), /pass message is not pinned/);
    } finally {
        fs.rmSync(mutatedContent, { recursive: true, force: true });
    }
}
{
    const mutatedContent = fs.mkdtempSync(path.join(os.tmpdir(), 'prereq-journal-line-'));
    const sources = [
        'scripts/quests/quest_blackarmgang/scripts/blackarmgang_journal.rs2',
        'scripts/quests/quest_blackarmgang/configs/quest_blackarmgang.constant',
        'scripts/general/configs/quest.constant',
    ];
    try {
        for (const relative of sources) {
            const target = path.join(mutatedContent, relative);
            fs.mkdirSync(path.dirname(target), { recursive: true });
            fs.copyFileSync(path.join(content, relative), target);
        }
        const page = path.join(mutatedContent, sources[0]);
        const original = fs.readFileSync(page, 'utf8');
        const changed = original.replace('I searched the Phoenix Gang Headquarters and found', 'I searched a different place and found');
        assert.notEqual(changed, original);
        fs.writeFileSync(page, changed);
        assert.throws(() => assertPrereqJournalPins(mutatedContent, [facts.journals[0]]), /needle is not pinned/);
    } finally {
        fs.rmSync(mutatedContent, { recursive: true, force: true });
    }
}

const overstatedHeroJournal = structuredClone(facts.journals[1]);
overstatedHeroJournal.rules[0].proves.max = 14;
assert.throws(() => assertPrereqJournalPins(content, [overstatedHeroJournal]), /no source-pinned stage range/);

const overstatedHeroEstablishment = structuredClone(facts.journals[1]);
overstatedHeroEstablishment.establishes.values.max = 14;
assert.throws(() => assertPrereqJournalPins(content, [overstatedHeroEstablishment]), /establishment window has no matching after rule/);

// Delay facts must still agree with the cited p_delay lines. The A2 inward
// count includes its door proc, while the outward branch has no such delay.
for (const row of [0, 1]) {
    const changedDelay = structuredClone(facts);
    changedDelay.rows[row].crossings[0].delay.ticks += 1;
    assert.throws(() => assertPrereqStepFacts(content, changedDelay, quests.rows), /differs from pinned p_delay count/);
}

// A parallel edge one tile west shares no anchor with the cited placement.
const misplaced = structuredClone(facts);
const shifted = misplaced.rows[1].crossings[0];
shifted.crossing.at.x -= 1;
shifted.crossing.takeoff.x -= 1;
shifted.crossing.to.x -= 1;
shifted.pass.landing.x -= 1;
assert.throws(() => assertPrereqStepFacts(content, misplaced, quests.rows), /cited map tile/);

const inputs = prereqStepInputs(content, facts).map((input) => input.path);
assert.deepEqual(inputs, [...inputs].sort());
for (const source of [
    'content/scripts/quests/quest_druidspirit/scripts/quest_druidspirit.rs2',
    'content/scripts/areas/area_mausoleum/scripts/drezel.rs2',
    'content/scripts/skill_combat/scripts/npc/npc_combat.rs2',
    'content/scripts/general/scripts/quests.rs2',
    'content/scripts/quests/quest_blackarmgang/scripts/blackarmgang_journal.rs2',
    'content/scripts/quests/quest_hero/scripts/hero_journal.rs2',
    'content/scripts/quests/quest_blackarmgang/configs/quest_blackarmgang.constant',
    'content/scripts/quests/quest_hero/configs/quest_hero.constant',
    'content/scripts/general/configs/quest.constant',
    'content/pack/loc.pack',
    'content/pack/npc.pack',
    'content/pack/varp.pack',
]) assert.ok(inputs.includes(source), `${source} is pinned by the prerequisite family`);

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
function writeProjection(entries: unknown[], shaOverride?: string, identicalEdgeGroups: unknown = []) {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'prereq-projection-'));
    const pack = path.join(dir, 'nav.pack');
    fs.writeFileSync(pack, 'nav-pack-bytes');
    const sha = crypto.createHash('sha256').update(fs.readFileSync(pack)).digest('hex');
    const file = path.join(dir, 'crossings.json');
    fs.writeFileSync(file, JSON.stringify({ schema: 2, navpack_sha256: shaOverride ?? sha, crossings: entries, identical_edge_groups: identicalEdgeGroups }));
    return { dir, pack, file };
}
function withProjection(entries: unknown[], run: (overrides: { projectionPath: string; navPack: string }) => void, shaOverride?: string, identicalEdgeGroups?: unknown) {
    const { dir, pack, file } = writeProjection(entries, shaOverride, identicalEdgeGroups);
    try {
        run({ projectionPath: file, navPack: pack });
    } finally {
        fs.rmSync(dir, { recursive: true, force: true });
    }
}

withProjection(baseEntries(), (overrides) => {
    const joined = extractPrereqStepsFacts(289, overrides);
    assert.equal(joined.rows[0].crossings.length, 2);
    assert.deepEqual(joined.rows[0].crossings.map((entry) => entry.crossing.takeoff), [tile(10, 21), tile(11, 20)]);
    assert.equal(joined.rows[1].crossings.length, 4);
    assert.ok(joined.rows.flatMap((row) => row.crossings).every((entry) => JSON.stringify(entry.pass.landing) === JSON.stringify(entry.crossing.to)));
});
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
withProjection(baseEntries(), (overrides) => {
    assert.throws(() => extractPrereqStepsFacts(289, overrides), /does not match GAME_DATA_289_NAV_PACK/);
}, '0'.repeat(64));
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

// Schema-2 condition groups: two stage-window alternatives for one physical
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
// Fully identical TransportEdge duplicates are bake metadata only: listing
// the repeated physical record in identical_edge_groups must not fail the
// export, must not change the joined pack rows, and the crossings themselves
// stay unique.
withProjection(baseEntries(), (overrides) => {
    const projection = loadPrereqCrossingProjection(289, overrides)!;
    assert.deepEqual(projection.identical_edge_groups, [['graph.edges[1833]', 'graph.edges[1834]']]);
    const joined = extractPrereqStepsFacts(289, overrides);
    assert.equal(joined.rows[0].crossings.length, 2);
    assert.equal(joined.rows[1].crossings.length, 4);
}, undefined, [['graph.edges[1833]', 'graph.edges[1834]']]);
{
    const malformed = baseEntries().map((entry, index) => index === 0
        ? { ...entry, quest_gate_alternatives: [{ Complete: 'priestperil' }] }
        : entry);
    withProjection(malformed, (overrides) => {
        assert.throws(() => extractPrereqStepsFacts(289, overrides), /must be a list of QuestGate/);
    });
}

// A bounded complement is authority only after every production writer is known.
// The proof itself does not borrow a quest colour or completion value.
{
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'prereq-journal-domain-'));
    try {
        fs.mkdirSync(path.join(root, 'scripts'));
        fs.writeFileSync(path.join(root, 'scripts/progress.varp'), '[progress]\nscope=perm\n');
        fs.writeFileSync(path.join(root, 'scripts/writer.rs2'), '[proc,writer]()()\n%progress = 61;\n');
        const lines = [
            'def_string $page;',
            'if (%progress <= 8) {',
            '    $page = "Earlier progress";',
            '} else if (%progress >= 10 & %progress <= 59) {',
            '    $page = "Bring more essence";',
            '} else {',
            '    $page = "Completed reward";',
            '}',
            '~quest_journal("Progress", $page);',
        ];
        const nodes = parseBody(lines.map((text, index) => ({ line: index + 1, text })));
        const covered = new Set(lines.map((_, index) => index + 1));
        const constants = new Map<string, number>();
        const domain = captureProgressWriterDomains(root)('progress');
        assert.deepEqual(domain, { min: 0, max: 61 });
        assert.deepEqual(boundedJournalRange(nodes, 'progress', constants, domain, covered, ['completed reward']), { min: 9, max: 61 });

        fs.writeFileSync(path.join(root, 'scripts/unbounded.rs2'), '[proc,unbounded](int $value)()\n%progress = $value;\n');
        const unknown = captureProgressWriterDomains(root)('progress');
        assert.equal(unknown, null, 'one unbounded production writer removes the domain authority');
        assert.equal(boundedJournalRange(nodes, 'progress', constants, unknown, covered, ['completed reward']), null);

        fs.writeFileSync(path.join(root, 'scripts/unbounded.rs2'), [
            '[proc,unbounded](int $left)()',
            'if (%progress >= 61) { return; }',
            'while ($left > 0) { %progress = add(%progress, 1); $left = sub($left, 1); }',
        ].join('\n'));
        assert.equal(captureProgressWriterDomains(root)('progress'), null, 'an entry guard does not bound an arbitrary increment loop');
        const counted = [
            '[proc,counted]()()',
            'def_int $needed = calc(60 - %progress);',
            'def_int $held = inv_total(inv, essence);',
            'def_int $left = min($needed, $held);',
            'while ($left > 0) { %progress = add(%progress, 1); $left = sub($left, 1); }',
        ].join('\n');
        fs.writeFileSync(path.join(root, 'scripts/writer.rs2'), '[proc,writer]()()\n%progress = 0;\n');
        fs.writeFileSync(path.join(root, 'scripts/unbounded.rs2'), counted);
        assert.deepEqual(captureProgressWriterDomains(root)('progress'), { min: 0, max: 60 }, 'the remaining-needed counter bounds every iteration');
        fs.writeFileSync(path.join(root, 'scripts/unbounded.rs2'), `${counted} %progress = add(%progress, 1);\n`);
        assert.equal(captureProgressWriterDomains(root)('progress'), null, 'an extra same-line increment cannot hide behind the counted loop');
        fs.writeFileSync(path.join(root, 'scripts/writer.rs2'), '[proc,writer]()()\n%progress = 61;\n');

        fs.rmSync(path.join(root, 'scripts/unbounded.rs2'));
        fs.mkdirSync(path.join(root, 'scripts/_test'));
        fs.writeFileSync(path.join(root, 'scripts/_test/debug.rs2'), '[proc,debug](int $value)()\n%progress = $value;\n');
        assert.deepEqual(captureProgressWriterDomains(root)('progress'), domain, 'debug writers are excluded by the production census');
        const incomplete = new Set(covered);
        incomplete.delete(4);
        assert.equal(boundedJournalRange(nodes, 'progress', constants, domain, incomplete, ['completed reward']), null, 'an uncited branch guard cannot authorize a complement');
    } finally {
        fs.rmSync(root, { recursive: true, force: true });
    }
}
