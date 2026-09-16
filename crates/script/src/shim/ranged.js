import { host } from '../../shim/_kernel.js';
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

// JSON/serde collapses undefined and non-finite Number into null before Rust.
// Apply JS ToNumber here and pass finite numbers as-is; encode NaN/±Infinity
// as their IEEE string tags so the Rust helper still sees Number `<= 0`
// operands. Predicate decision stays in Rust.
function bridgeRelationalNumber(value) {
    const n = Number(value);
    if (Number.isFinite(n)) {
        return n;
    }
    return String(n);
}

export function rangeSupplyEmpty(equipped, carried, ground) {
    return globalThis.rustyscript.functions.__rs2b0t_range_supply_empty(
        bridgeRelationalNumber(equipped),
        bridgeRelationalNumber(carried),
        bridgeRelationalNumber(ground),
    );
}
