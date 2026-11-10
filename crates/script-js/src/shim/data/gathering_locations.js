// Thin native binding. The option values, radius defaults and radius resolvers are `script::gather_locations`;
// the shared location helpers are the ones the gather location shim already exports.
export {
    DEFAULT_BOOTH_NAME,
    DEFAULT_BOOTH_OP,
    MAP_SQUARE,
    sameMapSquare,
    locationOptions,
    boothFields,
    resolveGatheringLocation,
} from '../shim/gathering_locations.js';

const native = globalThis.__rs2b0t_gathering_locations;

export const USE_CLOSEST = native(7);
export const USE_START_POSITION = native(8);
export const USE_CUSTOM_POSITION = native(9);
export const AUTO_LEGACY = native(10);
export const NONE_LEGACY = native(11);
export const DEFAULT_CAMP_RADIUS = native(12);
export const DEFAULT_CHASE_RADIUS = native(13);

export function resolveCampRadius(campRadius, fallback = DEFAULT_CAMP_RADIUS) {
    return native(14, campRadius, fallback);
}

export function resolveChaseRadius(chaseRadius, fallback = DEFAULT_CHASE_RADIUS) {
    return native(15, chaseRadius, fallback);
}
