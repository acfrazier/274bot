import { notImpl } from '../../shim/_kernel.js';

export const FOOD_OPTIONS = [
    'Shark', 'Lobster', 'Swordfish', 'Tuna', 'Salmon', 'Trout', 'Pike', 'Bass', 'Herring', 'Sardine', 'Anchovies', 'Shrimps',
    'Cooked meat', 'Cooked chicken', 'Bread', 'Stew',
    'Cake', 'Chocolate cake', 'Plain pizza', 'Meat pizza', 'Anchovy pizza', 'Pineapple pizza', 'Redberry pie', 'Meat pie', 'Apple pie',
];

export const MIN_EAT_HP = 5;

const FOOD_HEAL = {
    Shark: 20,
    Lobster: 12,
    Swordfish: 14,
    Tuna: 10,
    Salmon: 9,
    Trout: 7,
    Pike: 8,
    Bass: 13,
    Herring: 5,
    Sardine: 4,
    Anchovies: 1,
    Shrimps: 3,
    'Cooked meat': 3,
    'Cooked chicken': 3,
    Bread: 5,
    Stew: 11,
    Cake: 4,
    'Chocolate cake': 5,
    'Plain pizza': 7,
    'Meat pizza': 8,
    'Anchovy pizza': 9,
    'Pineapple pizza': 11,
    'Redberry pie': 6,
    'Meat pie': 6,
    'Apple pie': 7,
};

export function foodHealAmount(foodName) {
    const key = String(foodName || '').trim();
    if (Object.prototype.hasOwnProperty.call(FOOD_HEAL, key)) {
        return FOOD_HEAL[key];
    }
    const hit = Object.keys(FOOD_HEAL).find((n) => n.toLowerCase() === key.toLowerCase());
    if (hit) {
        return FOOD_HEAL[hit];
    }
    throw notImpl('foodHealAmount');
}

export function foodForms(_foodName) {
    throw notImpl('foodForms');
}

export function isFoodItem(_name, _foodName) {
    throw notImpl('isFoodItem');
}

export function foodCount(_items, _foodName) {
    throw notImpl('foodCount');
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
