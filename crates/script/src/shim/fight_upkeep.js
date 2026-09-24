// Fight-loop bury/swing onto the posted inv and the posted swing-start fact.
// No AttackClock and no cross-tick JS state.
import { snap } from '../../shim/_kernel.js';
import { Inventory } from '../inventory/Inventory.js';
import { Execution } from '../execution/Execution.js';

const BURY_CONFIRM_TICKS = 3;

/**
 * True on the tick our swing animation began. Rust posts `swing_started` from
 * the local player's primary animation, so no clock is kept here.
 */
export function swingStartedThisTick() {
    return snap().swing_started === true;
}

// Why: Long fight loops block sibling tasks, so bury bones during combat
// cooldowns and report only completed burials.

/** Bury one bone from inside a fight loop. */
export async function buryOneInFight(boneName) {
    if (swingStartedThisTick()) {
        return false;
    }
    const bones = Inventory.first(boneName);
    if (!bones) {
        return false;
    }
    const before = Inventory.used();
    if (!(await bones.interact('Bury'))) {
        return false;
    }
    return Execution.delayUntilTicks(() => Inventory.used() < before, BURY_CONFIRM_TICKS);
}
