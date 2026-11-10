// Thin native binding. The camp table and its labels are the immutable Rust table (`api::gather_camps`),
// materialized once per isolate; this module copies no JS game-data table.
import Tile from '../geometry/Tile.js';
import { locationOptions, resolveGatheringLocation } from '../shim/gathering_locations.js';

const catalog = globalThis.__rs2b0t_gather_catalog;

export const MINING_LOCATIONS = catalog(1, Tile);
export const MINING_LOCATION_OPTIONS = locationOptions(MINING_LOCATIONS);
export const MINING_LOCATION_OPTION_LABELS = catalog(7, Tile);

export function miningLocationLabel(loc) {
    return catalog(20, loc);
}

export function resolveMiningLocation(setting, startTile) {
    return resolveGatheringLocation(setting, startTile, MINING_LOCATIONS);
}
