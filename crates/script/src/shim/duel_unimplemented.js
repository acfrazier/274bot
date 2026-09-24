// Frozen src/bot/api/duel/Duel.ts:11-132; only the names are loadable.
import { notImpl, notImplValue, proxy } from '../../shim/_kernel.js';

export const DUEL_SELECT_MODAL = notImplValue('Duel.DUEL_SELECT_MODAL');
export const DUEL_CONFIRM_MODAL = notImplValue('Duel.DUEL_CONFIRM_MODAL');
export const DUEL_WIN_MODAL = notImplValue('Duel.DUEL_WIN_MODAL');
export const DUEL_FIGHT_ARENAS = notImplValue('Duel.DUEL_FIGHT_ARENAS');
export function parseDuelPartnerHeader() { throw notImpl('Duel.parseDuelPartnerHeader'); }
export function fightArenaAt() { throw notImpl('Duel.fightArenaAt'); }
export const Duel = proxy('Duel', {
    offerOpen() { throw notImpl('Duel.offerOpen'); },
    confirmOpen() { throw notImpl('Duel.confirmOpen'); },
    winOpen() { throw notImpl('Duel.winOpen'); },
    active() { throw notImpl('Duel.active'); },
    partner() { throw notImpl('Duel.partner'); },
    waitingForOther() { throw notImpl('Duel.waitingForOther'); },
    challenge() { throw notImpl('Duel.challenge'); },
    fight() { throw notImpl('Duel.fight'); },
    accept() { throw notImpl('Duel.accept'); },
    cancel() { throw notImpl('Duel.cancel'); },
    closeWin() { throw notImpl('Duel.closeWin'); },
});
