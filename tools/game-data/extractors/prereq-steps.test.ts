import assert from 'node:assert/strict';
import crypto from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { extractQuestIdentityFacts } from './quests.ts';
import { assertPrereqStepFacts, extractPrereqStepsFacts, loadPrereqCrossingProjection, prereqStepInputs, PREREQ_STEPS_SCHEMA } from './prereq-steps.ts';
import { revisions } from '../generate.ts';

assert.equal(PREREQ_STEPS_SCHEMA, 2);
assert.deepEqual(extractPrereqStepsFacts(274).rows, []);

const content = revisions.find((spec) => spec.revision === 289)!.content;
const facts = extractPrereqStepsFacts(289);
const quests = extractQuestIdentityFacts(content, 289);
assertPrereqStepFacts(content, facts, quests.rows);

assert.deepEqual(facts.rows.map((row) => row.id), ['priestperil:drezel-advice', 'druidspirit:drezel-start']);
assert.deepEqual(quests.rows.find((row) => row.id === 'priestperil')!.stage_domain, { min: 0, max: 61 });
assert.equal(quests.rows.find((row) => row.id === 'priestperil')!.transmission, 'not_transmitted');
assert.deepEqual(quests.rows.find((row) => row.id === 'druidspirit')!.stage_domain, { min: 0, max: 110 });
assert.equal(quests.rows.find((row) => row.id === 'druidspirit')!.transmission, 'not_transmitted');
assert.equal(quests.rows.find((row) => row.id === 'priestperil')!.colour_authoritative, true);
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
    'content/pack/loc.pack',
    'content/pack/npc.pack',
    'content/pack/varp.pack',
]) assert.ok(inputs.includes(source), `${source} is pinned by the prerequisite family`);

// Synthetic-projection checks below exercise the fail-closed projection joins
// without content: templates bind to pack geometry, never to edited constants.
const tile = (x: number, z: number, level = 0) => ({ x, z, level });
function packEntry(loc: number, at: { x: number; z: number; level: number }, takeoff: { x: number; z: number; level: number }, to: { x: number; z: number; level: number }, standable = true, admissible = true) {
    return {
        crossing: { loc_id: loc, option: 1, at, takeoff, to, kind: 'door' },
        standable,
        admissible,
        quest_gates: [] as unknown[],
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
function writeProjection(entries: unknown[], shaOverride?: string) {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'prereq-projection-'));
    const pack = path.join(dir, 'nav.pack');
    fs.writeFileSync(pack, 'nav-pack-bytes');
    const sha = crypto.createHash('sha256').update(fs.readFileSync(pack)).digest('hex');
    const file = path.join(dir, 'crossings.json');
    fs.writeFileSync(file, JSON.stringify({ schema: 1, navpack_sha256: shaOverride ?? sha, crossings: entries }));
    return { dir, pack, file };
}
function withProjection(entries: unknown[], run: (overrides: { projectionPath: string; navPack: string }) => void, shaOverride?: string) {
    const { dir, pack, file } = writeProjection(entries, shaOverride);
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
