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

function fail(step) {
    throw notImpl(step.feature);
}

function resetReq(state) {
    state.present = undefined;
    state.has_kind = false;
    state.kind = null;
    state.has_name = false;
    state.name = null;
    state.ok = undefined;
    state.need_le_0 = undefined;
    state.avail_le_0 = undefined;
    state.need = undefined;
    state.available = undefined;
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
    const inventory = typeof invCount === 'function';
    const state = { index: 0 };
    resetReq(state);
    for (;;) {
        const payload = {
            op: 'has_all',
            mode: inventory ? 'inventory' : 'skill',
            index: state.index,
            req_len: rows.length,
            skill_fn: typeof skillLevel === 'function',
        };
        if (state.present !== undefined) payload.present = state.present;
        if (state.has_kind) {
            payload.has_kind = true;
            payload.kind = state.kind;
        }
        if (state.has_name) {
            payload.has_name = true;
            payload.name = state.name;
        }
        if (state.ok !== undefined) payload.ok = state.ok;
        const step = call(payload);
        if (step.kind === 'error') fail(step);
        if (step.kind === 'done') return step.value;
        if (step.kind === 'advance') {
            state.index = step.index;
            resetReq(state);
            continue;
        }
        if (step.kind === 'read') {
            const r = rows[step.index];
            if (step.what === 'present') state.present = !!r;
            else if (step.what === 'kind') {
                state.has_kind = true;
                state.kind = r.kind ?? null;
            } else if (step.what === 'name') {
                state.has_name = true;
                state.name = r.name;
            }
            continue;
        }
        if (step.kind !== 'probe') return false;
        if (step.what === 'inv') {
            const r = rows[step.index];
            state.ok = invCount(step.name) >= (r.min ?? 1);
        } else {
            state.ok = !!skillLevel(step.name);
        }
    }
}

function bestFrom(kind, level, available) {
    if (typeof available !== 'function') return null;
    const snapshot = tools(kind);
    let index = -1;
    let accepted = false;
    let row = null;
    let rowIndex = null;
    let levelOk = undefined;
    let pending = null;
    for (;;) {
        const payload = {
            op: 'best',
            index,
            accepted,
            has_next: index + 1 < snapshot.length,
        };
        if (row) {
            payload.row = row;
            payload.row_index = rowIndex;
        }
        if (levelOk !== undefined) payload.level_ok = levelOk;
        const step = call(payload);
        if (step.kind === 'need_row') {
            pending = snapshot[step.index];
            row = pending
                ? { name: pending.name, use_skill: pending.use_skill ?? null }
                : null;
            rowIndex = step.index;
            levelOk = undefined;
            continue;
        }
        if (step.kind === 'need_level') {
            levelOk = Number(level) >= (pending.use_level ?? 0);
            continue;
        }
        if (step.kind === 'skip') {
            index = step.index;
            accepted = false;
            row = null;
            rowIndex = null;
            levelOk = undefined;
            pending = null;
            continue;
        }
        if (step.kind === 'done') return step.name;
        if (step.kind !== 'probe') return null;
        index = step.index;
        accepted = available(step.name) === true;
        if (!accepted) {
            row = null;
            rowIndex = null;
            levelOk = undefined;
            pending = null;
        }
    }
}

export function bestAxe(level, available) {
    return bestFrom('axes', level, available);
}

export function canWieldTool(name, attack) {
    const want = String(name ?? '').trim();
    if (!want) return false;
    const snapshot = tools('axes').concat(tools('pickaxes'));
    const names = snapshot.map((t) => t.name);
    let wield = undefined;
    let attackOk = undefined;
    for (;;) {
        const payload = { op: 'can_wield', name: want, names };
        if (wield !== undefined) payload.wield = wield;
        if (attackOk !== undefined) payload.attack_ok = attackOk;
        const step = call(payload);
        if (step.kind === 'value') return step.value;
        if (step.kind === 'need_wield') {
            const tool = snapshot[step.index];
            wield = tool && tool.wield_attack != null ? tool.wield_attack : null;
            continue;
        }
        if (step.kind === 'need_attack') {
            attackOk = Number(attack) >= wield;
            continue;
        }
        return false;
    }
}

export function toolRestockPlan(reqs, skillLevel, invCount, bankCount) {
    if (!Array.isArray(reqs) || typeof invCount !== 'function' || typeof bankCount !== 'function') {
        throw notImpl('Tools.toolRestockPlan');
    }
    const plan = [];
    const state = { index: 0 };
    resetReq(state);
    for (;;) {
        const payload = { op: 'restock', index: state.index, req_len: reqs.length };
        if (state.present !== undefined) payload.present = state.present;
        if (state.has_kind) {
            payload.has_kind = true;
            payload.kind = state.kind;
        }
        if (state.has_name) {
            payload.has_name = true;
            payload.name = state.name;
        }
        if (state.need_le_0 !== undefined) payload.need_le_0 = state.need_le_0;
        if (state.avail_le_0 !== undefined) payload.avail_le_0 = state.avail_le_0;
        const step = call(payload);
        if (step.kind === 'error') fail(step);
        if (step.kind === 'done') return plan;
        if (step.kind === 'advance') {
            state.index = step.index;
            resetReq(state);
            continue;
        }
        if (step.kind === 'read') {
            const r = reqs[step.index];
            if (step.what === 'present') state.present = !!r;
            else if (step.what === 'kind') {
                state.has_kind = true;
                state.kind = r.kind ?? null;
            } else if (step.what === 'name') {
                state.has_name = true;
                state.name = r.name;
            }
            continue;
        }
        if (step.kind === 'emit') {
            const r = reqs[step.index];
            plan.push({
                name: step.name,
                qty: Math.min(state.need, state.available),
                equip: r.equip === true,
            });
            state.index = step.index + 1;
            resetReq(state);
            continue;
        }
        if (step.kind !== 'probe') throw notImpl('Tools.toolRestockPlan');
        const r = reqs[step.index];
        if (step.what === 'inv') {
            const min = r.min ?? 1;
            const target = r.restock ?? min;
            const have = Number(invCount(step.name)) || 0;
            state.need = target - have;
            state.need_le_0 = state.need <= 0;
        } else {
            state.available = Number(bankCount(step.name)) || 0;
            state.avail_le_0 = state.available <= 0;
        }
    }
}

export function hasToolReq(available, req) {
    let present;
    let hasName = false;
    let name = null;
    let ok;
    for (;;) {
        const payload = { op: 'has_req' };
        if (present !== undefined) payload.present = present;
        if (hasName) {
            payload.has_name = true;
            payload.name = name;
        }
        if (ok !== undefined) payload.ok = ok;
        const step = call(payload);
        if (step.kind === 'done') return step.value === true;
        if (step.kind === 'read') {
            if (step.what === 'present') present = !!req;
            else if (step.what === 'name') {
                hasName = true;
                name = req.name;
            }
            continue;
        }
        if (step.kind !== 'probe') return false;
        ok = !!available(step.name);
    }
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
