// Thin native binding. The method table, gear names and every calculation over a method's gear are Rust
// (`api::fishing_gear`, `script::fishing_gear`); the caller's count functions stay the caller's and are called
// from there. This module copies no JS game-data table.
import Tile from '../geometry/Tile.js';
import { notImplValue } from '../shim/_kernel.js';

const catalog = globalThis.__rs2b0t_gather_catalog;

export const WHIRLPOOL_IDS = catalog(10, Tile) ?? notImplValue('WHIRLPOOL_IDS');
export const FISHING_METHODS = catalog(4, Tile);
export const FISHING_METHOD_OPTIONS = catalog(8, Tile);
export const ALL_FISHING_GEAR_NAMES = catalog(9, Tile);

export function resolveFishMethod(name) {
    return catalog(21, name);
}

export function gearKeepNames(method) {
    return catalog(22, method);
}

export function hasFishingGear(method, count) {
    return catalog(24, method, count);
}

export function missingFishingGear(method, count) {
    return catalog(25, method, count);
}

export function gearLabel(method) {
    return catalog(23, method);
}

export function fishingRestockPlan(method, invCount, bankCount) {
    return catalog(26, method, invCount, bankCount);
}

export function spotMatchesMethod(actions, method) {
    return catalog(27, actions, method);
}
