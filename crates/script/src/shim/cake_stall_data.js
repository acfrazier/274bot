import { tileFromPosted } from '../../geometry/Tile.js';
import { host, notImpl } from '../../shim/_kernel.js';

function postedRow() {
    const row = host().content && host().content.baker_stall;
    return row && typeof row === 'object' ? row : null;
}

const row = postedRow();
const cakeItems = Array.isArray(row && row.cake_items)
    ? row.cake_items.filter((name) => typeof name === 'string' && name.trim())
    : [];

export const STALL_TILE = tileFromPosted(row && row.stall);
export const STAND = tileFromPosted(row && row.stand);
export const STAND_ALT = tileFromPosted(row && row.stand_alt);
export const FLEE_TILE = tileFromPosted(row && row.flee);
export const STALL_NAME = typeof (row && row.name) === 'string' && row.name.trim() ? row.name : null;
export const STALL_OP = typeof (row && row.op) === 'string' && row.op.trim() ? row.op : null;
export const CAKE_ITEMS = cakeItems;
export const LOCKOUT_TICKS = 10;
export const RESET_AFTER_REFUSALS = 3;

export function classifySteal() {
    throw notImpl('cakeStallData.classifySteal');
}

export function shouldReset(consecutiveRefusals) {
    return consecutiveRefusals >= RESET_AFTER_REFUSALS;
}
