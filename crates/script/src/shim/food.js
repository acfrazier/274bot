import { host, notImpl } from '../../shim/_kernel.js';

export const FOOD_OPTIONS = [
    'Shark', 'Lobster', 'Swordfish', 'Tuna', 'Salmon', 'Trout', 'Pike', 'Bass', 'Herring', 'Sardine', 'Anchovies', 'Shrimps',
    'Cooked meat', 'Cooked chicken', 'Bread', 'Stew',
    'Cake', 'Chocolate cake', 'Plain pizza', 'Meat pizza', 'Anchovy pizza', 'Pineapple pizza', 'Redberry pie', 'Meat pie', 'Apple pie',
];

export const MIN_EAT_HP = 5;

const FOOD_HEAL = Object.fromEntries(globalThis.__rs2b0t_host?.content?.food_heals || []);

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

function itemRows() {
    const rows = host().content?.items;
    return Array.isArray(rows) ? rows : [];
}

export function foodForms(foodName) {
    const key = String(foodName).trim().toLowerCase();
    const item = itemRows().find(
        (row) => row && typeof row.name === 'string' && row.name.toLowerCase() === key,
    );
    if (!item || typeof item.obj !== 'string') return [key];
    const obj = item.obj;
    if (
        obj.startsWith('partial_') ||
        obj.startsWith('half_') ||
        obj.startsWith('half_a_') ||
        obj.startsWith('half_an_') ||
        obj.endsWith('_slice') ||
        obj.startsWith('cert_')
    ) {
        return [key];
    }
    const aliases = new Set([obj, `partial_${obj}`, `${obj}_slice`]);
    return itemRows()
        .filter((row) => row && typeof row.obj === 'string' && aliases.has(row.obj))
        .map((row) => String(row.name).toLowerCase())
        .filter((name, index, all) => all.indexOf(name) === index);
}

export function isFoodItem(name, foodName) {
    return foodForms(foodName).includes(String(name ?? '').toLowerCase());
}

export function foodCount(items, foodName) {
    if (!Array.isArray(items)) return 0;
    return items.filter((item) => item && isFoodItem(item.name, foodName)).length;
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
