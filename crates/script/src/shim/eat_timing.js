export const URGENT_HP_FRACTION = 0.35;

export function shouldHoldEat(input) {
    const hp = input && typeof input.hpFraction === 'number' ? input.hpFraction : 1;
    const urgent = input && typeof input.urgentAt === 'number' ? input.urgentAt : URGENT_HP_FRACTION;
    if (hp <= urgent) {
        return false;
    }
    return input && input.attackedThisTick === true;
}

export class AttackClock {
    constructor() {
        this.lastAnim = -1;
        this.startedTick = -1;
    }

    observe(anim, tick) {
        if (anim !== this.lastAnim) {
            this.lastAnim = anim;
            if (anim !== -1) {
                this.startedTick = tick;
            }
        }
    }

    attackedThisTick(tick) {
        return this.startedTick === tick;
    }

    reset() {
        this.lastAnim = -1;
        this.startedTick = -1;
    }
}
