import { snap } from '../../shim/_kernel.js';

export const URGENT_HP_FRACTION = 0.35;

/** Whether to hold this tick and eat on the next one instead. */
export function shouldHoldEat(input) {
    const hp = input && typeof input.hpFraction === 'number' ? input.hpFraction : 1;
    const urgent = input && typeof input.urgentAt === 'number' ? input.urgentAt : URGENT_HP_FRACTION;
    if (hp <= urgent) {
        return false;
    }
    return input && input.attackedThisTick === true;
}

/**
 * Frozen `AttackClock`: the tick our attack animation began, keyed on
 * animation changes. Rust owns the change and posts it as `swing_started`, so
 * the clock keeps no state of its own: `observe` takes the frozen arguments
 * and reads nothing, and `reset` has nothing to clear.
 */
export class AttackClock {
    observe(_anim, _tick) {}

    attackedThisTick(tick) {
        return snap().swing_started === true && snap().tick === tick;
    }

    reset() {}
}
