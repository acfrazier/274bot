// Fight-loop bury/swing onto posted inv + anim. No AttackClock.
import { snap } from '../../shim/_kernel.js';
import { Inventory } from '../inventory/Inventory.js';

export function swingStartedThisTick() {
    return snap().animating === true;
}

export function buryOneInFight(boneName) {
    if (snap().animating === true) return false;
    const bone = Inventory.first(boneName);
    if (!bone) return false;
    return bone.interact('Bury');
}
