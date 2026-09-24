import Tile from '../geometry/Tile.js';

const selectedFacts = globalThis.__rs2b0t_selected_facts;
const facts = selectedFacts('cow-locations');

function location(row) {
    return {
        name: row.name,
        anchor: new Tile(row.x, row.z, row.level ?? 0),
        usesAlKharidToll: row.usesAlKharidToll === true,
    };
}

export const COW_LOCATIONS = facts.locations.map(location);

export const COW_LOCATION_OPTIONS = [
    'Auto',
    ...COW_LOCATIONS.map((row) => row.name),
    'Start tile',
];

export const AL_KHARID_BANK = new Tile(
    facts.alKharidBank.x,
    facts.alKharidBank.z,
    facts.alKharidBank.level ?? 0,
);

export function resolveCowLocation(setting) {
    const want = String(setting || '').toLowerCase();
    return COW_LOCATIONS.find((row) => row.name.toLowerCase() === want) || null;
}

export function nearestCowLocation() {
    const nearest = selectedFacts('cow-nearest');
    return nearest ? resolveCowLocation(nearest.name) : undefined;
}
