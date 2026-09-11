import { queue, proxy } from '../../shim/_kernel.js';
import { Traversal } from '../../api/walking/Traversal.js';

// Same-scene walk click. `walk-to` is the scene packet; Traveller is `walk`.
// walkTo is the awaited Traversal scene-walk/wait with DirectNavigator
// positional defaults (radius 2, timeoutMs 45000). Explicit 0/10000 stay.
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
        return Traversal.walkTo(dest, { radius, timeoutMs });
    },
});
