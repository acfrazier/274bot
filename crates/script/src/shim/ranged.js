import { notImpl } from '../../shim/_kernel.js';
import { BOWS, DARTS } from './equipment.js';

export const RANGED_WEAPONS = [...BOWS, ...DARTS];
export const ROCK_CRAB_RANGED_WEAPONS = RANGED_WEAPONS;

export function rangeLoadoutOf() {
    throw notImpl('ranged.rangeLoadoutOf');
}

export function rockCrabRangeLoadout() {
    throw notImpl('ranged.rockCrabRangeLoadout');
}

export function rangeSupplyEmpty() {
    throw notImpl('ranged.rangeSupplyEmpty');
}
