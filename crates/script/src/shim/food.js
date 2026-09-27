import { notImpl } from '../../shim/_kernel.js';

export const FOOD_OPTIONS = [
    'Shark', 'Lobster', 'Swordfish', 'Tuna', 'Salmon', 'Trout', 'Pike', 'Bass', 'Herring', 'Sardine', 'Anchovies', 'Shrimps',
    'Cooked meat', 'Cooked chicken', 'Bread', 'Stew',
    'Cake', 'Chocolate cake', 'Plain pizza', 'Meat pizza', 'Anchovy pizza', 'Pineapple pizza', 'Redberry pie', 'Meat pie', 'Apple pie',
];

export const MIN_EAT_HP = 5;

/** Frozen resolution over the selected facts; throws only when game data is unavailable. */
export function foodHealAmount(foodName) {
    return globalThis.__rs2b0t_food_heal_amount(String(foodName || '').trim());
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

export function eatAtHpThreshold(maxHp, heal, minHp) {
    return globalThis.__rs2b0t_eat_at_hp_threshold(maxHp, heal, minHp);
}

/** Eat when a full heal fits, or HP is at/below the safety floor. */
export function shouldEatToUseFood(opts) {
    return globalThis.__rs2b0t_should_eat_to_use_food(opts);
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
