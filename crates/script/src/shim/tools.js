// Catalog Tools URL. Each callback-taking export is one native call
// (`__rs2b0t_tools`): Rust walks the frozen loop and calls the caller's own
// callbacks in the frozen order. Candidate tables and gates are Rust-owned.

export const TINDERBOX = 'Tinderbox';
export const HAMMER = 'Hammer';
export const KNIFE = 'Knife';
export const CHISEL = 'Chisel';
export const NEEDLE = 'Needle';

function native(op, ...args) {
    return globalThis.__rs2b0t_tools(op, ...args);
}

export const AXES = native('toolTiers', 'axes');
export const PICKAXES = native('toolTiers', 'pickaxes');

export function exactTool(name) {
    return { name: String(name) };
}

export function tinderboxReq() {
    return exactTool(TINDERBOX);
}

export function axeReq(equip = true) {
    return { kind: 'tiered', skill: 'woodcutting', tiers: AXES, label: 'axe', equip };
}

export function pickaxeReq(equip = true) {
    return { kind: 'tiered', skill: 'mining', tiers: PICKAXES, label: 'pickaxe', equip };
}

export function toolKeepNames(reqs) {
    return native('toolKeepNames', reqs || []);
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

export function missingToolLabels(reqs, skillLevel, count) {
    return native('missingToolLabels', reqs, skillLevel, count);
}

export function toolKitLabel(reqs, skillLevel, count) {
    return native('toolKitLabel', reqs, skillLevel, count);
}

export function bankHasBetterGatherTool(reqs, skillLevel, invCount, bankCount) {
    return native('bankHasBetterGatherTool', reqs, skillLevel, invCount, bankCount);
}
