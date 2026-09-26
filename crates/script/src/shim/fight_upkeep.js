// Fight-loop bury/swing: the frozen module `AttackClock` lives in Rust
// (`attack_clock.rs`), keyed on the posted local-player animation id.
import { runMachine } from '../../shim/_kernel.js';

/** True on the tick our swing animation began (frozen `swingStartedThisTick`). */
export function swingStartedThisTick() {
    return globalThis.__rs2b0t_attack_clock('fight');
}

/** One `fight-bury` machine: Rust gates, buries and confirms the burial. */
export async function buryOneInFight(boneName) {
    const out = await runMachine('fight-bury', { boneName: String(boneName) });
    return out.kind === 'done' && out.value === true;
}
