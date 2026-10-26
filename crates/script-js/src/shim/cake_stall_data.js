import { tileFromPosted } from '../../geometry/Tile.js';
import { host } from '../../shim/_kernel.js';

function postedRow() {
    const row = host().content && host().content.baker_stall;
    return row && typeof row === 'object' ? row : null;
}

const row = postedRow();
const data = globalThis.__rs2b0t_cake_stall('data');

export const STALL_TILE = tileFromPosted(row && row.stall);
export const STAND = tileFromPosted(row && row.stand);
export const STAND_ALT = tileFromPosted(row && row.stand_alt);
export const FLEE_TILE = tileFromPosted(row && row.flee);
export const STALL_NAME = typeof (row && row.name) === 'string' && row.name.trim() ? row.name : null;
export const STALL_OP = typeof (row && row.op) === 'string' && row.op.trim() ? row.op : null;
export const CAKE_ITEMS = data.cakeItems;
export const LOCKOUT_TICKS = data.lockoutTicks;
export const RESET_AFTER_REFUSALS = data.resetAfterRefusals;

export function classifySteal(signals) {
    return globalThis.__rs2b0t_cake_stall('classify_steal', signals);
}

export function shouldReset(consecutiveRefusals) {
    return globalThis.__rs2b0t_cake_stall('should_reset', consecutiveRefusals);
}
