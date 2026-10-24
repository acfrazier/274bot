// Frozen `ClueExecutor.setTeleports`: the card's trail teleport choice is
// held by the Rust clue module, where the bank return walk reads it.
import { notImpl } from '../../../shim/_kernel.js';
import { reader } from '../../../adapter/ClientAdapter.js';
export class ClueExecutor {
    static setTeleports(enabled) {
        globalThis.rustyscript.functions.__rs2b0t_clue({ op: 'setTeleports', on: !!enabled });
    }
}

export const TRACE_STORAGE_KEY = 'rs2b0t:cluetrace';

export function tilesTo(target) {
    return globalThis.__rs2b0t_clue_tiles_to(target, reader.worldTile);
}

export function trailWalkOpts() {
    throw notImpl('trailWalkOpts');
}
