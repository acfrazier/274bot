import { notImpl } from '../../shim/_kernel.js';
import { selectedLoadout } from './loadoutSetting.js';

export function foodOf(loadout, fallback) {
    if (loadout && Array.isArray(loadout.carry) && loadout.carry.some(entry => !entry || typeof entry.item !== 'string')) {
        throw notImpl('foodOf');
    }
    return globalThis.rustyscript.functions.__rs2b0t_food_of(loadout, fallback == null ? '' : String(fallback));
}

export function gearOf(_loadout) {
    throw notImpl('gearOf');
}

export function suppliesOf(_loadout) {
    throw notImpl('suppliesOf');
}

export function weaponOf(_loadout, _fallback) {
    throw notImpl('weaponOf');
}

export function scriptFood(bag, fallback) {
    return foodOf(selectedLoadout(bag), fallback);
}

export function scriptFoods(bag, fallback) {
    const chosen = scriptFood(bag, '');
    if (chosen.length > 0) {
        return [chosen];
    }
    return Array.isArray(fallback) ? fallback.slice() : [];
}
