import { Bank } from '../bank/Bank.js';
import { notV1 } from '../../shim/_kernel.js';

export const THIEVER_BANKING_OPTIONS = ['None', 'Auto'];
export const STUN_COMBAT_TICKS = 9;

export function nextWithdrawChunk(_need) {
    throw notV1('nextWithdrawChunk');
}

export async function withdrawTo(name, target) {
    Bank.withdraw(name, target);
}

export async function closeBankAndConfirmCount(_expected, _count) {
    throw notV1('closeBankAndConfirmCount');
}

export function autoFoodBanking(_mode) {
    throw notV1('autoFoodBanking');
}

export function foodMatches(_name, _keyword) {
    throw notV1('foodMatches');
}

export function countFood(_items, _keyword) {
    throw notV1('countFood');
}

export function shouldRestockFood(_enabled, _foodCount, _restockAt, _bankablePackFull) {
    throw notV1('shouldRestockFood');
}

export function safeToSteal(_hpFraction, _eatAt, _foodCount) {
    throw notV1('safeToSteal');
}

export function canStealNow(_foodCount, _hp, _minEatHp, _suicide) {
    throw notV1('canStealNow');
}
