import { BOWS, DARTS } from './equipment.js';

export const RANGED_WEAPONS = [...BOWS, ...DARTS];
export const ROCK_CRAB_RANGED_WEAPONS = RANGED_WEAPONS;

export function rangeLoadoutOf(weapon, ammo) {
    const fn = globalThis.__rs2b0t_selected_facts;
    if (typeof fn !== 'function') {
        return {
            weapon: String(weapon ?? ''),
            projectile: String(ammo ?? ''),
            thrown: false,
        };
    }
    return fn('range-loadout', String(weapon ?? ''), String(ammo ?? ''));
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
