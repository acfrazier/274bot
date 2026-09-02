import { selectedLoadout } from './loadoutSetting.js';

export function foodOf(loadout, fallback) {
    return fallback;
}

export function gearOf(_loadout) {
    return [];
}

export function suppliesOf(_loadout) {
    return [];
}

export function weaponOf(_loadout, fallback = null) {
    return fallback;
}

export function scriptFood(_bag, fallback) {
    return foodOf(selectedLoadout(_bag), fallback);
}

export function scriptFoods(_bag, fallback) {
    return [...fallback];
}
