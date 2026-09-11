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

// Posted step mask bit i matches native DIRS: W E N S NW NE SW SE.
function stepDirBit(dx, dz) {
    if (dx === -1 && dz === 0) return 0;
    if (dx === 1 && dz === 0) return 1;
    if (dx === 0 && dz === -1) return 2;
    if (dx === 0 && dz === 1) return 3;
    if (dx === -1 && dz === -1) return 4;
    if (dx === 1 && dz === -1) return 5;
    if (dx === -1 && dz === 1) return 6;
    if (dx === 1 && dz === 1) return 7;
    return -1;
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
    canStep(from, to) {
        const a = resolveTile(from);
        const b = resolveTile(to);
        if (!a || !b) return false;
        if (a.level !== b.level) return false;
        const bit = stepDirBit(b.x - a.x, b.z - a.z);
        if (bit < 0) return false;
        const reach = reachView();
        if (!reach) return false;
        if (a.level !== reach.level) return false;
        const lx = a.x - reach.base_x;
        const lz = a.z - reach.base_z;
        if (lx < 0 || lz < 0 || lx >= reach.width || lz >= reach.height) return false;
        const masks = reach.step;
        if (!masks) return false;
        const mask = masks[lx * reach.height + lz];
        if (mask == null) return false;
        return ((mask >>> bit) & 1) === 1;
    },
});
