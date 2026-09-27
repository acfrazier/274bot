import Tile from '../../geometry/Tile.js';
import { runMachine } from '../../shim/_kernel.js';

function tileBank(bank) {
    return bank ? {
        ...bank,
        tile: Tile.from(bank.tile),
        ...(bank.approach ? { approach: Tile.from(bank.approach) } : {}),
    } : null;
}

export const USE_MAGE_BANK = 'useMageBank';
export const USE_ZANARIS_BANK = 'useZanarisBank';
export const BANK_LOCATIONS = globalThis.__rs2b0t_bank_locations().map(tileBank);

export function approachOf(bank) { return bank.approach ?? bank.tile; }

/** Eligibility is evaluated against the native observed account facts. */
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
    return globalThis.__rs2b0t_bank_unlocked({
        name,
        x: tile.x,
        z: tile.z,
        level,
    });
}

/** Air ranking is synchronous; only the reachable selector asks the worker. */
export function nearestBank(here) {
    return tileBank(globalThis.__rs2b0t_nearest_bank(here));
}

export function nearestBanks(here) {
    return globalThis.__rs2b0t_nearest_banks(here).map(tileBank);
}

export function nearestUsableBank(here, usable) {
    return tileBank(globalThis.__rs2b0t_nearest_usable_bank(here, bank => usable(tileBank(bank))));
}

export async function nearestBankReachable(here, _navigator) {
    const out = await runMachine('bank_select', { from: here, allow_wilderness: true });
    return tileBank(out.kind === 'done' ? out.value : null);
}
