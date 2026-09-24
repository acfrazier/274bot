// Thin marshal for `/rs2b0t/bot/paint/jive.js`. Branding, XP deltas,
// level-row formats, the frame plan (strip, rail on the first page, byline)
// and the paintLevels room live in the Rust helper; this module only applies
// the paint calls it returns.
import { notImpl } from '../shim/_kernel.js';
import { Paint } from './Paint.js';

function jive(op, a, b, c, d, e) {
    const fn = globalThis.__rs2b0t_paint_jive;
    if (typeof fn !== 'function') {
        throw notImpl('jive.' + op);
    }
    return fn(op, a, b, c, d, e);
}

export const JIVE_ACCENT = jive('accent');
export const JIVE_BYLINE = jive('byline');
export const COMBAT_SKILLS = jive('combatSkills');

/**
 * Begin the frame, then apply the plan's paint calls in order. The plan is
 * asked for after `Paint.begin`, because the strip spends a dock row and
 * begin resets the budget.
 */
function applyFrame(cfg) {
    const frame = Paint.begin(null, { dock: cfg.dock, accent: cfg.accent });
    const plan = jive('frame', cfg);
    for (let i = 0; i < plan.ops.length; i++) {
        const op = plan.ops[i];
        if (op) frame[op.m].apply(frame, op.a);
    }
    return { frame, page: plan.page, section: plan.section };
}

export function scriptFrame(ctx, opts) {
    return applyFrame(jive('scriptFrame', opts || {}));
}

export function jiveFrame(ctx, opts) {
    return applyFrame(jive('jiveFrame', opts || {}));
}

export class XpTracker {
    constructor(skills, read) {
        this.skills = asNames(skills);
        this.read = read;
        this.id = jive('construct');
    }

    begin() {
        const xps = this.skills.map((skill) => this.read.xp(skill));
        jive('begin', this.id, this.skills, xps);
    }

    progress() {
        const xps = this.skills.map((skill) => this.read.xp(skill));
        const levels = this.skills.map((skill) => this.read.level(skill));
        return jive('progress', this.id, this.skills, xps, levels);
    }

    gains() {
        const xps = this.skills.map((skill) => this.read.xp(skill));
        const levels = this.skills.map((skill) => this.read.level(skill));
        return jive('gains', this.id, this.skills, xps, levels);
    }
}

function asNames(names) {
    if (!names) {
        return [];
    }
    const out = [];
    for (let i = 0; i < names.length; i++) {
        out.push(String(names[i]));
    }
    return out;
}

export function paintLevels(p, gains, mins, reserve, empty) {
    const rowsLeft = typeof p.rowsLeft === 'function' ? p.rowsLeft() : 0;
    const ops = jive('paintLevels', gains || [], mins, reserve, empty, rowsLeft);
    if (!ops) {
        return;
    }
    for (let i = 0; i < ops.length; i++) {
        const op = ops[i];
        if (op) p[op.m].apply(p, op.a);
    }
}
