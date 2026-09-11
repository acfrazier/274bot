import { host, notImpl } from '../../shim/_kernel.js';
import { BOWS, DARTS } from './equipment.js';

export const RANGED_WEAPONS = [...BOWS, ...DARTS];
export const ROCK_CRAB_RANGED_WEAPONS = RANGED_WEAPONS;

export function rangeLoadoutOf(weapon, ammo) {
    const rawWeapon = String(weapon ?? '');
    const wanted = rawWeapon.trim().toLowerCase();
    const items = host().content?.items;
    const dart = Array.isArray(items)
        ? items.find(
              (item) =>
                  item &&
                  typeof item.obj === 'string' &&
                  item.obj.endsWith('_dart') &&
                  typeof item.name === 'string' &&
                  item.name.toLowerCase() === wanted,
          )
        : undefined;
    return {
        weapon: dart ? dart.name : rawWeapon,
        projectile: dart ? dart.name : String(ammo ?? ''),
        thrown: dart !== undefined,
    };
}

export function rockCrabRangeLoadout(...args) {
    return rangeLoadoutOf(...args);
}

export function rangeSupplyEmpty() {
    throw notImpl('ranged.rangeSupplyEmpty');
}
