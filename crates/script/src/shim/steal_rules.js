import { Bank } from '../bank/Bank.js';
import { notImpl } from '../../shim/_kernel.js';

export const THIEVER_BANKING_OPTIONS = ['None', 'Auto'];
export const STUN_COMBAT_TICKS = 9;

export function nextWithdrawChunk(_need) {
    throw notImpl('nextWithdrawChunk');
}

export async function withdrawTo(name, target) {
    Bank.withdraw(name, target);
}

export async function closeBankAndConfirmCount(_expected, _count) {
    throw notImpl('closeBankAndConfirmCount');
}

export function autoFoodBanking(_mode) {
    throw notImpl('autoFoodBanking');
}

export function foodMatches(_name, _keyword) {
    throw notImpl('foodMatches');
}

export function countFood(_items, _keyword) {
    throw notImpl('countFood');
}

export function shouldRestockFood(_enabled, _foodCount, _restockAt, _bankablePackFull) {
    throw notImpl('shouldRestockFood');
}

export function safeToSteal(_hpFraction, _eatAt, _foodCount) {
    throw notImpl('safeToSteal');
}

export function canStealNow(_foodCount, _hp, _minEatHp, _suicide) {
    throw notImpl('canStealNow');
}
