// Frozen src/bot/api/duel/ClueDuel.ts:10-99; paired duel capability is not wired.
import { notImpl, notImplValue } from '../../shim/_kernel.js';

export const CLUE_DUEL_LOBBY = notImplValue('ClueDuel.CLUE_DUEL_LOBBY');
export const CLUE_DUEL_OPTIONS = notImplValue('ClueDuel.CLUE_DUEL_OPTIONS');
export function clueDuelName() { throw notImpl('ClueDuel.clueDuelName'); }
export function leaveClueDuel() { throw notImpl('ClueDuel.leaveClueDuel'); }
export class ClueDuelHandshake {
    constructor() { throw notImpl('ClueDuel.ClueDuelHandshake'); }
    tick() { throw notImpl('ClueDuel.ClueDuelHandshake.tick'); }
}
export class ClueDuelHelper {
    constructor() { throw notImpl('ClueDuel.ClueDuelHelper'); }
    validate() { throw notImpl('ClueDuel.ClueDuelHelper.validate'); }
    execute() { throw notImpl('ClueDuel.ClueDuelHelper.execute'); }
}
