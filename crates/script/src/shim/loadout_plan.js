import { notV1 } from '../../shim/_kernel.js';

export function foodOf(_loadout, _fallback) {
    throw notV1('foodOf');
}

export function gearOf(_loadout) {
    throw notV1('gearOf');
}

export function suppliesOf(_loadout) {
    throw notV1('suppliesOf');
}

export function weaponOf(_loadout, _fallback) {
    throw notV1('weaponOf');
}

export function scriptFood(_bag, _fallback) {
    throw notV1('scriptFood');
}

export function scriptFoods(_bag, _fallback) {
    throw notV1('scriptFoods');
}
