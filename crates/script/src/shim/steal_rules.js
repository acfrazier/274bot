import { Bank } from '../bank/Bank.js';
import { Execution } from '../execution/Execution.js';
import { Inventory } from '../inventory/Inventory.js';
import { notImpl } from '../../shim/_kernel.js';

export const THIEVER_BANKING_OPTIONS = ['None', 'Auto'];
export const STUN_COMBAT_TICKS = 9;

export function nextWithdrawChunk(_need) {
    throw notImpl('nextWithdrawChunk');
}

export async function withdrawTo(name, target, count = () => Inventory.count(name)) {
    let state = null, ok = false;
    for (;;) {
        const command = globalThis.rustyscript.functions.__rs2b0t_withdraw_step({state, ok, target, count: count(), full: Inventory.isFull()});
        if (command.kind === 'done') return command.value;
        state = command.state;
        if (command.kind === 'x') {
            ok = await Bank.withdrawX(name, command.count);
        } else {
            await Bank.withdraw(name, command.op);
            ok = await Execution.delayUntil(() => count() > command.before, 2500);
        }
    }
}

export async function closeBankAndConfirmCount(expected, count) {
    if (!(await Bank.close())) return false;
    await Execution.delayTicks(1);
    return Execution.delayUntil(() => count() >= expected, 3000);
}

export function autoFoodBanking(mode) {
    return String(mode || '').trim().toLowerCase() === 'auto';
}

export function foodMatches(name, keyword) {
    const wanted = String(keyword || '').trim().toLowerCase();
    return wanted.length > 0 && String(name ?? '').toLowerCase().includes(wanted);
}

export function countFood(items, keyword) {
    if (!Array.isArray(items)) {
        return 0;
    }
    return items.reduce((sum, item) => {
        if (!item || !foodMatches(item.name, keyword)) {
            return sum;
        }
        const n = Number(item.count);
        return sum + (Number.isFinite(n) ? n : 0);
    }, 0);
}

export function shouldRestockFood(enabled, foodCount, restockAt, bankablePackFull) {
    return Boolean(enabled) && (foodCount <= restockAt || Boolean(bankablePackFull));
}

export function safeToSteal(hpFraction, eatAt, foodCount) {
    return hpFraction >= eatAt || foodCount > 0;
}

export function canStealNow(foodCount, hp, minEatHp, suicide) {
    return Boolean(suicide) || foodCount > 0 || hp > minEatHp;
}
