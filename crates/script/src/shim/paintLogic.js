// paintLogic helpers the listed scripts import from
// `../../paint/paintLogic.js`. The formats are the reference rs2b0t ones and
// live in the Rust helper (`load/paint_jive.rs`), which the Jive level rows
// use too; this module only marshals the arguments.
import { notImpl } from '../shim/_kernel.js';

function jive(op, ...args) {
    const fn = globalThis.__rs2b0t_paint_jive;
    if (typeof fn !== 'function') {
        throw notImpl('paintLogic.' + op);
    }
    return fn(op, ...args);
}

export function fmtDuration(minutes) {
    return jive('fmtDuration', minutes);
}

export function fmtXpHr(gained, mins) {
    return jive('fmtXpHr', gained, mins);
}

export function wrapText(text, cols, indent = 0) {
    return jive('wrapText', text, cols, indent);
}

export function paintSkillShort(skill) {
    return jive('paintSkillShort', skill);
}
