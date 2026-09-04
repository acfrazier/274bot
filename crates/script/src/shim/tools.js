import { snap, notImpl } from '../../shim/_kernel.js';
import { Inventory } from '../inventory/Inventory.js';

export const TINDERBOX = 'Tinderbox';
export const HAMMER = 'Hammer';
export const KNIFE = 'Knife';
export const CHISEL = 'Chisel';
export const NEEDLE = 'Needle';

const AXE_NAMES = [
    'Bronze axe',
    'Iron axe',
    'Steel axe',
    'Mithril axe',
    'Adamant axe',
    'Rune axe',
];

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

export function hasAllTools(available, reqs) {
    return (reqs || []).every((r) => r && r.name && available(r.name));
}

export function bestAxe(_woodcuttingLevel, available) {
    for (let i = AXE_NAMES.length - 1; i >= 0; i--) {
        if (available(AXE_NAMES[i])) return AXE_NAMES[i];
    }
    const inv = snap().inv || [];
    const hit = [...AXE_NAMES].reverse().find((n) => inv.some((r) => r && r.name === n));
    return hit || null;
}

export function canWieldTool() {
    throw notImpl('Tools.canWieldTool');
}

export function toolRestockPlan() {
    throw notImpl('Tools.toolRestockPlan');
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

export function bestPickaxe() {
    throw notImpl('Tools.bestPickaxe');
}

export function bankHasBetterGatherTool() {
    throw notImpl('Tools.bankHasBetterGatherTool');
}
