import { notImpl } from '../../shim/_kernel.js';

export const URGENT_HP_FRACTION = 0.35;

export function shouldHoldEat() {
    throw notImpl('eatTiming.shouldHoldEat');
}

export class AttackClock {
    constructor() {
        throw notImpl('eatTiming.AttackClock');
    }
}
