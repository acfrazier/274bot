import assert from 'node:assert/strict';
import { extractQuestIdentityFacts } from './quests.ts';
import { assertPrereqStepFacts, extractPrereqStepsFacts, prereqStepInputs, PREREQ_STEPS_SCHEMA } from './prereq-steps.ts';
import { revisions } from '../generate.ts';

assert.equal(PREREQ_STEPS_SCHEMA, 1);
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
assert.deepEqual(facts.rows[1].crossings.map((entry) => [
    entry.crossing.loc_id,
    entry.crossing.at.x,
    entry.crossing.at.z,
    entry.crossing.to.x,
    entry.crossing.to.z,
    entry.delay.ticks,
]), [
    [3507, 3443, 3458, 3443, 3457, 2],
    [3506, 3444, 3458, 3444, 3457, 2],
    [3507, 3443, 3457, 3443, 3458, 0],
    [3506, 3444, 3457, 3444, 3458, 0],
]);

// The reverse gate edge starts south of the fixed loc and lands on its tile.
// A neighboring parallel edge is not proved by that loc's placement citation.
const misplaced = structuredClone(facts);
const reverse = misplaced.rows[1].crossings[2];
reverse.crossing.at.x -= 1;
reverse.crossing.to.x -= 1;
reverse.pass.landing.x -= 1;
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
