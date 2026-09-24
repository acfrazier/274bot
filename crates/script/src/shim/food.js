import { notImpl } from '../../shim/_kernel.js';

export const FOOD_OPTIONS = [
    'Shark', 'Lobster', 'Swordfish', 'Tuna', 'Salmon', 'Trout', 'Pike', 'Bass', 'Herring', 'Sardine', 'Anchovies', 'Shrimps',
    'Cooked meat', 'Cooked chicken', 'Bread', 'Stew',
    'Cake', 'Chocolate cake', 'Plain pizza', 'Meat pizza', 'Anchovy pizza', 'Pineapple pizza', 'Redberry pie', 'Meat pie', 'Apple pie',
];

export const MIN_EAT_HP = 5;

function foodHealNative() {
    return globalThis.__rs2b0t_food_heal_amount;
}

export function foodHealAmount(foodName) {
    const fn = foodHealNative();
    if (typeof fn === 'function') {
        const key = String(foodName || '').trim();
        const row = fn(key);
        if (row && row.ok === true && typeof row.value === 'number') {
            return row.value;
        }
        throw notImpl('foodHealAmount');
    }
    throw notImpl('foodHealAmount');
}

export function foodForms(foodName) {
    const fn = globalThis.__rs2b0t_selected_facts;
    if (typeof fn !== 'function') throw notImpl('foodForms');
    return fn('food-forms', String(foodName));
}

export function isFoodItem(name, foodName) {
    return foodForms(foodName).includes(String(name ?? '').toLowerCase());
}

export function foodCount(items, foodName) {
    if (!Array.isArray(items)) return 0;
    if (items.length === 0) return 0;
    const fn = globalThis.__rs2b0t_food_count;
    if (typeof fn !== 'function') throw notImpl('foodCount');
    return fn(items, foodName);
}

export function eatAtHpThreshold(_maxHp, _heal, _minHp) {
    throw notImpl('eatAtHpThreshold');
}

/** Eat when a full heal fits, or HP is at/below the safety floor. Posted opts only. */
export function shouldEatToUseFood(opts) {
    const o = opts || {};
    const hp = o.hp;
    const maxHp = o.maxHp;
    const heal = o.heal;
    const foodCount = o.foodCount;
    if (foodCount <= 0 || hp <= 0 || maxHp <= 0) {
        return false;
    }
    const minHp = o.minHp != null ? o.minHp : MIN_EAT_HP;
    if (hp <= minHp) {
        return true;
    }
    const h = Math.max(0, heal);
    // full heal fits (no overheal waste)
    return hp + h <= maxHp;
}

export function shouldEatFood(foodName, opts) {
    const o = opts || {};
    return shouldEatToUseFood({
        hp: o.hp,
        maxHp: o.maxHp,
        heal: foodHealAmount(foodName),
        foodCount: o.foodCount,
        minHp: o.minHp,
    });
}
