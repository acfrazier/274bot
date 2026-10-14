// paintLogic helpers the listed scripts import from
// `../../paint/paintLogic.js`. The formats are the reference rs2b0t ones and
// live in the Rust helper (`load/paint_jive.rs`), which the Jive level rows
// use too; this module only marshals the arguments.
import { notImpl, notImplValue } from '../shim/_kernel.js';

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

export function resolveDock(dock) {
    throw notImpl('resolveDock');
}

export function toCanvasPoint(clientX, clientY, rect) {
    throw notImpl('toCanvasPoint');
}

export function cycleOption(options, current, delta) {
    throw notImpl('cycleOption');
}

export function hitRegion(regions, x, y) {
    throw notImpl('hitRegion');
}

export function clipText(cols, text) {
    throw notImpl('clipText');
}

export const paintCols = notImplValue('paintCols');
export const cellWidths = notImplValue('cellWidths');
export const gridRows = notImplValue('gridRows');
export const listScroll = notImplValue('listScroll');
export const paintClip = notImplValue('paintClip');
export const gatherPaintAccent = notImplValue('gatherPaintAccent');
export const stripSegments = notImplValue('stripSegments');
export const railRows = notImplValue('railRows');
export const statColumns = notImplValue('statColumns');
export class PaintState {
    constructor() {
        throw notImpl('PaintState');
    }
}
export const paintState = notImplValue('paintState');
export const WHEEL_ROWS = notImplValue('WHEEL_ROWS');
