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

/**
 * Named posted stands only. Auto has no nearest-bank pairing or bankUnlocked
 * policy on this host, so it stays null like Custom and unknown names.
 */
export function resolveCookLocation(name) {
    const wanted = String(name || '').trim().toLowerCase();
    if (!wanted || wanted === 'auto' || wanted === CUSTOM_LOCATION.toLowerCase()) {
        return null;
    }
    return cookLocation(name);
}
