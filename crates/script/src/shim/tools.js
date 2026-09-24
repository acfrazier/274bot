// Catalog Tools URL. Each callback-taking export is one native call
// (`__rs2b0t_tools`): Rust walks the frozen loop and calls the caller's own
// callbacks in the frozen order. Candidate tables and gates are Rust-owned.
import { host, notImpl } from '../../shim/_kernel.js';

export const TINDERBOX = 'Tinderbox';
export const HAMMER = 'Hammer';
export const KNIFE = 'Knife';
export const CHISEL = 'Chisel';
export const NEEDLE = 'Needle';

function native(op, ...args) {
    return globalThis.__rs2b0t_tools(op, ...args);
}

function posted(kind) {
    const row = host().content && host().content.gather_tools;
    const list = row && typeof row === 'object' ? row[kind] : null;
    return Array.isArray(list) ? list : [];
}

export const AXES = posted('axes').map((t) => ({ name: t.name, id: t.id }));
export const PICKAXES = posted('pickaxes').map((t) => ({ name: t.name, id: t.id }));

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

export function hasToolReq(req, skillLevel, count) {
    return native('hasToolReq', req, skillLevel, count);
}

export function hasAllTools(reqs, skillLevel, count) {
    return native('hasAllTools', reqs, skillLevel, count);
}

export function bestAxe(level, available) {
    return native('bestAxe', level, available);
}

export function bestPickaxe(level, available) {
    return native('bestPickaxe', level, available);
}

export function bestFromTiers(level, tiers, available) {
    return native('bestFromTiers', level, tiers, available);
}

export function canWieldTool(name, attack) {
    return native('canWieldTool', name, attack);
}

export function toolRestockPlan(reqs, skillLevel, invCount, bankCount) {
    return native('toolRestockPlan', reqs, skillLevel, invCount, bankCount);
}

export function missingToolLabels() {
    throw notImpl('Tools.missingToolLabels');
}

export function toolKitLabel() {
    throw notImpl('Tools.toolKitLabel');
}

export function bankHasBetterGatherTool() {
    throw notImpl('Tools.bankHasBetterGatherTool');
}
