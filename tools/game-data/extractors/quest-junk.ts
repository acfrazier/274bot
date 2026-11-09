import { int, list, onlyKeys, pinnedLiterals, record, str } from './rs2b0t-literals.ts';

/** One bank quest-junk row (`QuestJunkEntry`, `api/bank/bankQuestJunk.ts:3-7`). */
export type QuestJunk = { id: number; name: string; quest: string };

/**
 * `QUEST_JUNK` (`bankQuestJunk.ts:15`): every `...junk('Quest', [[id, 'Name'], ...])` spread expanded to rows in
 * source order. The reader recognises the `junk` helper by its exact text and evaluates nothing.
 */
export async function extractQuestJunk(engine: string, source: string): Promise<QuestJunk[]> {
    const junk = await pinnedLiterals(engine, 'bankQuestJunk.ts', source, 'quest junk');
    const rows = list(junk('QUEST_JUNK'), 'QUEST_JUNK').map((input): QuestJunk => {
        const row = record(input, 'quest junk');
        onlyKeys(row, ['id', 'name', 'quest'], 'quest junk');
        return { id: int(row.id, 'id'), name: str(row.name, 'name'), quest: str(row.quest, 'quest') };
    });
    if (!rows.length) throw new Error('QUEST_JUNK: empty');
    return rows;
}
