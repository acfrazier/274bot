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

export async function withdrawTo(name, target, countInInv = () => Inventory.count(name)) {
    const start = countInInv();
    for (let guard = 0; guard < 40 && countInInv() < target && !Inventory.isFull(); guard++) {
        const before = countInInv();
        const need = target - before;
        const chunk = nextWithdrawChunk(need);
        if (!chunk) {
            break;
        }
        if (chunk.kind === 'x') {
            if (await Bank.withdrawX(name, chunk.count)) {
                if (countInInv() > before) {
                    continue;
                }
            }
            const fallback = nextWithdrawChunk(Math.min(need, 10));
            if (!fallback || fallback.kind !== 'op') {
                break;
            }
            await Bank.withdraw(name, fallback.op);
            if (!(await Execution.delayUntil(() => countInInv() > before, 2500))) {
                break;
            }
            continue;
        }
        await Bank.withdraw(name, chunk.op);
        if (!(await Execution.delayUntil(() => countInInv() > before, 2500))) {
            break;
        }
    }
    return countInInv() - start;
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
