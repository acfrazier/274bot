// Fight-loop bury/swing onto posted inv + anim. No AttackClock.
import { snap } from '../../shim/_kernel.js';
import { Inventory } from '../inventory/Inventory.js';

let prevTick = null;
let prevAnim = false;

export function swingStartedThisTick() {
    const s = snap();
    const tick = s.tick;
    const anim = s.animating === true;
    const started = anim && tick !== prevTick && !prevAnim;
    prevAnim = anim;
    prevTick = tick;
    return started;
}

export function buryOneInFight(boneName) {
    if (snap().animating === true) return false;
    const bone = Inventory.first(boneName);
    if (!bone) return false;
    return bone.interact('Bury');
}
