import { queue, proxy, snap, arrived } from '../../shim/_kernel.js';
import { Execution } from '../../api/execution/Execution.js';

// Same-scene walk click. `walk-to` is the scene packet; Traveller is `walk`.
// walkTo keeps the awaited scene-walk/wait with DirectNavigator positional
// defaults (radius 2, timeoutMs 45000). Explicit 0/10000 stay.
// Traversal.walkTo is the world route; do not share that mapping here.
// Do not copy the foreign retry/clamp/controller.
export const DirectNavigator = proxy('DirectNavigator', {
    walk(dest) {
        if (!dest || typeof dest.x !== 'number' || typeof dest.z !== 'number') return false;
        queue({
            op: 'walk-to',
            x: dest.x,
            z: dest.z,
            level: dest.level ?? 0,
        });
        return true;
    },
    async walkTo(dest, radius = 2, timeoutMs = 45000) {
        if (!dest || typeof dest.x !== 'number' || typeof dest.z !== 'number') return false;
        if (!snap().here) return false;
        const target = { x: dest.x, z: dest.z, level: dest.level ?? 0 };
        if (arrived(target, radius)) return true;
        queue({
            op: 'walk-to',
            x: target.x,
            z: target.z,
            level: target.level,
        });
        return Execution.delayUntil(() => arrived(target, radius), timeoutMs);
    },
});
