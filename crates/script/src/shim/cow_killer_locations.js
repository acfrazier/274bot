import Tile from '../geometry/Tile.js';

export const COW_LOCATIONS = [
    { name: 'Lumbridge cow field', anchor: new Tile(3253, 3282, 0), usesAlKharidToll: true },
    { name: 'North-west of Lumbridge', anchor: new Tile(3162, 3311, 0), usesAlKharidToll: false },
    { name: 'South of Falador', anchor: new Tile(3029, 3305, 0), usesAlKharidToll: false },
];

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
