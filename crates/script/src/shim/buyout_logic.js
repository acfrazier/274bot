import { notImpl } from '../../shim/_kernel.js';

// Marshal-only: convert the frozen call into the typed FlatBuffer query
// (`__rs2b0t_buyout_plan`). Ranking, stock prices and budget stay in Rust.
export function buyoutPlan(rec, stock, coins, chosen) {
    const fn = globalThis.__rs2b0t_buyout_plan;
    if (typeof fn !== 'function') {
        throw notImpl('BuyoutLogic.buyoutPlan');
    }
    const names = chosen == null ? [] : Array.from(chosen, (name) => String(name));
    const stockObjs = [];
    const stockCounts = [];
    if (stock && typeof stock === 'object') {
        for (const obj of Object.keys(stock)) {
            stockObjs.push(String(obj));
            stockCounts.push(Number(stock[obj]) || 0);
        }
    }
    const inv = rec && rec.inv != null ? String(rec.inv) : '';
    const keeper =
        rec && Array.isArray(rec.keepers) && rec.keepers.length > 0
            ? String(rec.keepers[0])
            : '';
    const result = fn(inv, keeper, coins ?? 0, stockObjs, stockCounts, names);
    if (!result || result.ok !== true || !Array.isArray(result.items)) {
        throw notImpl('BuyoutLogic.buyoutPlan', result && result.reason);
    }
    return result.items.map((row) => ({
        obj: String(row?.obj ?? ''),
        name: String(row?.name ?? ''),
        units: Number(row?.units) || 0,
        estCost: Number(row?.estCost) || 0,
    }));
}
