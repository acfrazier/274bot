import { snap, proxy } from '../../../shim/_kernel.js';

function resolveTile(target) {
    if (target && typeof target.tile === 'function') {
        const t = target.tile();
        return { x: t.x, z: t.z, level: t.level ?? 0 };
    }
    if (target && typeof target.x === 'number' && typeof target.z === 'number') {
        return { x: target.x, z: target.z, level: target.level ?? 0 };
    }
    return null;
}

function entityOnTile(tile) {
    const match = (row) =>
        row &&
        row.x === tile.x &&
        row.z === tile.z &&
        (row.level ?? 0) === tile.level;
    const npc = (snap().npcs || []).find(match);
    if (npc) return npc;
    return (snap().ground || []).find(match) ?? null;
}

export const Reachability = proxy('Reachability', {
    canReach(target, opts = {}) {
        const tile = resolveTile(target);
        if (!tile) return false;
        const row = entityOnTile(tile);
        if (!row) return false;
        return opts.adjacentOk ? row.reachable_adj === true : row.reachable === true;
    },
});
