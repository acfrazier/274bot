import Tile from '../geometry/Tile.js';
import { host } from '../shim/_kernel.js';

export const COW_LOCATIONS = ((host().content && host().content.cow_fields) || []).map((f) => ({
    name: f.name,
    anchor: new Tile(f.x, f.z, f.level ?? 0),
    usesAlKharidToll: f.name === 'Lumbridge cow field',
}));

export const COW_LOCATION_OPTIONS = [
    'Auto',
    ...COW_LOCATIONS.map((l) => l.name),
    'Start tile',
];

export const AL_KHARID_BANK = new Tile(3269, 3167, 0);

export function resolveCowLocation(setting) {
    const want = String(setting || '').toLowerCase();
    return COW_LOCATIONS.find((l) => l.name.toLowerCase() === want) || null;
}

export function nearestCowLocation() {
    return COW_LOCATIONS[0];
}
