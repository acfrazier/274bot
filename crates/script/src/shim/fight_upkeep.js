// Fight-loop bury/swing onto the posted inv and anim. No AttackClock and no
// cross-tick JS state.
import { snap } from '../../shim/_kernel.js';
import { Inventory } from '../inventory/Inventory.js';

/**
 * True on the tick our swing animation began. Rust posts `swing_started` from
 * the local player's primary animation, so no clock is kept here.
 */
export function swingStartedThisTick() {
    return snap().swing_started === true;
}

export function buryOneInFight(boneName) {
    if (snap().animating === true) return false;
    const bone = Inventory.first(boneName);
    if (!bone) return false;
    return bone.interact('Bury');
}
