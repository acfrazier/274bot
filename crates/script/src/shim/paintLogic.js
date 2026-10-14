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

export function paintCols(w, pad, charW) {
    throw notImpl('paintCols');
}

export function cellWidths(total, weights) {
    throw notImpl('cellWidths');
}

export function gridRows(len, columns) {
    throw notImpl('gridRows');
}

export function listScroll(len, rows, state, wheel, focus) {
    throw notImpl('listScroll');
}

export function paintClip(text, max = 52) {
    throw notImpl('paintClip');
}

export function gatherPaintAccent(kind) {
    throw notImpl('gatherPaintAccent');
}

export function stripSegments(w, tabWidths, brandWidth, pad) {
    throw notImpl('stripSegments');
}

export function railRows(h, count) {
    throw notImpl('railRows');
}

export function statColumns(panelW, railW, pad, columns) {
    throw notImpl('statColumns');
}

export class PaintState {
    constructor() {
        throw notImpl('PaintState');
    }

    publishRegions(regions) {
        throw notImpl('PaintState.publishRegions');
    }

    pointerDown(x, y) {
        throw notImpl('PaintState.pointerDown');
    }

    pointerMove(x, y) {
        throw notImpl('PaintState.pointerMove');
    }

    pointerIsInside(x, y) {
        throw notImpl('PaintState.pointerIsInside');
    }

    consumeClick(id) {
        throw notImpl('PaintState.consumeClick');
    }

    wheel(x, y, delta) {
        throw notImpl('PaintState.wheel');
    }

    consumeWheel(id) {
        throw notImpl('PaintState.consumeWheel');
    }

    isHovered(rect) {
        throw notImpl('PaintState.isHovered');
    }

    get(key, fallback) {
        throw notImpl('PaintState.get');
    }

    set(key, value) {
        throw notImpl('PaintState.set');
    }

    reset() {
        throw notImpl('PaintState.reset');
    }
}

export const paintState = notImplValue('paintState');
export const WHEEL_ROWS = notImplValue('WHEEL_ROWS');
