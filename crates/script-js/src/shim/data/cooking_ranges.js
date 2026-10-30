// Thin native binding. The fish-camp cook plans are the immutable Rust table (`api::cook_locations`) and the
// nearest-range search runs over the selected game data's placed ranges; the first call registers the Tile class.
import Tile from '../geometry/Tile.js';

const catalog = globalThis.__rs2b0t_gather_catalog;
catalog(13, Tile);

export function cookSurfaceForFishCamp(campName, role = 'pier') {
    return catalog(40, campName, role);
}

export function resolveFishCampCookSurface(campName, spot, maxCheb = 64, role = 'pier') {
    return catalog(41, campName, spot, maxCheb, role);
}
