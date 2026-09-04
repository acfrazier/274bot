import {
    CUSTOM_LOCATION,
    COOK_LOCATIONS,
    findCookLocation,
} from '../../data/cookLocations.js';

export { CUSTOM_LOCATION, COOK_LOCATIONS };

export const COOK_LOCATION_OPTIONS = ['Auto', ...COOK_LOCATIONS.map((l) => l.name), CUSTOM_LOCATION];

export function cookLocation(name) {
    return findCookLocation(COOK_LOCATIONS, name);
}

export function resolveCookLocation(name) {
    if (!name || name === 'Auto' || name === CUSTOM_LOCATION) return null;
    return cookLocation(name);
}
