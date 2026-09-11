import Tile from '../geometry/Tile.js';
import { host } from '../shim/_kernel.js';

export const CUSTOM_LOCATION = 'Custom';
export const MAX_SURFACE_CHEB = 20;

function postedTile(p) {
    if (!p || p.x == null || p.z == null) {
        return null;
    }
    return new Tile(p.x, p.z, p.level ?? 0);
}

function bankShape(name, tile) {
    if (!name || !tile) {
        return null;
    }
    return { name, tile };
}

function surfaceFromRange(range) {
    const stand = postedTile(range);
    if (!stand) {
        return null;
    }
    return {
        stand,
        approach: null,
        locName: null,
        loc: null,
        arriveRadius: null,
    };
}

export const COOK_LOCATIONS = ((host().content && host().content.cook_stands) || [])
    .map((s) => {
        if (!s || !s.name) {
            return null;
        }
        const bank = bankShape(s.name, postedTile(s.bank));
        if (!bank) {
            return null;
        }
        return {
            name: s.name,
            bank,
            surface: surfaceFromRange(s.range),
            verified: false,
        };
    })
    .filter((row) => row !== null);

export function findCookLocation(locs, name) {
    const want = String(name).trim().toLowerCase();
    return (locs || []).find((l) => l && String(l.name).toLowerCase() === want) || null;
}

export function buildCookLocations() {
    return COOK_LOCATIONS;
}
