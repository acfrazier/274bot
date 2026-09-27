import Tile from '../geometry/Tile.js';
import { BANK_LOCATIONS } from '../api/bank/BankLocations.js';

export const CUSTOM_LOCATION = 'Custom';
export const MAX_SURFACE_CHEB = 20;

const tile = (t) => new Tile(t.x, t.z, t.level);

// Rust pairs every bank with its cook surface (frozen `buildCookLocations`);
// each row maps onto Tiles and the bank's own `BANK_LOCATIONS` entry.
export const COOK_LOCATIONS = globalThis.__rs2b0t_cook_locations().map((row) => ({
    name: row.name,
    bank: BANK_LOCATIONS[row.bank],
    surface: row.surface && {
        ...row.surface,
        stand: tile(row.surface.stand),
        ...(row.surface.approach ? { approach: tile(row.surface.approach) } : {}),
        loc: tile(row.surface.loc),
    },
    obstacles: row.obstacles,
    verified: row.verified,
}));

export function findCookLocation(locs, name) {
    const want = String(name).trim().toLowerCase();
    return (locs || []).find((l) => l && String(l.name).toLowerCase() === want) || null;
}

export function buildCookLocations() {
    return COOK_LOCATIONS;
}
