import { notImpl } from '../../shim/_kernel.js';

function callShop(payload) {
    const fn = globalThis.rustyscript && globalThis.rustyscript.functions
        ? globalThis.rustyscript.functions.__rs2b0t_shop
        : undefined;
    if (typeof fn !== 'function') {
        throw notImpl('BuyoutLogic.buyoutPlan');
    }
    return fn(payload);
}

export function buyoutPlan(rec, stock, coins, chosen) {
    const names = chosen == null ? [] : Array.from(chosen, (name) => String(name));
    const result = callShop({
        op: 'buyout-plan',
        rec: rec ?? null,
        stock: stock && typeof stock === 'object' ? stock : {},
        coins: coins ?? 0,
        chosen: names,
    });
    if (!result || result.kind === 'notImpl') {
        throw notImpl('BuyoutLogic.buyoutPlan', result && result.reason);
    }
    if (result.kind !== 'plan' || !Array.isArray(result.items)) {
        throw notImpl('BuyoutLogic.buyoutPlan', 'unexpected result');
    }
    return result.items.map((row) => ({
        obj: String(row?.obj ?? ''),
        name: String(row?.name ?? ''),
        units: Number(row?.units) || 0,
        estCost: Number(row?.estCost) || 0,
    }));
}
