import { Inventory } from '../inventory/Inventory.js';
import { Locs } from '../locs/Locs.js';
import { STALL_NAME, STALL_OP, CAKE_ITEMS } from './cakeStallData.js';

export function carriedCakes() {
    return CAKE_ITEMS.reduce((n, name) => n + Inventory.count(name), 0);
}

export function needsCakeRestock(target) {
    return carriedCakes() < (target || 1);
}

export async function stealCakes() {
    const loc = Locs.query().nearest();
    if (!loc || String(loc.name).toLowerCase() !== STALL_NAME.toLowerCase()) return false;
    return loc.interact(STALL_OP);
}
