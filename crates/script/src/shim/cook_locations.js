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
 * `Custom`, unknown and locked names yield null; `Auto` takes the nearest
 * bank this account can open. Rust decides; a caller's `unlocked` gets the
 * `CookLocation` for each index Rust asks about.
 */
export function resolveCookLocation(setting, from, unlocked) {
    const index = globalThis.__rs2b0t_resolve_cook_location(
        String(setting ?? ''),
        from,
        unlocked === undefined ? undefined : (i) => unlocked(COOK_LOCATIONS[i]),
    );
    return index === null ? null : COOK_LOCATIONS[index];
}
