import { snap, queue, proxy, chebyshev } from '../../shim/_kernel.js';
import { Execution } from '../execution/Execution.js';

export const Traversal = proxy('Traversal', {
    async walkResilient(tile, opts = {}) {
        const radius = opts.radius ?? 0;
        const here = snap().here;
        if (!here) return false;
        const target = { x: tile.x, z: tile.z, level: tile.level ?? 0 };
        if (chebyshev(here, target) <= radius) return true;
        queue({ op: 'walk', x: target.x, z: target.z, level: target.level });
        return Execution.delayUntil(() => {
            const h = snap().here;
            return h && chebyshev(h, target) <= radius;
        }, opts.timeoutMs ?? 60_000);
    },
});
