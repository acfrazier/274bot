import Tile from '../../geometry/Tile.js';
import { notImpl, runMachine } from '../../shim/_kernel.js';


export const USE_MAGE_BANK = 'useMageBank';
export const USE_ZANARIS_BANK = 'useZanarisBank';
export const BANK_LOCATIONS = globalThis.__rs2b0t_bank_locations(Tile);

export function approachOf(bank) { return globalThis.__rs2b0t_bank_approach(bank); }

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

export function lockedBankAt(tile) {
    return globalThis.__rs2b0t_locked_bank_at(tile, Tile);
}

/** Air ranking is synchronous; only the reachable selector asks the worker. */
export function nearestBank(here) {
    return globalThis.__rs2b0t_nearest_bank(here, Tile);
}

export function nearestBanks(here) {
    return globalThis.__rs2b0t_nearest_banks(here, Tile);
}

export function nearestUsableBank(here, usable) {
    return globalThis.__rs2b0t_nearest_usable_bank(here, usable, Tile);
}

export async function nearestBankReachable(here, _navigator) {
    const out = await runMachine('bank_select', { from: here, allow_wilderness: true });
    return globalThis.__rs2b0t_bank_materialize(out.kind === 'done' ? out.value : null, Tile);
}

export function nearestWalkableBank() {
    throw notImpl('nearestWalkableBank');
}

export async function nearestWalkableBankAsync() {
    throw notImpl('nearestWalkableBankAsync');
}

export function bankCostForFinder() {
    throw notImpl('bankCostForFinder');
}

export function bankCostForNavigator() {
    throw notImpl('bankCostForNavigator');
}

export function bankDistance(from, bank) {
    return globalThis.__rs2b0t_bank_distance(from, bank);
}
