import { Bank } from '../bank/Bank.js';
import { Execution } from '../execution/Execution.js';
import { Inventory } from '../inventory/Inventory.js';

export const THIEVER_BANKING_OPTIONS = ['None', 'Auto'];
export const STUN_COMBAT_TICKS = 9;

export function nextWithdrawChunk(need) {
    if (need <= 0) {
        return null;
    }
    if (need > 10) {
        return { kind: 'x', count: need };
    }
    if (need >= 10) {
        return { kind: 'op', op: 'Withdraw-10' };
    }
    if (need >= 5) {
        return { kind: 'op', op: 'Withdraw-5' };
    }
    return { kind: 'op', op: 'Withdraw-1' };
}

export async function withdrawTo(name, target, _countInInv = () => Inventory.count(name)) {
    Bank.withdraw(name, target);
}

export async function closeBankAndConfirmCount(expected, count) {
    if (!(await Bank.close())) {
        return false;
    }
    await Execution.delayTicks(1);
    return Execution.delayUntil(() => count() >= expected, 3000);
}

export function autoFoodBanking(mode) {
    return mode.trim().toLowerCase() === 'auto';
}

export function foodMatches(name, keyword) {
    const wanted = keyword.trim().toLowerCase();
    return wanted.length > 0 && (name ?? '').toLowerCase().includes(wanted);
}

export function countFood(items, keyword) {
    return items.filter((item) => foodMatches(item.name, keyword)).reduce((sum, item) => sum + item.count, 0);
}

export function shouldRestockFood(enabled, foodCount, restockAt, bankablePackFull) {
    return enabled && (foodCount <= restockAt || bankablePackFull);
}

export function safeToSteal(hpFraction, eatAt, foodCount) {
    return hpFraction >= eatAt || foodCount > 0;
}

export function canStealNow(foodCount, hp, minEatHp, suicide) {
    return suicide || foodCount > 0 || hp > minEatHp;
}
