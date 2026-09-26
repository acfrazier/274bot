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
 * animation-id changes. Rust holds each instance's clock by slot; `observe`
 * reads the posted local-player animation (every frozen caller passes
 * `reader.selfAnim()`).
 */
export class AttackClock {
    #slot = globalThis.__rs2b0t_attack_clock('new');

    observe(_anim, tick) {
        globalThis.__rs2b0t_attack_clock('observe', this.#slot, tick);
    }

    attackedThisTick(tick) {
        return globalThis.__rs2b0t_attack_clock('attacked', this.#slot, tick);
    }

    reset() {
        globalThis.__rs2b0t_attack_clock('reset', this.#slot);
    }
}
