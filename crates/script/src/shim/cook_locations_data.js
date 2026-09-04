import Tile from '../geometry/Tile.js';
import { host } from '../shim/_kernel.js';

export const CUSTOM_LOCATION = 'Custom';
export const MAX_SURFACE_CHEB = 20;

export const COOK_LOCATIONS = ((host().content && host().content.cook_stands) || []).map((s) => ({
    name: s.name,
    bank: new Tile(s.bank.x, s.bank.z, s.bank.level ?? 0),
    range: new Tile(s.range.x, s.range.z, s.range.level ?? 0),
}));

export function findCookLocation(locs, name) {
    const want = String(name).toLowerCase();
    return (locs || []).find((l) => l && String(l.name).toLowerCase() === want) || null;
}

export function buildCookLocations() {
    return COOK_LOCATIONS;
}
