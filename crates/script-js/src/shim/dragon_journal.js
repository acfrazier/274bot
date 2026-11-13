import { runMachine } from '../../../../../shim/_kernel.js';

/** Mirrors quest_dragon.constant; the engine skips 4-6. */
export const DRAGON_STAGE = {
    NOT_STARTED: 0,
    SPOKEN_GUILDMASTER: 1,
    SPOKEN_OZIACH: 2,
    BOUGHT_SHIP: 3,
    REPAIRED_SHIP: 7,
    NED_GIVEN_MAP: 8,
    SAILED_TO_CRANDOR: 9,
    COMPLETE: 10,
};

/** One native journal read; Rust resolves the stage from the journal text. */
export async function readDragonProgress() {
    const out = await runMachine('quest-progress', {
        quest: 'Dragon Slayer',
        journal_only: true,
    });
    if (out.kind !== 'done' || out.value?.end !== 'done') return undefined;
    const stage = out.value.journal?.stage;
    return typeof stage === 'number' ? { stage } : undefined;
}
