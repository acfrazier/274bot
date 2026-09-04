import Tile from '../../geometry/Tile.js';
import { notImpl } from '../../shim/_kernel.js';

// Ardougne walk pin; stall name is the baker loc, not a stolen-goods planner.
export const STALL_TILE = new Tile(2661, 3301, 0);
export const STAND = new Tile(2661, 3301, 0);
export const STAND_ALT = new Tile(2661, 3301, 0);
export const FLEE_TILE = new Tile(2661, 3301, 0);
export const STALL_NAME = "Baker's stall";
export const STALL_OP = 'Steal from';
export const CAKE_ITEMS = ['Cake', 'Bread', 'Chocolate slice'];
export const LOCKOUT_TICKS = 10;
export const RESET_AFTER_REFUSALS = 3;

export function classifySteal() {
    throw notImpl('cakeStallData.classifySteal');
}

export function shouldReset(consecutiveRefusals) {
    return consecutiveRefusals >= RESET_AFTER_REFUSALS;
}
