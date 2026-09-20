// Thin marshal for `/rs2b0t/bot/paint/jive.js`. Branding, XP deltas,
// level-row formats and paintLevels room live in the Rust helper.
import { notImpl } from '../shim/_kernel.js';
import { Paint } from './Paint.js';

function jive(op, a, b, c, d, e) {
    const fn = globalThis.__rs2b0t_paint_jive;
    if (typeof fn !== 'function') {
        throw notImpl('jive.' + op);
    }
    return fn(op, a, b, c, d, e);
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

function applyFrame(cfg, opts) {
    const frame = Paint.begin(null, { dock: cfg.dock, accent: cfg.accent });
    const pages = asNames(opts && opts.pages);
    const sections = asNames(opts && opts.sections);
    const status = opts && opts.status != null ? String(opts.status) : '';
    const script = opts && opts.script != null ? String(opts.script) : '';
    const page = frame.strip(cfg.key, pages, status, script);
    const section = page === (pages[0] || '') ? frame.rail(cfg.key, sections) : '';
    frame.footer(cfg.byline);
    return { frame, page, section };
}

export const JIVE_ACCENT = jive('accent');
export const JIVE_BYLINE = jive('byline');
export const COMBAT_SKILLS = jive('combatSkills');

export function scriptFrame(ctx, opts) {
    return applyFrame(jive('scriptFrame', opts || {}), opts || {});
}

export function jiveFrame(ctx, opts) {
    return applyFrame(jive('jiveFrame', opts || {}), opts || {});
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

export function paintLevels(p, gains, mins, reserve, empty) {
    const rowsLeft = typeof p.rowsLeft === 'function' ? p.rowsLeft() : 0;
    const ops = jive('paintLevels', gains || [], mins, reserve, empty, rowsLeft);
    if (!ops) {
        return;
    }
    for (let i = 0; i < ops.length; i++) {
        const op = ops[i];
        if (!op) continue;
        if (op.kind === 'text') {
            p.text(op.text);
        } else if (op.kind === 'bar') {
            p.bar(op.label, op.fraction);
        } else if (op.kind === 'row') {
            p.row.apply(p, op.cells || []);
        }
    }
}
