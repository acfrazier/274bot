// String contains pack filters + posted HP/count predicates.
import { shouldEatToUseFood } from '../combat/food.js';

const native = (...args) => globalThis.__rs2b0t_pack_rules(...args);

export function matchesAny(name, patterns) {
    return native(0, name, patterns);
}

export function countMatching(items, patterns) {
    return native(1, items, patterns);
}

export function slotsMatching(items, patterns) {
    return native(2, items, patterns);
}

export function shouldBank(lootSlots, bankAt, invFull) {
    return native(3, lootSlots, bankAt, invFull);
}

export function shouldRestock(foodCount, threshold) {
    return native(4, foodCount, threshold);
}

export function shouldEat(hp, maxHp, heal, foodCount) {
    return shouldEatToUseFood({ hp, maxHp, heal, foodCount });
}

export function shouldPanic(hpFrac, gate, foodCount) {
    return native(5, hpFrac, gate, foodCount);
}
