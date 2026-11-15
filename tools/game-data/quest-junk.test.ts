import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { extractQuestJunk } from './extractors/quest-junk.ts';
import { requireEnvPath } from './extractors/rs2b0t-literals.ts';

const engine = requireEnvPath('GAME_DATA_289_ENGINE', 'the 289 engine root');

// `...junk('Quest', [[id, 'Name'], ...])` spreads expand to rows in source order.
{
    const junk = (body: string) => `function junk(quest: string, items: readonly (readonly [id: number, name: string])[]): QuestJunkEntry[] {
    ${body}
}
export const QUEST_JUNK: readonly QuestJunkEntry[] = [
    ...junk('Demon Slayer', [
        [2399, 'Key'],
        [2400, 'Key']
    ]),
    ...junk("The Knight's Sword", [[666, 'Portrait']])
];
`;
    const rows = await extractQuestJunk(engine, junk('return items.map(([id, name]) => ({ id, name, quest }));'));
    assert.deepEqual(rows, [
        { id: 2399, name: 'Key', quest: 'Demon Slayer' },
        { id: 2400, name: 'Key', quest: 'Demon Slayer' },
        { id: 666, name: 'Portrait', quest: "The Knight's Sword" },
    ]);
    // A row that is not `[id, name]` and a table of nothing both refuse.
    await assert.rejects(extractQuestJunk(engine, junk('return items.map(([id, name]) => ({ id, name, quest }));').replace("[2399, 'Key']", "['x', 'Key']")), /junk row/);
    await assert.rejects(extractQuestJunk(engine, "export const QUEST_JUNK = [];\n"), /empty/);
}

// The rs2b0t export: the table expands to the known rows.
{
    const root = requireEnvPath('RS2B0T', 'the pinned rs2b0t checkout root');
    const rows = await extractQuestJunk(engine, fs.readFileSync(path.join(root, 'src/bot/api/bank/bankQuestJunk.ts'), 'utf8'));
    assert.equal(rows.length, 319);
    assert.deepEqual(rows[0], { id: 2399, name: 'Key', quest: 'Demon Slayer' });
    assert.equal(new Set(rows.map((row) => row.quest)).size > 30, true);
}
