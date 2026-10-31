import Tile from '../../geometry/Tile.js';

const [WALK_DESTINATIONS, WALK_OPTIONS] = globalThis.__rs2b0t_walk_catalog(Tile);
const walkApi = globalThis.rustyscript.functions.__rs2b0t_walk_api;

export { WALK_DESTINATIONS, WALK_OPTIONS };

export function resolveDestination(name) {
    const index = walkApi('resolve-destination', name);
    return Number.isInteger(index) ? WALK_DESTINATIONS[index] ?? null : null;
}
