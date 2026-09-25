import Tile from '../../geometry/Tile.js';
import { host, runMachine } from '../../shim/_kernel.js';

/** Immutable host-resolved catalog stands. */
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

/** Air ranking is synchronous; only the reachable selector asks the worker. */
export function nearestBank(here) {
    const bank = globalThis.__rs2b0t_nearest_bank(here);
    return bank ? { ...bank, tile: Tile.from(bank.tile) } : null;
}

export async function nearestBankReachable(here, _navigator) {
    const out = await runMachine('bank_select', { from: here, allow_wilderness: true });
    const bank = out.kind === 'done' ? out.value : null;
    return bank ? { ...bank, tile: Tile.from(bank.tile) } : null;
}
