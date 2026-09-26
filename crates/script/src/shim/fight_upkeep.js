// Fight-loop bury/swing onto the posted inv and anim. No AttackClock and no
// cross-tick JS state.
import { runMachine, snap } from '../../shim/_kernel.js';

/**
 * True on the tick our swing animation began. Rust posts `swing_started` from
 * the local player's primary animation, so no clock is kept here.
 */
export function swingStartedThisTick() {
    return snap().swing_started === true;
}

/** One `fight-bury` machine: Rust gates, buries and confirms the burial. */
export async function buryOneInFight(boneName) {
    const out = await runMachine('fight-bury', { boneName: String(boneName) });
    return out.kind === 'done' && out.value === true;
}
