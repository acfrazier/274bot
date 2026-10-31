import { proxy } from '../../shim/_kernel.js';
import { reader } from '../../adapter/ClientAdapter.js';
import { Navigator } from './Navigator.js';

const walkApi = globalThis.rustyscript.functions.__rs2b0t_walk_api;

function tileOrNull(tile) {
    if (!tile || typeof tile !== 'object') return null;
    const level = tile.level ?? 0;
    if (!Number.isInteger(tile.x) || !Number.isInteger(tile.z) || !Number.isInteger(level)) {
        return null;
    }
    return { x: tile.x, z: tile.z, level };
}

export const WalkExecutor = proxy('WalkExecutor', {
    get lastOutcome() {
        return walkApi('last-outcome');
    },
    async probeDest(dest, maxExpansions) {
        const from = tileOrNull(reader.worldTile());
        if (!Number.isSafeInteger(maxExpansions) || maxExpansions <= 0) {
            return { ok: false, terminal: null };
        }
        const to = tileOrNull(dest);
        if (!from || !to) return { ok: false, terminal: null };
        const route = await Navigator.findPath(from, to, {
            maxExpansions,
            timeoutMs: 30_000,
        });
        const ok = route?.ok === true;
        return { ok, terminal: ok ? route.terminal ?? null : null };
    },
});
