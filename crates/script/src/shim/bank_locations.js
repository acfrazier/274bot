import Tile from '../../geometry/Tile.js';
import { host, snap } from '../../shim/_kernel.js';

/** Host-posted named stands; nearestBank stays the live path. */
export const BANK_LOCATIONS = ((host().content && host().content.named_banks) || []).map((b) => ({
    name: b.name,
    tile: new Tile(b.x, b.z, b.level ?? 0),
}));

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
