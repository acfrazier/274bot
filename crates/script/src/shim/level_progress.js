// `/rs2b0t/bot/paint/levelProgress.js`: the reference experience curve, the
// per-level progress, the eta and the level row all live in the Rust helper
// (`load/paint_jive.rs`), which serves the same numbers to the Jive level
// rows. This module only marshals the arguments.
import { notImpl } from '../shim/_kernel.js';

function jive(op, a, b, c) {
    const fn = globalThis.__rs2b0t_paint_jive;
    if (typeof fn !== 'function') {
        throw notImpl('levelProgress.' + op);
    }
    return fn(op, a, b, c);
}

/** Total experience required to reach `level` (1-99). */
export function xpAtLevel(level) {
    return jive('xpAtLevel', level);
}

/** Where `xp` sits between its level and the next. */
export function levelProgress(level, xp) {
    return jive('levelProgress', level, xp);
}

/** Hours to the next level at this rate, or null when it is not moving. */
export function etaHours(remaining, xpPerHour) {
    return jive('etaHours', remaining, xpPerHour);
}

/** The bar and the line under it for one skill, from what it gained over `mins`. */
export function levelRow(g, mins) {
    return jive('levelRow', g, mins);
}
