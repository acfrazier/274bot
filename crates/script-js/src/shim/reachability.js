import { proxy } from '../../../shim/_kernel.js';

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

export const Reachability = proxy('Reachability', {
    walkable(target) {
        const tile = resolveTile(target);
        if (!tile) return false;
        return globalThis.__rs2b0t_reach('walkable', tile);
    },
    canReach(target, opts = {}) {
        const tile = resolveTile(target);
        if (!tile) return false;
        return globalThis.__rs2b0t_reach('canReach', tile, opts);
    },
    lineOfSight(from, to, size) {
        const a = resolveTile(from);
        const b = resolveTile(to);
        if (!a || !b) return false;
        return globalThis.__rs2b0t_line_of_sight('v1', a, b, size === undefined ? undefined : size);
    },
    canStep(from, to) {
        const a = resolveTile(from);
        const b = resolveTile(to);
        if (!a || !b) return false;
        return globalThis.__rs2b0t_reach('canStep', a, b);
    },
});
