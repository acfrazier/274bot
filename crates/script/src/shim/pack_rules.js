// String contains pack filters. Food / bank policy stays not impl.
import { notImpl } from '../../shim/_kernel.js';

export function matchesAny(name, patterns) {
    if (name == null) return false;
    const n = String(name).toLowerCase();
    return (patterns || []).some((p) => {
        const pat = String(p).trim().toLowerCase();
        return pat.length > 0 && n.includes(pat);
    });
}

export function countMatching(items, patterns) {
    return (items || [])
        .filter((i) => i && matchesAny(i.name, patterns))
        .reduce((sum, i) => sum + (typeof i.count === 'number' ? i.count : 0), 0);
}

export function slotsMatching(items, patterns) {
    return (items || []).filter((i) => i && matchesAny(i.name, patterns)).length;
}

export function shouldBank(lootSlots, bankAt, invFull) {
    return lootSlots >= bankAt || (invFull && lootSlots > 0);
}

export function shouldRestock(foodCount, threshold) {
    return foodCount < threshold;
}

export function shouldEat() {
    throw notImpl('packRules.shouldEat');
}

export function shouldPanic(hpFrac, gate, foodCount) {
    return hpFrac < gate && foodCount === 0;
}
