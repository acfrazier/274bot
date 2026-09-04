import Tile from '../geometry/Tile.js';

export const CUSTOM_LOCATION = 'Custom';
export const MAX_SURFACE_CHEB = 20;

export const COOK_LOCATIONS = [
    {
        name: 'Catherby',
        bank: new Tile(2809, 3441, 0),
        range: new Tile(2817, 3443, 0),
    },
];

export function findCookLocation(locs, name) {
    const want = String(name).toLowerCase();
    return (locs || []).find((l) => l && String(l.name).toLowerCase() === want) || null;
}

export function buildCookLocations() {
    return COOK_LOCATIONS;
}
