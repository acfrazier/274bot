import { host, notImpl } from '../../shim/_kernel.js';

export const TINDERBOX = 'Tinderbox';
export const HAMMER = 'Hammer';
export const KNIFE = 'Knife';
export const CHISEL = 'Chisel';
export const NEEDLE = 'Needle';

function call(payload) {
    return globalThis.rustyscript.functions.__rs2b0t_tool_step(payload);
}

function posted(kind) {
    const row = host().content && host().content.gather_tools;
    if (!row || typeof row !== 'object') return [];
    const list = row[kind];
    return Array.isArray(list) ? list : [];
}

function asTool(row) {
    if (!row || typeof row !== 'object') return null;
    if (typeof row.name !== 'string' || !row.name.trim()) return null;
    if (typeof row.id !== 'number' || !Number.isFinite(row.id)) return null;
    return row;
}

function tools(kind) {
    return posted(kind).map(asTool).filter(Boolean);
}

export const AXES = tools('axes').map((t) => ({ name: t.name, id: t.id }));
export const PICKAXES = tools('pickaxes').map((t) => ({ name: t.name, id: t.id }));

/** Posted gather-tool facts for one decision (a single read per decision). */
function facts() {
    const row = host().content && host().content.gather_tools;
    return row && typeof row === 'object' ? row : null;
}

/** JS `Number()` coercion, JSON-safe: non-finite numbers travel as tokens. */
function numArg(value) {
    const n = Number(value);
    if (Number.isNaN(n)) return 'NaN';
    if (n === Infinity) return 'Infinity';
    if (n === -Infinity) return '-Infinity';
    return n;
}

function fail(step) {
    throw notImpl(step.feature);
}

export function exactTool(name) {
    return { name: String(name) };
}

export function tinderboxReq() {
    return exactTool(TINDERBOX);
}

export function axeReq() {
    return { kind: 'axe' };
}

export function pickaxeReq() {
    return { kind: 'pickaxe' };
}

export function toolKeepNames(reqs) {
    return (reqs || []).map((r) => r && r.name).filter(Boolean);
}

/** Current shim iterated `(reqs || []).every(...)`; a non-array still throws that TypeError. */
function reqRows(reqs) {
    const rows = reqs || [];
    if (!Array.isArray(rows)) rows.every(() => true);
    return rows;
}

export function hasAllTools(reqs, skillLevel, invCount) {
    const rows = reqRows(reqs);
    let index = 0;
    if (typeof invCount === 'function') {
        let count = null;
        for (;;) {
            const step = call({ op: 'has_all', mode: 'inventory', reqs: rows, index, count });
            if (step.kind === 'error') fail(step);
            if (step.kind === 'done') return step.value;
            index = step.index;
            count = numArg(invCount(step.name));
        }
    }
    let ok = null;
    for (;;) {
        const step = call({
            op: 'has_all',
            mode: 'skill',
            reqs: rows,
            index,
            ok,
            skill_fn: typeof skillLevel === 'function',
        });
        if (step.kind === 'error') fail(step);
        if (step.kind === 'done') return step.value;
        index = step.index;
        ok = !!skillLevel(step.name);
    }
}

function bestFrom(kind, level, available) {
    if (typeof available !== 'function') return null;
    const rows = facts();
    const gate = numArg(level);
    let index = -1;
    let accepted = false;
    for (;;) {
        const step = call({ op: 'best', kind, level: gate, facts: rows, index, accepted });
        if (step.kind === 'done') return step.name;
        if (step.kind !== 'probe') return null;
        index = step.index;
        accepted = available(step.name) === true;
    }
}

export function bestAxe(level, available) {
    return bestFrom('axes', level, available);
}

export function canWieldTool(name, attack) {
    const step = call({
        op: 'can_wield',
        name: String(name ?? ''),
        attack: numArg(attack),
        facts: facts(),
    });
    return step.kind === 'value' ? step.value : false;
}

export function toolRestockPlan(reqs, skillLevel, invCount, bankCount) {
    if (!Array.isArray(reqs) || typeof invCount !== 'function' || typeof bankCount !== 'function') {
        throw notImpl('Tools.toolRestockPlan');
    }
    const plan = [];
    let index = 0;
    let count = null;
    let bank = null;
    for (;;) {
        const step = call({ op: 'restock', reqs, index, count, bank });
        if (step.kind === 'error') fail(step);
        if (step.kind === 'done') return plan;
        if (step.kind === 'emit') {
            plan.push(step.step);
            index = step.index;
            count = null;
            bank = null;
            continue;
        }
        if (step.what === 'inv') count = numArg(invCount(step.name));
        else bank = numArg(bankCount(step.name));
    }
}

export function hasToolReq(available, req) {
    const probe = call({ op: 'has_req', req });
    if (probe.kind !== 'probe') return probe.value === true;
    const step = call({ op: 'has_req', req, ok: !!available(probe.name) });
    return step.value === true;
}

export function missingToolLabels() {
    throw notImpl('Tools.missingToolLabels');
}

export function toolKitLabel() {
    throw notImpl('Tools.toolKitLabel');
}

export function bestPickaxe(level, available) {
    return bestFrom('pickaxes', level, available);
}

export function bankHasBetterGatherTool() {
    throw notImpl('Tools.bankHasBetterGatherTool');
}
