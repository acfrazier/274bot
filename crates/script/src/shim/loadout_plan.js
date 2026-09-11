import { notImpl } from '../../shim/_kernel.js';
import { selectedLoadout } from './loadoutSetting.js';

export function foodOf(loadout, fallback) {
    if (loadout && Array.isArray(loadout.carry) && loadout.carry.some(entry => !entry || typeof entry.item !== 'string')) {
        throw notImpl('foodOf');
    }
    return globalThis.rustyscript.functions.__rs2b0t_food_of(loadout, fallback == null ? '' : String(fallback));
}

export function gearOf(loadout) {
    return globalThis.rustyscript.functions.__rs2b0t_gear_of(loadout == null ? null : loadout);
}

export function suppliesOf(loadout) {
    return globalThis.rustyscript.functions.__rs2b0t_supplies_of(loadout == null ? null : loadout);
}

export function weaponOf(loadout, fallback = null) {
    return globalThis.rustyscript.functions.__rs2b0t_weapon_of(
        loadout == null ? null : loadout,
        fallback == null ? null : String(fallback),
    );
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
