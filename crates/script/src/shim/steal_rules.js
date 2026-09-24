import { notImpl, runMachine } from '../../shim/_kernel.js';

export const THIEVER_BANKING_OPTIONS = ['None', 'Auto'];
export const STUN_COMBAT_TICKS = 9;

export function nextWithdrawChunk(_need) {
    throw notImpl('nextWithdrawChunk');
}

// Rust runs the frozen loop; `count` is called through the callback path
// (absent: Inventory.count(name) from the posted backpack).
export async function withdrawTo(name, target, count) {
    const out = await runMachine(
        'bank_withdraw_to',
        { name: String(name), target },
        { count: typeof count === 'function' ? count : undefined },
    );
    return out.kind === 'done' ? out.value : 0;
}

export async function closeBankAndConfirmCount(expected, count) {
    const out = await runMachine('bank_close_confirm', { expected }, { count });
    return out.kind === 'done' && out.value === true;
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
