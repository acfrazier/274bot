import { notV1 } from '../../shim/_kernel.js';

export const FOOD_OPTIONS = [
    'Shark', 'Lobster', 'Swordfish', 'Tuna', 'Salmon', 'Trout', 'Pike', 'Bass', 'Herring', 'Sardine', 'Anchovies', 'Shrimps',
    'Cooked meat', 'Cooked chicken', 'Bread', 'Stew',
    'Cake', 'Chocolate cake', 'Plain pizza', 'Meat pizza', 'Anchovy pizza', 'Pineapple pizza', 'Redberry pie', 'Meat pie', 'Apple pie',
];

export const MIN_EAT_HP = 5;

export function foodForms(_foodName) {
    throw notV1('foodForms');
}

export function isFoodItem(_name, _foodName) {
    throw notV1('isFoodItem');
}

export function foodCount(_items, _foodName) {
    throw notV1('foodCount');
}

export function foodHealAmount(_foodName) {
    throw notV1('foodHealAmount');
}

export function eatAtHpThreshold(_maxHp, _heal, _minHp) {
    throw notV1('eatAtHpThreshold');
}

export function shouldEatToUseFood(_opts) {
    throw notV1('shouldEatToUseFood');
}

export function shouldEatFood(_foodName, _opts) {
    throw notV1('shouldEatFood');
}
