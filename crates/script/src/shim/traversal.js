import { snap, queue, proxy, chebyshev, notV1 } from '../../shim/_kernel.js';
import { Execution } from '../execution/Execution.js';

function allowTeleports(opts) {
    return opts.useTeleportCatalog === true || opts.policy?.useTeleports === true;
}

function arrival(tile, opts) {
    const radius = opts.radius ?? 0;
    const here = snap().here;
    if (!here) return { here: null, target: null, radius, arrived: false };
    const target = { x: tile.x, z: tile.z, level: tile.level ?? 0 };
    return {
        here,
        target,
        radius,
        arrived: chebyshev(here, target) <= radius,
    };
}

export const Traversal = proxy('Traversal', {
    async walkTo(tile, opts = {}) {
        const { target, radius, arrived } = arrival(tile, opts);
        if (!target) return false;
        if (arrived) return true;
        queue({ op: 'walk-to', x: target.x, z: target.z, level: target.level });
        return Execution.delayUntil(() => {
            const h = snap().here;
            return h && chebyshev(h, target) <= radius;
        }, opts.timeoutMs ?? 60_000);
    },
    async walkResilient(tile, opts = {}) {
        const { target, radius, arrived } = arrival(tile, opts);
        if (!target) return false;
        if (arrived) return true;
        queue({
            op: 'walk',
            x: target.x,
            z: target.z,
            level: target.level,
            allow_teleports: allowTeleports(opts),
        });
        return Execution.delayUntil(() => {
            const h = snap().here;
            return h && chebyshev(h, target) <= radius;
        }, opts.timeoutMs ?? 60_000);
    },
    preload() {
        throw notV1('Traversal.preload');
    },
    remaining() {
        throw notV1('Traversal.remaining');
    },
    teleportsEnabled() {
        throw notV1('Traversal.teleportsEnabled');
    },
    requestRepath(_reason) {
        throw notV1('Traversal.requestRepath');
    },
    get pureWalk() {
        return { useTeleportCatalog: false, policy: { useTeleports: false } };
    },
    get withTeles() {
        return { useTeleportCatalog: true, policy: { useTeleports: true } };
    },
});
