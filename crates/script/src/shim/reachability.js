import { snap, proxy } from '../../../shim/_kernel.js';

function finiteInt(n) {
    return typeof n === 'number' && Number.isInteger(n);
}

function resolveTile(target) {
    if (target && typeof target.tile === 'function') {
        const t = target.tile();
        if (!t || !finiteInt(t.x) || !finiteInt(t.z)) return null;
        const level = t.level == null ? 0 : t.level;
        if (!finiteInt(level)) return null;
        return { x: t.x, z: t.z, level };
    }
    if (target && finiteInt(target.x) && finiteInt(target.z)) {
        const level = target.level == null ? 0 : target.level;
        if (!finiteInt(level)) return null;
        return { x: target.x, z: target.z, level };
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

function reachView() {
    const reach = snap().reach;
    if (!reach || reach.available !== true) return null;
    return reach;
}

function bitAt(words, reach, tile) {
    if (!words || tile.level !== reach.level) return false;
    const lx = tile.x - reach.base_x;
    const lz = tile.z - reach.base_z;
    if (lx < 0 || lz < 0 || lx >= reach.width || lz >= reach.height) return false;
    const i = lx * reach.height + lz;
    const word = words[(i / 32) | 0];
    if (word == null) return false;
    return ((word >>> (i & 31)) & 1) === 1;
}

export const Reachability = proxy('Reachability', {
    walkable(target) {
        const tile = resolveTile(target);
        if (!tile) return false;
        const reach = reachView();
        if (!reach) return false;
        return bitAt(reach.walkable, reach, tile);
    },
    canReach(target, opts = {}) {
        const tile = resolveTile(target);
        if (!tile) return false;
        const row = entityOnTile(tile);
        if (row) {
            return opts.adjacentOk ? row.reachable_adj === true : row.reachable === true;
        }
        const reach = reachView();
        if (!reach) return false;
        const words = opts.adjacentOk ? reach.reachable_adj : reach.reachable;
        return bitAt(words, reach, tile);
    },
});
