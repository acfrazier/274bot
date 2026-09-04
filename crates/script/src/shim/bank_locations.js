import Tile from '../../geometry/Tile.js';
import { snap } from '../../shim/_kernel.js';

/** Empty until host posts booths; nearestBank stays the live path. */
export const BANK_LOCATIONS = [];

/** Host-posted nearest Use-quickly booth on the player's plane. No booth → null. */
export function nearestBank(_hint) {
    const row = snap().nearest_booth;
    if (!row) {
        return null;
    }
    const name = row.name || 'Bank booth';
    const op = row.op || 'Use-quickly';
    return {
        tile: new Tile(row.x, row.z, row.level ?? 0),
        name,
        op,
        access: { name, op },
    };
}
