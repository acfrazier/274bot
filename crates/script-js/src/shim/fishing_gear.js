// Fishing-owned binding for the fishing members of `api/acquisition/ToolAcquire.ts`. The acquisition family owns
// that module; it re-exports these. The price table, vendors and plans are Rust (`api::fishing_gear`,
// `script::fishing_gear`); the caller's count functions stay the caller's and are called from there.
import Tile from '../geometry/Tile.js';

const catalog = globalThis.__rs2b0t_gather_catalog;

export const FISHING_SHOP_COSTS = catalog(11, Tile);
export const GERRANT_ONLY_FISHING = catalog(12, Tile);
export const GERRANT_VENDOR = catalog(5, Tile);
export const HARRY_VENDOR = catalog(6, Tile);

export function fishingVendorFor(name, near) {
    return catalog(29, name, near);
}

export function fishingShopCost(name) {
    return catalog(28, name);
}

export function isFishingBaitPiece(g) {
    return catalog(30, g);
}

export function withBaitTarget(method, baitQty) {
    return catalog(31, method, baitQty);
}

export function planFishingGearBuys(method, w, opts = {}) {
    return catalog(32, method, w, opts);
}

export function planFishingGearAcquire(method, w, opts = {}) {
    return catalog(33, method, w, opts);
}

export function buyPlansCost(plans) {
    return catalog(34, plans);
}

export function fishingGearShopCart(method, w, opts = {}) {
    return catalog(35, method, w, opts);
}

export function shopableMissingFishingGear(gear, count) {
    return catalog(36, gear, count);
}
