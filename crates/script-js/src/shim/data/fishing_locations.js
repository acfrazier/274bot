// Thin native binding. The camp table (with each camp's cook surface joined natively) and the Shilo water vendor
// are the immutable Rust tables (`api::gather_camps`), materialized once per isolate; this module copies no JS
// game-data table.
import Tile from '../geometry/Tile.js';
import { locationOptions, resolveGatheringLocation } from '../shim/gathering_locations.js';

const catalog = globalThis.__rs2b0t_gather_catalog;

export const SHILO_WATER_VENDOR = catalog(3, Tile);
export const FISHING_LOCATIONS = catalog(2, Tile);
export const FISHING_LOCATION_OPTIONS = locationOptions(FISHING_LOCATIONS);

export function resolveFishingLocation(setting, startTile) {
    return resolveGatheringLocation(setting, startTile, FISHING_LOCATIONS);
}
