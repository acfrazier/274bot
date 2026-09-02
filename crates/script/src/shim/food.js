export const FOOD_OPTIONS = [
    'Shark', 'Lobster', 'Swordfish', 'Tuna', 'Salmon', 'Trout', 'Pike', 'Bass', 'Herring', 'Sardine', 'Anchovies', 'Shrimps',
    'Cooked meat', 'Cooked chicken', 'Bread', 'Stew',
    'Cake', 'Chocolate cake', 'Plain pizza', 'Meat pizza', 'Anchovy pizza', 'Pineapple pizza', 'Redberry pie', 'Meat pie', 'Apple pie',
];

export const MIN_EAT_HP = 5;

export function foodForms(foodName) {
    return [String(foodName).toLowerCase()];
}

export function isFoodItem(name, foodName) {
    return foodForms(foodName).includes((name ?? '').toLowerCase());
}

export function foodCount(items, foodName) {
    return items.filter((i) => isFoodItem(i.name, foodName)).reduce((sum, i) => sum + i.count, 0);
}

export function foodHealAmount(_foodName) {
    throw new Error('not v1: foodHealAmount');
}

export function eatAtHpThreshold(_maxHp, _heal, minHp = MIN_EAT_HP) {
    return minHp;
}

export function shouldEatToUseFood(opts) {
    if (opts.foodCount <= 0 || opts.hp <= 0 || opts.maxHp <= 0) {
        return false;
    }
    const minHp = opts.minHp ?? MIN_EAT_HP;
    return opts.hp <= minHp;
}

export function shouldEatFood(_foodName, opts) {
    return shouldEatToUseFood(opts);
}
