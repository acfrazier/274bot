import { host, notImpl } from '../../shim/_kernel.js';

export const TINDERBOX = 'Tinderbox';
export const HAMMER = 'Hammer';
export const KNIFE = 'Knife';
export const CHISEL = 'Chisel';
export const NEEDLE = 'Needle';

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

function meetsUse(tool, level) {
    if (tool.use_skill !== 'mining') return true;
    return Number(level) >= (tool.use_level ?? 0);
}

function bestFrom(kind, level, available) {
    if (typeof available !== 'function') return null;
    for (const tool of tools(kind)) {
        if (!meetsUse(tool, level)) continue;
        if (available(tool.name) === true) return tool.name;
    }
    return null;
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

export function hasAllTools(reqs, skillLevel, invCount) {
    if (typeof invCount === 'function') {
        return (reqs || []).every((r) => {
            if (r && r.kind === 'tiered') throw notImpl('Tools.hasAllTools');
            return r && r.name && invCount(r.name) >= (r.min ?? 1);
        });
    }
    return (reqs || []).every((r) => r && r.name && typeof skillLevel === 'function' && skillLevel(r.name));
}

export function bestAxe(level, available) {
    return bestFrom('axes', level, available);
}

export function canWieldTool(name, attack) {
    const want = String(name ?? '').trim();
    if (!want) return false;
    const tool = tools('axes').concat(tools('pickaxes')).find((t) => t.name === want);
    if (!tool) return false;
    if (tool.wield_attack == null) return true;
    return Number(attack) >= tool.wield_attack;
}

export function toolRestockPlan(reqs, skillLevel, invCount, bankCount) {
    if (!Array.isArray(reqs) || typeof invCount !== 'function' || typeof bankCount !== 'function') {
        throw notImpl('Tools.toolRestockPlan');
    }
    const plan = [];
    for (const r of reqs) {
        if (!r || r.kind === 'tiered') {
            throw notImpl('Tools.toolRestockPlan');
        }
        const name = r.name;
        if (typeof name !== 'string' || name.toLowerCase() !== 'tinderbox') {
            throw notImpl('Tools.toolRestockPlan');
        }
        const min = r.min ?? 1;
        const target = r.restock ?? min;
        const have = Number(invCount(name)) || 0;
        const need = target - have;
        if (need <= 0) continue;
        const available = Number(bankCount(name)) || 0;
        if (available <= 0) continue;
        plan.push({ name, qty: Math.min(need, available), equip: r.equip === true });
    }
    return plan;
}

export function hasToolReq(available, req) {
    return !!(req && req.name && available(req.name));
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
