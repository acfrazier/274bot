import Tile from '../../geometry/Tile.js';
import { host, snap } from '../../shim/_kernel.js';

/** Host-posted named stands; nearestBank stays the live path. */
export const BANK_LOCATIONS = ((host().content && host().content.named_banks) || []).map((b) => ({
    name: b.name,
    tile: new Tile(b.x, b.z, b.level ?? 0),
}));

/** Published unrestricted alias rows only; identity must match posted facts. */
export function bankUnlocked(bank) {
    if (!bank || typeof bank !== 'object') {
        return false;
    }
    const name = bank.name;
    const tile = bank.tile;
    if (typeof name !== 'string' || !tile || typeof tile !== 'object') {
        return false;
    }
    const level = tile.level ?? 0;
    if (
        typeof tile.x !== 'number'
        || typeof tile.z !== 'number'
        || !Number.isFinite(tile.x)
        || !Number.isFinite(tile.z)
    ) {
        return false;
    }
    return globalThis.rustyscript.functions.__rs2b0t_bank_unlocked({
        name,
        x: tile.x,
        z: tile.z,
        level,
    });
}

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
